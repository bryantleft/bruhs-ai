//! Fast, zero-dependency pure-Rust TeX math → SVG renderer.
//!
//! Replaces `mathjax-svg-rs` (which embedded `boa_engine` + a minified MathJax
//! JS bundle) with a recursive-descent TeX parser, Knuth-inspired bounding-box
//! layout engine, and crisp vector SVG emitter using `currentColor`.

use std::fmt::Write as _;

use crate::num::ToFloatLossy as _;

/// Font size and style options for TeX math rendering.
#[derive(Clone, Copy, Debug)]
pub struct MathOptions {
    pub font_size: f32,
    pub display: bool,
}

/// Rendered TeX math SVG along with its layout box metrics for baseline alignment.
#[derive(Clone, Debug)]
pub struct MathRender {
    pub svg: String,
    pub width: f32,
    pub height: f32,
    pub baseline: f32,
}

/// Parse `source` as TeX math and render it to a self-contained SVG string.
pub fn render_tex(source: &str, options: &MathOptions) -> Result<String, String> {
    render_tex_with_metrics(source, options).map(|rendered| rendered.svg)
}

/// Parse `source` as TeX math and render it to a self-contained SVG with layout metrics.
pub fn render_tex_with_metrics(source: &str, options: &MathOptions) -> Result<MathRender, String> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return Err("empty math expression".to_string());
    }
    let tokens = tokenize(trimmed)?;
    let mut parser = Parser::new(&tokens);
    let ast = parser.parse_sequence(StopCondition::Eof)?;
    if ast.is_empty() {
        return Err("empty math expression".to_string());
    }

    let style = if options.display {
        MathStyle::Display
    } else {
        MathStyle::Text
    };
    let box_node = layout_row(&ast, options.font_size, style, FontVariant::Normal);

    let pad_x = (options.font_size * 0.25).max(3.0);
    let pad_y = (options.font_size * 0.2).max(2.5);
    let width = (box_node.width + pad_x * 2.0).max(8.0);
    let height = (box_node.ascent + box_node.descent + pad_y * 2.0).max(options.font_size);
    let baseline = pad_y + box_node.ascent;

    let mut svg = String::with_capacity(512 + box_node.elements.len() * 64);
    let _ = write!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.1}\" height=\"{height:.1}\" viewBox=\"0 0 {width:.1} {height:.1}\" role=\"img\">"
    );
    emit_box(&box_node, pad_x, baseline, &mut svg);
    svg.push_str("</svg>");
    Ok(MathRender {
        svg,
        width,
        height,
        baseline,
    })
}

// ---------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    Command(String),
    Char(char),
    LBrace,
    RBrace,
    Sup,
    Sub,
    Amp,
    Newline,
    Space,
}

fn tokenize(input: &str) -> Result<Vec<Tok>, String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        let ch = chars[i];
        match ch {
            '%' => {
                // TeX comment to end of line
                while i < len && chars[i] != '\n' {
                    i += 1;
                }
            }
            '\\' => {
                i += 1;
                if i >= len {
                    return Err("trailing backslash in TeX expression".to_string());
                }
                let next = chars[i];
                if next == '\\' {
                    tokens.push(Tok::Newline);
                    i += 1;
                } else if next.is_ascii_alphabetic() {
                    let start = i;
                    while i < len && chars[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    let name: String = chars[start..i].iter().collect();
                    tokens.push(Tok::Command(name));
                    // TeX consumes whitespace immediately following alphabetic control words
                    while i < len && chars[i].is_ascii_whitespace() {
                        i += 1;
                    }
                } else {
                    tokens.push(Tok::Command(next.to_string()));
                    i += 1;
                }
            }
            '{' => {
                tokens.push(Tok::LBrace);
                i += 1;
            }
            '}' => {
                tokens.push(Tok::RBrace);
                i += 1;
            }
            '^' => {
                tokens.push(Tok::Sup);
                i += 1;
            }
            '_' => {
                tokens.push(Tok::Sub);
                i += 1;
            }
            '&' => {
                tokens.push(Tok::Amp);
                i += 1;
            }
            c if c.is_ascii_whitespace() => {
                if !matches!(tokens.last(), Some(Tok::Space) | None) {
                    tokens.push(Tok::Space);
                }
                while i < len && chars[i].is_ascii_whitespace() {
                    i += 1;
                }
            }
            c => {
                tokens.push(Tok::Char(c));
                i += 1;
            }
        }
    }
    Ok(tokens)
}

