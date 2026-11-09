//! Pure git helpers: branch-name validation and `git status` parsing. The
//! process-running half lives in bruhs-core `git.rs`.

use std::fmt;

use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

/// Why a string is not a branch name `git check-ref-format --branch` accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BranchNameError {
    #[error("branch name is empty")]
    Empty,
    #[error("branch name starts with '-'")]
    LeadingDash,
    #[error("{0:?} is not a valid branch name")]
    Reserved(&'static str),
    #[error("branch name has an empty path component (leading, trailing or doubled '/')")]
    EmptyComponent,
    #[error("a branch name component starts with '.'")]
    DotComponent,
    #[error("a branch name component ends with \".lock\"")]
    LockComponent,
    #[error("branch name contains \"..\"")]
    DoubleDot,
    #[error("branch name ends with '.'")]
    TrailingDot,
    #[error("branch name contains \"@{{\"")]
    AtBrace,
    #[error("branch name contains {0:?}")]
    InvalidCharacter(char),
}

/// Check `name` against the rules `git check-ref-format --branch` enforces,
/// statically. It does not, and cannot, tell whether `name` is also a file
/// path in some work tree; callers must still separate it from paths
/// (`git switch -- <name>`).
pub fn validate_branch_name(name: &str) -> Result<(), BranchNameError> {
    if name.is_empty() {
        return Err(BranchNameError::Empty);
    }
    if name.starts_with('-') {
        return Err(BranchNameError::LeadingDash);
    }
    for reserved in ["HEAD", "@"] {
        if name == reserved {
            return Err(BranchNameError::Reserved(reserved));
        }
    }
    if let Some(c) = name.chars().find(|&c| {
        c.is_ascii_control() || matches!(c, ' ' | '~' | '^' | ':' | '?' | '*' | '[' | '\\')
    }) {
        return Err(BranchNameError::InvalidCharacter(c));
    }
    if name.contains("..") {
        return Err(BranchNameError::DoubleDot);
    }
    if name.contains("@{") {
        return Err(BranchNameError::AtBrace);
    }
    if name.ends_with('.') {
        return Err(BranchNameError::TrailingDot);
    }
    for component in name.split('/') {
        if component.is_empty() {
            return Err(BranchNameError::EmptyComponent);
        }
        if component.starts_with('.') {
            return Err(BranchNameError::DotComponent);
        }
        if component.ends_with(".lock") {
            return Err(BranchNameError::LockComponent);
        }
    }
    Ok(())
}

/// Whether `name` passes [`validate_branch_name`].
pub fn is_valid_branch(name: &str) -> bool {
    validate_branch_name(name).is_ok()
}

/// A branch name that passed [`validate_branch_name`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BranchName(String);

impl BranchName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for BranchName {
    type Error = BranchNameError;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        validate_branch_name(&name)?;
        Ok(Self(name))
    }
}

impl TryFrom<&str> for BranchName {
    type Error = BranchNameError;

    fn try_from(name: &str) -> Result<Self, Self::Error> {
        Self::try_from(name.to_string())
    }
}

impl From<BranchName> for String {
    fn from(name: BranchName) -> Self {
        name.0
    }
}

impl fmt::Display for BranchName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// How a file differs, from one `git status --porcelain` column pair.
/// On the wire it is the single porcelain letter (`"M"`, `"?"`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileStatus {
    Modified,
    TypeChanged,
    Added,
    Deleted,
    Renamed,
    Copied,
    /// Merge conflict.
    Unmerged,
    Untracked,
    Ignored,
}

impl FileStatus {
    /// The porcelain letter.
    pub fn letter(self) -> char {
        match self {
            Self::Modified => 'M',
            Self::TypeChanged => 'T',
            Self::Added => 'A',
            Self::Deleted => 'D',
            Self::Renamed => 'R',
            Self::Copied => 'C',
            Self::Unmerged => 'U',
            Self::Untracked => '?',
            Self::Ignored => '!',
        }
    }

    /// The status for a porcelain letter; `None` for `' '` (unmodified) and
    /// anything git does not emit.
    pub fn from_letter(letter: char) -> Option<Self> {
        Some(match letter {
            'M' => Self::Modified,
            'T' => Self::TypeChanged,
            'A' => Self::Added,
            'D' => Self::Deleted,
            'R' => Self::Renamed,
            'C' => Self::Copied,
            'U' => Self::Unmerged,
            '?' => Self::Untracked,
            '!' => Self::Ignored,
            _ => return None,
        })
    }
}

impl Serialize for FileStatus {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut buf = [0u8; 4];
        serializer.serialize_str(self.letter().encode_utf8(&mut buf))
    }
}

impl<'de> Deserialize<'de> for FileStatus {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        let mut chars = raw.chars();
        match (chars.next(), chars.next()) {
            (Some(letter), None) => Self::from_letter(letter),
            _ => None,
        }
        .ok_or_else(|| {
            de::Error::invalid_value(de::Unexpected::Str(&raw), &"a porcelain status letter")
        })
    }
}

