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

#[cfg(test)]
mod tests {
    use super::*;
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