// ---------------------------------------------------------------------------
// AST & Parser
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FontVariant {
    Normal,
    Roman,
    Bold,
    Italic,
    Sans,
    Mono,
    Calligraphic,
    Blackboard,
    Fraktur,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AtomKind {
    Ord,
    Op,
    Bin,
    Rel,
    Open,
    Close,
    Punct,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LimitsMode {
    Auto,
    Limits,
    NoLimits,
}

#[derive(Clone, Debug)]
enum Node {
    Symbol {
        text: String,
        kind: AtomKind,
        italic_if_normal: bool,
        large_op: bool,
    },
    Text {
        content: String,
        variant: FontVariant,
    },
    Space(f32),
    Group(Vec<Node>),
    Styled {
        variant: FontVariant,
        body: Vec<Node>,
    },
    Colored {
        color: String,
        body: Vec<Node>,
    },
    SizedStyle {
        style: MathStyle,
        body: Vec<Node>,
    },
    Scripts {
        base: Box<Node>,
        sub: Option<Box<Node>>,
        sup: Option<Box<Node>>,
        limits: LimitsMode,
    },
    Frac {
        num: Vec<Node>,
        den: Vec<Node>,
        rule_thickness: f32,
        force_style: Option<MathStyle>,
    },
    Sqrt {
        index: Option<Vec<Node>>,
        radicand: Vec<Node>,
    },
    Delimited {
        left: String,
        right: String,
        body: Vec<Node>,
    },
    ScaledDelim {
        scale: f32,
        delim: String,
    },
    Accent {
        kind: AccentKind,
        body: Vec<Node>,
    },
    OverUnder {
        over: bool,
        script: Vec<Node>,
        base: Vec<Node>,
    },
    Boxed(Vec<Node>),
    Matrix {
        left: String,
        right: String,
        rows: Vec<Vec<Vec<Node>>>,
        col_align: Vec<Align>,
        small: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AccentKind {
    Hat,
    WideHat,
    Bar,
    Overline,
    Underline,
    Tilde,
    WideTilde,
    Vec,
    Dot,
    DDot,
    Overbrace,
    Underbrace,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StopCondition<'a> {
    Eof,
    RBrace,
    RightDelim,
    EndEnv(&'a str),
}

struct Parser<'a> {
    tokens: &'a [Tok],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(tokens: &'a [Tok]) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> Option<&'a Tok> {
        self.tokens.get(self.pos)
    }

    fn skip_spaces(&mut self) {
        while matches!(self.peek(), Some(Tok::Space)) {
            self.pos += 1;
        }
    }

    fn parse_sequence(&mut self, stop: StopCondition<'_>) -> Result<Vec<Node>, String> {
        let mut items = Vec::new();
        while let Some(tok) = self.peek() {
            match (stop, tok) {
                (StopCondition::RBrace, Tok::RBrace) => {
                    self.pos += 1;
                    return Ok(items);
                }
                (StopCondition::RightDelim, Tok::Command(cmd)) if cmd == "right" => {
                    return Ok(items);
                }
                (StopCondition::EndEnv(_), Tok::Command(cmd)) if cmd == "end" => {
                    return Ok(items);
                }
                (StopCondition::EndEnv(_), Tok::Amp | Tok::Newline) => {
                    return Ok(items);
                }
                (StopCondition::Eof, Tok::RBrace) => {
                    return Err("unmatched '}'".to_string());
                }
                (StopCondition::RightDelim, Tok::RBrace) => {
                    return Err("unmatched '}' inside \\left ... \\right".to_string());
                }
                (StopCondition::EndEnv(env), Tok::RBrace) => {
                    return Err(format!("unmatched '}}' inside \\begin{{{env}}}"));
                }
                (_, Tok::Space) => {
                    self.pos += 1;
                }
                (_, Tok::Command(cmd)) if cmd == "over" || cmd == "choose" => {
                    let rule = if cmd == "over" { 1.0 } else { 0.0 };
                    let is_choose = cmd == "choose";
                    self.pos += 1;
                    let num = std::mem::take(&mut items);
                    let den = self.parse_sequence(stop)?;
                    let frac = Node::Frac {
                        num,
                        den,
                        rule_thickness: rule,
                        force_style: None,
                    };
                    if is_choose {
                        items.push(Node::Delimited {
                            left: "(".to_string(),
                            right: ")".to_string(),
                            body: vec![frac],
                        });
                    } else {
                        items.push(frac);
                    }
                    return Ok(items);
                }
                _ => {
                    let atom = self.parse_scripted_atom()?;
                    items.push(atom);
                }
            }
        }
        match stop {
            StopCondition::Eof => Ok(items),
            StopCondition::RBrace => Err("unclosed '{'".to_string()),
            StopCondition::RightDelim => Err("missing \\right delimiter".to_string()),
            StopCondition::EndEnv(env) => Err(format!("unclosed \\begin{{{env}}}")),
        }
    }

    fn parse_scripted_atom(&mut self) -> Result<Node, String> {
        self.skip_spaces();
        let mut base = match self.peek() {
            Some(Tok::Sub | Tok::Sup) => Node::Group(Vec::new()),
            _ => self.parse_atom()?,
        };
        let mut sub: Option<Box<Node>> = None;
        let mut sup: Option<Box<Node>> = None;
        let mut limits = LimitsMode::Auto;

        loop {
            self.skip_spaces();
            match self.peek() {
                Some(Tok::Command(cmd)) if cmd == "limits" => {
                    self.pos += 1;
                    limits = LimitsMode::Limits;
                }
                Some(Tok::Command(cmd)) if cmd == "nolimits" => {
                    self.pos += 1;
                    limits = LimitsMode::NoLimits;
                }
                Some(Tok::Sub) => {
                    self.pos += 1;
                    let arg = self.parse_single_arg_node()?;
                    sub = Some(Box::new(arg));
                }
                Some(Tok::Sup) => {
                    self.pos += 1;
                    let arg = self.parse_single_arg_node()?;
                    sup = Some(Box::new(arg));
                }
                Some(Tok::Char('\'')) => {
                    self.pos += 1;
                    let prime = Node::Symbol {
                        text: "′".to_string(),
                        kind: AtomKind::Ord,
                        italic_if_normal: false,
                        large_op: false,
                    };
                    sup = Some(match sup {
                        None => Box::new(prime),
                        Some(prev) => Box::new(Node::Group(vec![*prev, prime])),
                    });
                }
                _ => break,
            }
        }

        if sub.is_some() || sup.is_some() {
            base = Node::Scripts {
                base: Box::new(base),
                sub,
                sup,
                limits,
            };
        }
        Ok(base)
    }

    fn parse_single_arg_node(&mut self) -> Result<Node, String> {
        self.skip_spaces();
        if matches!(self.peek(), Some(Tok::LBrace)) {
            self.pos += 1;
            Ok(Node::Group(self.parse_sequence(StopCondition::RBrace)?))
        } else {
            self.parse_atom()
        }
    }

    fn parse_group_arg(&mut self) -> Result<Vec<Node>, String> {
        self.skip_spaces();
        if matches!(self.peek(), Some(Tok::LBrace)) {
            self.pos += 1;
            self.parse_sequence(StopCondition::RBrace)
        } else {
            Ok(vec![self.parse_atom()?])
        }
    }

    fn parse_raw_text_arg(&mut self) -> Result<String, String> {
        self.skip_spaces();
        if !matches!(self.peek(), Some(Tok::LBrace)) {
            return match self.peek() {
                Some(Tok::Char(c)) => {
                    let ch = *c;
                    self.pos += 1;
                    Ok(ch.to_string())
                }
                Some(Tok::Command(cmd)) => {
                    let s = cmd.clone();
                    self.pos += 1;
                    Ok(s)
                }
                _ => Err("expected text argument".to_string()),
            };
        }
        self.pos += 1;
        let mut depth = 1usize;
        let mut out = String::new();
        while let Some(tok) = self.peek() {
            self.pos += 1;
            match tok {
                Tok::LBrace => {
                    depth += 1;
                    out.push('{');
                }
                Tok::RBrace => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(out);
                    }
                    out.push('}');
                }
                Tok::Space => out.push(' '),
                Tok::Char(c) => out.push(*c),
                Tok::Amp => out.push('&'),
                Tok::Sub => out.push('_'),
                Tok::Sup => out.push('^'),
                Tok::Newline => out.push(' '),
                Tok::Command(cmd) => {
                    if let Some((sym, _, _)) = lookup_symbol(cmd) {
                        out.push_str(sym);
                    } else if cmd.len() == 1 {
                        out.push_str(cmd);
                    } else {
                        out.push('\\');
                        out.push_str(cmd);
                    }
                }
            }
        }
        Err("unclosed '{' in text argument".to_string())
    }

    fn parse_optional_bracket_arg(&mut self) -> Result<Option<Vec<Node>>, String> {
        self.skip_spaces();
        if !matches!(self.peek(), Some(Tok::Char('['))) {
            return Ok(None);
        }
        self.pos += 1;
        let mut nodes = Vec::new();
        while let Some(tok) = self.peek() {
            if matches!(tok, Tok::Char(']')) {
                self.pos += 1;
                return Ok(Some(nodes));
            }
            if matches!(tok, Tok::Space) {
                self.pos += 1;
                continue;
            }
            nodes.push(self.parse_scripted_atom()?);
        }
        Err("unclosed '[' optional argument".to_string())
    }

    fn parse_delimiter(&mut self) -> Result<String, String> {
        self.skip_spaces();
        let Some(tok) = self.peek() else {
            return Err("expected delimiter".to_string());
        };
        let delim = match tok {
            Tok::Char(c @ ('(' | ')' | '[' | ']' | '|' | '/' | '.')) => {
                self.pos += 1;
                c.to_string()
            }
            Tok::Command(cmd) => {
                let mapped = match cmd.as_str() {
                    "{" | "lbrace" => "{",
                    "}" | "rbrace" => "}",
                    "|" | "Vert" | "lVert" | "rVert" => "‖",
                    "vert" | "lvert" | "rvert" => "|",
                    "langle" => "⟨",
                    "rangle" => "⟩",
                    "lfloor" => "⌊",
                    "rfloor" => "⌋",
                    "lceil" => "⌈",
                    "rceil" => "⌉",
                    "backslash" => "\\",
                    "uparrow" => "↑",
                    "downarrow" => "↓",
                    "updownarrow" => "↕",
                    _ => return Err(format!("unsupported delimiter \\{cmd}")),
                };
                self.pos += 1;
                mapped.to_string()
            }
            other => return Err(format!("expected delimiter, found {other:?}")),
        };
        Ok(delim)
    }

    fn parse_atom(&mut self) -> Result<Node, String> {
        self.skip_spaces();
        let Some(tok) = self.peek() else {
            return Err("unexpected end of TeX expression".to_string());
        };
        match tok {
            Tok::LBrace => {
                self.pos += 1;
                Ok(Node::Group(self.parse_sequence(StopCondition::RBrace)?))
            }
            Tok::Char(c) => {
                let ch = *c;
                self.pos += 1;
                Ok(char_to_node(ch))
            }
            Tok::Newline => {
                self.pos += 1;
                Ok(Node::Space(0.5))
            }
            Tok::Amp => {
                self.pos += 1;
                Ok(Node::Space(0.5))
            }
            Tok::Command(cmd) => {
                let cmd = cmd.clone();
                self.pos += 1;
                self.parse_command(&cmd)
            }
            Tok::RBrace => Err("unexpected '}'".to_string()),
            Tok::Sub | Tok::Sup => Err("unexpected script operator".to_string()),
            Tok::Space => {
                self.pos += 1;
                Ok(Node::Space(0.25))
            }
        }
    }

    fn parse_command(&mut self, cmd: &str) -> Result<Node, String> {
        match cmd {
            // Fractions & binomials
            "frac" | "dfrac" | "tfrac" | "cfrac" => {
                let num = self.parse_group_arg()?;
                let den = self.parse_group_arg()?;
                let force_style = match cmd {
                    "dfrac" | "cfrac" => Some(MathStyle::Display),
                    "tfrac" => Some(MathStyle::Text),
                    _ => None,
                };
                Ok(Node::Frac {
                    num,
                    den,
                    rule_thickness: 1.0,
                    force_style,
                })
            }
            "binom" | "dbinom" | "tbinom" => {
                let num = self.parse_group_arg()?;
                let den = self.parse_group_arg()?;
                let force_style = match cmd {
                    "dbinom" => Some(MathStyle::Display),
                    "tbinom" => Some(MathStyle::Text),
                    _ => None,
                };
                Ok(Node::Delimited {
                    left: "(".to_string(),
                    right: ")".to_string(),
                    body: vec![Node::Frac {
                        num,
                        den,
                        rule_thickness: 0.0,
                        force_style,
                    }],
                })
            }
            // Roots
            "sqrt" => {
                let index = self.parse_optional_bracket_arg()?;
                let radicand = self.parse_group_arg()?;
                Ok(Node::Sqrt { index, radicand })
            }
            // Stretchy delimiters
            "left" => {
                let left = self.parse_delimiter()?;
                let body = self.parse_sequence(StopCondition::RightDelim)?;
                match self.peek() {
                    Some(Tok::Command(c)) if c == "right" => {
                        self.pos += 1;
                    }
                    _ => return Err("missing \\right".to_string()),
                }
                let right = self.parse_delimiter()?;
                Ok(Node::Delimited { left, right, body })
            }
            "right" => Err("unmatched \\right without \\left".to_string()),
            "big" | "bigl" | "bigr" | "bigm" => Ok(Node::ScaledDelim {
                scale: 1.25,
                delim: self.parse_delimiter()?,
            }),
            "Big" | "Bigl" | "Bigr" | "Bigm" => Ok(Node::ScaledDelim {
                scale: 1.6,
                delim: self.parse_delimiter()?,
            }),
            "bigg" | "biggl" | "biggr" | "biggm" => Ok(Node::ScaledDelim {
                scale: 2.0,
                delim: self.parse_delimiter()?,
            }),
            "Bigg" | "Biggl" | "Biggr" | "Biggm" => Ok(Node::ScaledDelim {
                scale: 2.4,
                delim: self.parse_delimiter()?,
            }),
            // Environments
            "begin" => self.parse_environment(),
            "end" => Err("unmatched \\end without \\begin".to_string()),
            // Text & font variants
            "text" | "textrm" | "mbox" | "hbox" => Ok(Node::Text {
                content: self.parse_raw_text_arg()?,
                variant: FontVariant::Roman,
            }),
            "textbf" => Ok(Node::Text {
                content: self.parse_raw_text_arg()?,
                variant: FontVariant::Bold,
            }),
            "textit" => Ok(Node::Text {
                content: self.parse_raw_text_arg()?,
                variant: FontVariant::Italic,
            }),
            "texttt" => Ok(Node::Text {
                content: self.parse_raw_text_arg()?,
                variant: FontVariant::Mono,
            }),
            "operatorname" => Ok(Node::Symbol {
                text: self.parse_raw_text_arg()?,
                kind: AtomKind::Op,
                italic_if_normal: false,
                large_op: false,
            }),
            "mathrm" | "rm" => Ok(Node::Styled {
                variant: FontVariant::Roman,
                body: self.parse_group_arg()?,
            }),
            "mathbf" | "bf" | "boldsymbol" | "bm" => Ok(Node::Styled {
                variant: FontVariant::Bold,
                body: self.parse_group_arg()?,
            }),
            "mathit" | "it" => Ok(Node::Styled {
                variant: FontVariant::Italic,
                body: self.parse_group_arg()?,
            }),
            "mathsf" => Ok(Node::Styled {
                variant: FontVariant::Sans,
                body: self.parse_group_arg()?,
            }),
            "mathtt" => Ok(Node::Styled {
                variant: FontVariant::Mono,
                body: self.parse_group_arg()?,
            }),
            "mathcal" | "mathscr" => Ok(Node::Styled {
                variant: FontVariant::Calligraphic,
                body: self.parse_group_arg()?,
            }),
            "mathbb" => Ok(Node::Styled {
                variant: FontVariant::Blackboard,
                body: self.parse_group_arg()?,
            }),
            "mathfrak" => Ok(Node::Styled {
                variant: FontVariant::Fraktur,
                body: self.parse_group_arg()?,
            }),
            // Math size overrides
            "displaystyle" => Ok(Node::SizedStyle {
                style: MathStyle::Display,
                body: self.parse_rest_or_group(),
            }),
            "textstyle" => Ok(Node::SizedStyle {
                style: MathStyle::Text,
                body: self.parse_rest_or_group(),
            }),
            "scriptstyle" => Ok(Node::SizedStyle {
                style: MathStyle::Script,
                body: self.parse_rest_or_group(),
            }),
            "scriptscriptstyle" => Ok(Node::SizedStyle {
                style: MathStyle::ScriptScript,
                body: self.parse_rest_or_group(),
            }),
            // Accents & decorations
            "hat" => Ok(Node::Accent {
                kind: AccentKind::Hat,
                body: self.parse_group_arg()?,
            }),
            "widehat" => Ok(Node::Accent {
                kind: AccentKind::WideHat,
                body: self.parse_group_arg()?,
            }),
            "bar" => Ok(Node::Accent {
                kind: AccentKind::Bar,
                body: self.parse_group_arg()?,
            }),
            "overline" => Ok(Node::Accent {
                kind: AccentKind::Overline,
                body: self.parse_group_arg()?,
            }),
            "underline" => Ok(Node::Accent {
                kind: AccentKind::Underline,
                body: self.parse_group_arg()?,
            }),
            "tilde" => Ok(Node::Accent {
                kind: AccentKind::Tilde,
                body: self.parse_group_arg()?,
            }),
            "widetilde" => Ok(Node::Accent {
                kind: AccentKind::WideTilde,
                body: self.parse_group_arg()?,
            }),
            "vec" => Ok(Node::Accent {
                kind: AccentKind::Vec,
                body: self.parse_group_arg()?,
            }),
            "dot" => Ok(Node::Accent {
                kind: AccentKind::Dot,
                body: self.parse_group_arg()?,
            }),
            "ddot" => Ok(Node::Accent {
                kind: AccentKind::DDot,
                body: self.parse_group_arg()?,
            }),
            "overbrace" => Ok(Node::Accent {
                kind: AccentKind::Overbrace,
                body: self.parse_group_arg()?,
            }),
            "underbrace" => Ok(Node::Accent {
                kind: AccentKind::Underbrace,
                body: self.parse_group_arg()?,
            }),
            "cancel" => Ok(Node::Accent {
                kind: AccentKind::Cancel,
                body: self.parse_group_arg()?,
            }),
            "boxed" => Ok(Node::Boxed(self.parse_group_arg()?)),
            "overset" | "stackrel" => {
                let script = self.parse_group_arg()?;
                let base = self.parse_group_arg()?;
                Ok(Node::OverUnder {
                    over: true,
                    script,
                    base,
                })
            }
            "underset" => {
                let script = self.parse_group_arg()?;
                let base = self.parse_group_arg()?;
                Ok(Node::OverUnder {
                    over: false,
                    script,
                    base,
                })
            }
            "textcolor" | "color" => {
                let raw_color = self.parse_raw_text_arg()?;
                let color = sanitize_svg_color(&raw_color);
                let body = self.parse_group_arg()?;
                Ok(Node::Colored { color, body })
            }
            // Spacing
            "," | "thinspace" => Ok(Node::Space(0.18)),
            ":" | ">" | "medspace" => Ok(Node::Space(0.24)),
            ";" | "thickspace" => Ok(Node::Space(0.32)),
            "!" | "negthinspace" => Ok(Node::Space(-0.14)),
            " " | "enspace" => Ok(Node::Space(0.45)),
            "quad" => Ok(Node::Space(1.0)),
            "qquad" => Ok(Node::Space(2.0)),
            "hspace" => {
                let _ = self.parse_raw_text_arg()?;
                Ok(Node::Space(0.5))
            }
            "phantom" => {
                let _ = self.parse_group_arg()?;
                Ok(Node::Space(0.5))
            }
            "pmod" => {
                let arg = self.parse_group_arg()?;
                let mut inner = vec![
                    Node::Symbol {
                        text: "mod".to_string(),
                        kind: AtomKind::Op,
                        italic_if_normal: false,
                        large_op: false,
                    },
                    Node::Space(0.25),
                ];
                inner.extend(arg);
                Ok(Node::Group(vec![
                    Node::Space(0.5),
                    Node::Delimited {
                        left: "(".to_string(),
                        right: ")".to_string(),
                        body: inner,
                    },
                ]))
            }
            // Known symbols & named operators
            other => {
                if let Some((sym, kind, large_op)) = lookup_symbol(other) {
                    Ok(Node::Symbol {
                        text: sym.to_string(),
                        kind,
                        italic_if_normal: false,
                        large_op,
                    })
                } else if let Some(func) = lookup_function(other) {
                    Ok(Node::Symbol {
                        text: func.to_string(),
                        kind: AtomKind::Op,
                        italic_if_normal: false,
                        large_op: false,
                    })
                } else {
                    // Fallback for unrecognized macro: render its literal name cleanly.
                    Ok(Node::Symbol {
                        text: format!("\\{other}"),
                        kind: AtomKind::Ord,
                        italic_if_normal: false,
                        large_op: false,
                    })
                }
            }
        }
    }

    fn parse_rest_or_group(&mut self) -> Vec<Node> {
        if matches!(self.peek(), Some(Tok::LBrace)) {
            self.parse_group_arg().unwrap_or_default()
        } else {
            Vec::new()
        }
    }

    fn parse_environment(&mut self) -> Result<Node, String> {
        let env = self.parse_raw_text_arg()?;
        let env_trimmed = env.trim().to_string();

        // Optional column spec for `array`, e.g. `\begin{array}{lcr}`
        let mut col_align = Vec::new();
        if env_trimmed == "array" {
            let spec = self.parse_raw_text_arg()?;
            for ch in spec.chars() {
                match ch {
                    'l' => col_align.push(Align::Left),
                    'c' => col_align.push(Align::Center),
                    'r' => col_align.push(Align::Right),
                    _ => {}
                }
            }
        }

        let mut rows: Vec<Vec<Vec<Node>>> = Vec::new();
        let mut cur_row: Vec<Vec<Node>> = Vec::new();

        loop {
            let cell = self.parse_sequence(StopCondition::EndEnv(&env_trimmed))?;
            cur_row.push(cell);
            match self.peek() {
                Some(Tok::Amp) => {
                    self.pos += 1;
                }
                Some(Tok::Newline) => {
                    self.pos += 1;
                    // Optional `[spacing]` after `\\`
                    let _ = self.parse_optional_bracket_arg();
                    rows.push(std::mem::take(&mut cur_row));
                }
                Some(Tok::Command(c)) if c == "end" => {
                    self.pos += 1;
                    let end_name = self.parse_raw_text_arg()?;
                    if end_name.trim() != env_trimmed {
                        return Err(format!(
                            "mismatched environment: \\begin{{{env_trimmed}}} closed by \\end{{{end_name}}}"
                        ));
                    }
                    if !(cur_row.len() == 1 && cur_row[0].is_empty() && !rows.is_empty()) {
                        rows.push(cur_row);
                    }
                    break;
                }
                _ => return Err(format!("unclosed \\begin{{{env_trimmed}}}")),
            }
        }

        let (left, right, default_align, small) = match env_trimmed.as_str() {
            "matrix" => ("", "", vec![], false),
            "smallmatrix" => ("", "", vec![], true),
            "pmatrix" => ("(", ")", vec![], false),
            "bmatrix" => ("[", "]", vec![], false),
            "Bmatrix" => ("{", "}", vec![], false),
            "vmatrix" => ("|", "|", vec![], false),
            "Vmatrix" => ("‖", "‖", vec![], false),
            "cases" | "dcases" => ("{", "", vec![Align::Left, Align::Left], false),
            "aligned" | "align" | "align*" | "split" => {
                ("", "", vec![Align::Right, Align::Left], false)
            }
            "gathered" | "gather" | "gather*" | "array" => ("", "", vec![], false),
            _ => ("", "", vec![], false),
        };
        if col_align.is_empty() {
            col_align = default_align;
        }

        Ok(Node::Matrix {
            left: left.to_string(),
            right: right.to_string(),
            rows,
            col_align,
            small,
        })
    }
}