/// One changed path in the working tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirtyFile {
    /// Current path (the destination of a rename or copy).
    pub path: String,
    /// Work-tree status when there is one, else the index status.
    pub status: FileStatus,
    /// Whether the index (staging area) has a change for this path.
    pub staged: bool,
}

/// Branch, upstream distance and changed files of a work tree. Same JSON as
/// bruhs-core's `git::GitStatus` (`/api/agent/git-status`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitStatus {
    /// `None` when HEAD is detached.
    pub branch: Option<String>,
    #[serde(default)]
    pub ahead: u32,
    #[serde(default)]
    pub behind: u32,
    #[serde(default)]
    pub dirty: Vec<DirtyFile>,
}

fn apply_branch_header(status: &mut GitStatus, header: &str) {
    let (head, rest) = match header.find(' ') {
        Some(i) => (&header[..i], &header[i..]),
        None => (header, ""),
    };
    if header.starts_with("HEAD (no branch)") {
        status.branch = None;
    } else if let Some(b) = header.strip_prefix("No commits yet on ") {
        status.branch = Some(b.trim().to_string());
    } else if let Some(b) = header.strip_prefix("Initial commit on ") {
        status.branch = Some(b.trim().to_string());
    } else {
        let name = head.split("...").next().unwrap_or(head);
        if !name.is_empty() {
            status.branch = Some(name.to_string());
        }
    }
    if let Some(start) = rest.find('[') {
        let end = rest.find(']').unwrap_or(rest.len());
        for part in rest[start + 1..end].split(',') {
            let part = part.trim();
            if let Some(n) = part.strip_prefix("ahead ") {
                status.ahead = n.trim().parse().unwrap_or(0);
            } else if let Some(n) = part.strip_prefix("behind ") {
                status.behind = n.trim().parse().unwrap_or(0);
            }
        }
    }
}

