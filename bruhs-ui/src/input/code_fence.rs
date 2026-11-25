//! Fenced Markdown metadata and syntax colors shared by the editor and its language menu.
use super::markdown::MarkdownEdit;
use crate::theme;
use bruhs_models::syntax::{self, TokenKind};
use std::ops::Range;

pub(super) struct Language {
    pub id: &'static str,
    pub label: &'static str,
    pub aliases: &'static str,
}

const fn entry(id: &'static str, label: &'static str, aliases: &'static str) -> Language {
    Language { id, label, aliases }
}

pub(super) const LANGUAGES: &[Language] = &[
    entry("", "Plain text", "text txt plaintext"),
    entry("asm", "Assembly", "assembly nasm x86 s"),
    entry("bash", "Bash", "sh shell zsh"),
    entry("c", "C", ""),
    entry("cpp", "C++", "c++ cc cxx"),
    entry("csharp", "C#", "cs c#"),
    entry("css", "CSS", ""),
    entry("dart", "Dart", ""),
    entry("diff", "Diff", ""),
    entry("dockerfile", "Dockerfile", "docker"),
    entry("elixir", "Elixir", "ex exs"),
    entry("go", "Go", ""),
    entry("graphql", "GraphQL", "gql"),
    entry("haskell", "Haskell", "hs"),
    entry("html", "HTML", ""),
    entry("java", "Java", ""),
    entry("javascript", "JavaScript", "js mjs cjs"),
    entry("json", "JSON", ""),
    entry("jsx", "JSX", "javascriptreact"),
    entry("kotlin", "Kotlin", "kt kts"),
    entry("lua", "Lua", ""),
    entry("markdown", "Markdown", "md"),
    entry("php", "PHP", ""),
    entry("powershell", "PowerShell", "ps1 pwsh"),
    entry("python", "Python", "py"),
    entry("r", "R", ""),
    entry("ruby", "Ruby", "rb"),
    entry("rust", "Rust", "rs"),
    entry("scala", "Scala", ""),
    entry("scss", "SCSS", ""),
    entry("sql", "SQL", ""),
    entry("svelte", "Svelte", ""),
    entry("swift", "Swift", ""),
    entry("toml", "TOML", ""),
    entry("tsx", "TSX", "typescriptreact"),
    entry("typescript", "TypeScript", "ts mts cts"),
    entry("vue", "Vue", ""),
    entry("xml", "XML", ""),
    entry("yaml", "YAML", "yml"),
    entry("zig", "Zig", ""),
];

