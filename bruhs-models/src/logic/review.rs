//! How a diff review sorts and remembers files.
//!
//! The kinds come from path conventions only,
//! so a local working tree and a turn diff can share one filter. Marks are
//! the panel's own record. A later edit of the loaded hunk text is
//! [`ViewedState::Changed`] and is no longer counted as viewed.

use std::collections::BTreeMap;

/// One scope's viewed files: path to the fingerprint of the hunk text that
/// was marked. The outer key is [`review_scope_key`].
pub type ViewedBook = BTreeMap<String, BTreeMap<String, String>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffFileKind {
    Code,
    Tests,
    Mocks,
    Generated,
    Docs,
}

impl DiffFileKind {
    pub fn kind_name(self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Tests => "tests",
            Self::Mocks => "mocks",
            Self::Generated => "generated",
            Self::Docs => "docs",
        }
    }
}

/// The review filter. Mocks and generated files stay under [`Self::All`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DiffReviewFilter {
    #[default]
    All,
    Code,
    Tests,
    Docs,
}

impl DiffReviewFilter {
    pub const CHOICES: [Self; 4] = [Self::All, Self::Code, Self::Tests, Self::Docs];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Code => "Code",
            Self::Tests => "Tests",
            Self::Docs => "Docs",
        }
    }

    pub fn includes(self, kind: DiffFileKind) -> bool {
        match self {
            Self::All => true,
            Self::Code => kind == DiffFileKind::Code,
            Self::Tests => kind == DiffFileKind::Tests,
            Self::Docs => kind == DiffFileKind::Docs,
        }
    }

    /// Shown when this filter hides every file and the scope itself is not empty.
    pub fn empty_label(self) -> &'static str {
        match self {
            Self::All => "No changes.",
            Self::Code => "No code changes.",
            Self::Tests => "No test changes.",
            Self::Docs => "No doc changes.",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewedState {
    Unseen,
    Viewed,
    /// A mark exists, and the loaded hunk text is no longer the marked text.
    Changed,
}

/// `project` and `scope` together name one review. A turn scope is the
/// user message id on the web and the prompting text on the desktop.
pub fn review_scope_key(project: &str, scope: &str) -> String {
    format!("{project}\u{1f}{scope}")
}

pub fn classify_path(path: &str) -> DiffFileKind {
    let normalized = path.replace('\\', "/");
    let mut segments = normalized.split('/').filter(|part| !part.is_empty());
    let name = segments.next_back().unwrap_or("");
    let lower_name = name.to_ascii_lowercase();
    let dirs: Vec<String> = segments.map(|part| part.to_ascii_lowercase()).collect();
    let in_dir = |names: &[&str]| dirs.iter().any(|dir| names.contains(&dir.as_str()));
    if GENERATED_FILES.contains(&lower_name.as_str())
        || generated_name(name)
        || in_dir(GENERATED_DIRS)
    {
        return DiffFileKind::Generated;
    }
    if mock_name(name) || in_dir(MOCK_DIRS) {
        return DiffFileKind::Mocks;
    }
    if test_name(&lower_name) || class_test_name(name) || in_dir(TEST_DIRS) {
        return DiffFileKind::Tests;
    }
    if docs_name(name) && !not_docs_name(name) {
        return DiffFileKind::Docs;
    }
    DiffFileKind::Code
}

/// Stable fingerprint of the loaded hunk text. Stored as hex so a JSON
/// number never rounds it.
pub fn content_fingerprint(parts: impl IntoIterator<Item = impl AsRef<[u8]>>) -> String {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;
    let mut hash = OFFSET;
    for part in parts {
        for byte in part.as_ref() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(PRIME);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{hash:016x}")
}

pub fn viewed_state(
    book: &ViewedBook,
    scope_key: &str,
    path: &str,
    fingerprint: &str,
) -> ViewedState {
    match book
        .get(scope_key)
        .and_then(|marks| marks.get(path))
        .map(String::as_str)
    {
        Some(saved) if saved == fingerprint => ViewedState::Viewed,
        Some(_) => ViewedState::Changed,
        None => ViewedState::Unseen,
    }
}

pub fn set_viewed(
    book: &mut ViewedBook,
    scope_key: &str,
    path: &str,
    fingerprint: &str,
    viewed: bool,
) {
    let marks = book.entry(scope_key.to_string()).or_default();
    if viewed {
        marks.insert(path.to_string(), fingerprint.to_string());
    } else {
        marks.remove(path);
    }
}

pub fn viewed_count(book: &ViewedBook, scope_key: &str, files: &[(&str, &str)]) -> usize {
    files
        .iter()
        .filter(|(path, fingerprint)| {
            viewed_state(book, scope_key, path, fingerprint) == ViewedState::Viewed
        })
        .count()
}

const GENERATED_FILES: &[&str] = &[
    "bun.lock",
    "bun.lockb",
    "cargo.lock",
    "composer.lock",
    "deno.lock",
    "flake.lock",
    "gemfile.lock",
    "go.sum",
    "gradle.lockfile",
    "mix.lock",
    "npm-shrinkwrap.json",
    "package-lock.json",
    "packages.lock.json",
    "pipfile.lock",
    "pnpm-lock.yaml",
    "podfile.lock",
    "poetry.lock",
    "pubspec.lock",
    "uv.lock",
    "yarn.lock",
];
const GENERATED_DIRS: &[&str] = &["__generated__", "generated"];
const MOCK_DIRS: &[&str] = &[
    "__fixtures__",
    "__mocks__",
    "__snapshots__",
    "fakes",
    "fixture",
    "fixtures",
    "mock",
    "mocks",
    "stubs",
    "testdata",
];
const TEST_DIRS: &[&str] = &[
    "__tests__",
    "__test__",
    "cypress",
    "e2e",
    "spec",
    "specs",
    "test",
    "tests",
];

fn generated_name(name: &str) -> bool {
    name.contains(".generated.")
        || name.contains(".gen.")
        || name.ends_with(".g.dart")
        || name.ends_with(".pb.go")
        || name.ends_with("_pb2.py")
        || name.ends_with("_pb2_grpc.py")
        || name.ends_with(".pb.cc")
        || name.ends_with(".pb.h")
        || name.ends_with(".min.js")
        || name.ends_with(".min.css")
        || name.ends_with(".js.map")
        || name.ends_with(".css.map")
}

fn mock_name(name: &str) -> bool {
    name.ends_with(".snap")
        || dotted_stem(
            name,
            &[
                "mock", "mocks", "fixture", "fixtures", "stub", "stubs", "fake", "fakes",
            ],
        )
        || mock_prefix(name)
}

fn mock_prefix(name: &str) -> bool {
    const PREFIXES: [&str; 8] = [
        "mocks", "fixtures", "stubs", "fakes", "mock", "fixture", "stub", "fake",
    ];
    PREFIXES
        .iter()
        .any(|prefix| match name.strip_prefix(prefix) {
            Some("") => true,
            Some(rest) => rest
                .chars()
                .next()
                .is_some_and(|ch| matches!(ch, '-' | '_' | '.' | 'A'..='Z')),
            None => false,
        })
}

fn test_name(lower: &str) -> bool {
    dotted_stem(lower, &["test", "spec", "e2e"])
        || underscore_kind(lower, &["_test", "_spec"])
        || (lower.starts_with("test_") && lower.ends_with(".py") && lower != "test_.py")
        || lower == "conftest.py"
        || setup_file(lower)
}

fn setup_file(lower: &str) -> bool {
    lower.rsplit_once('.').is_some_and(|(head, ext)| {
        !ext.is_empty() && (head == "vitest.setup" || head == "jest.setup")
    })
}

fn class_test_name(name: &str) -> bool {
    let Some((head, ext)) = name.rsplit_once('.') else {
        return false;
    };
    matches!(
        ext,
        "java" | "kt" | "kts" | "scala" | "groovy" | "cs" | "php" | "swift" | "m"
    ) && (head.ends_with("Test") || head.ends_with("Tests") || head.ends_with("Spec"))
}

fn docs_name(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, ext)| {
        matches!(
            ext.to_ascii_lowercase().as_str(),
            "md" | "mdx" | "markdown" | "rst" | "adoc" | "txt"
        )
    })
}

