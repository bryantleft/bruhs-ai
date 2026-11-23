//! Byte offsets into a private patch file. Memory scales with files, hunks and
//! page boundaries. Source lines are read only for the requested page or citation.
use super::{invalid, storage, DeliveryError, DeliveryResult, GuideSide};
use bruhs_models::logic::diff::{parse_hunk_header, parse_unified, FileDiff, HunkRange};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufRead, BufReader, Seek, SeekFrom},
};

pub(super) const PAGE: usize = 80;
const EXCERPT_BYTES: usize = 65_536;

// Consume a complete line, retaining only its prefix. Generated files may
// contain an entire bundle on one line.
fn line_prefix(
    reader: &mut impl BufRead,
    bytes: &mut Vec<u8>,
    limit: usize,
) -> std::io::Result<usize> {
    bytes.clear();
    let mut size = 0;
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            return Ok(size);
        }
        let newline = chunk.iter().position(|b| *b == b'\n');
        let n = newline.map_or(chunk.len(), |i| i + 1);
        let keep = (limit - bytes.len()).min(n);
        bytes.extend_from_slice(&chunk[..keep]);
        reader.consume(n);
        size += n;
        if newline.is_some() {
            return Ok(size);
        }
    }
}

#[derive(Clone, Copy)]
struct Position {
    byte: u64,
    old: usize,
    new: usize,
}
impl Position {
    fn advance(&mut self, tag: u8) {
        self.old += usize::from(tag != b'+');
        self.new += usize::from(tag != b'-');
    }
    fn number(self, side: GuideSide) -> usize {
        match side {
            GuideSide::Old => self.old,
            GuideSide::New => self.new,
        }
    }
}
pub(super) struct IndexedHunk {
    pub header: String,
    range: HunkRange,
    pages: Vec<Position>,
    pub rows: usize,
}
impl IndexedHunk {
    fn contains(&self, side: GuideSide, start: usize, end: usize) -> bool {
        let (first, count) = match side {
            GuideSide::Old => (self.range.old_start, self.range.old_len),
            GuideSide::New => (self.range.new_start, self.range.new_len),
        };
        start > 0 && end >= start && start >= first && end - first < count
    }
}
pub(super) struct PatchFile {
    pub metadata: FileDiff,
    pub hunks: Vec<IndexedHunk>,
}
pub(super) struct Patch {
    data: File,
    pub files: BTreeMap<String, PatchFile>,
}

impl Patch {
    pub fn open(data: File) -> DeliveryResult<Self> {
        let mut input = BufReader::new(&data);
        let mut files = BTreeMap::new();
        let mut headers = String::new();
        let mut hunks: Vec<IndexedHunk> = Vec::new();
        let mut pos = Position {
            byte: 0,
            old: 0,
            new: 0,
        };
        let (mut old_left, mut new_left) = (0, 0);
        let mut line = Vec::new();
        loop {
            let start = pos.byte;
            let n = line_prefix(&mut input, &mut line, 16_384).map_err(storage)?;
            pos.byte += n as u64;
            if n == 0 || line.starts_with(b"diff --git ") {
                if old_left != 0 || new_left != 0 {
                    return Err(invalid(
                        "The reviewed diff ended before the hunk was complete.",
                    ));
                }
                if !headers.is_empty() {
                    let metadata = parse_unified(&headers)
                        .pop()
                        .ok_or_else(|| invalid("Invalid diff file header."))?;
                    let path = metadata.display_path();
                    if path.is_empty() {
                        return Err(invalid("Missing path in a review diff."));
                    }
                    // Git represents some file type changes as a deletion and
                    // an addition with the same name. Preserve both sets of lines.
                    let entry: &mut PatchFile =
                        files.entry(path.to_owned()).or_insert_with(|| PatchFile {
                            metadata: FileDiff {
                                old_path: None,
                                new_path: None,
                                hunks: Vec::new(),
                                is_binary: false,
                            },
                            hunks: Vec::new(),
                        });
                    if metadata.old_path.is_some() {
                        entry.metadata.old_path = metadata.old_path;
                    }
                    if metadata.new_path.is_some() {
                        entry.metadata.new_path = metadata.new_path;
                    }
                    entry.metadata.is_binary |= metadata.is_binary;
                    entry.hunks.append(&mut hunks);
                    headers.clear();
                }
                if n == 0 {
                    break;
                }
            }
            if headers.is_empty() && !line.starts_with(b"diff --git ") {
                continue;
            }
            if old_left != 0 || new_left != 0 {
                let tag = line[0];
                if tag == b'\\' {
                    continue;
                }
                if !matches!(tag, b' ' | b'+' | b'-')
                    || (tag != b'+' && old_left == 0)
                    || (tag != b'-' && new_left == 0)
                {
                    return Err(invalid("The reviewed patch contains an incomplete hunk."));
                }
                let h = hunks
                    .last_mut()
                    .expect("a hunk owns the remaining line counts");
                if h.rows.is_multiple_of(PAGE) {
                    h.pages.push(Position { byte: start, ..pos });
                }
                h.rows += 1;
                pos.advance(tag);
                old_left -= usize::from(tag != b'+');
                new_left -= usize::from(tag != b'-');
            } else if line.starts_with(b"@@ ") {
                let raw = String::from_utf8_lossy(&line);
                let end = raw[3..]
                    .find(" @@")
                    .ok_or_else(|| invalid("Invalid diff hunk header."))?
                    + 6;
                let header = raw[..end].to_owned();
                let r = parse_hunk_header(&header);
                if r.old_start.checked_add(r.old_len).is_none()
                    || r.new_start.checked_add(r.new_len).is_none()
                {
                    return Err(invalid("Invalid review hunk line range."));
                }
                (pos.old, pos.new) = (r.old_start, r.new_start);
                (old_left, new_left) = (r.old_len, r.new_len);
                hunks.push(IndexedHunk {
                    header,
                    range: r,
                    pages: Vec::new(),
                    rows: 0,
                });
            } else if hunks.is_empty()
                && [
                    b"diff --git ".as_slice(),
                    b"--- ",
                    b"+++ ",
                    b"deleted file mode ",
                    b"new file mode ",
                    b"Binary files ",
                    b"GIT binary patch",
                    b"rename from ",
                    b"rename to ",
                    b"copy from ",
                    b"copy to ",
                ]
                .iter()
                .any(|p| line.starts_with(p))
            {
                headers.push_str(String::from_utf8_lossy(&line).trim_end_matches(['\r', '\n']));
                headers.push('\n');
            }
        }
        Ok(Self { data, files })
    }

