//! Line diffs: an LCS diff for tool inputs (Edit/Write before → after) and a
//! parser for `git diff` unified output. Moved from bruhs-web `diff.rs`
//! (the reference) and bruhs-desktop `views/diff_panel.rs`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Add,
    Remove,
}

/// One diff row with 1-based line numbers on the side(s) it exists on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_no: Option<usize>,
    pub new_no: Option<usize>,
    pub text: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiffStats {
    pub additions: usize,
    pub deletions: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// `@@ -a,b +c,d @@`, without the trailing function context.
    pub header: String,
    pub lines: Vec<DiffLine>,
}

/// One file of a unified diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    /// `None` for a new file (`--- /dev/null`).
    pub old_path: Option<String>,
    /// `None` for a deleted file (`+++ /dev/null`).
    pub new_path: Option<String>,
    pub hunks: Vec<Hunk>,
    pub is_binary: bool,
}

impl FileDiff {
    /// The new path, else the old one, else `""`.
    pub fn display_path(&self) -> &str {
        self.new_path
            .as_deref()
            .or(self.old_path.as_deref())
            .unwrap_or("")
    }

    pub fn is_added(&self) -> bool {
        self.old_path.is_none()
    }

    pub fn is_deleted(&self) -> bool {
        self.new_path.is_none()
    }

    pub fn stats(&self) -> DiffStats {
        let mut total = DiffStats::default();
        for hunk in &self.hunks {
            let s = stats(&hunk.lines);
            total.additions += s.additions;
            total.deletions += s.deletions;
        }
        total
    }
}

/// Above this many cells the O(n·m) table is skipped and a pairwise walk is
/// used instead; tool inputs are small so this rarely triggers.
pub const MAX_LCS_CELLS: usize = 4_000_000;

fn split_lines(s: &str) -> Vec<&str> {
    if s.is_empty() {
        Vec::new()
    } else {
        s.lines().collect()
    }
}

fn context(old_no: usize, new_no: usize, text: &str) -> DiffLine {
    DiffLine {
        kind: LineKind::Context,
        old_no: Some(old_no),
        new_no: Some(new_no),
        text: text.to_string(),
    }
}

fn removed(old_no: usize, text: &str) -> DiffLine {
    DiffLine {
        kind: LineKind::Remove,
        old_no: Some(old_no),
        new_no: None,
        text: text.to_string(),
    }
}

fn added(new_no: usize, text: &str) -> DiffLine {
    DiffLine {
        kind: LineKind::Add,
        old_no: None,
        new_no: Some(new_no),
        text: text.to_string(),
    }
}

/// Longest-common-subsequence line diff. Shared lines become `Context`,
/// everything else `Remove`/`Add` with 1-based line numbers on each side.
pub fn lcs_diff(old: &str, new: &str) -> Vec<DiffLine> {
    let a = split_lines(old);
    let b = split_lines(new);
    let (n, m) = (a.len(), b.len());

    if n.saturating_mul(m) > MAX_LCS_CELLS {
        return pairwise(&a, &b);
    }

    // dp[i][j] = LCS length of a[i..] and b[j..]
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    let mut out = Vec::with_capacity(n.max(m));
    let (mut i, mut j) = (0, 0);
    let (mut old_no, mut new_no) = (0usize, 0usize);
    while i < n && j < m {
        if a[i] == b[j] {
            old_no += 1;
            new_no += 1;
            out.push(context(old_no, new_no, a[i]));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            old_no += 1;
            out.push(removed(old_no, a[i]));
            i += 1;
        } else {
            new_no += 1;
            out.push(added(new_no, b[j]));
            j += 1;
        }
    }
    while i < n {
        old_no += 1;
        out.push(removed(old_no, a[i]));
        i += 1;
    }
    while j < m {
        new_no += 1;
        out.push(added(new_no, b[j]));
        j += 1;
    }
    out
}

fn pairwise(a: &[&str], b: &[&str]) -> Vec<DiffLine> {
    let mut out = Vec::new();
    for idx in 0..a.len().max(b.len()) {
        match (a.get(idx), b.get(idx)) {
            (Some(x), Some(y)) if x == y => out.push(context(idx + 1, idx + 1, x)),
            (x, y) => {
                if let Some(x) = x {
                    out.push(removed(idx + 1, x));
                }
                if let Some(y) = y {
                    out.push(added(idx + 1, y));
                }
            }
        }
    }
    out
}