/// Parse `git status --porcelain=v1 -b -z` output.
///
/// With `-z` entries are NUL-terminated and paths are never quoted, so names
/// containing spaces, quotes, ` -> ` or newlines come through intact. A
/// rename or copy entry is followed by a second entry holding the source
/// path, which is consumed.
pub fn parse_porcelain(out: &str) -> GitStatus {
    let mut status = GitStatus::default();
    let mut entries = out.split('\0');
    while let Some(entry) = entries.next() {
        if let Some(header) = entry.strip_prefix("## ") {
            apply_branch_header(&mut status, header);
            continue;
        }
        // "XY path": two ASCII status columns, a space, then the path.
        let (Some(x), Some(y), Some(path)) = (
            entry.chars().next(),
            entry.chars().nth(1),
            entry.get(3..).filter(|path| !path.is_empty()),
        ) else {
            continue;
        };
        if matches!(x, 'R' | 'C') || matches!(y, 'R' | 'C') {
            // The rename/copy source follows as its own entry.
            entries.next();
        }
        let letter = if x == '?' {
            '?'
        } else if y != ' ' {
            y
        } else {
            x
        };
        let Some(file_status) = FileStatus::from_letter(letter) else {
            continue;
        };
        status.dirty.push(DirtyFile {
            path: path.to_string(),
            status: file_status,
            staged: x != ' ' && x != '?' && x != '!',
        });
    }
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_names_follow_check_ref_format() {
        // Probed against `git check-ref-format --branch`.
        for ok in [
            "main",
            "feature/x-1.2_ok",
            "semi;colon",
            "héllo",
            "a$b",
            "FETCH_HEAD",
            "refs/heads/x",
            "a.b",
            "a/b.c",
        ] {
            assert_eq!(validate_branch_name(ok), Ok(()), "{ok}");
        }
        for (bad, err) in [
            ("", BranchNameError::Empty),
            ("-bad", BranchNameError::LeadingDash),
            ("-", BranchNameError::LeadingDash),
            ("HEAD", BranchNameError::Reserved("HEAD")),
            ("@", BranchNameError::Reserved("@")),
            (".", BranchNameError::TrailingDot),
            ("..", BranchNameError::DoubleDot),
            ("a..b", BranchNameError::DoubleDot),
            (".a", BranchNameError::DotComponent),
            ("a/.b", BranchNameError::DotComponent),
            ("a/.", BranchNameError::TrailingDot),
            ("a.", BranchNameError::TrailingDot),
            ("a.lock", BranchNameError::LockComponent),
            ("foo.lock", BranchNameError::LockComponent),
            ("a/b.lock/c", BranchNameError::LockComponent),
            ("a//b", BranchNameError::EmptyComponent),
            ("/a", BranchNameError::EmptyComponent),
            ("a/", BranchNameError::EmptyComponent),
            ("a@{b", BranchNameError::AtBrace),
            ("@{-1}", BranchNameError::AtBrace),
            ("has space", BranchNameError::InvalidCharacter(' ')),
            ("a~b", BranchNameError::InvalidCharacter('~')),
            ("a^b", BranchNameError::InvalidCharacter('^')),
            ("a:b", BranchNameError::InvalidCharacter(':')),
            ("a?", BranchNameError::InvalidCharacter('?')),
            ("a*", BranchNameError::InvalidCharacter('*')),
            ("a[b", BranchNameError::InvalidCharacter('[')),
            ("x\\y", BranchNameError::InvalidCharacter('\\')),
            ("a\nb", BranchNameError::InvalidCharacter('\n')),
            ("a\u{7f}", BranchNameError::InvalidCharacter('\u{7f}')),
        ] {
            assert_eq!(validate_branch_name(bad), Err(err), "{bad:?}");
            assert!(!is_valid_branch(bad));
        }
    }

    #[test]
    fn branch_name_newtype_validates_on_deserialize() {
        let name = BranchName::try_from("feature/x").unwrap();
        assert_eq!(serde_json::to_string(&name).unwrap(), "\"feature/x\"");
        assert_eq!(
            serde_json::from_str::<BranchName>("\"feature/x\"").unwrap(),
            name
        );
        assert!(serde_json::from_str::<BranchName>("\".\"").is_err());
        assert_eq!(name.as_str(), "feature/x");
    }

    #[test]
    fn parses_porcelain_z_with_branch_header() {
        let out = "## main...origin/main [ahead 2, behind 1]\0 M src/a.rs\0A  src/b.rs\0?? notes.md\0R  new.rs\0old.rs\0";
        let s = parse_porcelain(out);
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!((s.ahead, s.behind), (2, 1));
        let rows: Vec<(&str, FileStatus, bool)> = s
            .dirty
            .iter()
            .map(|d| (d.path.as_str(), d.status, d.staged))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("src/a.rs", FileStatus::Modified, false),
                ("src/b.rs", FileStatus::Added, true),
                ("notes.md", FileStatus::Untracked, false),
                ("new.rs", FileStatus::Renamed, true),
            ]
        );
    }

    #[test]
    fn z_paths_are_taken_verbatim() {
        let out = "## main\0 M has space.rs\0?? \"quoted\".md\0R  to -> name.rs\0from\nline.rs\0 D gone.rs\0";
        let paths: Vec<String> = parse_porcelain(out)
            .dirty
            .into_iter()
            .map(|d| d.path)
            .collect();
        assert_eq!(
            paths,
            vec!["has space.rs", "\"quoted\".md", "to -> name.rs", "gone.rs"]
        );
    }

    #[test]
    fn detached_unborn_and_both_columns() {
        let s = parse_porcelain("## HEAD (no branch)\0");
        assert_eq!(s.branch, None);
        assert!(s.dirty.is_empty());
        let s = parse_porcelain("## feature/x\0MM both.rs\0UU conflict.rs\0C  copy.rs\0orig.rs\0");
        assert_eq!(s.branch.as_deref(), Some("feature/x"));
        assert_eq!((s.ahead, s.behind), (0, 0));
        assert_eq!(
            (s.dirty[0].status, s.dirty[0].staged),
            (FileStatus::Modified, true)
        );
        assert_eq!(s.dirty[1].status, FileStatus::Unmerged);
        assert_eq!(
            (s.dirty[2].path.as_str(), s.dirty[2].status),
            ("copy.rs", FileStatus::Copied)
        );
        assert_eq!(s.dirty.len(), 3);
        let s = parse_porcelain("## No commits yet on main\0?? a\0");
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!(
            parse_porcelain("## Initial commit on dev\0")
                .branch
                .as_deref(),
            Some("dev")
        );
        assert_eq!(parse_porcelain(""), GitStatus::default());
    }

    #[test]
    fn status_json_shape_matches_the_web_contract() {
        // bruhs-core `status_json_shape_matches_the_web_contract`.
        let s = GitStatus {
            branch: Some("main".into()),
            ahead: 1,
            behind: 0,
            dirty: vec![DirtyFile {
                path: "a.rs".into(),
                status: FileStatus::Modified,
                staged: false,
            }],
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "branch": "main", "ahead": 1, "behind": 0,
                "dirty": [{ "path": "a.rs", "status": "M", "staged": false }]
            })
        );
        assert_eq!(serde_json::from_value::<GitStatus>(v).unwrap(), s);
        for letter in ['M', 'T', 'A', 'D', 'R', 'C', 'U', '?', '!'] {
            let status = FileStatus::from_letter(letter).unwrap();
            let json = serde_json::to_string(&status).unwrap();
            assert_eq!(json, format!("\"{letter}\""));
            assert_eq!(serde_json::from_str::<FileStatus>(&json).unwrap(), status);
        }
        assert!(serde_json::from_str::<FileStatus>("\"MM\"").is_err());
        assert!(serde_json::from_str::<FileStatus>("\" \"").is_err());
    }
}