    fn visit(
        &self,
        hunk: &IndexedHunk,
        page: usize,
        line_limit: usize,
        mut visitor: impl FnMut(usize, Position, u8, &str, bool) -> DeliveryResult<bool>,
    ) -> DeliveryResult<()> {
        let Some(mut pos) = hunk.pages.get(page).copied() else {
            return Ok(());
        };
        let mut input = BufReader::new(&self.data);
        input.seek(SeekFrom::Start(pos.byte)).map_err(storage)?;
        let mut line = Vec::new();
        let mut row = page * PAGE;
        while row < hunk.rows {
            let count = line_prefix(&mut input, &mut line, line_limit + 2).map_err(storage)?;
            let tag = *line
                .first()
                .ok_or_else(|| invalid("The review patch could not be read."))?;
            if tag == b'\\' {
                continue;
            }
            let text = String::from_utf8_lossy(&line[1..]);
            let text = text.strip_suffix('\n').unwrap_or(&text);
            let text = text.strip_suffix('\r').unwrap_or(text);
            if !visitor(row, pos, tag, text, count > line.len())? {
                break;
            }
            row += 1;
            pos.advance(tag);
        }
        Ok(())
    }

    pub fn manifest(&self, offset: usize) -> Vec<Value> {
        let mut result = Vec::new();
        let mut bytes = 0;
        for (index, (path, file)) in self.files.iter().enumerate().skip(offset).take(PAGE) {
            let hunks: Vec<_> = file.hunks.iter().take(12).map(|h| &h.header).collect();
            let entry = json!({"index":index,"path":path,"hunks":hunks,"total_hunks":file.hunks.len(),"binary":file.metadata.is_binary});
            bytes += entry.to_string().len();
            if bytes > 16_384 && !result.is_empty() {
                break;
            }
            result.push(entry);
        }
        result
    }

    pub fn page(&self, path: &str, index: usize, offset: usize) -> DeliveryResult<Option<Value>> {
        let Some(h) = self
            .files
            .get(path)
            .and_then(|f| f.hunks.get(index))
            .filter(|h| offset < h.rows)
        else {
            return Ok(None);
        };
        let mut lines = Vec::new();
        let mut bytes = 0;
        self.visit(h, offset / PAGE, 2048, |row, pos, tag, text, _| {
            if row >= offset {
                let line = json!([
                    (tag != b'+').then_some(pos.old),
                    (tag != b'-').then_some(pos.new),
                    text.chars().take(512).collect::<String>()
                ]);
                bytes += line.to_string().len();
                if bytes > 16_384 && !lines.is_empty() {
                    return Ok(false);
                }
                lines.push(line);
            }
            Ok(lines.len() < PAGE)
        })?;
        let next = offset + lines.len();
        Ok(Some(
            json!({"path":path,"hunk":index,"offset":offset,"total_lines":h.rows,"next_offset":(next<h.rows).then_some(next),"lines":lines}),
        ))
    }