fn char_to_node(ch: char) -> Node {
    let (text, kind, italic) = match ch {
        '+' => ("+".to_string(), AtomKind::Bin, false),
        '-' => ("−".to_string(), AtomKind::Bin, false),
        '*' => ("∗".to_string(), AtomKind::Bin, false),
        '=' => ("=".to_string(), AtomKind::Rel, false),
        '<' => ("<".to_string(), AtomKind::Rel, false),
        '>' => (">".to_string(), AtomKind::Rel, false),
        '(' | '[' => (ch.to_string(), AtomKind::Open, false),
        ')' | ']' => (ch.to_string(), AtomKind::Close, false),
        ',' | ';' | ':' => (ch.to_string(), AtomKind::Punct, false),
        '~' => return Node::Space(0.35),
        c if c.is_ascii_alphabetic() => (c.to_string(), AtomKind::Ord, true),
        c => (c.to_string(), AtomKind::Ord, false),
    };
    Node::Symbol {
        text,
        kind,
        italic_if_normal: italic,
        large_op: false,
    }
}

fn sanitize_svg_color(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.chars().all(|c| {
        c.is_ascii_alphanumeric() || matches!(c, '#' | '-' | '(' | ')' | ',' | '.' | '%' | ' ')
    }) && !trimmed.is_empty()
    {
        trimmed.to_string()
    } else {
        "currentColor".to_string()
    }
}

// ---------------------------------------------------------------------------
// Layout Box Model
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MathStyle {
    Display,
    Text,
    Script,
    ScriptScript,
}

impl MathStyle {
    fn scale(self) -> f32 {
        match self {
            Self::Display | Self::Text => 1.0,
            Self::Script => 0.72,
            Self::ScriptScript => 0.56,
        }
    }

    fn script_style(self) -> Self {
        match self {
            Self::Display | Self::Text => Self::Script,
            Self::Script | Self::ScriptScript => Self::ScriptScript,
        }
    }

    fn frac_style(self) -> Self {
        match self {
            Self::Display => Self::Text,
            Self::Text => Self::Script,
            Self::Script | Self::ScriptScript => Self::ScriptScript,
        }
    }
}