pub(super) fn language(tag: &str) -> Option<&'static Language> {
    LANGUAGES.iter().find(|l| {
        l.id.eq_ignore_ascii_case(tag)
            || l.aliases
                .split_whitespace()
                .any(|a| a.eq_ignore_ascii_case(tag))
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Fence {
    pub header: Range<usize>,
    pub tag: Range<usize>,
    pub body: Range<usize>,
    pub closing: Range<usize>,
}

impl Fence {
    pub fn parse(source: &str, range: Range<usize>) -> Option<Self> {
        let raw = &source[range.clone()];
        let (first, _) = raw.split_once('\n')?;
        let marker = first.as_bytes().first().copied()?;
        if !matches!(marker, b'`' | b'~') {
            return None;
        }
        let width = first.bytes().take_while(|b| *b == marker).count();
        let info = &first[width..];
        let trimmed = info.trim_start();
        let tag_start = range.start + first.len() - trimmed.len();
        let tag_len = trimmed.split_whitespace().next().map_or(0, str::len);
        let header_end = range.start + first.len();
        let end = raw.trim_end_matches(['\r', '\n']);
        let last = end.rsplit('\n').next()?;
        let mut closing = range.end..range.end;
        let quoted = source[..range.start]
            .rsplit('\n')
            .next()
            .is_some_and(|prefix| prefix.contains('>'));
        let line = if quoted {
            last.trim_start_matches([' ', '\t', '>'])
        } else {
            last.trim_start()
        };
        let delimiter = line.trim_end();
        if end.len() > first.len()
            && delimiter.len() >= width
            && delimiter.bytes().all(|b| b == marker)
        {
            closing = range.start + end.len() - last.len()..range.end;
        }
        Some(Self {
            header: range.start..header_end,
            tag: tag_start..tag_start + tag_len,
            body: header_end + 1..closing.start,
            closing,
        })
    }
}

impl Fence {
    pub fn language_edit<'a>(&self, source: &'a str, value: &'a str) -> (Range<usize>, &'a str) {
        let current = &source[self.tag.clone()];
        if language(current).is_some_and(|l| l.id == value) {
            return (self.tag.clone(), current);
        }
        if value.is_empty() {
            if source[self.tag.end..self.header.end].trim().is_empty() {
                let marker_len = opening_marker(&source[self.header.clone()]).map_or(0, str::len);
                return (self.header.start + marker_len..self.header.end, "");
            }
            // Keep a plain-text identifier so the remaining metadata stays metadata.
            return (self.tag.clone(), "text");
        }
        (self.tag.clone(), value)
    }

    fn content_end(&self, source: &str) -> usize {
        self.body.end
            - usize::from(!self.closing.is_empty() && source[self.body.clone()].ends_with('\n'))
    }
    pub fn covers(&self, source: &str, selection: &Range<usize>) -> bool {
        self.body.start <= selection.start && selection.end <= self.content_end(source)
    }
}

pub(super) fn newline(
    source: &str,
    selection: Range<usize>,
    fences: &[Fence],
) -> Option<MarkdownEdit> {
    let at = selection.start;
    let line_start = source[..at].rfind('\n').map_or(0, |p| p + 1);
    let before = &source[line_start..at];
    if let Some(fence) = fences.iter().find(|f| f.covers(source, &selection)) {
        let end = fence.content_end(source);
        let content = &source[fence.body.start..end];
        // Two empty lines followed by Enter return to a paragraph after the block.
        if selection.is_empty() && at == end {
            if let Some((earlier, last)) = content.rsplit_once('\n') {
                if let Some((kept, previous)) = earlier.rsplit_once('\n') {
                    if last.trim().is_empty() && previous.trim().is_empty() {
                        let cut = fence.body.start + kept.len();
                        let closing = if fence.closing.is_empty() {
                            opening_marker(&source[fence.header.clone()])?
                        } else {
                            source[fence.closing.clone()].trim_end_matches('\n')
                        };
                        let separator = if kept.is_empty() { "" } else { "\n" };
                        let text = format!("{separator}{closing}\n");
                        let caret = cut + text.len();
                        return Some(MarkdownEdit::insert(cut..fence.closing.end, text, caret));
                    }
                }
            }
        }
        let indent = &before[..before.len() - before.trim_start_matches([' ', '\t']).len()];
        let mut text = format!("\n{indent}");
        let caret = at + text.len();
        if fence.body.is_empty() && !fence.closing.is_empty() {
            text.push('\n');
        }
        return Some(MarkdownEdit::insert(selection, text, caret));
    }
    if !selection.is_empty() || (!source[at..].is_empty() && !source[at..].starts_with('\n')) {
        return None;
    }
    if let Some(fence) = fences
        .iter()
        .find(|f| f.closing.start == line_start && !f.closing.is_empty())
    {
        // Typing a closing delimiter inside an already closed block leaves its
        // original closer directly underneath the newly typed line.
        let tail = &source[at..];
        if let Some(after) = tail.strip_prefix('\n') {
            let next = after.lines().next().unwrap_or("");
            let marker = opening_marker(&source[fence.header.clone()])?;
            if next.trim_end() == marker {
                let end = at + 1 + next.len();
                let replacement = if source[end..].starts_with('\n') {
                    ""
                } else {
                    "\n"
                };
                return Some(MarkdownEdit::insert(at..end, replacement.into(), at + 1));
            }
            return Some(MarkdownEdit::insert(at..at, String::new(), at + 1));
        }
        return Some(MarkdownEdit::insert(at..at, "\n".into(), at + 1));
    }
    let marker = opening_marker(before)?;
    if marker.starts_with('`') && before[marker.len()..].contains('`') {
        return None;
    }
    Some(MarkdownEdit::insert(at..at, format!("\n{marker}"), at + 1))
}

fn opening_marker(line: &str) -> Option<&str> {
    let first = line.as_bytes().first().copied()?;
    if !matches!(first, b'`' | b'~') {
        return None;
    }
    let count = line.bytes().take_while(|b| *b == first).count();
    (count >= 3).then(|| &line[..count])
}

pub(super) fn indent(
    source: &str,
    selection: Range<usize>,
    fences: &[Fence],
    outdent: bool,
) -> Option<MarkdownEdit> {
    let fence = fences.iter().find(|f| f.covers(source, &selection))?;
    if selection.is_empty() && !outdent {
        let text = if fence.body.is_empty() && !fence.closing.is_empty() {
            "  \n"
        } else {
            "  "
        };
        return Some(MarkdownEdit::insert(
            selection.clone(),
            text.into(),
            selection.start + 2,
        ));
    }
    let start = source[..selection.start].rfind('\n').map_or(0, |p| p + 1);
    let last = selection
        .end
        .saturating_sub(usize::from(!selection.is_empty()))
        .max(start);
    let end = source.as_bytes()[last..]
        .iter()
        .position(|b| *b == b'\n')
        .map_or(source.len(), |i| last + i)
        .min(fence.content_end(source));
    let mut text = String::new();
    let mut shifts = Vec::new();
    let mut pos = start;
    for (i, line) in source[start..end].split('\n').enumerate() {
        if i > 0 {
            text.push('\n');
        }
        let remove = if outdent {
            if line.starts_with("  ") {
                2
            } else {
                usize::from(line.starts_with([' ', '\t']))
            }
        } else {
            0
        };
        let added = if !outdent && !line.is_empty() {
            text.push_str("  ");
            2
        } else {
            0
        };
        text.push_str(&line[remove..]);
        shifts.push((pos, remove, added));
        pos += line.len() + 1;
    }
    let shift = |offset: usize| {
        shifts
            .iter()
            .filter(|(at, _, _)| *at <= offset)
            .fold(offset, |value, (at, remove, add)| {
                value + add - remove.min(&(offset - at))
            })
    };
    Some(MarkdownEdit {
        range: start..end,
        text,
        selection: shift(selection.start)..shift(selection.end),
    })
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) enum TokenColor {
    Comment,
    String,
    Keyword,
    Constant,
    Name,
    Type,
}
impl TokenColor {
    pub fn color(self) -> gpui::Hsla {
        match self {
            Self::Comment => theme::rambutan_500(),
            Self::String => theme::pandan_400(),
            Self::Keyword => theme::mangosteen_400(),
            Self::Constant => theme::persimmon_400(),
            Self::Name => theme::blueberry_400(),
            Self::Type => theme::durian_400(),
        }
    }
}

impl From<TokenKind> for TokenColor {
    fn from(kind: TokenKind) -> Self {
        match kind {
            TokenKind::Comment => Self::Comment,
            TokenKind::String => Self::String,
            TokenKind::Keyword => Self::Keyword,
            TokenKind::Constant => Self::Constant,
            TokenKind::Name => Self::Name,
            TokenKind::Type => Self::Type,
        }
    }
}

pub(super) type Token = (Range<usize>, TokenColor);

/// Run on the background executor. Each block starts its own scanner state,
/// preserving multiline strings and comments without leaking into the next fence.
pub(super) fn highlight(source: &str, fences: &[Fence]) -> Vec<Token> {
    let mut raw = Vec::new();
    for fence in fences {
        let tag = &source[fence.tag.clone()];
        let Some(lang) = language(tag) else {
            continue;
        };
        if lang.id.is_empty() || fence.body.is_empty() {
            continue;
        }
        syntax::highlight_code(
            lang.id,
            &source[fence.body.clone()],
            fence.body.start,
            &mut raw,
        );
    }
    raw.into_iter()
        .map(|(range, kind)| (range, TokenColor::from(kind)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::markdown::Projection;
    #[test]
    fn language_choices_and_aliases_use_native_grammars() {
        assert_eq!(LANGUAGES.len(), 40);
        for lang in &LANGUAGES[1..] {
            assert!(
                syntax::supports(lang.id),
                "missing highlighter for {}",
                lang.label
            );
        }
        let mut colors = Vec::new();
        syntax::highlight_code("powershell", "if ($x) { Get-Item '猫' }", 0, &mut colors);
        assert!(colors.iter().any(|(_, color)| *color == TokenKind::Keyword));
        assert!(colors.iter().any(|(_, color)| *color == TokenKind::Name));
        assert!(colors.iter().any(|(_, color)| *color == TokenKind::String));
        assert_eq!(language("s").unwrap().id, "asm");
        assert_eq!(language("RS").unwrap().id, "rust");
        assert_eq!(language("js").unwrap().id, "javascript");
    }

    #[test]
    fn indentation_projection_and_language_edits_preserve_source_boundaries() {
        let source = "~~~~  rs  extra\n猫\n  友\n~~~~";
        let projection = Projection::new(source, None);
        let fence = &projection.fences[0];
        let (tag, plain) = fence.language_edit(source, "");
        assert_eq!(plain, "text");
        assert_eq!(&source[tag], "rs");
        assert_eq!(fence.language_edit(source, "rust").1, "rs");
        for ch in ['猫', '友'] {
            let at = source.find(ch).unwrap();
            let drawn = projection.to_display(at);
            assert_eq!(projection.text[drawn..].chars().next(), Some(ch));
            assert_eq!(projection.to_value(drawn), at);
        }
        let selected = source.find('猫').unwrap()..source.find('友').unwrap() + '友'.len_utf8();
        let edit = indent(source, selected.clone(), &projection.fences, false).unwrap();
        let mut next = source.to_owned();
        next.replace_range(edit.range, &edit.text);
        assert!(next.contains("  猫\n    友"));
        let unindent = indent(
            &next,
            edit.selection,
            &Projection::new(&next, None).fences,
            true,
        )
        .unwrap();
        next.replace_range(unindent.range, &unindent.text);
        assert_eq!(next, source);
        assert_eq!(unindent.selection, selected);

        let source = "```py\nfield\n```\n```";
        let cursor = source.find("\n```\n").unwrap() + 4;
        let edit = newline(
            source,
            cursor..cursor,
            &Projection::new(source, None).fences,
        )
        .unwrap();
        let mut next = source.to_owned();
        next.replace_range(edit.range, &edit.text);
        assert_eq!(next, "```py\nfield\n```\n");
    }
    #[test]
    fn fenced_source_ranges_keep_metadata_and_multiline_colors() {
        let source = "~~~~rs extra\n/* 猫\nstill */ let x = 42;\n~~~\n~~~~\nnext";
        let projection = Projection::new(source, None);
        let fence = &projection.fences[0];
        assert_eq!(&source[fence.tag.clone()], "rs");
        assert_eq!(
            &source[fence.body.clone()],
            "/* 猫\nstill */ let x = 42;\n~~~\n"
        );
        assert_eq!(source[fence.closing.clone()].trim(), "~~~~");
        let tokens = highlight(source, &projection.fences);
        for (pattern, color) in [("still", TokenColor::Comment), ("42", TokenColor::Constant)] {
            let position = source.find(pattern).unwrap();
            assert!(tokens
                .iter()
                .any(|(range, c)| range.contains(&position) && *c == color));
        }
        assert!(!projection.text.contains("~~~~"));
        let unfinished = Projection::new("```py\nprint('猫')", None);
        assert!(unfinished.fences[0].closing.is_empty());
    }
}
