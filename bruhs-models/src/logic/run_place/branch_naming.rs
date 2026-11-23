//! Branch naming preferences for the first send in a new worktree.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    ThreadId,
    #[default]
    Static,
    Semantic,
    Custom,
}
impl Mode {
    pub fn key(self) -> &'static str {
        match self {
            Self::ThreadId => "thread_id",
            Self::Static => "static",
            Self::Semantic => "semantic",
            Self::Custom => "custom",
        }
    }
    pub fn parse(key: &str) -> Option<Self> {
        Some(match key {
            "thread_id" => Self::ThreadId,
            "static" => Self::Static,
            "semantic" => Self::Semantic,
            "custom" => Self::Custom,
            _ => return None,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Naming {
    pub mode: Mode,
    pub prefix: String,
    pub instructions: String,
}
impl Default for Naming {
    fn default() -> Self {
        Self {
            mode: Mode::Static,
            prefix: "bruhs".into(),
            instructions: String::new(),
        }
    }
}
impl Naming {
    /// Turn a model suggestion into the requested Git ref. Custom mode
    /// preserves case, punctuation and namespaces exactly.
    pub fn branch(&self, generated: &str) -> Result<String, String> {
        use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};
        let name = if self.mode == Mode::Custom {
            generated.trim().to_owned()
        } else {
            let mut slug = String::new();
            for ch in generated.nfd().filter(|c| !is_combining_mark(*c)) {
                let ch = match ch {
                    c if c.is_ascii_alphanumeric() => c.to_ascii_lowercase(),
                    '/' | '_' | '-' => ch,
                    '\'' | '"' | '`' => continue,
                    _ => '-',
                };
                if (ch == '-' || ch == '/') && slug.ends_with(ch) {
                    continue;
                }
                slug.push(ch);
            }
            let slug = slug.trim_matches(['-', '/', '_']);
            let slug = slug
                .get(..64)
                .unwrap_or(slug)
                .trim_end_matches(['-', '/', '_']);
            let fragment = if slug.is_empty() { "update" } else { slug };
            if self.mode == Mode::Static {
                let prefix = self
                    .prefix
                    .split('/')
                    .map(|part| {
                        part.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                            .filter(|s| !s.is_empty())
                            .collect::<Vec<_>>()
                            .join("-")
                    })
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join("/");
                if prefix.is_empty() {
                    fragment.to_owned()
                } else {
                    format!("{prefix}/{fragment}")
                }
            } else {
                fragment.to_owned()
            }
        };
        crate::logic::git::validate_branch_name(&name).map_err(|e| e.to_string())?;
        Ok(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_preserve_exact_custom_output_and_normalize_generated_fragments() {
        let mut n = Naming::default();
        assert_eq!(n.branch("  Café search! ").unwrap(), "bruhs/cafe-search");
        n.prefix = " Team /Review/".into();
        assert_eq!(n.branch("Add search").unwrap(), "Team/Review/add-search");
        n.prefix.clear();
        assert_eq!(n.branch("Add search").unwrap(), "add-search");
        n.mode = Mode::Semantic;
        assert_eq!(n.branch("fix/Login timeout").unwrap(), "fix/login-timeout");
        n.mode = Mode::Custom;
        assert_eq!(
            n.branch(" team/APP-42/修正.search ").unwrap(),
            "team/APP-42/修正.search"
        );
        assert!(n.branch("team/../other").is_err());
    }
}