    fn hunk(&self, path: &str, side: GuideSide, start: usize, end: usize) -> Option<&IndexedHunk> {
        self.files
            .get(path)?
            .hunks
            .iter()
            .find(|h| h.contains(side, start, end))
    }
    pub fn has_line(&self, path: &str, line: usize) -> bool {
        self.hunk(path, GuideSide::New, line, line).is_some()
    }
    pub fn excerpt(
        &self,
        path: &str,
        side: GuideSide,
        start: usize,
        end: usize,
    ) -> Result<String, String> {
        let h = self
            .hunk(path, side, start, end)
            .ok_or_else(|| format!("Choose an excerpt within one changed hunk of {path}."))?;
        let page = h
            .pages
            .partition_point(|p| p.number(side) <= start)
            .saturating_sub(1);
        let mut code = String::new();
        self.visit(h, page, EXCERPT_BYTES, |_, pos, tag, text, clipped| {
            let present = match side {
                GuideSide::Old => tag != b'+',
                GuideSide::New => tag != b'-',
            };
            let n = pos.number(side);
            if present && (start..=end).contains(&n) {
                if clipped || code.len() + text.len() + 1 > EXCERPT_BYTES {
                    return Err(DeliveryError::Invalid(
                        "Choose a shorter code excerpt.".into(),
                    ));
                }
                if n > start {
                    code.push('\n');
                }
                code.push_str(text);
                if n == end {
                    return Ok(false);
                }
            }
            Ok(true)
        })
        .map_err(|e| e.to_string())?;
        Ok(code)
    }
}

pub(super) fn source_lines(file: File, start: usize) -> DeliveryResult<Vec<String>> {
    let mut input = BufReader::new(file);
    let mut line = Vec::new();
    let mut page = Vec::new();
    let mut bytes = 0;
    for i in 1.. {
        if line_prefix(&mut input, &mut line, 2048).map_err(storage)? == 0 {
            break;
        }
        if i >= start {
            let text: String = String::from_utf8_lossy(&line)
                .trim_end_matches(['\r', '\n'])
                .chars()
                .take(512)
                .collect();
            bytes += json!(text).to_string().len();
            if bytes > 16_384 && !page.is_empty() {
                break;
            }
            page.push(text);
            if page.len() == PAGE {
                break;
            }
        }
    }
    Ok(page)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn pages_seek_across_a_large_replacement_and_read_both_sides_after_a_type_change() {
        let mut file = tempfile::tempfile().unwrap();
        writeln!(file,"diff --git a/search b/search\ndeleted file mode 100644\n--- a/search\n+++ /dev/null\n@@ -1,6000 +0,0 @@").unwrap();
        for i in 1..=6000 {
            writeln!(file, "-old {i} {}", "a".repeat(100)).unwrap();
        }
        writeln!(file,"\\ No newline at end of file\ndiff --git a/search b/search\nnew file mode 120000\n--- /dev/null\n+++ b/search\n@@ -0,0 +1,6000 @@").unwrap();
        for i in 1..=6000 {
            writeln!(file, "+new {i} {}", "z".repeat(100)).unwrap();
        }
        file.rewind().unwrap();
        let patch = Patch::open(file).unwrap();
        assert_eq!(patch.files.len(), 1);
        assert_eq!(patch.files["search"].hunks.len(), 2);
        assert!(!patch.files["search"].metadata.is_deleted());
        let page = patch.page("search", 1, 5943).unwrap().unwrap();
        assert_eq!(page["lines"][0][0], Value::Null);
        assert_eq!(page["lines"][0][1], 5944);
        assert_eq!(
            patch.excerpt("search", GuideSide::New, 5944, 5945).unwrap(),
            format!("new 5944 {}\nnew 5945 {}", "z".repeat(100), "z".repeat(100))
        );
        assert_eq!(
            patch.excerpt("search", GuideSide::Old, 6000, 6000).unwrap(),
            format!("old 6000 {}", "a".repeat(100))
        );
    }

    #[test]
    fn incomplete_hunks_never_become_citable_and_minified_lines_can_be_paged_past() {
        let mut incomplete = tempfile::tempfile().unwrap();
        writeln!(incomplete, "diff --git a/a b/a\n@@ -0,0 +1,2 @@\n+first").unwrap();
        incomplete.rewind().unwrap();
        assert!(Patch::open(incomplete).is_err());
        let mut file = tempfile::tempfile().unwrap();
        writeln!(
            file,
            "diff --git a/a b/a\n@@ -0,0 +1,2 @@\n+{}\n+last",
            "x".repeat(2 * 1024 * 1024)
        )
        .unwrap();
        file.rewind().unwrap();
        let patch = Patch::open(file).unwrap();
        let page = patch.page("a", 0, 0).unwrap().unwrap();
        assert_eq!(page["lines"][0][2].as_str().unwrap().len(), 512);
        assert!(patch.excerpt("a", GuideSide::New, 1, 1).is_err());
        assert_eq!(patch.excerpt("a", GuideSide::New, 2, 2).unwrap(), "last");
    }
}
