/// The existing lowercase-substring semantics, including contextual Unicode casing.
// Keep the ASCII flags at their pre-fragment offsets; native scan controls
// measure this layout alongside the Unicode rejection path.
#[repr(C)]
pub(super) struct LowercaseQuery {
    text: String,
    ascii: bool,
    has_i: bool,
    has_k: bool,
    ascii_sources: bool,
    ascii_fragment: std::ops::Range<u16>,
}

impl LowercaseQuery {
    pub(super) fn matcher(&self) -> Matcher<'_> {
        Matcher {
            query: self,
            normalized: String::new(),
        }
    }
    pub(super) fn new(query: &str) -> Self {
        let text = query.to_lowercase();
        let ascii = text.is_ascii();
        let ascii_fragment = if ascii {
            0..0
        } else {
            Self::ascii_fragment_range(&text)
        };
        let probe = if ascii {
            &text
        } else {
            &text[usize::from(ascii_fragment.start)..usize::from(ascii_fragment.end)]
        };
        let has_i = probe.contains('i');
        let has_k = probe.contains('k');
        Self {
            ascii,
            has_i,
            has_k,
            ascii_sources: ascii && (has_i || has_k),
            ascii_fragment,
            text,
        }
    }

    fn ascii_fragment_range(text: &str) -> std::ops::Range<u16> {
        text.split(|ch: char| !ch.is_ascii())
            .max_by_key(|fragment| fragment.len())
            .filter(|fragment| !fragment.is_empty())
            .and_then(|fragment| {
                let start = text.find(fragment)?;
                let end = start.checked_add(fragment.len())?;
                Some(u16::try_from(start).ok()?..u16::try_from(end).ok()?)
            })
            // Public query bounds fit this compact span. Larger internal queries
            // keep the original path if their optional span cannot be represented.
            .unwrap_or(0..0)
    }

    pub(super) fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    fn matches_fields_with(&self, fields: &[&str], output: &mut String) -> bool {
        if self.is_empty() {
            return true;
        }
        // Choose once for the list instead of testing query flags for each field.
        if self.ascii_sources {
            fields
                .iter()
                .any(|field| self.matches_ascii_sources(field, output))
        } else if self.ascii {
            fields.iter().any(|field| self.matches_ascii(field))
        } else {
            fields
                .iter()
                .any(|field| !field.is_ascii() && self.matches_unicode(field))
        }
    }

    // Paired release prototypes require inlining here to retain ASCII throughput.
    #[inline(always)]
    fn matches_with(&self, field: &str, output: &mut String) -> bool {
        if self.is_empty() {
            return true;
        }
        if self.ascii_sources {
            self.matches_ascii_sources(field, output)
        } else if self.ascii {
            self.matches_ascii(field)
        } else {
            !field.is_ascii() && self.matches_unicode(field)
        }
    }

    #[inline(always)]
    fn matches_ascii_sources(&self, field: &str, output: &mut String) -> bool {
        // The mapping test below guards this bound on ASCII match lengths.
        if field.len() < self.text.len() {
            return false;
        }
        let unicode = !field.is_ascii();
        // The test below checks these ASCII-producing sources against all of std's mappings.
        let dotted_i = unicode && self.has_i && field.contains('İ');
        let kelvin = unicode && self.has_k && field.contains('K');
        // A relevant Unicode source already proves a one-byte needle is present.
        if self.text.len() == 1 && (dotted_i || kelvin) {
            return true;
        }
        if dotted_i || kelvin {
            Self::normalize_ascii_sources(field, output);
            self.matches_ascii(output)
        } else {
            self.matches_ascii(field)
        }
    }

    #[inline(never)]
    fn matches_unicode(&self, field: &str) -> bool {
        if !self.ascii_fragment.is_empty() {
            let fragment = &self.text
                [usize::from(self.ascii_fragment.start)..usize::from(self.ascii_fragment.end)];
            // A whole-query match requires its ASCII fragment. Relevant Unicode
            // sources defer to std instead of allocating a normalized probe.
            let ambiguous =
                (self.has_i && field.contains('İ')) || (self.has_k && field.contains('K'));
            if !ambiguous && !field.contains(fragment) && !Self::matches_ascii_text(field, fragment)
            {
                return false;
            }
        }
        field.to_lowercase().contains(&self.text)
    }

    #[inline(always)]
    fn matches_ascii(&self, field: &str) -> bool {
        Self::matches_ascii_text(field, &self.text)
    }

    #[inline(always)]
    fn matches_ascii_text(field: &str, text: &str) -> bool {
        let needle = text.as_bytes();
        let hay = field.as_bytes();
        let Some(last_start) = hay.len().checked_sub(needle.len()) else {
            return false;
        };
        let mut previous_end = 0;
        for start in memchr::memchr2_iter(
            needle[0],
            needle[0].to_ascii_uppercase(),
            &hay[..=last_start],
        ) {
            let end = start + needle.len();
            if hay[start..end].eq_ignore_ascii_case(needle) {
                return true;
            }
            // Overlapping failed windows could repeatedly compare the same bytes.
            // Resume the built-in search after the candidate just ruled out.
            // An ASCII candidate and start + 1 are UTF-8 boundaries.
            if start < previous_end {
                return Self::matches_ascii_overlap(&field[start + 1..], text);
            }
            previous_end = end;
        }
        false
    }

    // Keep the overlapping-window fallback out of ordinary scan prologues.
    #[inline(never)]
    fn matches_ascii_overlap(field: &str, text: &str) -> bool {
        field.to_ascii_lowercase().contains(text)
    }

    // Other Unicode casing cannot change containment of an ASCII needle.
    #[inline(never)]
    fn normalize_ascii_sources(field: &str, output: &mut String) {
        let bytes = field.as_bytes();
        let dotted = "İ".as_bytes();
        let kelvin = "K".as_bytes();
        // Reuse capacity only within this matcher; later fields must overwrite
        // every previous byte before the normalized text is searched.
        output.clear();
        output.reserve(field.len());
        let mut copied = 0;
        // Search final bytes, then verify the complete UTF-8 encodings.
        for end in memchr::memchr2_iter(dotted[1], kelvin[2], bytes) {
            let (start, replacement) = if end >= 1 && &bytes[end - 1..=end] == dotted {
                (end - 1, "i\u{307}")
            } else if end >= 2 && &bytes[end - 2..=end] == kelvin {
                (end - 2, "k")
            } else {
                continue;
            };
            debug_assert!(field.is_char_boundary(start) && field.is_char_boundary(end + 1));
            output.push_str(&field[copied..start]);
            output.push_str(replacement);
            copied = end + 1;
        }
        output.push_str(&field[copied..]);
    }
}