#[derive(Clone, Debug)]
enum SvgEl {
    Text {
        x: f32,
        y: f32,
        font_size: f32,
        italic: bool,
        bold: bool,
        mono: bool,
        sans: bool,
        color: Option<String>,
        text: String,
    },
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        stroke_width: f32,
        color: Option<String>,
    },
    Path {
        d: String,
        stroke_width: f32,
        fill: bool,
        color: Option<String>,
    },
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        stroke_width: f32,
        color: Option<String>,
    },
    Group {
        dx: f32,
        dy: f32,
        color: Option<String>,
        children: Vec<SvgEl>,
    },
}

#[derive(Clone, Debug)]
struct LayoutBox {
    width: f32,
    ascent: f32,
    descent: f32,
    kind: AtomKind,
    large_op: bool,
    elements: Vec<SvgEl>,
}

impl LayoutBox {
    fn empty() -> Self {
        Self {
            width: 0.0,
            ascent: 0.0,
            descent: 0.0,
            kind: AtomKind::Ord,
            large_op: false,
            elements: Vec::new(),
        }
    }

    fn height(&self) -> f32 {
        self.ascent + self.descent
    }

    fn translate(mut self, dx: f32, dy: f32) -> Vec<SvgEl> {
        if dx.abs() < 0.001 && dy.abs() < 0.001 {
            return self.elements;
        }
        vec![SvgEl::Group {
            dx,
            dy,
            color: None,
            children: std::mem::take(&mut self.elements),
        }]
    }
}

fn layout_row(nodes: &[Node], base_font: f32, style: MathStyle, variant: FontVariant) -> LayoutBox {
    if nodes.is_empty() {
        return LayoutBox::empty();
    }
    let fs = base_font * style.scale();
    let mut x = 0.0_f32;
    let mut ascent = fs * 0.72;
    let mut descent = fs * 0.22;
    let mut elements = Vec::new();
    let mut prev_kind: Option<AtomKind> = None;

    for node in nodes {
        let mut child = layout_node(node, base_font, style, variant);
        // Binary minus/plus at start of expression or after Open/Bin/Rel/Punct is unary.
        if child.kind == AtomKind::Bin
            && matches!(
                prev_kind,
                None | Some(AtomKind::Open | AtomKind::Bin | AtomKind::Rel | AtomKind::Punct)
            )
        {
            child.kind = AtomKind::Ord;
        }
        let gap = inter_atom_space(prev_kind, child.kind, style) * fs;
        x += gap;
        ascent = ascent.max(child.ascent);
        descent = descent.max(child.descent);
        let w = child.width;
        let kind = child.kind;
        elements.extend(child.translate(x, 0.0));
        x += w;
        if !matches!(node, Node::Space(_)) {
            prev_kind = Some(kind);
        }
    }

    LayoutBox {
        width: x.max(0.0),
        ascent,
        descent,
        kind: prev_kind.unwrap_or(AtomKind::Ord),
        large_op: false,
        elements,
    }
}

fn inter_atom_space(prev: Option<AtomKind>, next: AtomKind, style: MathStyle) -> f32 {
    if matches!(style, MathStyle::Script | MathStyle::ScriptScript) {
        return 0.0;
    }
    let Some(prev) = prev else {
        return 0.0;
    };
    match (prev, next) {
        (_, AtomKind::Rel) | (AtomKind::Rel, _) => 0.26,
        (_, AtomKind::Bin) | (AtomKind::Bin, _) => 0.20,
        (AtomKind::Punct, _) => 0.16,
        (AtomKind::Op, AtomKind::Ord | AtomKind::Op) => 0.14,
        (AtomKind::Close, AtomKind::Ord | AtomKind::Op) => 0.08,
        _ => 0.0,
    }
}

fn layout_node(node: &Node, base_font: f32, style: MathStyle, variant: FontVariant) -> LayoutBox {
    let fs = base_font * style.scale();
    match node {
        Node::Symbol {
            text,
            kind,
            italic_if_normal,
            large_op,
        } => {
            let eff_fs = if *large_op && style == MathStyle::Display {
                fs * 1.45
            } else if *large_op {
                fs * 1.15
            } else {
                fs
            };
            let mapped = apply_unicode_variant(text, variant);
            let italic = match variant {
                FontVariant::Normal => *italic_if_normal,
                FontVariant::Italic => true,
                _ => false,
            };
            let bold = variant == FontVariant::Bold;
            let mono = variant == FontVariant::Mono;
            let sans = variant == FontVariant::Sans;
            let width = measure_text(&mapped, eff_fs, italic, mono);
            let y_shift = if *large_op { eff_fs * 0.14 } else { 0.0 };
            let ascent = eff_fs * 0.78 - y_shift;
            let descent = eff_fs * 0.24 + y_shift;
            LayoutBox {
                width,
                ascent,
                descent,
                kind: *kind,
                large_op: *large_op,
                elements: vec![SvgEl::Text {
                    x: 0.0,
                    y: y_shift,
                    font_size: eff_fs,
                    italic,
                    bold,
                    mono,
                    sans,
                    color: None,
                    text: mapped,
                }],
            }
        }
        Node::Text {
            content,
            variant: tv,
        } => {
            let bold = *tv == FontVariant::Bold;
            let italic = *tv == FontVariant::Italic;
            let mono = *tv == FontVariant::Mono;
            let sans = *tv == FontVariant::Sans;
            let width = measure_text(content, fs, italic, mono);
            LayoutBox {
                width,
                ascent: fs * 0.78,
                descent: fs * 0.22,
                kind: AtomKind::Ord,
                large_op: false,
                elements: vec![SvgEl::Text {
                    x: 0.0,
                    y: 0.0,
                    font_size: fs,
                    italic,
                    bold,
                    mono,
                    sans,
                    color: None,
                    text: content.clone(),
                }],
            }
        }
        Node::Space(em) => LayoutBox {
            width: em * fs,
            ascent: 0.0,
            descent: 0.0,
            kind: AtomKind::Ord,
            large_op: false,
            elements: Vec::new(),
        },
        Node::Group(children) => layout_row(children, base_font, style, variant),
        Node::Styled {
            variant: new_var,
            body,
        } => layout_row(body, base_font, style, *new_var),
        Node::Colored { color, body } => {
            let mut b = layout_row(body, base_font, style, variant);
            b.elements = vec![SvgEl::Group {
                dx: 0.0,
                dy: 0.0,
                color: Some(color.clone()),
                children: b.elements,
            }];
            b
        }
        Node::SizedStyle {
            style: forced,
            body,
        } => layout_row(body, base_font, *forced, variant),
        Node::Scripts {
            base,
            sub,
            sup,
            limits,
        } => layout_scripts(
            base,
            sub.as_deref(),
            sup.as_deref(),
            *limits,
            base_font,
            style,
            variant,
        ),
        Node::Frac {
            num,
            den,
            rule_thickness,
            force_style,
        } => layout_frac(
            num,
            den,
            *rule_thickness,
            *force_style,
            base_font,
            style,
            variant,
        ),
        Node::Sqrt { index, radicand } => {
            layout_sqrt(index.as_deref(), radicand, base_font, style, variant)
        }
        Node::Delimited { left, right, body } => {
            layout_delimited(left, right, body, base_font, style, variant)
        }
        Node::ScaledDelim { scale, delim } => {
            let h = fs * scale;
            make_delimiter_box(delim, h * 0.72, h * 0.28, fs)
        }
        Node::Accent { kind, body } => layout_accent(*kind, body, base_font, style, variant),
        Node::OverUnder { over, script, base } => {
            layout_over_under(*over, script, base, base_font, style, variant)
        }
        Node::Boxed(body) => {
            let inner = layout_row(body, base_font, style, variant);
            let pad = fs * 0.22;
            let w = inner.width + pad * 2.0;
            let asc = inner.ascent + pad;
            let desc = inner.descent + pad;
            let mut elements = inner.translate(pad, 0.0);
            elements.push(SvgEl::Rect {
                x: 0.5,
                y: -asc + 0.5,
                w: (w - 1.0).max(1.0),
                h: (asc + desc - 1.0).max(1.0),
                stroke_width: (fs * 0.06).max(0.9),
                color: None,
            });
            LayoutBox {
                width: w,
                ascent: asc,
                descent: desc,
                kind: AtomKind::Ord,
                large_op: false,
                elements,
            }
        }
        Node::Matrix {
            left,
            right,
            rows,
            col_align,
            small,
        } => layout_matrix(
            MatrixInput {
                left,
                right,
                rows,
                col_align,
                small: *small,
            },
            base_font,
            style,
            variant,
        ),
    }
}

fn layout_scripts(
    base: &Node,
    sub: Option<&Node>,
    sup: Option<&Node>,
    limits: LimitsMode,
    base_font: f32,
    style: MathStyle,
    variant: FontVariant,
) -> LayoutBox {
    let base_box = layout_node(base, base_font, style, variant);
    let script_style = style.script_style();
    let sub_box = sub.map(|n| layout_node(n, base_font, script_style, variant));
    let sup_box = sup.map(|n| layout_node(n, base_font, script_style, variant));
    let fs = base_font * style.scale();

    let stack_limits = match limits {
        LimitsMode::Limits => true,
        LimitsMode::NoLimits => false,
        LimitsMode::Auto => base_box.large_op && style == MathStyle::Display,
    };

    if stack_limits {
        let total_w = base_box
            .width
            .max(sub_box.as_ref().map_or(0.0, |b| b.width))
            .max(sup_box.as_ref().map_or(0.0, |b| b.width));
        let gap = fs * 0.12;
        let mut ascent = base_box.ascent;
        let mut descent = base_box.descent;
        let mut elements = Vec::new();

        let base_x = (total_w - base_box.width) * 0.5;
        let base_kind = base_box.kind;
        elements.extend(base_box.translate(base_x, 0.0));

        if let Some(sb) = sup_box {
            let sx = (total_w - sb.width) * 0.5;
            let sy = -(ascent + gap + sb.descent);
            ascent += gap + sb.height();
            elements.extend(sb.translate(sx, sy));
        }
        if let Some(sb) = sub_box {
            let sx = (total_w - sb.width) * 0.5;
            let sy = descent + gap + sb.ascent;
            descent += gap + sb.height();
            elements.extend(sb.translate(sx, sy));
        }
        return LayoutBox {
            width: total_w,
            ascent,
            descent,
            kind: base_kind,
            large_op: false,
            elements,
        };
    }

    // Side sub/superscripts
    let x_script = base_box.width + fs * 0.04;
    let mut total_w = base_box.width;
    let mut ascent = base_box.ascent;
    let mut descent = base_box.descent;
    let base_kind = base_box.kind;
    let mut elements = base_box.elements;

    if let Some(sb) = sup_box {
        let sup_shift = (ascent * 0.55).max(fs * 0.42);
        let y = -sup_shift;
        ascent = ascent.max(sup_shift + sb.ascent);
        total_w = total_w.max(x_script + sb.width);
        elements.extend(sb.translate(x_script, y));
    }
    if let Some(sb) = sub_box {
        let sub_shift = (descent * 0.55).max(fs * 0.28);
        let y = sub_shift;
        descent = descent.max(sub_shift + sb.descent);
        total_w = total_w.max(x_script + sb.width);
        elements.extend(sb.translate(x_script, y));
    }

    LayoutBox {
        width: total_w,
        ascent,
        descent,
        kind: base_kind,
        large_op: false,
        elements,
    }
}

