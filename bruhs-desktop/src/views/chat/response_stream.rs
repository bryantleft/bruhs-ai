//! Reveal completed paragraphs without reparsing the preceding turn for every token.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Mode {
    Turn,
    Paragraph,
    #[serde(other)]
    #[default]
    Live,
}
impl Mode {
    pub fn key(self) -> &'static str {
        match self {
            Self::Turn => "turn",
            Self::Paragraph => "paragraph",
            Self::Live => "live",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "turn" => Some(Self::Turn),
            "paragraph" => Some(Self::Paragraph),
            "live" => Some(Self::Live),
            _ => None,
        }
    }
}

#[derive(Default)]
pub(super) struct ParagraphProgress {
    scanned: usize,
    line_start: usize,
    visible: usize,
    fence: Option<(u8, usize)>,
}
impl ParagraphProgress {
    pub fn advance(&mut self, text: &str) {
        for (offset, _) in text[self.scanned..].match_indices('\n') {
            let end = self.scanned + offset + 1;
            let line = text[self.line_start..end].trim_end_matches(['\r', '\n']);
            let stripped = line.trim_start_matches(' ');
            let delimiter = stripped
                .as_bytes()
                .first()
                .copied()
                .filter(|b| matches!(b, b'`' | b'~'));
            let run = delimiter.map_or(0, |c| stripped.bytes().take_while(|x| *x == c).count());
            let valid = line.len() - stripped.len() <= 3 && run >= 3;
            match self.fence {
                Some((fence, width))
                    if valid
                        && delimiter == Some(fence)
                        && run >= width
                        && stripped[run..].trim().is_empty() =>
                {
                    self.visible = end;
                    self.fence = None;
                }
                Some(_) => {}
                None if valid && (delimiter != Some(b'`') || !stripped[run..].contains('`')) => {
                    self.visible = self.line_start;
                    self.fence = delimiter.map(|c| (c, run));
                }
                None if line.trim().is_empty() => self.visible = end,
                None => {}
            }
            self.line_start = end;
        }
        self.scanned = text.len();
    }
    pub fn visible<'a>(&self, text: &'a str, mode: Mode, streaming: bool) -> &'a str {
        if !streaming {
            return text;
        }
        match mode {
            Mode::Live => text,
            Mode::Turn => "",
            Mode::Paragraph => text.get(..self.visible).unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Mode, ParagraphProgress};
    #[test]
    fn chunked_unicode_paragraphs_and_fences_only_reveal_complete_blocks() {
        let mut p = ParagraphProgress::default();
        let mut source = String::new();
        let para = "Café 🍊\n\n";
        let code = "~~~rust\nlet a = 0;\n\n~~~\n";
        for (chunk, shown) in [
            ("Café", ""),
            (" 🍊\n", ""),
            ("\n", para),
            ("~~~rust\nlet a", para),
            (" = 0;\n\n~~", para),
        ] {
            source.push_str(chunk);
            p.advance(&source);
            assert_eq!(p.visible(&source, Mode::Paragraph, true), shown);
        }
        source.push_str("~\npartial");
        p.advance(&source);
        assert_eq!(
            p.visible(&source, Mode::Paragraph, true),
            format!("{para}{code}")
        );
        assert_eq!(p.visible(&source, Mode::Turn, true), "");
        assert_eq!(p.visible(&source, Mode::Turn, false), source);
    }
}
