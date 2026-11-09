//! One embedded terminal: `alacritty_terminal` keeps the VT state,
//! `bruhs_core::terminal::TerminalManager` owns the PTY, gpui draws the grid
//! and feeds keys back. Output reaches gpui through the same tokio →
//! async-channel bridge the local-server scan uses.

use bruhs_models::num::{ToFloatLossy, ToIntSaturating};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use alacritty_terminal::event::{Event as TermEvent, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point as GridPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config as TermConfig, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color as AnsiColor, CursorShape, NamedColor, Processor, Rgb};
use bruhs_core::terminal::{TerminalInfo, TerminalManager};
use gpui::prelude::*;
use gpui::*;
use tokio::sync::broadcast::error::RecvError;

use crate::theme;

pub const TERMINAL_FONT_FAMILY: &str = "Monaspace Neon";
pub const TERMINAL_FONT_SIZE_PX: f32 = 12.0;
pub const TERMINAL_LINE_HEIGHT_PX: f32 = 17.0;
pub const TERMINAL_PADDING_X_PX: f32 = 8.0;
pub const TERMINAL_PADDING_Y_PX: f32 = 4.0;
const SCROLLBACK_LINES: usize = 5000;
const MIN_COLS: usize = 2;
const MIN_ROWS: usize = 1;
/// Used until the font has been measured (Monaspace Neon at 12px is ~7.2px).
const FALLBACK_CELL_WIDTH_PX: f32 = 7.2;
const CURSOR_BAR_PX: f32 = 2.0;

// bruhs tones for the 16 ANSI slots, mirroring the xterm theme in
// bruhs-web/index.html.
const ANSI_NORMAL: [u32; 8] = [
    0x2a241d, 0xe57373, 0x8fcf8f, 0xe9c46a, 0x7fa7ff, 0xc99be0, 0x7fd1d1, 0xd9cdb9,
];
const ANSI_BRIGHT: [u32; 8] = [
    0x8a7b68, 0xff9a9a, 0xa8e6a8, 0xf5d78a, 0xa7cbff, 0xdbb9ec, 0xa0e6e6, 0xf6efe3,
];
const ANSI_DIM: [u32; 8] = [
    0x1f1a15, 0x9e5050, 0x628f62, 0xa08748, 0x5773b0, 0x8a6b9b, 0x579090, 0x968d80,
];
const DEFAULT_FOREGROUND: u32 = 0xd9cdb9;
const CUBE_STEPS: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// Grid size handed to alacritty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TermSize {
    pub cols: usize,
    pub rows: usize,
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// Columns × rows that fit in `size` with the given cell metrics.
pub fn grid_size_for(size: Size<Pixels>, cell_width: Pixels, line_height: Pixels) -> TermSize {
    let cols = (f32::from(size.width) / f32::from(cell_width))
        .floor()
        .max(0.0)
        .to_usize_saturating();
    let rows = (f32::from(size.height) / f32::from(line_height))
        .floor()
        .max(0.0)
        .to_usize_saturating();
    TermSize {
        cols: cols.max(MIN_COLS),
        rows: rows.max(MIN_ROWS),
    }
}

/// Terminal-side events: PTY write-backs (DA/DSR answers) go to the shell,
/// OSC titles land in the shared cell the tab reads.
#[derive(Clone)]
struct PtyBridge {
    manager: Option<TerminalManager>,
    id: String,
    title: Rc<RefCell<Option<String>>>,
}