pub fn stats(lines: &[DiffLine]) -> DiffStats {
    let mut s = DiffStats::default();
    for line in lines {
        match line.kind {
            LineKind::Add => s.additions += 1,
            LineKind::Remove => s.deletions += 1,
            LineKind::Context => {}
        }
    }
    s
}

/// The ranges of a hunk header, `@@ -old_start,old_len +new_start,new_len @@`.
/// A missing length means 1; an unparsable start means 1, a length 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HunkRange {
    pub old_start: usize,
    pub old_len: usize,
    pub new_start: usize,
    pub new_len: usize,
}

/// Parse the ranges of a `@@ … @@` hunk header. Only the text between the
/// two `@@` markers is read: git appends the enclosing function after the
/// second one (`@@ -1,2 +1,2 @@ fn f() -> u8`), and its tokens are not ranges.
pub fn parse_hunk_header(header: &str) -> HunkRange {
    fn range(spec: &str) -> (usize, usize) {
        let mut parts = spec.splitn(2, ',');
        let start = parts.next().and_then(|n| n.parse().ok()).unwrap_or(1);
        let len = match parts.next() {
            Some(n) => n.parse().unwrap_or(0),
            None => 1,
        };
        (start, len)
    }
    let mut out = HunkRange {
        old_start: 1,
        old_len: 0,
        new_start: 1,
        new_len: 0,
    };
    let ranges = header.trim_start().strip_prefix("@@").unwrap_or(header);
    let ranges = ranges.split("@@").next().unwrap_or(ranges);
    for part in ranges.split_whitespace() {
        if let Some(rest) = part.strip_prefix('-') {
            (out.old_start, out.old_len) = range(rest);
        } else if let Some(rest) = part.strip_prefix('+') {
            (out.new_start, out.new_len) = range(rest);
        }
    }
    out
}

fn strip_prefix_path(s: &str) -> Option<String> {
    let s = s.trim();
    if s == "/dev/null" {
        return None;
    }
    let s = s
        .strip_prefix("a/")
        .or_else(|| s.strip_prefix("b/"))
        .unwrap_or(s);
    Some(s.to_string())
}

