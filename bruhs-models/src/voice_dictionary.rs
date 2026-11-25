//! Preferred spellings for composer dictation. Project entries take precedence.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceDictionary(pub BTreeMap<String, Vec<String>>);

impl VoiceDictionary {
    pub fn add(&mut self, input: &str, alias_for: Option<&str>) -> Result<(), String> {
        let words: Vec<_> = input
            .split(',')
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .collect();
        if words.is_empty() {
            return Err("Enter a word or phrase.".into());
        }
        if words
            .iter()
            .any(|v| v.chars().count() > 50 || v.chars().any(char::is_control))
        {
            return Err("Each entry must be a single line of up to 50 characters.".into());
        }
        let mut next = self.clone();
        if let Some(term) = alias_for {
            let aliases = next
                .0
                .get_mut(term)
                .ok_or("Choose a dictionary entry first.")?;
            for alias in words {
                if !aliases.iter().any(|word| word.eq_ignore_ascii_case(alias))
                    && !term.eq_ignore_ascii_case(alias)
                {
                    aliases.push(alias.into());
                }
            }
            if aliases.len() > 8 {
                return Err("Each word can have up to eight aliases.".into());
            }
        } else {
            for term in words {
                next.0.entry(term.into()).or_default();
            }
            if next.0.len() > 100 {
                return Err("A dictionary can contain up to 100 entries.".into());
            }
        }
        *self = next;
        Ok(())
    }

    /// Highest-priority scope first. Aliases reserve spellings but never leave as hints.
    pub fn preferred_terms(scopes: &[&Self], hints: &[&str]) -> Vec<String> {
        use std::collections::HashSet;
        use unicode_normalization::UnicodeNormalization as _;
        let key = |s: &str| s.trim().nfc().collect::<String>().to_lowercase();
        let known: HashSet<_> = scopes
            .iter()
            .flat_map(|scope| &scope.0)
            .flat_map(|(term, aliases)| std::iter::once(term).chain(aliases))
            .map(|word| key(word))
            .collect();
        let mut used = HashSet::new();
        let mut terms = Vec::new();
        for hint in hints {
            let term = hint.split_whitespace().collect::<Vec<_>>().join(" ");
            let id = key(&term);
            if !term.is_empty()
                && term.encode_utf16().count() <= 50
                && !known.contains(&id)
                && used.insert(id)
            {
                terms.push(term);
            }
        }
        for scope in scopes {
            for (term, aliases) in &scope.0 {
                if used.insert(key(term)) {
                    terms.push(term.clone());
                    used.extend(aliases.iter().map(|alias| key(alias)));
                }
            }
        }
        terms.truncate(100);
        terms
    }