fn layout_frac(
    num: &[Node],
    den: &[Node],
    rule_thickness: f32,
    force_style: Option<MathStyle>,
    base_font: f32,
    style: MathStyle,
    variant: FontVariant,
) -> LayoutBox {
    let eff_style = force_style.unwrap_or(style);
    let child_style = eff_style.frac_style();
    let num_box = layout_row(num, base_font, child_style, variant);
    let den_box = layout_row(den, base_font, child_style, variant);
    let fs = base_font * style.scale();
    let axis = fs * 0.30; // Math axis above baseline
    let pad = fs * 0.14;
    let gap = fs * 0.16;
    let stroke = if rule_thickness > 0.0 {
        (fs * 0.06 * rule_thickness).max(0.9)
    } else {
        0.0
    };
    let width = num_box.width.max(den_box.width) + pad * 2.0;

    let num_x = (width - num_box.width) * 0.5;
    let den_x = (width - den_box.width) * 0.5;
    let num_y = -(axis + stroke * 0.5 + gap + num_box.descent);
    let den_y = -axis + stroke * 0.5 + gap + den_box.ascent;

    let ascent = (axis + stroke * 0.5 + gap + num_box.height()).max(fs * 0.72);
    let descent = (den_y + den_box.descent).max(fs * 0.22);

    let mut elements = Vec::new();
    elements.extend(num_box.translate(num_x, num_y));
    elements.extend(den_box.translate(den_x, den_y));
    if stroke > 0.0 {
        elements.push(SvgEl::Line {
            x1: pad * 0.3,
            y1: -axis,
            x2: width - pad * 0.3,
            y2: -axis,
            stroke_width: stroke,
            color: None,
        });
    }

    LayoutBox {
        width,
        ascent,
        descent,
        kind: AtomKind::Ord,
        large_op: false,
        elements,
    }
}

fn layout_sqrt(
    index: Option<&[Node]>,
    radicand: &[Node],
    base_font: f32,
    style: MathStyle,
    variant: FontVariant,
) -> LayoutBox {
    let fs = base_font * style.scale();
    let inner = layout_row(radicand, base_font, style, variant);
    let stroke = (fs * 0.065).max(1.0);
    let top_gap = fs * 0.18;
    let right_pad = fs * 0.12;
    let hook_w = fs * 0.62;

    let top_y = -(inner.ascent + top_gap);
    let bottom_y = inner.descent + fs * 0.06;

    let mut x_offset = 0.0_f32;
    let mut elements = Vec::new();
    let mut ascent = inner.ascent + top_gap + stroke;

    if let Some(idx_nodes) = index {
        let idx_box = layout_row(idx_nodes, base_font, MathStyle::ScriptScript, variant);
        let idx_y = top_y + (bottom_y - top_y) * 0.38;
        let idx_w = (idx_box.width - hook_w * 0.35).max(0.0);
        x_offset = idx_w;
        ascent = ascent.max(-(idx_y - idx_box.ascent));
        elements.extend(idx_box.translate(0.0, idx_y));
    }

    let rad_x = x_offset;
    let mid_y = (top_y + bottom_y) * 0.45;
    let bar_end_x = rad_x + hook_w + inner.width + right_pad;

    let d = format!(
        "M {:.2} {:.2} L {:.2} {:.2} L {:.2} {:.2} L {:.2} {:.2} L {:.2} {:.2}",
        rad_x,
        mid_y,
        rad_x + hook_w * 0.22,
        mid_y - fs * 0.06,
        rad_x + hook_w * 0.48,
        bottom_y,
        rad_x + hook_w * 0.92,
        top_y,
        bar_end_x,
        top_y,
    );
    elements.push(SvgEl::Path {
        d,
        stroke_width: stroke,
        fill: false,
        color: None,
    });
    elements.extend(inner.translate(rad_x + hook_w, 0.0));

    LayoutBox {
        width: bar_end_x,
        ascent,
        descent: bottom_y,
        kind: AtomKind::Ord,
        large_op: false,
        elements,
    }
}

fn layout_delimited(
    left: &str,
    right: &str,
    body: &[Node],
    base_font: f32,
    style: MathStyle,
    variant: FontVariant,
) -> LayoutBox {
    let fs = base_font * style.scale();
    let inner = layout_row(body, base_font, style, variant);
    let pad = fs * 0.06;
    let lbox = make_delimiter_box(left, inner.ascent, inner.descent, fs);
    let rbox = make_delimiter_box(right, inner.ascent, inner.descent, fs);

    let mut x = 0.0;
    let mut elements = Vec::new();
    let ascent = inner.ascent.max(lbox.ascent).max(rbox.ascent);
    let descent = inner.descent.max(lbox.descent).max(rbox.descent);

    if lbox.width > 0.0 {
        let w = lbox.width;
        elements.extend(lbox.translate(x, 0.0));
        x += w + pad;
    }
    let iw = inner.width;
    elements.extend(inner.translate(x, 0.0));
    x += iw;
    if rbox.width > 0.0 {
        x += pad;
        let w = rbox.width;
        elements.extend(rbox.translate(x, 0.0));
        x += w;
    }

    LayoutBox {
        width: x,
        ascent,
        descent,
        kind: AtomKind::Ord,
        large_op: false,
        elements,
    }
}

fn make_delimiter_box(delim: &str, target_ascent: f32, target_descent: f32, fs: f32) -> LayoutBox {
    if delim.is_empty() || delim == "." {
        return LayoutBox::empty();
    }
    let total_h = (target_ascent + target_descent).max(fs * 0.95);
    let top = -target_ascent.max(fs * 0.72);
    let bot = target_descent.max(fs * 0.22);
    let stroke = (fs * 0.07).max(1.0);

    // For ordinary single-line height, crisp text glyph is cleanest for non-bar delims.
    if total_h <= fs * 1.25 && !matches!(delim, "|" | "‖") {
        let w = measure_text(delim, fs, false, false);
        return LayoutBox {
            width: w,
            ascent: fs * 0.78,
            descent: fs * 0.24,
            kind: AtomKind::Ord,
            large_op: false,
            elements: vec![SvgEl::Text {
                x: 0.0,
                y: 0.0,
                font_size: fs,
                italic: false,
                bold: false,
                mono: false,
                sans: false,
                color: None,
                text: delim.to_string(),
            }],
        };
    }

    let w = (fs * 0.42).max(6.0);
    let mid_y = (top + bot) * 0.5;
    let d = match delim {
        "(" => format!(
            "M {:.2} {:.2} Q {:.2} {:.2} {:.2} {:.2}",
            w * 0.85,
            top,
            -w * 0.15,
            mid_y,
            w * 0.85,
            bot
        ),
        ")" => format!(
            "M {:.2} {:.2} Q {:.2} {:.2} {:.2} {:.2}",
            w * 0.15,
            top,
            w * 1.15,
            mid_y,
            w * 0.15,
            bot
        ),
        "[" => format!(
            "M {:.2} {:.2} L {:.2} {:.2} L {:.2} {:.2} L {:.2} {:.2}",
            w * 0.85,
            top,
            w * 0.25,
            top,
            w * 0.25,
            bot,
            w * 0.85,
            bot
        ),
        "]" => format!(
            "M {:.2} {:.2} L {:.2} {:.2} L {:.2} {:.2} L {:.2} {:.2}",
            w * 0.15,
            top,
            w * 0.75,
            top,
            w * 0.75,
            bot,
            w * 0.15,
            bot
        ),
        "{" => {
            let q = (bot - top) * 0.15;
            format!(
                "M {:.2} {:.2} Q {:.2} {:.2} {:.2} {:.2} L {:.2} {:.2} Q {:.2} {:.2} {:.2} {:.2} Q {:.2} {:.2} {:.2} {:.2} L {:.2} {:.2} Q {:.2} {:.2} {:.2} {:.2}",
                w * 0.88, top,
                w * 0.45, top, w * 0.45, top + q,
                w * 0.45, mid_y - q * 0.6,
                w * 0.45, mid_y, w * 0.12, mid_y,
                w * 0.45, mid_y, w * 0.45, mid_y + q * 0.6,
                w * 0.45, bot - q,
                w * 0.45, bot, w * 0.88, bot,
            )
        }
        "}" => {
            let q = (bot - top) * 0.15;
            format!(
                "M {:.2} {:.2} Q {:.2} {:.2} {:.2} {:.2} L {:.2} {:.2} Q {:.2} {:.2} {:.2} {:.2} Q {:.2} {:.2} {:.2} {:.2} L {:.2} {:.2} Q {:.2} {:.2} {:.2} {:.2}",
                w * 0.12, top,
                w * 0.55, top, w * 0.55, top + q,
                w * 0.55, mid_y - q * 0.6,
                w * 0.55, mid_y, w * 0.88, mid_y,
                w * 0.55, mid_y, w * 0.55, mid_y + q * 0.6,
                w * 0.55, bot - q,
                w * 0.55, bot, w * 0.12, bot,
            )
        }
        "|" => format!("M {:.2} {:.2} L {:.2} {:.2}", w * 0.5, top, w * 0.5, bot),
        "‖" => format!(
            "M {:.2} {:.2} L {:.2} {:.2} M {:.2} {:.2} L {:.2} {:.2}",
            w * 0.3,
            top,
            w * 0.3,
            bot,
            w * 0.7,
            top,
            w * 0.7,
            bot
        ),
        "⟨" => format!(
            "M {:.2} {:.2} L {:.2} {:.2} L {:.2} {:.2}",
            w * 0.85,
            top,
            w * 0.18,
            mid_y,
            w * 0.85,
            bot
        ),
        "⟩" => format!(
            "M {:.2} {:.2} L {:.2} {:.2} L {:.2} {:.2}",
            w * 0.15,
            top,
            w * 0.82,
            mid_y,
            w * 0.15,
            bot
        ),
        "⌊" => format!(
            "M {:.2} {:.2} L {:.2} {:.2} L {:.2} {:.2}",
            w * 0.25,
            top,
            w * 0.25,
            bot,
            w * 0.85,
            bot
        ),
        "⌋" => format!(
            "M {:.2} {:.2} L {:.2} {:.2} L {:.2} {:.2}",
            w * 0.75,
            top,
            w * 0.75,
            bot,
            w * 0.15,
            bot
        ),
        "⌈" => format!(
            "M {:.2} {:.2} L {:.2} {:.2} L {:.2} {:.2}",
            w * 0.85,
            top,
            w * 0.25,
            top,
            w * 0.25,
            bot
        ),
        "⌉" => format!(
            "M {:.2} {:.2} L {:.2} {:.2} L {:.2} {:.2}",
            w * 0.15,
            top,
            w * 0.75,
            top,
            w * 0.75,
            bot
        ),
        "/" => format!("M {:.2} {:.2} L {:.2} {:.2}", w * 0.85, top, w * 0.15, bot),
        "\\" => format!("M {:.2} {:.2} L {:.2} {:.2}", w * 0.15, top, w * 0.85, bot),
        _ => format!("M {:.2} {:.2} L {:.2} {:.2}", w * 0.5, top, w * 0.5, bot),
    };

    LayoutBox {
        width: w,
        ascent: -top,
        descent: bot,
        kind: AtomKind::Ord,
        large_op: false,
        elements: vec![SvgEl::Path {
            d,
            stroke_width: stroke,
            fill: false,
            color: None,
        }],
    }
}

