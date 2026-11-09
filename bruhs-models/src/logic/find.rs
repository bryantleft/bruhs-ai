//! Case-insensitive search over labeled texts.
//!
//! Offsets are Unicode scalar indexes, so a match never splits a character.
//! An empty query matches nothing. Matches do not overlap.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextHit {
    pub source_id: String,
    /// Inclusive start, as a scalar index into that source.
    pub start: usize,
    /// Exclusive end, as a scalar index.
    pub end: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffHit {
    pub path: String,
    /// Flattened hunk-line index. `None` when the match is in the path.
    pub line: Option<usize>,
    pub start: usize,
    pub end: usize,
}

/// Hits for `query` across `(id, text)` pairs, in source order.
pub fn search_texts(query: &str, sources: &[(&str, &str)]) -> Vec<TextHit> {
    if query.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for (id, text) in sources {
        for (start, end) in spans(text, query) {
            hits.push(TextHit {
                source_id: (*id).to_string(),
                start,
                end,
            });
        }
    }
    hits
}

/// Hits in file paths, then in each file's hunk lines, in the order given.
///
/// `lines` is the flattened hunk text. Hidden unmodified lines are absent
/// here, so they are not searched.
pub fn search_diff(query: &str, files: &[(&str, &[&str])]) -> Vec<DiffHit> {
    if query.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for (path, lines) in files {
        for (start, end) in spans(path, query) {
            hits.push(DiffHit {
                path: (*path).to_string(),
                line: None,
                start,
                end,
            });
        }
        for (line_index, line) in lines.iter().enumerate() {
            for (start, end) in spans(line, query) {
                hits.push(DiffHit {
                    path: (*path).to_string(),
                    line: Some(line_index),
                    start,
                    end,
                });
            }
        }
    }
    hits
}

/// The next index, wrapping. An empty list stays at 0.
pub fn cycle(len: usize, index: usize, forward: bool) -> usize {
    if len == 0 {
        return 0;
    }
    let index = index % len;
    if forward {
        (index + 1) % len
    } else {
        (index + len - 1) % len
    }
}

fn spans(text: &str, query: &str) -> Vec<(usize, usize)> {
    let haystack: Vec<char> = text.chars().collect();
    let needle: Vec<char> = query.chars().collect();
    if needle.is_empty() || needle.len() > haystack.len() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i + needle.len() <= haystack.len() {
        let matched = haystack[i..i + needle.len()]
            .iter()
            .zip(&needle)
            .all(|(hay, needle_char)| same_letter(*hay, *needle_char));
        if matched {
            out.push((i, i + needle.len()));
            i += needle.len();
        } else {
            i += 1;
        }
    }
    out
}

fn same_letter(hay: char, needle: char) -> bool {
    if hay == needle {
        return true;
    }
    let hay: Vec<char> = hay.to_lowercase().collect();
    let needle: Vec<char> = needle.to_lowercase().collect();
    hay == needle
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_matches_nothing() {
        assert!(search_texts("", &[("a", "hello")]).is_empty());
        assert!(search_diff("", &[("src/a.rs", &["let x = 1"][..])]).is_empty());
    }

    #[test]
    fn search_is_case_insensitive_and_keeps_source_order() {
        let hits = search_texts("Rust", &[("m1", "rust and"), ("m2", "no"), ("m3", "RUST")]);
        assert_eq!(
            hits,
            vec![
                TextHit {
                    source_id: "m1".into(),
                    start: 0,
                    end: 4
                },
                TextHit {
                    source_id: "m3".into(),
                    start: 0,
                    end: 4
                },
            ]
        );
    }

    #[test]
    fn matches_do_not_overlap() {
        let hits = search_texts("aa", &[("m", "aaaa")]);
        assert_eq!(
            hits,
            vec![
                TextHit {
                    source_id: "m".into(),
                    start: 0,
                    end: 2
                },
                TextHit {
                    source_id: "m".into(),
                    start: 2,
                    end: 4
                },
            ]
        );
    }

    #[test]
    fn cycle_wraps_both_ways() {
        assert_eq!(cycle(0, 3, true), 0);
        assert_eq!(cycle(3, 2, true), 0);
        assert_eq!(cycle(3, 0, false), 2);
    }

    #[test]
    fn diff_search_checks_the_path_then_each_line() {
        let lines = ["fn untouched() {}", "fn renamed() {}"];
        let hits = search_diff("renam", &[("src/renamed.rs", &lines[..])]);
        assert_eq!(hits[0].path, "src/renamed.rs");
        assert_eq!(hits[0].line, None);
        assert_eq!(hits[1].line, Some(1));
        assert_eq!(hits[1].start, 3);
    }

    #[test]
    fn scalar_indexes_do_not_split_a_character() {
        let hits = search_texts("é", &[("m", "café")]);
        assert_eq!(hits[0].start, 3);
        assert_eq!(hits[0].end, 4);
    }
}