    pub fn apply(&self, project: Option<&Self>, text: &str) -> String {
        Self::apply_scopes(std::iter::once(self).chain(project), text)
    }
    pub fn apply_scopes<'a>(scopes: impl IntoIterator<Item = &'a Self>, text: &str) -> String {
        let mut spellings = BTreeMap::new();
        for dictionary in scopes {
            for (term, aliases) in &dictionary.0 {
                for spelling in aliases.iter().chain(std::iter::once(term)) {
                    if !spelling.is_empty() {
                        spellings.insert(spelling.to_lowercase(), term.as_str());
                    }
                }
            }
        }
        if spellings.is_empty() {
            return text.to_owned();
        }
        let mut words: Vec<(&str, &str)> = spellings
            .iter()
            .map(|(spelling, &term)| (spelling.as_str(), term))
            .collect();
        words.sort_by_key(|(s, _)| std::cmp::Reverse(s.len()));
        let mut out = String::with_capacity(text.len());
        let mut pos = 0usize;
        let mut prev_is_word = false;
        while pos < text.len() {
            if !prev_is_word {
                let mut matched = None;
                for &(needle, replacement) in &words {
                    if let Some(len) = match_ci_prefix(&text[pos..], needle) {
                        let end = pos + len;
                        if !text[end..].chars().next().is_some_and(is_word_char) {
                            matched = Some((end, replacement));
                            break;
                        }
                    }
                }
                if let Some((end, replacement)) = matched {
                    out.push_str(replacement);
                    prev_is_word = text[pos..end].chars().next_back().is_some_and(is_word_char);
                    pos = end;
                    continue;
                }
            }
            let Some(ch) = text[pos..].chars().next() else {
                break;
            };
            out.push(ch);
            prev_is_word = is_word_char(ch);
            pos += ch.len_utf8();
        }
        out
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// If `text` begins with `needle_lower` (compared case-insensitively on Unicode
/// scalar values), return the matched byte length in `text`.
fn match_ci_prefix(text: &str, needle_lower: &str) -> Option<usize> {
    let mut target = needle_lower.chars();
    let mut next_target = target.next()?;
    for (idx, ch) in text.char_indices() {
        for lc in ch.to_lowercase() {
            if lc != next_target {
                return None;
            }
            match target.next() {
                Some(n) => next_target = n,
                None => {
                    // Ensure the lowercased expansion of `ch` has no extra trailing chars.
                    let mut check = ch.to_lowercase();
                    let consumed = check.by_ref().position(|c| c == lc).unwrap_or(0);
                    let remaining = ch.to_lowercase().skip(consumed + 1).count();
                    return (remaining == 0).then_some(idx + ch.len_utf8());
                }
            }
        }
    }
    None
}

fn match_cue(rest: &str, cue_words: &[String], prev_is_word: bool) -> Option<usize> {
    let first = cue_words.first()?;
    let first_starts_word = first.chars().next().is_some_and(is_word_char);
    if first_starts_word && prev_is_word {
        return None;
    }
    let mut pos = match_ci_prefix(rest, first)?;
    for word in &cue_words[1..] {
        let mut ws_len = 0usize;
        for ch in rest[pos..].chars() {
            if ch.is_whitespace() {
                ws_len += ch.len_utf8();
            } else {
                break;
            }
        }
        if ws_len == 0 {
            return None;
        }
        pos += ws_len;
        let len = match_ci_prefix(&rest[pos..], word)?;
        pos += len;
    }
    let last_ends_word = cue_words
        .last()
        .and_then(|w| w.chars().next_back())
        .is_some_and(is_word_char);
    if last_ends_word && rest[pos..].chars().next().is_some_and(is_word_char) {
        return None;
    }
    Some(pos)
}

/// Remove standalone speech hesitations while retaining dictated paragraphs, code and the cleanup cue.
pub fn without_fillers(text: &str, language: Option<&str>, correction: &str) -> String {
    let mut fillers: Vec<&'static str> = vec![
        "uhhh", "uhh", "uhm", "umm", "uh", "ehh", "ehm", "ahm", "hmm", "hm", "mmm", "хм", "ммм",
    ];
    match language.and_then(|code| code.split(['-', '_']).next()) {
        Some("en") => fillers.extend_from_slice(&["um", "ah", "eh"]),
        Some("fr") => fillers.push("euh"),
        Some("de") => fillers.extend_from_slice(&["ähm", "äh"]),
        _ => {}
    }
    fillers.sort_by_key(|f| std::cmp::Reverse(f.len()));

    let cue_words: Vec<String> = correction
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();

    let mut filtered = String::with_capacity(text.len());
    let mut pos = 0usize;
    let mut prev_is_word = false;
    while pos < text.len() {
        if !cue_words.is_empty() {
            if let Some(len) = match_cue(&text[pos..], &cue_words, prev_is_word) {
                let end = pos + len;
                filtered.push_str(&text[pos..end]);
                prev_is_word = text[pos..end].chars().next_back().is_some_and(is_word_char);
                pos = end;
                continue;
            }
        }
        if !prev_is_word {
            let mut matched_filler = None;
            for &filler in &fillers {
                if let Some(len) = match_ci_prefix(&text[pos..], filler) {
                    let mut end = pos + len;
                    if !text[end..].chars().next().is_some_and(is_word_char) {
                        if text[end..].starts_with([',', '.']) {
                            end += 1;
                        }
                        matched_filler = Some(end);
                        break;
                    }
                }
            }
            if let Some(end) = matched_filler {
                if text[..pos].ends_with('-') || text[end..].starts_with('-') {
                    filtered.push_str(&text[pos..end]);
                }
                prev_is_word = text[pos..end].chars().next_back().is_some_and(is_word_char);
                pos = end;
                continue;
            }
        }
        let Some(ch) = text[pos..].chars().next() else {
            break;
        };
        filtered.push(ch);
        prev_is_word = is_word_char(ch);
        pos += ch.len_utf8();
    }
    filtered
        .trim()
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filler_filter_honors_language_correction_phrases_and_paragraphs() {
        assert_eq!(
            without_fillers(
                "Euh, bonjour hmm.\n\nEuh attends, euh salut",
                Some("fr"),
                "euh attends"
            ),
            "bonjour\n\nEuh attends, salut"
        );
        assert_eq!(
            without_fillers("Ähm, der Albumtitel ist äh gut.", Some("de"), ""),
            "der Albumtitel ist gut."
        );
        assert_eq!(
            without_fillers("um, um actually, uh-huh", Some("en"), "um actually"),
            "um actually, uh-huh"
        );
        assert_eq!(
            without_fillers("Euh, ah, um, hmm.", None, ""),
            "Euh, ah, um,"
        );
    }

    #[test]
    fn cloud_vocabulary_reserves_project_terms_and_limits_the_combined_list() {
        let mut personal = VoiceDictionary::default();
        for index in 0..100 {
            personal.add(&format!("Personal {index}"), None).unwrap();
        }
        let mut project = VoiceDictionary::default();
        project.add("Montréal", None).unwrap();
        project.add("Montreal", Some("Montréal")).unwrap();
        let words = VoiceDictionary::preferred_terms(
            &[&project, &personal],
            &["Montre\u{301}al", "montreal", "Project Name"],
        );
        assert_eq!(&words[..2], ["Project Name", "Montréal"]);
        assert_eq!(words.len(), 100);
    }

    #[test]
    fn project_aliases_override_device_spellings_without_rewriting_substrings() {
        let mut device = VoiceDictionary::default();
        device.add("Bruhs, C++, New York", None).unwrap();
        device.add("brews", Some("Bruhs")).unwrap();
        let mut project = VoiceDictionary::default();
        project.add("BRUHS", None).unwrap();
        project.add("brews", Some("BRUHS")).unwrap();
        assert_eq!(
            device.apply(Some(&project), "brews in new york, cold brewski and c++."),
            "BRUHS in New York, cold brewski and C++."
        );
        assert_eq!(device.apply(None, "brews"), "Bruhs");
        assert_eq!(
            VoiceDictionary::preferred_terms(
                &[&project, &device],
                &["sorry", "BReWs", " NY   office "]
            ),
            ["sorry", "NY office", "BRUHS", "C++", "New York"]
        );
    }
}

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct HostDictionaries {
    pub environment: VoiceDictionary,
    pub project: VoiceDictionary,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DictionaryRequest {
    pub project: Option<String>,
    pub edit: Option<DictionaryEdit>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DictionaryEdit {
    #[serde(default)]
    pub for_project: bool,
    #[serde(flatten)]
    pub operation: DictionaryOperation,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum DictionaryOperation {
    Add {
        words: String,
        alias_for: Option<String>,
    },
    Remove {
        term: String,
    },
}