fn layout_accent(
    kind: AccentKind,
    body: &[Node],
    base_font: f32,
    style: MathStyle,
    variant: FontVariant,
) -> LayoutBox {
    let fs = base_font * style.scale();
    let inner = layout_row(body, base_font, style, variant);
    let stroke = (fs * 0.065).max(0.95);
    let w = inner.width.max(fs * 0.45);
    let mut ascent = inner.ascent;
    let mut descent = inner.descent;
    let mut elements = inner.elements;

    match kind {
        AccentKind::Bar | AccentKind::Overline => {
            let y = -(ascent + fs * 0.12);
            ascent += fs * 0.22;
            elements.push(SvgEl::Line {
                x1: 0.0,
                y1: y,
                x2: w,
                y2: y,
                stroke_width: stroke,
                color: None,
            });
        }
        AccentKind::Underline => {
            let y = descent + fs * 0.14;
            descent += fs * 0.24;
            elements.push(SvgEl::Line {
                x1: 0.0,
                y1: y,
                x2: w,
                y2: y,
                stroke_width: stroke,
                color: None,
            });
        }
        AccentKind::Hat | AccentKind::WideHat => {
            let y_base = -(ascent + fs * 0.06);
            let y_peak = y_base - fs * 0.22;
            ascent += fs * 0.34;
            elements.push(SvgEl::Path {
                d: format!(
                    "M {:.2} {:.2} L {:.2} {:.2} L {:.2} {:.2}",
                    w * 0.08,
                    y_base,
                    w * 0.5,
                    y_peak,
                    w * 0.92,
                    y_base
                ),
                stroke_width: stroke,
                fill: false,
                color: None,
            });
        }
        AccentKind::Tilde | AccentKind::WideTilde => {
            let y = -(ascent + fs * 0.16);
            let amp = fs * 0.10;
            ascent += fs * 0.32;
            elements.push(SvgEl::Path {
                d: format!(
                    "M {:.2} {:.2} Q {:.2} {:.2} {:.2} {:.2} T {:.2} {:.2}",
                    w * 0.05,
                    y,
                    w * 0.28,
                    y - amp,
                    w * 0.5,
                    y,
                    w * 0.95,
                    y
                ),
                stroke_width: stroke,
                fill: false,
                color: None,
            });
        }
        AccentKind::Vec => {
            let y = -(ascent + fs * 0.16);
            let tip = fs * 0.12;
            ascent += fs * 0.32;
            elements.push(SvgEl::Path {
                d: format!(
                    "M {:.2} {:.2} L {:.2} {:.2} M {:.2} {:.2} L {:.2} {:.2} L {:.2} {:.2}",
                    0.0,
                    y,
                    w,
                    y,
                    w - tip,
                    y - tip * 0.7,
                    w,
                    y,
                    w - tip,
                    y + tip * 0.7
                ),
                stroke_width: stroke,
                fill: false,
                color: None,
            });
        }
        AccentKind::Dot | AccentKind::DDot => {
            let y = -(ascent + fs * 0.14);
            ascent += fs * 0.28;
            let text = if kind == AccentKind::Dot {
                "·"
            } else {
                "··"
            };
            let tw = measure_text(text, fs, false, false);
            elements.push(SvgEl::Text {
                x: (w - tw) * 0.5,
                y: y + fs * 0.22,
                font_size: fs,
                italic: false,
                bold: true,
                mono: false,
                sans: false,
                color: None,
                text: text.to_string(),
            });
        }
        AccentKind::Overbrace => {
            let y = -(ascent + fs * 0.10);
            let h = fs * 0.22;
            ascent += fs * 0.36;
            elements.push(SvgEl::Path {
                d: format!(
                    "M 0 {:.2} Q 0 {:.2} {:.2} {:.2} L {:.2} {:.2} Q {:.2} {:.2} {:.2} {:.2} Q {:.2} {:.2} {:.2} {:.2} L {:.2} {:.2} Q {:.2} {:.2} {:.2} {:.2}",
                    y,
                    y - h * 0.5, w * 0.15, y - h * 0.5,
                    w * 0.42, y - h * 0.5,
                    w * 0.5, y - h * 0.5, w * 0.5, y - h,
                    w * 0.5, y - h * 0.5, w * 0.58, y - h * 0.5,
                    w * 0.85, y - h * 0.5,
                    w, y - h * 0.5, w, y,
                ),
                stroke_width: stroke,
                fill: false,
                color: None,
            });
        }
        AccentKind::Underbrace => {
            let y = descent + fs * 0.10;
            let h = fs * 0.22;
            descent += fs * 0.36;
            elements.push(SvgEl::Path {
                d: format!(
                    "M 0 {:.2} Q 0 {:.2} {:.2} {:.2} L {:.2} {:.2} Q {:.2} {:.2} {:.2} {:.2} Q {:.2} {:.2} {:.2} {:.2} L {:.2} {:.2} Q {:.2} {:.2} {:.2} {:.2}",
                    y,
                    y + h * 0.5, w * 0.15, y + h * 0.5,
                    w * 0.42, y + h * 0.5,
                    w * 0.5, y + h * 0.5, w * 0.5, y + h,
                    w * 0.5, y + h * 0.5, w * 0.58, y + h * 0.5,
                    w * 0.85, y + h * 0.5,
                    w, y + h * 0.5, w, y,
                ),
                stroke_width: stroke,
                fill: false,
                color: None,
            });
        }
        AccentKind::Cancel => {
            elements.push(SvgEl::Line {
                x1: 0.0,
                y1: descent,
                x2: w,
                y2: -ascent,
                stroke_width: stroke,
                color: None,
            });
        }
    }

    LayoutBox {
        width: w,
        ascent,
        descent,
        kind: AtomKind::Ord,
        large_op: false,
        elements,
    }
}

fn layout_over_under(
    over: bool,
    script: &[Node],
    base: &[Node],
    base_font: f32,
    style: MathStyle,
    variant: FontVariant,
) -> LayoutBox {
    let fs = base_font * style.scale();
    let base_box = layout_row(base, base_font, style, variant);
    let script_box = layout_row(script, base_font, style.script_style(), variant);
    let w = base_box.width.max(script_box.width);
    let gap = fs * 0.12;
    let mut ascent = base_box.ascent;
    let mut descent = base_box.descent;
    let mut elements = Vec::new();
    let bx = (w - base_box.width) * 0.5;
    let sx = (w - script_box.width) * 0.5;
    elements.extend(base_box.translate(bx, 0.0));
    if over {
        let sy = -(ascent + gap + script_box.descent);
        ascent += gap + script_box.height();
        elements.extend(script_box.translate(sx, sy));
    } else {
        let sy = descent + gap + script_box.ascent;
        descent += gap + script_box.height();
        elements.extend(script_box.translate(sx, sy));
    }
    LayoutBox {
        width: w,
        ascent,
        descent,
        kind: AtomKind::Ord,
        large_op: false,
        elements,
    }
}

struct MatrixInput<'a> {
    left: &'a str,
    right: &'a str,
    rows: &'a [Vec<Vec<Node>>],
    col_align: &'a [Align],
    small: bool,
}

