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
        let mut words: Vec<_> = spellings.keys().collect();
        words.sort_by_key(|s| std::cmp::Reverse(s.len()));
        let alternatives = words
            .into_iter()
            .map(|s| regex::escape(s))
            .collect::<Vec<_>>()
            .join("|");
        let Ok(pattern) = regex::RegexBuilder::new(&alternatives)
            .case_insensitive(true)
            .build()
        else {
            return text.to_owned();
        };
        let word_char = |c: char| c.is_alphanumeric() || c == '_';
        pattern
            .replace_all(text, |captures: &regex::Captures<'_>| {
                let part = captures
                    .get(0)
                    .expect("capture zero is the complete spelling");
                if text[..part.start()]
                    .chars()
                    .next_back()
                    .is_some_and(word_char)
                    || text[part.end()..].chars().next().is_some_and(word_char)
                {
                    part.as_str().to_owned()
                } else {
                    spellings
                        .get(&part.as_str().to_lowercase())
                        .copied()
                        .unwrap_or(part.as_str())
                        .to_owned()
                }
            })
            .into_owned()
    }
}

/// Remove standalone speech hesitations while retaining dictated paragraphs, code and the cleanup cue.
pub fn without_fillers(text: &str, language: Option<&str>, correction: &str) -> String {
    let language_words = match language.and_then(|code| code.split(['-', '_']).next()) {
        Some("en") => "|um|ah|eh",
        Some("fr") => "|euh",
        Some("de") => "|ähm|äh",
        _ => "",
    };
    let fillers =
        format!(r"\b(?:uhhh|uhh|uhm|umm|uh|ehh|ehm|ahm|hmm|hm|mmm|хм|ммм{language_words})\b[,.]?");
    let cue = correction
        .split_whitespace()
        .map(regex::escape)
        .collect::<Vec<_>>()
        .join(r"\s+");
    let pattern = if cue.is_empty() {
        fillers
    } else {
        format!(r"(?P<cue>\b{cue}\b)|{fillers}")
    };
    let Ok(pattern) = regex::RegexBuilder::new(&pattern)
        .case_insensitive(true)
        .build()
    else {
        return text.to_owned();
    };
    let filtered = pattern.replace_all(text, |captures: &regex::Captures<'_>| {
        let found = captures.get(0).expect("complete match");
        if captures.name("cue").is_some()
            || text[..found.start()].ends_with('-')
            || text[found.end()..].starts_with('-')
        {
            found.as_str().to_owned()
        } else {
            String::new()
        }
    });
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