/// Scratch belongs to one read call and is released with its matcher.
/// Keeping it separate preserves the immutable query layout.
pub(super) struct Matcher<'q> {
    query: &'q LowercaseQuery,
    normalized: String,
}
impl Matcher<'_> {
    // Preserve the direct matcher's inline boundary through this mutable facade.
    #[inline(always)]
    pub(super) fn matches(&mut self, field: &str) -> bool {
        self.query.matches_with(field, &mut self.normalized)
    }
    pub(super) fn matches_fields(&mut self, fields: &[&str]) -> bool {
        self.query.matches_fields_with(fields, &mut self.normalized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reused_matcher_preserves_search_across_changing_unicode_fields() {
        let large = "İ".repeat(100_000);
        let fields = [
            "İ",
            "",
            "I",
            "K",
            "xİx",
            "ΟΣ Café",
            &large,
            "İ K",
            "plain",
            "i\u{307} k",
        ];
        for text in [
            "",
            "i",
            "k",
            "missing",
            "i\u{307} k",
            "İ K",
            "ς",
            "ζmetadata",
        ] {
            let query = LowercaseQuery::new(text);
            let mut matcher = query.matcher();
            let lower = text.to_lowercase();
            for _ in 0..3 {
                for field in fields {
                    let expected = field.to_lowercase().contains(&lower);
                    assert_eq!(matcher.matches(field), expected, "{text:?}, {field:?}");
                    assert_eq!(
                        matcher.matches_fields(&["", field, ""]),
                        text.is_empty() || expected,
                        "{text:?}, {field:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn repeated_normalization_replaces_previous_search_bytes() {
        let query = LowercaseQuery::new("ki");
        let mut matcher = query.matcher();
        for field in ["KI", "İ", "Kİ", "xKxİ", "İKI", "xK", "K", "KI"] {
            assert_eq!(
                matcher.matches(field),
                field.to_lowercase().contains("ki"),
                "{field:?}"
            );
        }
    }

    #[test]
    fn empty_ascii_and_single_alias_paths_keep_scratch_unallocated() {
        for text in ["", "quartz", "i", "k"] {
            let query = LowercaseQuery::new(text);
            let mut matcher = query.matcher();
            for field in ["plain", "İ", "K", "ASCII quartz", "ΟΣ Café İ K", ""] {
                assert_eq!(matcher.matches(field), field.to_lowercase().contains(text));
            }
            assert_eq!(matcher.normalized.capacity(), 0, "{text:?}");
        }
    }

    #[test]
    fn matcher_scratch_is_not_shared_between_reads() {
        let query = LowercaseQuery::new("missing");
        let mut first = query.matcher();
        let second = query.matcher();
        assert!(first.matches("İ missing"));
        assert!(first.normalized.capacity() > 0);
        assert_eq!(second.normalized.capacity(), 0);
    }

    fn check(field: &str, query: &str) {
        let expected = field.to_lowercase().contains(&query.to_lowercase());
        let actual = LowercaseQuery::new(query);
        assert_eq!(
            actual.matcher().matches(field),
            expected,
            "{field:?}, {query:?}"
        );
        assert_eq!(
            actual.matcher().matches_fields(&["", field]),
            query.is_empty() || expected
        );
    }

    #[test]
    fn preserves_contextual_unicode_expansion_and_literal_substrings() {
        for (field, query, expected) in [
            ("Merchant 100000", "mErChAnT 100000", true),
            ("", "", true),
            ("", "x", false),
            ("ΟΣ", "Σ", false),
            ("ΟΣ", "ς", true),
            ("ΟΣΑ", "Σ", true),
            ("İ", "i", true),
            ("I", "İ", false),
            ("K", "K", true),
            ("K", "k", true),
            ("ſ", "s", false),
            ("ẞ", "ß", true),
            ("ß", "ss", false),
            ("Café", "cafe\u{301}", false),
            ("👩🏽‍💻 café", "🏽‍", true),
            (" A ", " a ", true),
            ("a", " a", false),
            ("\0AB\0", "\0ab", true),
        ] {
            assert_eq!(
                LowercaseQuery::new(query).matcher().matches(field),
                expected
            );
            check(field, query);
        }
        assert!(LowercaseQuery::new("").matcher().matches_fields(&[]));
        assert!(!LowercaseQuery::new("x").matcher().matches_fields(&[]));
    }

    #[test]
    fn all_ascii_bytes_and_substrings_match_the_original_search() {
        let field: String = (0..=127_u8).map(char::from).collect();
        for start in 0..=field.len() {
            for end in start..=field.len() {
                check(&field, &field[start..end]);
                check(&field, &field[start..end].to_uppercase());
            }
        }
    }

    #[test]
    fn mixed_text_matches_the_original_search() {
        let units = [
            "a", "A", "b", "B", "0", "-", "\0", " ", "é", "E", "İ", "K", "Σ", "ς", "\u{301}", "👩",
            "🏽", "\u{200d}", "💻",
        ];
        let mut state = 17_usize;
        for sample in 0..10_000 {
            let mut field = String::new();
            for _ in 0..sample % 80 {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                field.push_str(units[state % units.len()]);
            }
            let mut bounds: Vec<_> = field.char_indices().map(|(index, _)| index).collect();
            bounds.push(field.len());
            let left = state % bounds.len();
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let right = state % bounds.len();
            let mut query = field[bounds[left.min(right)]..bounds[left.max(right)]].to_string();
            if sample % 2 == 0 {
                query = query.to_uppercase();
            }
            check(&field, &query);
            query.push('!');
            check(&field, &query);
        }
    }

    #[test]
    fn overlapping_and_long_prefixes_keep_late_matches() {
        for query in [
            "a".repeat(511) + "b",
            "a".repeat(255) + "b" + &"a".repeat(256),
        ] {
            check(&"a".repeat(100_000), &query);
            check(&("a".repeat(100_000) + "B"), &query);
        }
        for ending in ["aAABa", "aaabb"] {
            check(&("abX--".repeat(4000) + ending), "aba");
        }
    }
    #[test]
    fn unicode_ascii_sources_and_length_bound_match_the_standard_library() {
        let mut sources = Vec::new();
        for code in 0..=u32::from(char::MAX) {
            let Some(ch) = char::from_u32(code) else {
                continue;
            };
            let lowercase = ch.to_lowercase();
            assert!(
                lowercase.clone().next().is_some(),
                "empty lowercase for {ch:?}"
            );
            let ascii_count = lowercase.clone().filter(char::is_ascii).count();
            assert!(
                ascii_count <= ch.len_utf8(),
                "ASCII length bound changed for {ch:?}"
            );
            if !ch.is_ascii() && ascii_count != 0 {
                sources.push((ch, lowercase.collect::<String>()));
            }
        }
        assert_eq!(
            sources,
            [('İ', String::from("i\u{307}")), ('K', String::from("k"))],
            "Unicode {:?} changed; review ASCII query fallbacks and length bound",
            std::char::UNICODE_VERSION
        );
    }

    #[test]
    fn unicode_ascii_queries_keep_expansions_context_and_literal_boundaries() {
        for field in [
            "",
            "İ",
            "K",
            "I",
            "K",
            "Aİ",
            "İA",
            "abKcd",
            "abİcd",
            "İK",
            "Kİ",
            "ΟΣ",
            "ΟΣΑ",
            "aΣ'a",
            "ſẞﬃＡ",
            "Café e\u{301} 東京 👩🏽‍💻",
            "\0İ\\\"K\0",
        ] {
            for query in [
                "",
                "i",
                "k",
                "ik",
                "ki",
                "ai",
                "ia",
                "abkcd",
                "abicd",
                "İ",
                "i\u{307}",
                "Σ",
                "ς",
                "ΟΣ",
                "ss",
                "ffi",
                "a",
                "CAFÉ",
                "cafe\u{301}",
                "\0",
                "\\",
            ] {
                check(field, query);
            }
        }
        for query in [
            "a".repeat(511) + "i",
            "i".repeat(511) + "k",
            "a".repeat(511) + "b",
        ] {
            for field in [
                "İ".to_string() + &"a".repeat(10_000),
                "a".repeat(10_000) + "İ",
                "i".repeat(10_000) + "K",
                "ΟΣ ".repeat(1_000) + &"a".repeat(10_000) + "B",
            ] {
                check(&field, &query);
            }
        }
    }

    #[test]
    fn ascii_queries_match_reference_across_generated_multiscript_fields() {
        let units = [
            "Alpha",
            "Ω",
            "Σ",
            "ς",
            "İ",
            "K",
            "ß",
            "Café",
            "e\u{301}",
            "東京",
            "Ж",
            "𐐀",
            "👩🏽‍💻",
            "'",
            "\\",
            "\0",
            " ",
            "i",
            "K",
            "ss",
            "Ａ",
            "ſ",
            "ﬃ",
        ];
        let queries = [
            "alpha",
            "ALPHA",
            "i",
            "k",
            "ik",
            "ki",
            "abc",
            "ss",
            "\\",
            "'",
            "\0",
            " ",
            "ab",
            "id",
            "links",
            "missing-kelvin",
            "lastquartz",
        ];
        let mut state = 31_usize;
        for sample in 0..4_000 {
            let mut field = String::new();
            for _ in 0..sample % 80 {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                field.push_str(units[state % units.len()]);
            }
            for query in queries {
                check(&field, query);
            }
        }
    }

    #[test]
    fn unicode_source_scanning_rejects_other_utf8_suffixes() {
        let queries = ["ai", "ak", "ik", "ab"].map(|text| (text, LowercaseQuery::new(text)));
        for code in 0..=u32::from(char::MAX) {
            let Some(ch) = char::from_u32(code) else {
                continue;
            };
            let mut encoded = [0; 4];
            let bytes = ch.encode_utf8(&mut encoded).as_bytes();
            if !bytes.iter().any(|byte| matches!(byte, 0xb0 | 0xaa)) {
                continue;
            }
            let field = format!("A{ch}BİK");
            let expected = field.to_lowercase();
            for (text, query) in &queries {
                assert_eq!(
                    query.matcher().matches(&field),
                    expected.contains(text),
                    "{ch:?}, {text}"
                );
            }
        }
    }

    #[test]
    fn mixed_unicode_queries_keep_ascii_expansions_and_contextual_casing() {
        for field in [
            "ΟΣ Café İ K 👩🏽‍💻 context links",
            "İ Ké AKB ai\u{307}B",
            "ΟΣΑ Σ's 東京context",
            "é context links",
            "\0 é \\\"İK\0",
            "e\u{301} ΑΣ\u{301}a ",
        ] {
            for query in [
                "ζ-unseen",
                "CAFÉ",
                "İ K",
                "ΟΣ Café",
                "ζ",
                "ζcontext",
                "ζlinks",
                "ζ-missing-kelvin",
                "i\u{307} k",
                "ké",
                "akb",
                "ai\u{307}b",
                "écontext",
                "\0é",
                "é \\\"i\u{307}k\0",
                "Σ\u{301}A",
                "東京CONTEXT",
            ] {
                check(field, query);
            }
        }
    }

    #[test]
    fn mixed_unicode_queries_keep_overlapping_fragments_and_literal_boundaries() {
        for ending in ["BΟΣ", "Bος", "BİK", "B東京"] {
            let field = "a".repeat(100_000) + ending;
            for query in [
                "a".repeat(511) + "bΟΣ",
                "a".repeat(511) + "bİK",
                "a".repeat(511) + "b東京",
                "a".repeat(511) + "c東京",
                "a".repeat(255) + "b" + &"a".repeat(256) + "ς",
            ] {
                check(&field, &query);
            }
        }
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn query_metadata_keeps_the_original_state_size_and_alignment() {
        assert_eq!(
            std::mem::size_of::<LowercaseQuery>(),
            std::mem::size_of::<(String, bool, bool, bool)>(),
        );
        assert_eq!(
            std::mem::align_of::<LowercaseQuery>(),
            std::mem::align_of::<(String, bool, bool, bool)>(),
        );
    }

    #[test]
    fn unrepresentable_optional_fragments_keep_full_query_semantics() {
        for query in ["a".repeat(70_000) + "Σ", "Σ".repeat(40_000) + " Last"] {
            check(&query, &query);
            check(&(query.clone() + "!"), &query);
            check(&query, &(query.clone() + "!"));
        }
    }
}