fn layout_matrix(
    input: MatrixInput<'_>,
    base_font: f32,
    style: MathStyle,
    variant: FontVariant,
) -> LayoutBox {
    let MatrixInput {
        left,
        right,
        rows,
        col_align,
        small,
    } = input;
    let fs = base_font * style.scale();
    let cell_style = if small {
        MathStyle::Script
    } else {
        MathStyle::Text
    };
    let col_gap = fs * if small { 0.45 } else { 0.85 };
    let row_gap = fs * if small { 0.25 } else { 0.40 };

    let mut grid: Vec<Vec<LayoutBox>> = Vec::with_capacity(rows.len());
    let mut max_cols = 0usize;
    for row in rows {
        let mut laid = Vec::with_capacity(row.len());
        for cell in row {
            laid.push(layout_row(cell, base_font, cell_style, variant));
        }
        max_cols = max_cols.max(laid.len());
        grid.push(laid);
    }
    if max_cols == 0 || grid.is_empty() {
        return LayoutBox::empty();
    }

    let mut col_widths = vec![0.0_f32; max_cols];
    let mut row_ascents = vec![fs * 0.72; grid.len()];
    let mut row_descents = vec![fs * 0.22; grid.len()];

    for (r, row) in grid.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            col_widths[c] = col_widths[c].max(cell.width);
            row_ascents[r] = row_ascents[r].max(cell.ascent);
            row_descents[r] = row_descents[r].max(cell.descent);
        }
    }

    let total_w: f32 =
        col_widths.iter().sum::<f32>() + col_gap * (max_cols.saturating_sub(1)).to_f32_lossy();
    let total_h: f32 = row_ascents
        .iter()
        .zip(row_descents.iter())
        .map(|(a, d)| a + d)
        .sum::<f32>()
        + row_gap * (grid.len().saturating_sub(1)).to_f32_lossy();

    let axis = fs * 0.30;
    let ascent = total_h * 0.5 + axis;
    let descent = (total_h * 0.5 - axis).max(fs * 0.22);

    let mut elements = Vec::new();
    let mut y = -ascent;
    for (r, row) in grid.into_iter().enumerate() {
        let baseline = y + row_ascents[r];
        let mut x = 0.0;
        for (c, cell) in row.into_iter().enumerate() {
            let align = col_align.get(c).copied().unwrap_or(Align::Center);
            let dx = match align {
                Align::Left => 0.0,
                Align::Center => (col_widths[c] - cell.width) * 0.5,
                Align::Right => col_widths[c] - cell.width,
            };
            elements.extend(cell.translate(x + dx, baseline));
            x += col_widths[c] + col_gap;
        }
        y += row_ascents[r] + row_descents[r] + row_gap;
    }

    let inner = LayoutBox {
        width: total_w,
        ascent,
        descent,
        kind: AtomKind::Ord,
        large_op: false,
        elements,
    };

    if left.is_empty() && right.is_empty() {
        return inner;
    }

    let pad = fs * 0.14;
    let lbox = make_delimiter_box(left, inner.ascent, inner.descent, fs);
    let rbox = make_delimiter_box(right, inner.ascent, inner.descent, fs);
    let mut x = 0.0;
    let mut out = Vec::new();
    if lbox.width > 0.0 {
        let lw = lbox.width;
        out.extend(lbox.translate(x, 0.0));
        x += lw + pad;
    }
    let iw = inner.width;
    out.extend(inner.translate(x, 0.0));
    x += iw;
    if rbox.width > 0.0 {
        x += pad;
        let rw = rbox.width;
        out.extend(rbox.translate(x, 0.0));
        x += rw;
    }

    LayoutBox {
        width: x,
        ascent,
        descent,
        kind: AtomKind::Ord,
        large_op: false,
        elements: out,
    }
}

// ---------------------------------------------------------------------------
// Text Measurement & Unicode Math Variants
// ---------------------------------------------------------------------------

fn measure_text(text: &str, font_size: f32, italic: bool, mono: bool) -> f32 {
    if mono {
        return text.chars().count().to_f32_lossy() * font_size * 0.62;
    }
    let mut units = 0.0_f32;
    for ch in text.chars() {
        let w = match ch {
            ' ' => 0.28,
            'i' | 'l' | 'j' | 'I' | '|' | '!' | '.' | ',' | ':' | ';' | '\'' | '′' => 0.32,
            'f' | 'r' | 't' | '(' | ')' | '[' | ']' => 0.38,
            'm' | 'w' | 'M' | 'W' | '∑' | '∏' | '∐' | '⋃' | '⋂' | '⨁' | '⨂' => 0.82,
            'A'..='Z' => 0.68,
            '0'..='9' => 0.54,
            '+' | '−' | '=' | '<' | '>' | '≤' | '≥' | '≠' | '≈' | '≡' | '±' | '×' | '÷' => {
                0.66
            }
            '→' | '←' | '↔' | '⇒' | '⇐' | '⇔' | '↦' | '∞' => 0.84,
            c if c as u32 > 0x2E7F => 0.95,
            _ => 0.54,
        };
        units += w;
    }
    let italic_pad = if italic && !text.is_empty() {
        0.08
    } else {
        0.0
    };
    (units + italic_pad) * font_size
}

fn apply_unicode_variant(text: &str, variant: FontVariant) -> String {
    match variant {
        FontVariant::Blackboard => text
            .chars()
            .map(|c| match c {
                'C' => 'ℂ',
                'H' => 'ℍ',
                'N' => 'ℕ',
                'P' => 'ℙ',
                'Q' => 'ℚ',
                'R' => 'ℝ',
                'Z' => 'ℤ',
                'A'..='Z' => char::from_u32(0x1D538 + u32::from(c as u8 - b'A')).unwrap_or(c),
                _ => c,
            })
            .collect(),
        FontVariant::Calligraphic => text
            .chars()
            .map(|c| match c {
                'B' => 'ℬ',
                'E' => 'ℰ',
                'F' => 'ℱ',
                'H' => 'ℋ',
                'I' => 'ℐ',
                'L' => 'ℒ',
                'M' => 'ℳ',
                'R' => 'ℛ',
                'A'..='Z' => char::from_u32(0x1D49C + u32::from(c as u8 - b'A')).unwrap_or(c),
                _ => c,
            })
            .collect(),
        FontVariant::Fraktur => text
            .chars()
            .map(|c| match c {
                'C' => 'ℭ',
                'H' => 'ℌ',
                'I' => 'ℑ',
                'R' => 'ℜ',
                'Z' => 'ℨ',
                'A'..='Z' => char::from_u32(0x1D504 + u32::from(c as u8 - b'A')).unwrap_or(c),
                'a'..='z' => char::from_u32(0x1D51E + u32::from(c as u8 - b'a')).unwrap_or(c),
                _ => c,
            })
            .collect(),
        _ => text.to_string(),
    }
}

// ---------------------------------------------------------------------------
// SVG Emitter
// ---------------------------------------------------------------------------

fn emit_box(node: &LayoutBox, dx: f32, dy: f32, out: &mut String) {
    emit_elements(&node.elements, dx, dy, None, out);
}

fn emit_elements(
    elements: &[SvgEl],
    ox: f32,
    oy: f32,
    inherited_color: Option<&str>,
    out: &mut String,
) {
    for el in elements {
        match el {
            SvgEl::Group {
                dx,
                dy,
                color,
                children,
            } => {
                let eff_color = color.as_deref().or(inherited_color);
                emit_elements(children, ox + dx, oy + dy, eff_color, out);
            }
            SvgEl::Text {
                x,
                y,
                font_size,
                italic,
                bold,
                mono,
                sans,
                color,
                text,
            } => {
                let fill = color
                    .as_deref()
                    .or(inherited_color)
                    .unwrap_or("currentColor");
                let family = if *mono {
                    "Monaspace Neon, ui-monospace, monospace"
                } else if *sans {
                    "ui-sans-serif, system-ui, sans-serif"
                } else {
                    "STIX Two Math, Cambria Math, Georgia, Times New Roman, serif"
                };
                let _ = write!(
                    out,
                    "<text x=\"{:.2}\" y=\"{:.2}\" font-size=\"{:.2}\" font-family=\"{family}\" fill=\"{fill}\"",
                    ox + x,
                    oy + y,
                    font_size
                );
                if *italic {
                    out.push_str(" font-style=\"italic\"");
                }
                if *bold {
                    out.push_str(" font-weight=\"700\"");
                }
                out.push('>');
                push_xml_escaped(out, text);
                out.push_str("</text>");
            }
            SvgEl::Line {
                x1,
                y1,
                x2,
                y2,
                stroke_width,
                color,
            } => {
                let stroke = color
                    .as_deref()
                    .or(inherited_color)
                    .unwrap_or("currentColor");
                let _ = write!(
                    out,
                    "<line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"{stroke}\" stroke-width=\"{stroke_width:.2}\" stroke-linecap=\"round\"/>",
                    ox + x1,
                    oy + y1,
                    ox + x2,
                    oy + y2
                );
            }
            SvgEl::Path {
                d,
                stroke_width,
                fill,
                color,
            } => {
                let paint = color
                    .as_deref()
                    .or(inherited_color)
                    .unwrap_or("currentColor");
                let fill_attr = if *fill { paint } else { "none" };
                let _ = write!(
                    out,
                    "<path d=\"{d}\" transform=\"translate({ox:.2},{oy:.2})\" fill=\"{fill_attr}\" stroke=\"{paint}\" stroke-width=\"{stroke_width:.2}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>"
                );
            }
            SvgEl::Rect {
                x,
                y,
                w,
                h,
                stroke_width,
                color,
            } => {
                let stroke = color
                    .as_deref()
                    .or(inherited_color)
                    .unwrap_or("currentColor");
                let _ = write!(
                    out,
                    "<rect x=\"{:.2}\" y=\"{:.2}\" width=\"{w:.2}\" height=\"{h:.2}\" fill=\"none\" stroke=\"{stroke}\" stroke-width=\"{stroke_width:.2}\"/>",
                    ox + x,
                    oy + y
                );
            }
        }
    }
}

fn push_xml_escaped(out: &mut String, s: &str) {
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
}

// ---------------------------------------------------------------------------
// Symbol & Function Tables
// ---------------------------------------------------------------------------

fn lookup_function(cmd: &str) -> Option<&str> {
    match cmd {
        "sin" | "cos" | "tan" | "cot" | "sec" | "csc" | "arcsin" | "arccos" | "arctan" | "sinh"
        | "cosh" | "tanh" | "coth" | "log" | "ln" | "exp" | "det" | "dim" | "gcd" | "hom"
        | "ker" | "deg" | "arg" | "Pr" | "rank" | "tr" | "mod" | "bmod" => Some(cmd),
        _ => None,
    }
}