impl EventListener for PtyBridge {
    fn send_event(&self, event: TermEvent) {
        match event {
            TermEvent::PtyWrite(text) => {
                if let Some(manager) = &self.manager {
                    let _ = manager.write(&self.id, text.as_bytes());
                }
            }
            TermEvent::Title(title) => *self.title.borrow_mut() = Some(title),
            TermEvent::ResetTitle => *self.title.borrow_mut() = None,
            _ => {}
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalStatus {
    Live,
    Ended,
}

impl TerminalStatus {
    pub fn label(self) -> &'static str {
        match self {
            TerminalStatus::Live => "Live",
            TerminalStatus::Ended => "Session ended",
        }
    }
}

/// Style of one run of cells, already resolved to colours.
#[derive(Clone, Debug, PartialEq)]
pub struct CellStyle {
    pub fg: Hsla,
    pub bg: Option<Hsla>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
}

/// One viewport row: text plus `(byte_len, style)` runs covering it.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderedRow {
    pub text: String,
    pub runs: Vec<(usize, CellStyle)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderedCursor {
    pub row: usize,
    pub col: usize,
    pub shape: CursorShape,
    pub ch: char,
}

fn hsla_from_hex(hex: u32) -> Hsla {
    rgb(hex).into()
}

fn hsla_from_rgb(c: Rgb) -> Hsla {
    hsla_from_hex((u32::from(c.r) << 16) | (u32::from(c.g) << 8) | u32::from(c.b))
}

fn named_hex(named: NamedColor) -> u32 {
    match named {
        NamedColor::Black => ANSI_NORMAL[0],
        NamedColor::Red => ANSI_NORMAL[1],
        NamedColor::Green => ANSI_NORMAL[2],
        NamedColor::Yellow => ANSI_NORMAL[3],
        NamedColor::Blue => ANSI_NORMAL[4],
        NamedColor::Magenta => ANSI_NORMAL[5],
        NamedColor::Cyan => ANSI_NORMAL[6],
        NamedColor::White => ANSI_NORMAL[7],
        NamedColor::BrightBlack => ANSI_BRIGHT[0],
        NamedColor::BrightRed => ANSI_BRIGHT[1],
        NamedColor::BrightGreen => ANSI_BRIGHT[2],
        NamedColor::BrightYellow => ANSI_BRIGHT[3],
        NamedColor::BrightBlue => ANSI_BRIGHT[4],
        NamedColor::BrightMagenta => ANSI_BRIGHT[5],
        NamedColor::BrightCyan => ANSI_BRIGHT[6],
        NamedColor::BrightWhite => ANSI_BRIGHT[7],
        NamedColor::DimBlack => ANSI_DIM[0],
        NamedColor::DimRed => ANSI_DIM[1],
        NamedColor::DimGreen => ANSI_DIM[2],
        NamedColor::DimYellow => ANSI_DIM[3],
        NamedColor::DimBlue => ANSI_DIM[4],
        NamedColor::DimMagenta => ANSI_DIM[5],
        NamedColor::DimCyan => ANSI_DIM[6],
        NamedColor::DimWhite => ANSI_DIM[7],
        NamedColor::Foreground | NamedColor::BrightForeground => DEFAULT_FOREGROUND,
        NamedColor::DimForeground => ANSI_DIM[7],
        NamedColor::Background => 0x141210,
        NamedColor::Cursor => 0xec7f2c,
    }
}

/// Resolve an ANSI colour to a bruhs tone. `bold` promotes the eight base
/// colours to their bright variants (xterm's "bold text in bright colors").
pub fn ansi_color(
    color: AnsiColor,
    overrides: &alacritty_terminal::term::color::Colors,
    bold: bool,
) -> Hsla {
    match color {
        AnsiColor::Spec(rgb) => hsla_from_rgb(rgb),
        AnsiColor::Named(named) => {
            if let Some(rgb) = overrides[named] {
                return hsla_from_rgb(rgb);
            }
            let promoted = match (bold, named) {
                (true, NamedColor::Black) => NamedColor::BrightBlack,
                (true, NamedColor::Red) => NamedColor::BrightRed,
                (true, NamedColor::Green) => NamedColor::BrightGreen,
                (true, NamedColor::Yellow) => NamedColor::BrightYellow,
                (true, NamedColor::Blue) => NamedColor::BrightBlue,
                (true, NamedColor::Magenta) => NamedColor::BrightMagenta,
                (true, NamedColor::Cyan) => NamedColor::BrightCyan,
                (true, NamedColor::White) => NamedColor::BrightWhite,
                _ => named,
            };
            hsla_from_hex(named_hex(promoted))
        }
        AnsiColor::Indexed(i) => {
            if let Some(rgb) = overrides[i as usize] {
                return hsla_from_rgb(rgb);
            }
            match i {
                0..=7 => hsla_from_hex(if bold {
                    ANSI_BRIGHT[i as usize]
                } else {
                    ANSI_NORMAL[i as usize]
                }),
                8..=15 => hsla_from_hex(ANSI_BRIGHT[i as usize - 8]),
                16..=231 => {
                    let n = i as usize - 16;
                    let (r, g, b) = (n / 36, (n % 36) / 6, n % 6);
                    hsla_from_hex(
                        (u32::from(CUBE_STEPS[r]) << 16)
                            | (u32::from(CUBE_STEPS[g]) << 8)
                            | u32::from(CUBE_STEPS[b]),
                    )
                }
                _ => {
                    let v = 8 + (u32::from(i) - 232) * 10;
                    hsla_from_hex((v << 16) | (v << 8) | v)
                }
            }
        }
    }
}

fn cell_style(
    cell: &alacritty_terminal::term::cell::Cell,
    colors: &alacritty_terminal::term::color::Colors,
    selected: bool,
) -> CellStyle {
    let flags = cell.flags;
    let bold = flags.contains(Flags::BOLD);
    let mut fg = ansi_color(cell.fg, colors, bold);
    let mut bg = match cell.bg {
        AnsiColor::Named(NamedColor::Background) => None,
        other => Some(ansi_color(other, colors, false)),
    };
    if flags.contains(Flags::INVERSE) {
        let old_bg = bg.unwrap_or_else(theme::longan_950);
        bg = Some(fg);
        fg = old_bg;
    }
    if flags.contains(Flags::DIM) {
        fg = fg.opacity(0.6);
    }
    if flags.contains(Flags::HIDDEN) {
        fg = bg.unwrap_or_else(theme::longan_950);
    }
    if selected {
        bg = Some(theme::longan_700());
    }
    CellStyle {
        fg,
        bg,
        bold,
        italic: flags.contains(Flags::ITALIC),
        underline: flags.intersects(
            Flags::UNDERLINE
                | Flags::DOUBLE_UNDERLINE
                | Flags::UNDERCURL
                | Flags::DOTTED_UNDERLINE
                | Flags::DASHED_UNDERLINE,
        ),
        strike: flags.contains(Flags::STRIKEOUT),
    }
}

fn is_plain(style: &CellStyle) -> bool {
    style.bg.is_none() && !style.underline && !style.strike
}

/// Snapshot the viewport as rows of merged style runs plus the cursor
/// (`None` when hidden or scrolled out of view).
pub fn rows_from_term<T: EventListener>(
    term: &Term<T>,
) -> (Vec<RenderedRow>, Option<RenderedCursor>) {
    let content = term.renderable_content();
    let rows = term.screen_lines();
    let offset = i32::try_from(content.display_offset).unwrap_or(i32::MAX);
    let mut out: Vec<RenderedRow> = (0..rows)
        .map(|_| RenderedRow {
            text: String::new(),
            runs: Vec::new(),
        })
        .collect();
    for indexed in content.display_iter {
        let cell = indexed.cell;
        if cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        {
            continue;
        }
        let Some(row) = usize::try_from(indexed.point.line.0.saturating_add(offset))
            .ok()
            .filter(|row| *row < rows)
        else {
            continue;
        };
        let selected = content
            .selection
            .is_some_and(|range| range.contains(indexed.point));
        let style = cell_style(cell, content.colors, selected);
        let ch = if cell.c == '\0' { ' ' } else { cell.c };
        let target = &mut out[row];
        target.text.push(ch);
        match target.runs.last_mut() {
            Some((len, last)) if *last == style => *len += ch.len_utf8(),
            _ => target.runs.push((ch.len_utf8(), style)),
        }
    }
    for row in &mut out {
        // Drop trailing plain spaces so short lines shape quickly.
        while let Some((len, style)) = row.runs.last_mut() {
            if !is_plain(style) || !row.text.ends_with(' ') {
                break;
            }
            row.text.pop();
            *len -= 1;
            if *len == 0 {
                row.runs.pop();
            }
        }
    }
    let cursor_row = usize::try_from(content.cursor.point.line.0.saturating_add(offset))
        .ok()
        .filter(|row| *row < rows);
    let cursor = cursor_row
        .filter(|_| content.cursor.shape != CursorShape::Hidden)
        .map(|row| RenderedCursor {
            row,
            col: content.cursor.point.column.0,
            shape: content.cursor.shape,
            ch: term.grid()[content.cursor.point].c,
        });
    (out, cursor)
}

fn modifier_param(m: &Modifiers) -> u8 {
    1 + u8::from(m.shift) + (u8::from(m.alt) << 1) + (u8::from(m.control) << 2)
}

/// Bytes to send for a keystroke, or `None` when the terminal has no use for
/// it (⌘-chords, bare modifiers) and the workspace should see it instead.
pub fn keystroke_to_bytes(keystroke: &Keystroke, mode: TermMode) -> Option<Vec<u8>> {
    let m = &keystroke.modifiers;
    if m.platform || m.function {
        return None;
    }
    let key = keystroke.key.as_str();
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    let plain = !m.shift && !m.alt && !m.control;
    let csi = |suffix: &str| {
        if plain {
            format!("\x1b[{suffix}").into_bytes()
        } else {
            let (num, letter) = suffix.split_at(suffix.len() - 1);
            let num = if num.is_empty() { "1" } else { num };
            format!("\x1b[{num};{}{letter}", modifier_param(m)).into_bytes()
        }
    };
    let tilde = |code: u8| {
        if plain {
            format!("\x1b[{code}~").into_bytes()
        } else {
            format!("\x1b[{code};{}~", modifier_param(m)).into_bytes()
        }
    };
    let arrow = |letter: &str| {
        if plain && app_cursor {
            format!("\x1bO{letter}").into_bytes()
        } else {
            csi(letter)
        }
    };
    let bytes = match key {
        "enter" => b"\r".to_vec(),
        "backspace" => {
            if m.alt {
                b"\x1b\x7f".to_vec()
            } else {
                b"\x7f".to_vec()
            }
        }
        "tab" => {
            if m.shift {
                b"\x1b[Z".to_vec()
            } else {
                b"\t".to_vec()
            }
        }
        "escape" => b"\x1b".to_vec(),
        "space" => {
            if m.control {
                vec![0]
            } else {
                b" ".to_vec()
            }
        }
        "up" => arrow("A"),
        "down" => arrow("B"),
        "right" => arrow("C"),
        "left" => arrow("D"),
        "home" => arrow("H"),
        "end" => arrow("F"),
        "insert" => tilde(2),
        "delete" => tilde(3),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "f1" => b"\x1bOP".to_vec(),
        "f2" => b"\x1bOQ".to_vec(),
        "f3" => b"\x1bOR".to_vec(),
        "f4" => b"\x1bOS".to_vec(),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        _ => {
            let mut chars = key.chars();
            let single = match (chars.next(), chars.next()) {
                (Some(c), None) => Some(c),
                _ => None,
            };
            if m.control {
                let c = single?;
                let byte = match c.to_ascii_lowercase() {
                    c @ 'a'..='z' => c as u8 - b'a' + 1,
                    '[' | '3' => 0x1b,
                    '\\' | '4' => 0x1c,
                    ']' | '5' => 0x1d,
                    '^' | '6' => 0x1e,
                    '_' | '7' | '/' => 0x1f,
                    '@' | '2' => 0x00,
                    '?' | '8' => 0x7f,
                    _ => return None,
                };
                if m.alt {
                    vec![0x1b, byte]
                } else {
                    vec![byte]
                }
            } else if m.alt {
                let c = single?;
                let mut out = vec![0x1b];
                let typed = if m.shift { c.to_ascii_uppercase() } else { c };
                let mut buf = [0u8; 4];
                out.extend_from_slice(typed.encode_utf8(&mut buf).as_bytes());
                out
            } else if let Some(text) = keystroke.key_char.as_deref().filter(|t| !t.is_empty()) {
                text.as_bytes().to_vec()
            } else {
                let c = single?;
                let typed = if m.shift { c.to_ascii_uppercase() } else { c };
                let mut buf = [0u8; 4];
                typed.encode_utf8(&mut buf).as_bytes().to_vec()
            }
        }
    };
    Some(bytes)
}

/// Pasted text as the shell expects it: CR line endings, bracketed when the
/// program asked for it.
pub fn paste_bytes(text: &str, mode: TermMode) -> Vec<u8> {
    let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
    if mode.contains(TermMode::BRACKETED_PASTE) {
        format!("\x1b[200~{normalized}\x1b[201~").into_bytes()
    } else {
        normalized.into_bytes()
    }
}

pub struct TerminalView {
    pub info: TerminalInfo,
    manager: Option<TerminalManager>,
    /// Title set by the program (OSC 0/2); the tab falls back to `info.title`.
    osc_title: Rc<RefCell<Option<String>>>,
    term: Term<PtyBridge>,
    parser: Processor,
    focus_handle: FocusHandle,
    size: TermSize,
    cell_width: Pixels,
    /// Content bounds from the last frame (mouse → grid mapping).
    bounds: Bounds<Pixels>,
    selecting: bool,
    scroll_remainder: f32,
    status: TerminalStatus,
    /// Where the output forwarder runs; `None` for a view without a PTY.
    runtime: Option<Arc<tokio::runtime::Runtime>>,
    _pump: Option<Task<()>>,
}

/// Output chunks the forwarder may queue for the UI before it waits; while
/// it waits, the session's broadcast buffer absorbs output, and past that
/// the view resyncs from scrollback (see [`PumpMessage::Lagged`]).
const PUMP_QUEUE_CHUNKS: usize = 256;

/// What the output forwarder hands the UI thread.
pub(crate) enum PumpMessage {
    Output(Arc<[u8]>),
    /// The view fell behind and output was dropped before it was read; the
    /// screen is rebuilt from the session's scrollback.
    Lagged,
    /// The program exited.
    Closed,
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TerminalView {
    /// Attach to a live session: replay its scrollback, then stream.
    pub fn new(
        info: TerminalInfo,
        manager: TerminalManager,
        runtime: Arc<tokio::runtime::Runtime>,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let size = TermSize {
            cols: (info.cols as usize).max(MIN_COLS),
            rows: (info.rows as usize).max(MIN_ROWS),
        };
        let bridge = PtyBridge {
            manager: Some(manager.clone()),
            id: info.id.clone(),
            title: Rc::new(RefCell::new(None)),
        };
        let mut view = Self::build(info, Some(manager.clone()), size, bridge, cx);
        view.runtime = Some(runtime);
        match manager.attach(&view.info.id) {
            Ok(attachment) => {
                view.process_bytes(&attachment.replay);
                view.start_pump(attachment.output, cx);
            }
            Err(_) => view.status = TerminalStatus::Ended,
        }
        view
    }

    /// Forward the session's output to this view: a forwarder task on the
    /// runtime feeds a bounded queue, and the UI drains everything queued on
    /// each wake, so a burst of output costs one update and one redraw.
    fn start_pump(
        &mut self,
        mut output: tokio::sync::broadcast::Receiver<Arc<[u8]>>,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(runtime) = self.runtime.clone() else {
            return;
        };
        let (tx, rx) = async_channel::bounded::<PumpMessage>(PUMP_QUEUE_CHUNKS);
        runtime.spawn(async move {
            loop {
                let message = match output.recv().await {
                    Ok(chunk) => PumpMessage::Output(chunk),
                    Err(RecvError::Lagged(_)) => PumpMessage::Lagged,
                    Err(RecvError::Closed) => PumpMessage::Closed,
                };
                let last = !matches!(message, PumpMessage::Output(_));
                // A closed queue means the view is gone.
                if tx.send(message).await.is_err() || last {
                    break;
                }
            }
        });
        self._pump = Some(cx.spawn(async move |this, cx| {
            while let Ok(first) = rx.recv().await {
                let mut batch = vec![first];
                while let Ok(next) = rx.try_recv() {
                    batch.push(next);
                }
                let keep_going = this.update(cx, |view, cx| view.apply_pump(batch, cx));
                if !matches!(keep_going, Ok(true)) {
                    break;
                }
            }
        }));
    }

    /// Apply one drained batch; false once this pump is finished (the
    /// program exited, or a resync replaced it).
    pub(crate) fn apply_pump(
        &mut self,
        batch: Vec<PumpMessage>,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let mut keep_going = true;
        for message in batch {
            match message {
                PumpMessage::Output(bytes) => self.process_bytes(&bytes),
                PumpMessage::Lagged => {
                    self.resync(cx);
                    keep_going = false;
                    break;
                }
                PumpMessage::Closed => {
                    self.status = TerminalStatus::Ended;
                    keep_going = false;
                    break;
                }
            }
        }
        cx.notify();
        keep_going
    }

    /// Output was dropped before the view read it: rebuild the screen from
    /// the session's scrollback and continue from there, rather than
    /// showing a screen with silently missing bytes.
    fn resync(&mut self, cx: &mut Context<'_, Self>) {
        let Some(manager) = self.manager.clone() else {
            return;
        };
        match manager.attach(&self.info.id) {
            Ok(attachment) => {
                tracing::warn!(
                    "terminal {} fell behind its output; redrawing from scrollback",
                    self.info.id
                );
                self.reset_screen();
                self.process_bytes(&attachment.replay);
                self.start_pump(attachment.output, cx);
            }
            Err(_) => self.status = TerminalStatus::Ended,
        }
    }

    /// A blank emulator of the current size (the program's title is kept).
    fn reset_screen(&mut self) {
        let bridge = PtyBridge {
            manager: self.manager.clone(),
            id: self.info.id.clone(),
            title: Rc::clone(&self.osc_title),
        };
        self.term = Term::new(Self::term_config(), &self.size, bridge);
        self.parser = Processor::new();
    }

    fn term_config() -> TermConfig {
        TermConfig {
            scrolling_history: SCROLLBACK_LINES,
            ..TermConfig::default()
        }
    }

    /// A view with no PTY behind it; bytes arrive through `feed`.
    #[cfg(test)]
    pub fn detached(info: TerminalInfo, size: TermSize, cx: &mut Context<'_, Self>) -> Self {
        let bridge = PtyBridge {
            manager: None,
            id: info.id.clone(),
            title: Rc::new(RefCell::new(None)),
        };
        Self::build(info, None, size, bridge, cx)
    }

    fn build(
        info: TerminalInfo,
        manager: Option<TerminalManager>,
        size: TermSize,
        bridge: PtyBridge,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let config = Self::term_config();
        let osc_title = Rc::clone(&bridge.title);
        Self {
            info,
            manager,
            osc_title,
            term: Term::new(config, &size, bridge),
            parser: Processor::new(),
            focus_handle: cx.focus_handle(),
            size,
            cell_width: px(FALLBACK_CELL_WIDTH_PX),
            bounds: Bounds::default(),
            selecting: false,
            scroll_remainder: 0.0,
            status: TerminalStatus::Live,
            runtime: None,
            _pump: None,
        }
    }

    pub fn status(&self) -> TerminalStatus {
        self.status
    }

    /// Tab label: the program's OSC title when it set one, else the
    /// project/folder name the session was opened with.
    pub fn title(&self) -> String {
        self.osc_title
            .borrow()
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| self.info.title.clone())
    }

    #[cfg(any(test, debug_assertions))]
    pub fn size(&self) -> TermSize {
        self.size
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    /// Feed raw output into the emulator.
    pub fn process_bytes(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    #[cfg(test)]
    pub fn feed(&mut self, bytes: &[u8], cx: &mut Context<'_, Self>) {
        self.process_bytes(bytes);
        cx.notify();
    }

    /// Rows and cursor as they would be drawn.
    #[cfg(any(test, debug_assertions))]
    pub fn snapshot(&self) -> (Vec<RenderedRow>, Option<RenderedCursor>) {
        rows_from_term(&self.term)
    }

    #[cfg(test)]
    pub fn selection_text(&self) -> Option<String> {
        self.term.selection_to_string()
    }

    /// The highlighted terminal text, when the selection is not blank.
    pub fn selected_text(&self) -> Option<String> {
        self.term
            .selection_to_string()
            .filter(|text| !text.trim().is_empty())
    }

    fn write(&self, bytes: &[u8]) {
        if let Some(manager) = &self.manager {
            // Non-blocking enqueue, so the UI thread never waits on the PTY.
            // The error is dropped on purpose: when the program has stopped
            // reading and its input queue is full, or the shell is gone, the
            // keystrokes are discarded like a wedged terminal would.
            let _ = manager.write(&self.info.id, bytes);
        }
    }

    fn set_content_bounds(&mut self, bounds: Bounds<Pixels>, cx: &mut Context<'_, Self>) {
        self.bounds = bounds;
        let next = grid_size_for(bounds.size, self.cell_width, px(TERMINAL_LINE_HEIGHT_PX));
        if next != self.size {
            self.size = next;
            self.term.resize(next);
            if let Some(manager) = &self.manager {
                let _ = manager.resize(
                    &self.info.id,
                    u16::try_from(next.cols).unwrap_or(u16::MAX),
                    u16::try_from(next.rows).unwrap_or(u16::MAX),
                );
            }
            cx.notify();
        }
    }

    fn grid_point(&self, position: gpui::Point<Pixels>) -> (GridPoint, Side) {
        let cell_w = f32::from(self.cell_width).max(1.0);
        let x = f32::from(position.x - self.bounds.origin.x).max(0.0);
        let y = f32::from(position.y - self.bounds.origin.y).max(0.0);
        let col = (x / cell_w)
            .floor()
            .to_usize_saturating()
            .min(self.size.cols.saturating_sub(1));
        let row = (y / TERMINAL_LINE_HEIGHT_PX)
            .floor()
            .to_usize_saturating()
            .min(self.size.rows.saturating_sub(1));
        let side = if x - col.to_f32_lossy() * cell_w > cell_w / 2.0 {
            Side::Right
        } else {
            Side::Left
        };
        let offset = i32::try_from(self.term.grid().display_offset()).unwrap_or(i32::MAX);
        let line = i32::try_from(row)
            .unwrap_or(i32::MAX)
            .saturating_sub(offset);
        (GridPoint::new(Line(line), Column(col)), side)
    }

    fn handle_key(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let keystroke = &event.keystroke;
        if keystroke.modifiers.platform {
            match keystroke.key.as_str() {
                "v" if !keystroke.modifiers.shift => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                        self.write(&paste_bytes(&text, *self.term.mode()));
                        self.term.scroll_display(Scroll::Bottom);
                        cx.stop_propagation();
                        cx.notify();
                    }
                }
                "c" if !keystroke.modifiers.shift => {
                    if let Some(text) = self.term.selection_to_string() {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                        cx.stop_propagation();
                    }
                }
                _ => {}
            }
            return;
        }
        if let Some(bytes) = keystroke_to_bytes(keystroke, *self.term.mode()) {
            self.write(&bytes);
            self.term.selection = None;
            self.term.scroll_display(Scroll::Bottom);
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn handle_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        if event.button != MouseButton::Left {
            return;
        }
        let (point, side) = self.grid_point(event.position);
        let kind = match event.click_count {
            2 => SelectionType::Semantic,
            n if n >= 3 => SelectionType::Lines,
            _ => SelectionType::Simple,
        };
        self.term.selection = Some(Selection::new(kind, point, side));
        self.selecting = true;
        cx.notify();
    }

    fn handle_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.selecting || event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let (point, side) = self.grid_point(event.position);
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, side);
            cx.notify();
        }
    }