/// Parse `git diff` output into per-file hunks. Tolerates binary markers,
/// new/deleted files, renames and `\ No newline at end of file`.
///
/// Inside a hunk, a line is body until the header's line counts are used
/// up, so removing a line that starts with `-- ` (it shows as `--- …`) is a
/// removal, not a new file header.
pub fn parse_unified(text: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut cur: Option<FileDiff> = None;
    let mut hunk: Option<Hunk> = None;
    let (mut old_no, mut new_no) = (0usize, 0usize);
    let (mut old_left, mut new_left) = (0usize, 0usize);

    fn flush_hunk(cur: &mut Option<FileDiff>, hunk: &mut Option<Hunk>) {
        if let (Some(f), Some(h)) = (cur.as_mut(), hunk.take()) {
            f.hunks.push(h);
        }
    }

    for line in text.lines() {
        if let Some(h) = hunk.as_mut() {
            if old_left > 0 {
                if let Some(t) = line.strip_prefix('-') {
                    h.lines.push(removed(old_no, t));
                    old_no += 1;
                    old_left -= 1;
                    continue;
                }
            }
            if new_left > 0 {
                if let Some(t) = line.strip_prefix('+') {
                    h.lines.push(added(new_no, t));
                    new_no += 1;
                    new_left -= 1;
                    continue;
                }
            }
        }
        if let Some(rest) = line.strip_prefix("diff --git ") {
            flush_hunk(&mut cur, &mut hunk);
            if let Some(f) = cur.take() {
                files.push(f);
            }
            let mut parts = rest.splitn(2, " b/");
            let a = parts.next().and_then(strip_prefix_path);
            let b = parts.next().map(|p| p.trim().to_string());
            cur = Some(FileDiff {
                old_path: a,
                new_path: b,
                hunks: Vec::new(),
                is_binary: false,
            });
        } else if line.starts_with("Binary files") {
            if let Some(f) = cur.as_mut() {
                f.is_binary = true;
            }
        } else if let Some(rest) = line.strip_prefix("--- ") {
            if let Some(f) = cur.as_mut() {
                f.old_path = strip_prefix_path(rest);
            }
        } else if let Some(rest) = line.strip_prefix("+++ ") {
            if let Some(f) = cur.as_mut() {
                f.new_path = strip_prefix_path(rest);
            }
        } else if line.starts_with("@@") {
            flush_hunk(&mut cur, &mut hunk);
            let header = line
                .split(" @@")
                .next()
                .map_or_else(|| line.to_string(), |h| format!("{h} @@"));
            let range = parse_hunk_header(line);
            old_no = range.old_start;
            new_no = range.new_start;
            old_left = range.old_len;
            new_left = range.new_len;
            hunk = Some(Hunk {
                header,
                lines: Vec::new(),
            });
        } else if let Some(h) = hunk.as_mut() {
            if let Some(t) = line.strip_prefix('+') {
                h.lines.push(added(new_no, t));
                new_no += 1;
                new_left = new_left.saturating_sub(1);
            } else if let Some(t) = line.strip_prefix('-') {
                h.lines.push(removed(old_no, t));
                old_no += 1;
                old_left = old_left.saturating_sub(1);
            } else if let Some(t) = line.strip_prefix(' ') {
                h.lines.push(context(old_no, new_no, t));
                old_no += 1;
                new_no += 1;
                old_left = old_left.saturating_sub(1);
                new_left = new_left.saturating_sub(1);
            }
            // Lines starting with `\` ("No newline at end of file") are ignored.
        }
    }
    flush_hunk(&mut cur, &mut hunk);
    if let Some(f) = cur.take() {
        files.push(f);
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(lines: &[DiffLine]) -> String {
        lines
            .iter()
            .map(|l| match l.kind {
                LineKind::Context => ' ',
                LineKind::Add => '+',
                LineKind::Remove => '-',
            })
            .collect()
    }

    #[test]
    fn identical_inputs_are_all_context() {
        let d = lcs_diff("a\nb\nc", "a\nb\nc");
        assert_eq!(kinds(&d), "   ");
        assert_eq!(d[2].old_no, Some(3));
        assert_eq!(d[2].new_no, Some(3));
    }

    #[test]
    fn insertion_in_middle() {
        let d = lcs_diff("a\nc", "a\nb\nc");
        assert_eq!(kinds(&d), " + ");
        assert_eq!(d[1].text, "b");
        assert_eq!(d[1].old_no, None);
        assert_eq!(d[1].new_no, Some(2));
    }

    #[test]
    fn replacement_keeps_shared_lines() {
        let d = lcs_diff(
            "fn a() {}\nfn b() {}\nfn c() {}",
            "fn a() {}\nfn B() {}\nfn c() {}",
        );
        assert_eq!(kinds(&d), " -+ ");
        assert_eq!(
            stats(&d),
            DiffStats {
                additions: 1,
                deletions: 1
            }
        );
        // Desktop `lcs_rows_numbering`.
        assert_eq!((d[1].old_no, d[1].new_no), (Some(2), None));
        assert_eq!((d[2].old_no, d[2].new_no), (None, Some(2)));
        assert_eq!((d[3].old_no, d[3].new_no), (Some(3), Some(3)));
    }

    #[test]
    fn empty_old_is_all_additions() {
        let d = lcs_diff("", "x\ny");
        assert_eq!(kinds(&d), "++");
        assert_eq!(stats(&d).additions, 2);
    }

    #[test]
    fn oversized_inputs_fall_back_to_pairwise() {
        let old = "a\n".repeat(2_001);
        let new = format!("b\n{}", "a\n".repeat(2_000));
        let d = lcs_diff(&old, &new);
        // Pairwise: line 1 differs, the rest line up.
        assert_eq!(&kinds(&d)[..3], "-+ ");
    }

    const RAW: &str = "diff --git a/src/a.rs b/src/a.rs\nindex 1..2 100644\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,3 +1,4 @@\n fn main() {\n-    old();\n+    new();\n+    more();\n }\n@@ -12,2 +13,2 @@ fn x()\n a\n b\ndiff --git a/img.png b/img.png\nBinary files a/img.png and b/img.png differ\ndiff --git a/new.txt b/new.txt\nnew file mode 100644\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1,2 @@\n+hello\n+world\n";

    #[test]
    fn parses_multi_file_unified_diff() {
        let files = parse_unified(RAW);
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].display_path(), "src/a.rs");
        assert_eq!(files[0].hunks.len(), 2);
        assert_eq!(files[0].hunks[0].header, "@@ -1,3 +1,4 @@");
        assert_eq!(files[0].hunks[1].header, "@@ -12,2 +13,2 @@");
        assert_eq!(kinds(&files[0].hunks[0].lines), " -++ ");
        let rows = &files[0].hunks[0].lines;
        assert_eq!((rows[0].old_no, rows[0].new_no), (Some(1), Some(1)));
        assert_eq!(rows[1].old_no, Some(2));
        assert_eq!(rows[2].new_no, Some(2));
        assert_eq!(rows[3].new_no, Some(3));
        assert_eq!(files[0].hunks[1].lines[0].old_no, Some(12));
        assert_eq!(files[0].hunks[1].lines[0].new_no, Some(13));
        assert_eq!(
            files[0].stats(),
            DiffStats {
                additions: 2,
                deletions: 1
            }
        );
        assert!(files[1].is_binary);
        assert!(!files[1].is_added() && !files[1].is_deleted());
        assert_eq!(files[2].old_path, None);
        assert!(files[2].is_added() && !files[2].is_deleted());
        assert_eq!(files[2].display_path(), "new.txt");
        assert_eq!(files[2].stats().additions, 2);
    }

    #[test]
    fn deleted_files_have_no_new_path() {
        let files = parse_unified(
            "diff --git a/gone.rs b/gone.rs\ndeleted file mode 100644\n--- a/gone.rs\n+++ /dev/null\n@@ -1,1 +0,0 @@\n-bye\n",
        );
        assert!(files[0].is_deleted());
        assert_eq!(files[0].display_path(), "gone.rs");
        assert_eq!(files[0].stats().deletions, 1);
    }

    #[test]
    fn body_lines_that_look_like_file_headers_stay_in_the_hunk() {
        // Removing "-- note" and adding "++ x" render as "--- note" / "+++ x".
        let files = parse_unified(
            "diff --git a/q.sql b/q.sql\n--- a/q.sql\n+++ b/q.sql\n@@ -1,2 +1,2 @@\n select 1;\n--- note\n+++ x\n",
        );
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].old_path.as_deref(), Some("q.sql"));
        assert_eq!(files[0].new_path.as_deref(), Some("q.sql"));
        let lines = &files[0].hunks[0].lines;
        assert_eq!(kinds(lines), " -+");
        assert_eq!(lines[1].text, "-- note");
        assert_eq!(lines[2].text, "++ x");
    }

    #[test]
    fn hunk_header_ranges() {
        assert_eq!(
            parse_hunk_header("@@ -12,7 +12,9 @@ fn x()"),
            HunkRange {
                old_start: 12,
                old_len: 7,
                new_start: 12,
                new_len: 9
            }
        );
        let short = parse_hunk_header("@@ -5 +5 @@");
        assert_eq!((short.old_len, short.new_len), (1, 1));
        let added = parse_hunk_header("@@ -0,0 +1,2 @@");
        assert_eq!((added.old_start, added.old_len), (0, 0));
    }

    #[test]
    fn hunk_header_function_context_is_not_parsed_as_ranges() {
        // Git appends the enclosing function after the second `@@`; its
        // `-`/`+` tokens are not ranges.
        let want = HunkRange {
            old_start: 12,
            old_len: 7,
            new_start: 12,
            new_len: 9,
        };
        assert_eq!(parse_hunk_header("@@ -12,7 +12,9 @@ fn f() -> u8"), want);
        assert_eq!(parse_hunk_header("@@ -12,7 +12,9 @@ a - b + c"), want);
        assert_eq!(parse_hunk_header("@@ -12,7 +12,9 @@ x -1,2 +3,4"), want);
        let files = parse_unified(
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,2 +1,2 @@ fn f() -> u8 {\n a\n-b\n+c\n",
        );
        assert_eq!(kinds(&files[0].hunks[0].lines), " -+");
        assert_eq!(files[0].hunks[0].lines[1].old_no, Some(2));
    }

    #[test]
    fn no_newline_marker_is_ignored() {
        let files = parse_unified(
            "diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1 +1 @@\n-x\n\\ No newline at end of file\n+y\n\\ No newline at end of file\n",
        );
        assert_eq!(kinds(&files[0].hunks[0].lines), "-+");
    }
}