fn lookup_symbol(cmd: &str) -> Option<(&'static str, AtomKind, bool)> {
    use AtomKind::*;
    let entry = match cmd {
        // Large operators (limits above/below in display mode)
        "sum" => ("∑", Op, true),
        "prod" => ("∏", Op, true),
        "coprod" => ("∐", Op, true),
        "int" => ("∫", Op, true),
        "iint" => ("∬", Op, true),
        "iiint" => ("∭", Op, true),
        "oint" => ("∮", Op, true),
        "bigcup" => ("⋃", Op, true),
        "bigcap" => ("⋂", Op, true),
        "bigoplus" => ("⨁", Op, true),
        "bigotimes" => ("⨂", Op, true),
        "bigvee" => ("⋁", Op, true),
        "bigwedge" => ("⋀", Op, true),
        "lim" => ("lim", Op, true),
        "limsup" => ("lim sup", Op, true),
        "liminf" => ("lim inf", Op, true),
        "sup" => ("sup", Op, true),
        "inf" => ("inf", Op, true),
        "max" => ("max", Op, true),
        "min" => ("min", Op, true),
        "argmax" => ("arg max", Op, true),
        "argmin" => ("arg min", Op, true),

        // Lowercase Greek
        "alpha" => ("α", Ord, false),
        "beta" => ("β", Ord, false),
        "gamma" => ("γ", Ord, false),
        "delta" => ("δ", Ord, false),
        "epsilon" => ("ϵ", Ord, false),
        "varepsilon" => ("ε", Ord, false),
        "zeta" => ("ζ", Ord, false),
        "eta" => ("η", Ord, false),
        "theta" => ("θ", Ord, false),
        "vartheta" => ("ϑ", Ord, false),
        "iota" => ("ι", Ord, false),
        "kappa" => ("κ", Ord, false),
        "varkappa" => ("ϰ", Ord, false),
        "lambda" => ("λ", Ord, false),
        "mu" => ("μ", Ord, false),
        "nu" => ("ν", Ord, false),
        "xi" => ("ξ", Ord, false),
        "omicron" => ("ο", Ord, false),
        "pi" => ("π", Ord, false),
        "varpi" => ("ϖ", Ord, false),
        "rho" => ("ρ", Ord, false),
        "varrho" => ("ϱ", Ord, false),
        "sigma" => ("σ", Ord, false),
        "varsigma" => ("ς", Ord, false),
        "tau" => ("τ", Ord, false),
        "upsilon" => ("υ", Ord, false),
        "phi" => ("ϕ", Ord, false),
        "varphi" => ("φ", Ord, false),
        "chi" => ("χ", Ord, false),
        "psi" => ("ψ", Ord, false),
        "omega" => ("ω", Ord, false),

        // Uppercase Greek
        "Gamma" => ("Γ", Ord, false),
        "Delta" => ("Δ", Ord, false),
        "Theta" => ("Θ", Ord, false),
        "Lambda" => ("Λ", Ord, false),
        "Xi" => ("Ξ", Ord, false),
        "Pi" => ("Π", Ord, false),
        "Sigma" => ("Σ", Ord, false),
        "Upsilon" => ("Υ", Ord, false),
        "Phi" => ("Φ", Ord, false),
        "Psi" => ("Ψ", Ord, false),
        "Omega" => ("Ω", Ord, false),

        // Binary operators
        "pm" => ("±", Bin, false),
        "mp" => ("∓", Bin, false),
        "times" => ("×", Bin, false),
        "div" => ("÷", Bin, false),
        "cdot" => ("·", Bin, false),
        "ast" => ("∗", Bin, false),
        "star" => ("⋆", Bin, false),
        "circ" => ("∘", Bin, false),
        "bullet" => ("•", Bin, false),
        "oplus" => ("⊕", Bin, false),
        "ominus" => ("⊖", Bin, false),
        "otimes" => ("⊗", Bin, false),
        "oslash" => ("⊘", Bin, false),
        "odot" => ("⊙", Bin, false),
        "cup" => ("∪", Bin, false),
        "cap" => ("∩", Bin, false),
        "setminus" | "smallsetminus" => ("∖", Bin, false),
        "wedge" | "land" => ("∧", Bin, false),
        "vee" | "lor" => ("∨", Bin, false),
        "dagger" => ("†", Bin, false),
        "ddagger" => ("‡", Bin, false),

        // Relations
        "le" | "leq" => ("≤", Rel, false),
        "ge" | "geq" => ("≥", Rel, false),
        "neq" | "ne" => ("≠", Rel, false),
        "approx" => ("≈", Rel, false),
        "equiv" => ("≡", Rel, false),
        "sim" => ("∼", Rel, false),
        "simeq" => ("≃", Rel, false),
        "cong" => ("≅", Rel, false),
        "propto" => ("∝", Rel, false),
        "ll" => ("≪", Rel, false),
        "gg" => ("≫", Rel, false),
        "subset" => ("⊂", Rel, false),
        "subseteq" => ("⊆", Rel, false),
        "supset" => ("⊃", Rel, false),
        "supseteq" => ("⊇", Rel, false),
        "in" => ("∈", Rel, false),
        "notin" => ("∉", Rel, false),
        "ni" => ("∋", Rel, false),
        "perp" => ("⊥", Rel, false),
        "parallel" => ("∥", Rel, false),
        "mid" => ("∣", Rel, false),
        "models" => ("⊨", Rel, false),
        "vdash" => ("⊢", Rel, false),
        "dashv" => ("⊣", Rel, false),
        "prec" => ("≺", Rel, false),
        "succ" => ("≻", Rel, false),
        "preceq" => ("⪯", Rel, false),
        "succeq" => ("⪰", Rel, false),
        "triangleq" => ("≜", Rel, false),
        "coloneqq" => ("≔", Rel, false),

        // Arrows
        "to" | "rightarrow" => ("→", Rel, false),
        "leftarrow" | "gets" => ("←", Rel, false),
        "leftrightarrow" => ("↔", Rel, false),
        "Rightarrow" | "implies" => ("⇒", Rel, false),
        "Leftarrow" | "impliedby" => ("⇐", Rel, false),
        "Leftrightarrow" | "iff" => ("⇔", Rel, false),
        "mapsto" => ("↦", Rel, false),
        "longrightarrow" => ("⟶", Rel, false),
        "longleftarrow" => ("⟵", Rel, false),
        "longleftrightarrow" => ("⟷", Rel, false),
        "Longrightarrow" => ("⟹", Rel, false),
        "Longleftarrow" => ("⟸", Rel, false),
        "Longleftrightarrow" => ("⟺", Rel, false),
        "uparrow" => ("↑", Rel, false),
        "downarrow" => ("↓", Rel, false),
        "updownarrow" => ("↕", Rel, false),
        "nearrow" => ("↗", Rel, false),
        "searrow" => ("↘", Rel, false),
        "hookrightarrow" => ("↪", Rel, false),
        "rightharpoonup" => ("⇀", Rel, false),

        // Ellipses & punctuation
        "cdots" => ("⋯", Ord, false),
        "ldots" | "dots" => ("…", Ord, false),
        "vdots" => ("⋮", Ord, false),
        "ddots" => ("⋱", Ord, false),
        "colon" => (":", Punct, false),

        // Misc math symbols
        "infty" => ("∞", Ord, false),
        "partial" => ("∂", Ord, false),
        "nabla" => ("∇", Ord, false),
        "forall" => ("∀", Ord, false),
        "exists" => ("∃", Ord, false),
        "nexists" => ("∄", Ord, false),
        "emptyset" | "varnothing" => ("∅", Ord, false),
        "neg" | "lnot" => ("¬", Ord, false),
        "angle" => ("∠", Ord, false),
        "triangle" => ("△", Ord, false),
        "square" | "Box" => ("□", Ord, false),
        "aleph" => ("ℵ", Ord, false),
        "hbar" => ("ℏ", Ord, false),
        "ell" => ("ℓ", Ord, false),
        "Re" => ("ℜ", Ord, false),
        "Im" => ("ℑ", Ord, false),
        "wp" => ("℘", Ord, false),
        "prime" => ("′", Ord, false),
        "degree" => ("°", Ord, false),
        "top" => ("⊤", Ord, false),
        "bot" => ("⊥", Ord, false),
        "checkmark" => ("✓", Ord, false),
        "therefore" => ("∴", Rel, false),
        "because" => ("∵", Rel, false),
        "{" | "lbrace" => ("{", Open, false),
        "}" | "rbrace" => ("}", Close, false),
        "|" | "Vert" => ("‖", Ord, false),
        "vert" => ("|", Ord, false),
        "langle" => ("⟨", Open, false),
        "rangle" => ("⟩", Close, false),
        "lfloor" => ("⌊", Open, false),
        "rfloor" => ("⌋", Close, false),
        "lceil" => ("⌈", Open, false),
        "rceil" => ("⌉", Close, false),
        "backslash" => ("\\", Ord, false),
        "%" => ("%", Ord, false),
        "$" => ("$", Ord, false),
        "&" => ("&", Ord, false),
        "#" => ("#", Ord, false),
        "_" => ("_", Ord, false),
        _ => return None,
    };
    Some(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(display: bool) -> MathOptions {
        MathOptions {
            font_size: if display { 15.0 } else { 13.0 },
            display,
        }
    }

    #[test]
    fn renders_quadratic_formula_with_fraction_and_radical() {
        let svg = render_tex(r"x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}", &opts(true)).unwrap();
        assert!(svg.starts_with("<svg "));
        assert!(svg.ends_with("</svg>"));
        assert!(svg.contains("<line "));
        assert!(svg.contains("<path "));
        assert!(svg.contains("±"));
    }

    #[test]
    fn renders_sum_and_integral_limits_in_display_and_inline() {
        let display_svg =
            render_tex(r"\sum_{i=1}^{n} i^2 = \frac{n(n+1)(2n+1)}{6}", &opts(true)).unwrap();
        assert!(display_svg.contains("∑"));
        let inline_svg = render_tex(r"\int_0^\infty e^{-x^2}\,dx", &opts(false)).unwrap();
        assert!(inline_svg.contains("∫"));
        assert!(inline_svg.contains("∞"));
    }

    #[test]
    fn renders_matrices_cases_and_aligned_environments() {
        let mat = render_tex(r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}", &opts(true)).unwrap();
        assert!(mat.contains("<svg "));
        assert!(mat.contains("<path "));

        let cases = render_tex(
            r"|x| = \begin{cases} x & \text{if } x \ge 0 \\ -x & \text{otherwise} \end{cases}",
            &opts(true),
        )
        .unwrap();
        assert!(cases.contains("≥"));
        assert!(cases.contains("otherwise"));
    }

    #[test]
    fn renders_unicode_variants_accents_and_xml_escapes() {
        let svg = render_tex(
            r"\forall x \in \mathbb{R}^n, \hat{x} < \vec{y} & \tilde{z}",
            &opts(true),
        )
        .unwrap();
        assert!(svg.contains("ℝ"));
        assert!(svg.contains("∀"));
        assert!(svg.contains("&lt;"));
    }

    #[test]
    fn rejects_invalid_and_empty_tex() {
        assert!(render_tex("   ", &opts(false)).is_err());
        assert!(render_tex(r"\frac{a}{b", &opts(true)).is_err());
        assert!(render_tex(r"x + }", &opts(false)).is_err());
        assert!(render_tex(r"\left( x + 1", &opts(false)).is_err());
        assert!(render_tex(r"\begin{pmatrix} 1 \end{bmatrix}", &opts(true)).is_err());
    }
}