fn not_docs_name(name: &str) -> bool {
    let Some((head, ext)) = name.rsplit_once('.') else {
        return false;
    };
    if ext != "txt" {
        return false;
    }
    head == "CMakeLists"
        || head == "robots"
        || head == "llms"
        || head.starts_with("requirements")
        || head.starts_with("constraints")
}

fn dotted_stem(name: &str, stems: &[&str]) -> bool {
    let mut parts = name.rsplitn(3, '.');
    let Some(ext) = parts.next() else {
        return false;
    };
    let Some(stem) = parts.next() else {
        return false;
    };
    parts.next().is_some() && !ext.is_empty() && stems.contains(&stem)
}

fn underscore_kind(name: &str, suffixes: &[&str]) -> bool {
    let Some((head, ext)) = name.rsplit_once('.') else {
        return false;
    };
    !ext.is_empty() && suffixes.iter().any(|suffix| head.ends_with(suffix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_paths_the_way_t3_does() {
        let cases = [
            ("src/contest.py", DiffFileKind::Code),
            ("test/fixtures/pr.json", DiffFileKind::Mocks),
            ("pnpm-lock.yaml", DiffFileKind::Generated),
            ("README.md", DiffFileKind::Docs),
            ("docs/internals/overview.mdx", DiffFileKind::Docs),
            ("NOTES.txt", DiffFileKind::Docs),
            ("requirements-dev.txt", DiffFileKind::Code),
            ("CMakeLists.txt", DiffFileKind::Code),
            ("tests/README.md", DiffFileKind::Tests),
            ("src/a.test.ts", DiffFileKind::Tests),
            ("src/routeTree.gen.ts", DiffFileKind::Generated),
            ("src/__generated__/types.ts", DiffFileKind::Generated),
            ("dist/app.min.js", DiffFileKind::Generated),
            ("src/FooTest.java", DiffFileKind::Tests),
            ("src/Latest.java", DiffFileKind::Code),
        ];
        for (path, kind) in cases {
            assert_eq!(classify_path(path), kind, "{path}");
        }
    }

    #[test]
    fn a_changed_hunk_is_no_longer_viewed() {
        let key = review_scope_key("/repo", "uncommitted");
        let mut book = ViewedBook::new();
        let marked = content_fingerprint(["old"]);
        set_viewed(&mut book, &key, "src/a.rs", &marked, true);
        assert_eq!(
            viewed_state(&book, &key, "src/a.rs", &marked),
            ViewedState::Viewed
        );
        let edited = content_fingerprint(["new"]);
        assert_ne!(marked, edited);
        assert_eq!(
            viewed_state(&book, &key, "src/a.rs", &edited),
            ViewedState::Changed
        );
        assert_eq!(
            viewed_count(&book, &key, &[("src/a.rs", edited.as_str())]),
            0
        );
        set_viewed(&mut book, &key, "src/a.rs", &marked, false);
        assert_eq!(
            viewed_state(&book, &key, "src/a.rs", &marked),
            ViewedState::Unseen
        );
    }

    #[test]
    fn the_filter_keeps_mocks_and_generated_files_on_all() {
        assert!(DiffReviewFilter::All.includes(DiffFileKind::Mocks));
        assert!(DiffReviewFilter::All.includes(DiffFileKind::Generated));
        assert!(!DiffReviewFilter::Code.includes(DiffFileKind::Mocks));
        assert!(DiffReviewFilter::Code.includes(DiffFileKind::Code));
        assert!(DiffReviewFilter::Docs.includes(DiffFileKind::Docs));
        assert!(!DiffReviewFilter::Tests.includes(DiffFileKind::Docs));
    }
}