    fn handle_mouse_up(
        &mut self,
        _event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.selecting {
            return;
        }
        self.selecting = false;
        if self
            .term
            .selection
            .as_ref()
            .is_some_and(|s| s.to_range(&self.term).is_none())
        {
            self.term.selection = None;
        }
        cx.notify();
    }

    fn handle_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let lines = match event.delta {
            ScrollDelta::Lines(delta) => delta.y,
            ScrollDelta::Pixels(delta) => f32::from(delta.y) / TERMINAL_LINE_HEIGHT_PX,
        };
        self.scroll_remainder += lines;
        let whole = self.scroll_remainder.trunc();
        if whole == 0.0 {
            return;
        }
        self.scroll_remainder -= whole;
        let mode = *self.term.mode();
        if mode.contains(TermMode::ALT_SCREEN) && mode.contains(TermMode::ALTERNATE_SCROLL) {
            // Full-screen programs get arrow keys instead of history.
            let key = if whole > 0.0 { b"\x1bOA" } else { b"\x1bOB" };
            let count = whole.abs().to_usize_saturating();
            let mut out = Vec::with_capacity(count * 3);
            for _ in 0..count {
                out.extend_from_slice(if mode.contains(TermMode::APP_CURSOR) {
                    key
                } else if whole > 0.0 {
                    b"\x1b[A"
                } else {
                    b"\x1b[B"
                });
            }
            self.write(&out);
        } else {
            self.term
                .scroll_display(Scroll::Delta(whole.to_i32_saturating()));
        }
        cx.notify();
    }

    fn run_font(style: &CellStyle) -> Font {
        let mut f = font(TERMINAL_FONT_FAMILY);
        if style.bold {
            f.weight = FontWeight::BOLD;
        }
        if style.italic {
            f.style = FontStyle::Italic;
        }
        f
    }

    fn render_row(row: RenderedRow) -> AnyElement {
        let runs: Vec<TextRun> = row
            .runs
            .iter()
            .map(|(len, style)| TextRun {
                len: *len,
                font: Self::run_font(style),
                color: style.fg,
                background_color: style.bg,
                underline: style.underline.then(|| UnderlineStyle {
                    thickness: px(1.0),
                    color: Some(style.fg),
                    wavy: false,
                }),
                strikethrough: style.strike.then(|| StrikethroughStyle {
                    thickness: px(1.0),
                    color: Some(style.fg),
                }),
            })
            .collect();
        let text: SharedString = row.text.into();
        div()
            .h(px(TERMINAL_LINE_HEIGHT_PX))
            .flex_shrink_0()
            .whitespace_nowrap()
            .child(StyledText::new(text).with_runs(runs))
            .into_any_element()
    }

    fn render_cursor(&self, cursor: &RenderedCursor, focused: bool) -> AnyElement {
        let left = self.cell_width * cursor.col.to_f32_lossy();
        let top = px(TERMINAL_LINE_HEIGHT_PX * cursor.row.to_f32_lossy());
        let base = div()
            .debug_selector(|| "desktop-terminal-cursor".to_string())
            .absolute()
            .left(left)
            .top(top)
            .h(px(TERMINAL_LINE_HEIGHT_PX));
        match (cursor.shape, focused) {
            (CursorShape::Beam, _) => base.w(px(CURSOR_BAR_PX)).bg(theme::persimmon_400()),
            (CursorShape::Underline, _) => base
                .w(self.cell_width)
                .top(top + px(TERMINAL_LINE_HEIGHT_PX - CURSOR_BAR_PX))
                .h(px(CURSOR_BAR_PX))
                .bg(theme::persimmon_400()),
            (_, false) => base
                .w(self.cell_width)
                .border_1()
                .border_color(theme::persimmon_400()),
            (_, true) => {
                let ch = if cursor.ch == '\0' || cursor.ch == ' ' {
                    String::new()
                } else {
                    cursor.ch.to_string()
                };
                base.w(self.cell_width)
                    .bg(theme::persimmon_400())
                    .text_color(theme::longan_950())
                    .child(ch)
            }
        }
        .into_any_element()
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let font_id = window
            .text_system()
            .resolve_font(&font(TERMINAL_FONT_FAMILY));
        if let Ok(width) = window
            .text_system()
            .em_advance(font_id, px(TERMINAL_FONT_SIZE_PX))
        {
            if width > px(0.0) {
                self.cell_width = width;
            }
        }
        let focused = self.focus_handle.is_focused(window);
        let (rows, cursor) = rows_from_term(&self.term);
        let row_elements: Vec<AnyElement> = rows.into_iter().map(Self::render_row).collect();
        let cursor_element = cursor.map(|c| self.render_cursor(&c, focused));
        let view = cx.entity();
        let measure = canvas(
            move |bounds, _window, cx| {
                view.update(cx, |this, cx| this.set_content_bounds(bounds, cx));
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();

        div()
            .id(ElementId::Name(format!("terminal-{}", self.info.id).into()))
            .debug_selector(|| "desktop-terminal-view".to_string())
            .key_context("Terminal")
            .track_focus(&self.focus_handle)
            .role(Role::Terminal)
            .aria_label(format!("Terminal {}", self.info.title))
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .bg(theme::longan_950())
            .cursor(CursorStyle::IBeam)
            .px(px(TERMINAL_PADDING_X_PX))
            .py(px(TERMINAL_PADDING_Y_PX))
            .on_key_down(cx.listener(Self::handle_key))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::handle_mouse_down))
            .on_mouse_move(cx.listener(Self::handle_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::handle_mouse_up))
            .on_scroll_wheel(cx.listener(Self::handle_scroll))
            .child(
                div()
                    .relative()
                    .size_full()
                    .overflow_hidden()
                    .font_family(TERMINAL_FONT_FAMILY)
                    .text_size(px(TERMINAL_FONT_SIZE_PX))
                    .line_height(px(TERMINAL_LINE_HEIGHT_PX))
                    .text_color(hsla_from_hex(DEFAULT_FOREGROUND))
                    .flex()
                    .flex_col()
                    .child(measure)
                    .children(row_elements)
                    .children(cursor_element),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ansi_color, grid_size_for, keystroke_to_bytes, paste_bytes, rows_from_term, CellStyle,
        PtyBridge, TermSize, ANSI_BRIGHT, ANSI_NORMAL,
    };
    use alacritty_terminal::term::color::Colors;
    use alacritty_terminal::term::{Config as TermConfig, Term, TermMode};
    use alacritty_terminal::vte::ansi::{
        Color as AnsiColor, CursorShape, NamedColor, Processor, Rgb,
    };
    use gpui::{px, rgb, size, Hsla, Keystroke, Modifiers};

    fn ks(key: &str, modifiers: Modifiers, key_char: Option<&str>) -> Keystroke {
        Keystroke {
            modifiers,
            key: key.to_string(),
            key_char: key_char.map(str::to_string),
        }
    }

    fn term_with(bytes: &[u8], cols: usize, rows: usize) -> Term<PtyBridge> {
        let bridge = PtyBridge {
            manager: None,
            id: "t".into(),
            title: std::rc::Rc::new(std::cell::RefCell::new(None)),
        };
        let mut term = Term::new(TermConfig::default(), &TermSize { cols, rows }, bridge);
        let mut parser: Processor = Processor::new();
        parser.advance(&mut term, bytes);
        term
    }

    fn hex(h: u32) -> Hsla {
        rgb(h).into()
    }

    #[test]
    fn grid_size_floors_and_clamps() {
        assert_eq!(
            grid_size_for(size(px(720.0), px(170.0)), px(7.2), px(17.0)),
            TermSize {
                cols: 100,
                rows: 10
            }
        );
        assert_eq!(
            grid_size_for(size(px(0.0), px(0.0)), px(7.2), px(17.0)),
            TermSize { cols: 2, rows: 1 }
        );
    }

    #[test]
    fn plain_keys_and_control_chords() {
        let none = Modifiers::default();
        let control = Modifiers {
            control: true,
            ..Modifiers::default()
        };
        let shift = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        let alt = Modifiers {
            alt: true,
            ..Modifiers::default()
        };
        let cmd = Modifiers {
            platform: true,
            ..Modifiers::default()
        };
        let mode = TermMode::empty();
        assert_eq!(
            keystroke_to_bytes(&ks("a", none, Some("a")), mode),
            Some(b"a".to_vec())
        );
        assert_eq!(
            keystroke_to_bytes(&ks("a", shift, Some("A")), mode),
            Some(b"A".to_vec())
        );
        assert_eq!(
            keystroke_to_bytes(&ks("enter", none, None), mode),
            Some(b"\r".to_vec())
        );
        assert_eq!(
            keystroke_to_bytes(&ks("backspace", none, None), mode),
            Some(b"\x7f".to_vec())
        );
        assert_eq!(
            keystroke_to_bytes(&ks("tab", shift, None), mode),
            Some(b"\x1b[Z".to_vec())
        );
        assert_eq!(
            keystroke_to_bytes(&ks("c", control, None), mode),
            Some(vec![0x03])
        );
        assert_eq!(
            keystroke_to_bytes(&ks("[", control, None), mode),
            Some(vec![0x1b])
        );
        assert_eq!(
            keystroke_to_bytes(&ks("b", alt, Some("∫")), mode),
            Some(b"\x1bb".to_vec())
        );
        assert_eq!(keystroke_to_bytes(&ks("k", cmd, None), mode), None);
        assert_eq!(
            keystroke_to_bytes(&ks("é", none, Some("é")), mode),
            Some("é".as_bytes().to_vec())
        );
    }

    #[test]
    fn arrows_follow_application_cursor_mode() {
        let none = Modifiers::default();
        let shift = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        assert_eq!(
            keystroke_to_bytes(&ks("up", none, None), TermMode::empty()),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            keystroke_to_bytes(&ks("up", none, None), TermMode::APP_CURSOR),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            keystroke_to_bytes(&ks("left", shift, None), TermMode::APP_CURSOR),
            Some(b"\x1b[1;2D".to_vec())
        );
        assert_eq!(
            keystroke_to_bytes(&ks("pageup", none, None), TermMode::empty()),
            Some(b"\x1b[5~".to_vec())
        );
        assert_eq!(
            keystroke_to_bytes(&ks("delete", shift, None), TermMode::empty()),
            Some(b"\x1b[3;2~".to_vec())
        );
        assert_eq!(
            keystroke_to_bytes(&ks("f5", none, None), TermMode::empty()),
            Some(b"\x1b[15~".to_vec())
        );
    }

    #[test]
    fn paste_normalizes_newlines_and_brackets_on_request() {
        assert_eq!(
            paste_bytes("a\r\nb\nc", TermMode::empty()),
            b"a\rb\rc".to_vec()
        );
        assert_eq!(
            paste_bytes("ls\n", TermMode::BRACKETED_PASTE),
            b"\x1b[200~ls\r\x1b[201~".to_vec()
        );
    }

    #[test]
    fn colors_map_to_bruhs_tones() {
        let colors = Colors::default();
        assert_eq!(
            ansi_color(AnsiColor::Named(NamedColor::Red), &colors, false),
            hex(ANSI_NORMAL[1])
        );
        assert_eq!(
            ansi_color(AnsiColor::Named(NamedColor::Red), &colors, true),
            hex(ANSI_BRIGHT[1])
        );
        assert_eq!(
            ansi_color(AnsiColor::Indexed(9), &colors, false),
            hex(ANSI_BRIGHT[1])
        );
        assert_eq!(
            ansi_color(AnsiColor::Indexed(196), &colors, false),
            hex(0xff0000)
        );
        assert_eq!(
            ansi_color(AnsiColor::Indexed(232), &colors, false),
            hex(0x080808)
        );
        assert_eq!(
            ansi_color(AnsiColor::Spec(Rgb { r: 1, g: 2, b: 3 }), &colors, false),
            hex(0x010203)
        );
    }

    #[test]
    fn rows_merge_runs_and_track_the_cursor() {
        let term = term_with(b"hi \x1b[31mred\x1b[0m!\r\nnext", 20, 3);
        let (rows, cursor) = rows_from_term(&term);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].text, "hi red!");
        let styles: Vec<(usize, bool)> = rows[0]
            .runs
            .iter()
            .map(|(len, s)| (*len, s.fg == hex(ANSI_NORMAL[1])))
            .collect();
        assert_eq!(styles, vec![(3, false), (3, true), (1, false)]);
        assert_eq!(rows[1].text, "next");
        assert!(rows[2].text.is_empty() && rows[2].runs.is_empty());
        let cursor = cursor.expect("cursor visible");
        assert_eq!((cursor.row, cursor.col), (1, 4));
        assert_eq!(cursor.shape, CursorShape::Block);
    }

    #[test]
    fn osc_titles_reach_the_bridge_and_reset() {
        let title = std::rc::Rc::new(std::cell::RefCell::new(None));
        let bridge = PtyBridge {
            manager: None,
            id: "t".into(),
            title: std::rc::Rc::clone(&title),
        };
        let mut term = Term::new(
            TermConfig::default(),
            &TermSize { cols: 20, rows: 2 },
            bridge,
        );
        let mut parser: Processor = Processor::new();
        parser.advance(&mut term, b"\x1b]0;my shell\x07");
        assert_eq!(title.borrow().as_deref(), Some("my shell"));
        parser.advance(&mut term, b"\x1b]2;\x07");
        assert!(title.borrow().as_deref().is_none_or(str::is_empty));
    }

    #[test]
    fn inverse_and_hidden_cells_swap_or_erase() {
        let term = term_with(b"\x1b[7mX\x1b[0m\x1b[8mY", 10, 1);
        let (rows, _) = rows_from_term(&term);
        let (_, inverse) = &rows[0].runs[0];
        let plain = CellStyle {
            fg: hex(0xd9cdb9),
            bg: None,
            bold: false,
            italic: false,
            underline: false,
            strike: false,
        };
        assert_eq!(inverse.bg, Some(plain.fg));
        let (_, hidden) = &rows[0].runs[1];
        assert_eq!(hidden.fg, crate::theme::longan_950());
    }
}

#[cfg(test)]
mod pump_tests {
    use super::{PumpMessage, TermSize, TerminalStatus, TerminalView};
    use bruhs_core::terminal::TerminalInfo;
    use gpui::{AppContext as _, Entity, TestAppContext};
    use std::cell::Cell;
    use std::rc::Rc;
    use std::sync::Arc;

    fn detached_view(cx: &mut TestAppContext) -> Entity<TerminalView> {
        let info = TerminalInfo {
            id: "sess-pump".into(),
            cwd: None,
            title: "pump".into(),
            project_name: None,
            cols: 40,
            rows: 5,
            created_at_ms: 1,
        };
        cx.new(|cx| TerminalView::detached(info, TermSize { cols: 40, rows: 5 }, cx))
    }

    fn output(text: &str) -> PumpMessage {
        PumpMessage::Output(Arc::from(text.as_bytes()))
    }

    #[gpui::test]
    fn a_drained_batch_is_applied_in_order_with_one_notify(cx: &mut TestAppContext) {
        let view = detached_view(cx);
        let notifies = Rc::new(Cell::new(0));
        let count = notifies.clone();
        let _observer = cx.update(|cx| cx.observe(&view, move |_, _| count.set(count.get() + 1)));

        let keep_going = view.update(cx, |view, cx| {
            view.apply_pump(vec![output("one "), output("two "), output("three")], cx)
        });
        cx.run_until_parked();

        assert!(keep_going);
        assert_eq!(notifies.get(), 1);
        view.read_with(cx, |view, _| {
            assert_eq!(view.snapshot().0[0].text, "one two three");
            assert_eq!(view.status(), TerminalStatus::Live);
        });
    }

    #[gpui::test]
    fn output_after_exit_in_the_same_batch_still_draws_what_came_before(cx: &mut TestAppContext) {
        let view = detached_view(cx);
        let keep_going = view.update(cx, |view, cx| {
            view.apply_pump(vec![output("bye"), PumpMessage::Closed], cx)
        });
        assert!(!keep_going);
        view.read_with(cx, |view, _| {
            assert_eq!(view.snapshot().0[0].text, "bye");
            assert_eq!(view.status(), TerminalStatus::Ended);
        });
    }

    #[gpui::test]
    fn falling_behind_ends_the_pump_for_a_resync(cx: &mut TestAppContext) {
        let view = detached_view(cx);
        let keep_going = view.update(cx, |view, cx| {
            view.apply_pump(vec![output("partial"), PumpMessage::Lagged], cx)
        });
        // Without a session there is nothing to resync from; with one, a new
        // pump starts from the scrollback replay.
        assert!(!keep_going);
    }
}
