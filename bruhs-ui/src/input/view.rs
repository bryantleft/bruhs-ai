//! The field components: [`Input`] (one line) and [`Textarea`] (many).

use gpui::prelude::*;
use gpui::{
    div, px, rems, AnyElement, App, Edges, ElementId, Entity, Focusable, Font, Pixels, Role,
    SharedString, StyleRefinement, StyledText, TextRun, UnderlineStyle, Window,
};

use bruhs_models::num::ToFloatLossy as _;

use super::actions::*;
use super::element::EditorElement;
use super::state::{EditorState, FieldKind, InputContentType, InputState, TextareaState};
use crate::icons::{Icon, IconName};
use crate::theme;
use crate::zoom::px as css_px;

/// A field's size: its height and inner padding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Size {
    XSmall,
    Small,
    #[default]
    Medium,
    Large,
}

impl Size {
    fn height(self) -> f32 {
        match self {
            Size::XSmall => 20.0,
            Size::Small => 24.0,
            Size::Medium => 32.0,
            Size::Large => 44.0,
        }
    }

    /// Horizontal padding: the web's shared `Input` is `px-2.5`.
    fn px(self) -> f32 {
        match self {
            Size::XSmall => 4.0,
            Size::Small => 8.0,
            Size::Medium => 10.0,
            Size::Large => 12.0,
        }
    }

    fn py(self) -> f32 {
        match self {
            Size::XSmall => 0.0,
            Size::Small => 2.0,
            Size::Medium => 8.0,
            Size::Large => 10.0,
        }
    }
}

/// Components that come in sizes.
pub trait Sizable: Sized {
    fn with_size(self, size: Size) -> Self;
    fn xsmall(self) -> Self {
        self.with_size(Size::XSmall)
    }
    fn small(self) -> Self {
        self.with_size(Size::Small)
    }
    fn large(self) -> Self {
        self.with_size(Size::Large)
    }
}

/// The look and behaviour both field components share.
struct Chrome {
    style: StyleRefinement,
    size: Size,
    appearance: bool,
    bordered: bool,
    focus_bordered: bool,
    disabled: bool,
    /// Web `readonly`: focusable, selectable and copyable, never edited.
    readonly: bool,
    aria_label: Option<SharedString>,
    tab_index: isize,
    role: Option<Role>,
    content_type: Option<InputContentType>,
}

impl Default for Chrome {
    fn default() -> Self {
        Self {
            style: StyleRefinement::default(),
            size: Size::Medium,
            appearance: true,
            bordered: true,
            focus_bordered: true,
            disabled: false,
            readonly: false,
            aria_label: None,
            tab_index: 0,
            role: None,
            content_type: None,
        }
    }
}

const LINE_HEIGHT_REM: f32 = 1.25;
const RADIUS_PX: f32 = 8.0;
const DISABLED_OPACITY: f32 = 0.5;
const BUTTON_ICON_PX: f32 = 14.0;

/// The drawn text: the value (masked, with its ghost), or the placeholder.
fn text_element<K: FieldKind>(
    state: &Entity<EditorState<K>>,
    chrome: &Chrome,
    window: &Window,
    cx: &App,
) -> StyledText {
    let s = state.read(cx);
    let base = window.text_style();
    let mut font: Font = base.font();
    if let Some(family) = chrome.style.text.font_family.clone() {
        font.family = family;
    }
    let color = chrome.style.text.color.unwrap_or(theme::rambutan_100());
    if s.core.text.is_empty() && s.inline_completion.is_none() {
        let placeholder = if s.placeholder.is_empty() {
            SharedString::from(" ")
        } else {
            s.placeholder.clone()
        };
        let len = placeholder.len();
        return StyledText::new(placeholder).with_runs(vec![TextRun {
            len,
            font,
            color: theme::rambutan_500(),
            background_color: None,
            underline: None,
            strikethrough: None,
        }]);
    }
    let map = s.display_map();
    let value: String = if s.masked {
        s.core.text.chars().map(|_| super::state::MASK).collect()
    } else {
        s.core.text.clone()
    };
    let (display, ghost) = match (&s.inline_completion, map.ghost) {
        (Some(ghost), Some((at, _))) => {
            let at = map.to_display(at);
            let mut display = value;
            display.insert_str(at, ghost);
            (display, Some(at..at + ghost.len()))
        }
        _ => (value, None),
    };
    let selection = s.core.selection.range();
    let selection = map.to_display(selection.start)..map.to_display(selection.end);
    let marked = s
        .marked
        .clone()
        .map(|range| map.to_display(range.start)..map.to_display(range.end));
    // Cut runs at every boundary, then style each piece.
    let mut cuts = vec![0, display.len()];
    for range in [Some(selection.clone()), ghost.clone(), marked.clone()]
        .into_iter()
        .flatten()
    {
        cuts.extend([range.start, range.end]);
    }
    cuts.retain(|at| *at <= display.len());
    cuts.sort_unstable();
    cuts.dedup();
    let runs = cuts
        .windows(2)
        .map(|pair| {
            let (a, b) = (pair[0], pair[1]);
            let inside = |range: &Option<std::ops::Range<usize>>| {
                range
                    .as_ref()
                    .is_some_and(|r| a >= r.start && b <= r.end && r.start < r.end)
            };
            let in_ghost = inside(&ghost);
            TextRun {
                len: b - a,
                font: font.clone(),
                color: if in_ghost {
                    theme::rambutan_500()
                } else {
                    color
                },
                background_color: (!in_ghost && inside(&Some(selection.clone())))
                    .then(|| theme::persimmon_500().opacity(0.3)),
                underline: inside(&marked).then_some(UnderlineStyle {
                    thickness: px(1.0),
                    color: Some(color),
                    wavy: false,
                }),
                strikethrough: None,
            }
        })
        .collect();
    StyledText::new(display).with_runs(runs)
}

/// The focusable frame both components draw: key context, actions, focus
/// ring, accessibility.
fn frame<K: FieldKind>(
    id: ElementId,
    state: &Entity<EditorState<K>>,
    chrome: &Chrome,
    window: &mut Window,
    cx: &mut App,
) -> gpui::Stateful<gpui::Div> {
    // The field's tab stop: its index this frame, and none while disabled
    // or taken out of the order (a negative index).
    let focus = state
        .focus_handle(cx)
        .tab_index(chrome.tab_index)
        .tab_stop(!chrome.disabled && chrome.tab_index >= 0);
    let focused = focus.contains_focused(window, cx);
    let s = state.read(cx);
    // A password never reaches assistive tech, shown or hidden.
    let secret = s.masked
        || matches!(
            chrome.content_type,
            Some(InputContentType::Password | InputContentType::NewPassword)
        );
    let value = if secret {
        SharedString::default()
    } else {
        s.value()
    };
    let placeholder = s.placeholder.clone();
    // Unnamed, a field is called by its placeholder, as a web input is.
    let label = chrome
        .aria_label
        .clone()
        .or_else(|| (!placeholder.is_empty()).then(|| placeholder.clone()));
    let role = chrome.role.unwrap_or(if K::MULTI_LINE {
        Role::MultilineTextInput
    } else if secret {
        Role::PasswordInput
    } else {
        Role::TextInput
    });
    let mut el = div()
        .id(id)
        .key_context(CONTEXT)
        // A disabled field takes no focus, so its keys never resolve.
        .when(!chrome.disabled, |el| el.track_focus(&focus))
        .role(role)
        .aria_value(value)
        .aria_placeholder(placeholder)
        .when_some(label, |el, label| el.aria_label(label))
        .flex()
        .w_full()
        .line_height(rems(LINE_HEIGHT_REM))
        .text_size(css_px(theme::APP_INPUT_TEXT_SIZE_PX))
        .when(chrome.appearance, |el| {
            el.bg(gpui::white().opacity(0.05))
                .rounded(css_px(RADIUS_PX))
                .when(chrome.bordered, |el| {
                    el.border_1()
                        .border_color(if focused && chrome.focus_bordered {
                            theme::persimmon_400()
                        } else {
                            theme::ring_card()
                        })
                })
        })
        .when(chrome.disabled, |el| el.opacity(DISABLED_OPACITY));
    // A read-only field keeps its moves, selection and copy, not its edits.
    if !chrome.disabled && !chrome.readonly {
        el = el
            .on_action(window.listener_for(state, EditorState::backspace))
            .on_action(window.listener_for(state, EditorState::delete))
            .on_action(window.listener_for(state, EditorState::delete_to_beginning_of_line))
            .on_action(window.listener_for(state, EditorState::delete_to_end_of_line))
            .on_action(window.listener_for(state, EditorState::delete_to_previous_word_start))
            .on_action(window.listener_for(state, EditorState::delete_to_next_word_end))
            .on_action(window.listener_for(state, EditorState::cut))
            .on_action(window.listener_for(state, EditorState::paste))
            .on_action(window.listener_for(state, EditorState::undo))
            .on_action(window.listener_for(state, EditorState::redo));
    }
    if !chrome.disabled {
        el = el
            .on_action(window.listener_for(state, EditorState::left))
            .on_action(window.listener_for(state, EditorState::right))
            .on_action(window.listener_for(state, EditorState::up))
            .on_action(window.listener_for(state, EditorState::down))
            .on_action(window.listener_for(state, EditorState::select_left))
            .on_action(window.listener_for(state, EditorState::select_right))
            .on_action(window.listener_for(state, EditorState::select_up))
            .on_action(window.listener_for(state, EditorState::select_down))
            .on_action(window.listener_for(state, EditorState::home))
            .on_action(window.listener_for(state, EditorState::end))
            .on_action(window.listener_for(state, EditorState::move_to_start))
            .on_action(window.listener_for(state, EditorState::move_to_end))
            .on_action(window.listener_for(state, EditorState::previous_word))
            .on_action(window.listener_for(state, EditorState::next_word))
            .on_action(window.listener_for(state, EditorState::select_to_start_of_line))
            .on_action(window.listener_for(state, EditorState::select_to_end_of_line))
            .on_action(window.listener_for(state, EditorState::select_to_start))
            .on_action(window.listener_for(state, EditorState::select_to_end))
            .on_action(window.listener_for(state, EditorState::select_to_previous_word_start))
            .on_action(window.listener_for(state, EditorState::select_to_next_word_end))
            .on_action(window.listener_for(state, EditorState::select_all))
            .on_action(window.listener_for(state, EditorState::copy))
            .on_action(window.listener_for(state, EditorState::enter))
            .on_action(window.listener_for(state, EditorState::escape))
            .on_action(window.listener_for(state, EditorState::tab))
            .on_action(window.listener_for(state, EditorState::shift_tab))
            .on_action(window.listener_for(state, EditorState::character_palette))
            .on_action(window.listener_for(state, EditorState::page_up))
            .on_action(window.listener_for(state, EditorState::page_down));
    }
    el
}

fn icon_button(
    id: impl Into<ElementId>,
    icon: IconName,
    label: &'static str,
    on_press: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    crate::press::on_press(
        div()
            .id(id)
            .flex_none()
            .ml(css_px(4.0))
            .rounded(css_px(4.0))
            .role(Role::Button)
            .aria_label(label)
            .cursor_pointer()
            .text_color(theme::rambutan_400())
            .hover(|el| el.text_color(theme::rambutan_200()))
            .child(Icon::new(icon).size(css_px(BUTTON_ICON_PX))),
        on_press,
    )
    .into_any_element()
}

/// The edit menu a right click opens, at the pointer.
fn edit_menu<K: FieldKind>(state: &Entity<EditorState<K>>, cx: &App) -> Option<AnyElement> {
    let s = state.read(cx);
    let at = s.context_menu?;
    let can_copy = s.can_copy();
    let entity = state.entity_id();
    let item = |id: &'static str,
                label: &'static str,
                enabled: bool,
                run: fn(
        &mut EditorState<K>,
        &mut Window,
        &mut gpui::Context<'_, EditorState<K>>,
    )| {
        let state = state.clone();
        let row = div()
            .id((id, entity))
            .role(Role::MenuItem)
            .aria_label(label)
            .px(css_px(8.0))
            .py(css_px(4.0))
            .rounded(css_px(4.0))
            .text_size(css_px(12.0))
            .child(label);
        if !enabled {
            return row.text_color(theme::rambutan_500()).into_any_element();
        }
        crate::press::on_press(
            row.text_color(theme::rambutan_100())
                .cursor_pointer()
                .hover(|el| el.bg(gpui::white().opacity(0.05))),
            move |window, cx| {
                state.update(cx, |state, cx| {
                    state.close_context_menu(cx);
                    run(state, window, cx);
                });
            },
        )
        .into_any_element()
    };
    let close = state.clone();
    let menu = div()
        .id(("edit-menu", entity))
        .role(Role::Menu)
        .min_w(css_px(140.0))
        .p(css_px(4.0))
        .flex()
        .flex_col()
        .rounded(css_px(8.0))
        .bg(theme::longan_700())
        .border_1()
        .border_color(theme::ring_floating())
        .on_mouse_down_out(move |_, _, cx| {
            close.update(cx, |state, cx| state.close_context_menu(cx))
        })
        .child(item("edit-cut", "Cut", can_copy, |s, w, cx| {
            s.cut(&Cut, w, cx)
        }))
        .child(item("edit-copy", "Copy", can_copy, |s, w, cx| {
            s.copy(&Copy, w, cx)
        }))
        .child(item("edit-paste", "Paste", true, |s, w, cx| {
            s.paste(&Paste, w, cx)
        }))
        .child(item("edit-select-all", "Select All", true, |s, w, cx| {
            s.select_all(&SelectAll, w, cx)
        }));
    Some(
        gpui::deferred(gpui::anchored().position(at).child(menu))
            .with_priority(2)
            .into_any_element(),
    )
}

/// A single-line text field drawing an [`InputState`].
#[derive(IntoElement)]
pub struct Input {
    state: Entity<InputState>,
    chrome: Chrome,
    cleanable: bool,
    mask_toggle: bool,
}

impl Input {
    pub fn new(state: &Entity<InputState>) -> Self {
        Self {
            state: state.clone(),
            chrome: Chrome::default(),
            cleanable: false,
            mask_toggle: false,
        }
    }

    /// Whether it draws its own background and border (on by default).
    pub fn appearance(mut self, appearance: bool) -> Self {
        self.chrome.appearance = appearance;
        self
    }

    pub fn bordered(mut self, bordered: bool) -> Self {
        self.chrome.bordered = bordered;
        self
    }

    /// Whether focus turns the border persimmon (on by default).
    pub fn focus_bordered(mut self, bordered: bool) -> Self {
        self.chrome.focus_bordered = bordered;
        self
    }

    /// A clear button while it holds text.
    pub fn cleanable(mut self, cleanable: bool) -> Self {
        self.cleanable = cleanable;
        self
    }

    /// An eye button that shows a masked field's text.
    pub fn mask_toggle(mut self) -> Self {
        self.mask_toggle = true;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.chrome.disabled = disabled;
        self
    }

    /// Web `readonly`: the field still takes focus, moves its caret, selects
    /// and copies, but typing, pasting, cutting, deleting and undo do nothing.
    /// Single-line only: a textarea's Enter would still type a newline.
    pub fn readonly(mut self, readonly: bool) -> Self {
        self.chrome.readonly = readonly;
        self
    }

    pub fn aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.chrome.aria_label = Some(label.into());
        self
    }

    pub fn role(mut self, role: Role) -> Self {
        self.chrome.role = Some(role);
        self
    }

    pub fn tab_index(mut self, index: isize) -> Self {
        self.chrome.tab_index = index;
        self
    }

    /// What it holds, for platform autofill.
    pub fn content_type(mut self, content_type: InputContentType) -> Self {
        self.chrome.content_type = Some(content_type);
        self
    }
}

impl Sizable for Input {
    fn with_size(mut self, size: Size) -> Self {
        self.chrome.size = size;
        self
    }
}

impl Styled for Input {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.chrome.style
    }
}

impl RenderOnce for Input {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = self.state.clone();
        let size = self.chrome.size;
        let text = text_element(&state, &self.chrome, window, cx);
        let editor = EditorElement {
            state: state.clone(),
            text,
            caret_color: theme::persimmon_400(),
            disabled: self.chrome.disabled,
            readonly: self.chrome.readonly,
        };
        let s = state.read(cx);
        let show_clear = self.cleanable
            && !s.core.text.is_empty()
            && !self.chrome.disabled
            && !self.chrome.readonly;
        let masked = s.masked;
        let mut el = frame(
            ("input", state.entity_id()).into(),
            &state,
            &self.chrome,
            window,
            cx,
        )
        .items_center()
        .h(css_px(size.height()))
        .px(css_px(size.px()))
        .py(css_px(size.py()))
        .child(div().flex_1().min_w_0().overflow_hidden().child(editor));
        if show_clear {
            let clear = state.clone();
            el = el.child(icon_button(
                ("input-clear", state.entity_id()),
                IconName::Close,
                "Clear",
                move |window, cx| {
                    clear.update(cx, |state, cx| {
                        state.clean(window, cx);
                        state.focus(window, cx);
                    })
                },
            ));
        }
        if self.mask_toggle {
            let toggle = state.clone();
            el = el.child(icon_button(
                ("input-mask", state.entity_id()),
                if masked {
                    IconName::Eye
                } else {
                    IconName::EyeOff
                },
                if masked { "Show" } else { "Hide" },
                move |window, cx| {
                    toggle.update(cx, |state, cx| {
                        let masked = state.is_masked();
                        state.set_masked(!masked, window, cx);
                    })
                },
            ));
        }
        // The caller's styles win over the size's.
        el.style().refine(&self.chrome.style);
        el.children(edit_menu(&state, cx))
    }
}

/// A multi-line text field drawing a [`TextareaState`].
#[derive(IntoElement)]
pub struct Textarea {
    state: Entity<TextareaState>,
    chrome: Chrome,
    editor_padding: Option<Edges<Pixels>>,
}

impl Textarea {
    pub fn new(state: &Entity<TextareaState>) -> Self {
        Self {
            state: state.clone(),
            chrome: Chrome::default(),
            editor_padding: None,
        }
    }

    pub fn appearance(mut self, appearance: bool) -> Self {
        self.chrome.appearance = appearance;
        self
    }

    pub fn bordered(mut self, bordered: bool) -> Self {
        self.chrome.bordered = bordered;
        self
    }

    pub fn focus_bordered(mut self, bordered: bool) -> Self {
        self.chrome.focus_bordered = bordered;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.chrome.disabled = disabled;
        self
    }

    pub fn aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.chrome.aria_label = Some(label.into());
        self
    }

    pub fn tab_index(mut self, index: isize) -> Self {
        self.chrome.tab_index = index;
        self
    }

    /// The padding inside the scrolled area, in place of the size's: the
    /// text scrolls through it, and the scrollbar keeps clear of the text.
    pub fn editor_padding(mut self, padding: Edges<Pixels>) -> Self {
        self.editor_padding = Some(padding);
        self
    }
}

impl Sizable for Textarea {
    fn with_size(mut self, size: Size) -> Self {
        self.chrome.size = size;
        self
    }
}

impl Styled for Textarea {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.chrome.style
    }
}

impl RenderOnce for Textarea {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = self.state.clone();
        let size = self.chrome.size;
        let text = text_element(&state, &self.chrome, window, cx);
        let editor = EditorElement {
            state: state.clone(),
            text,
            caret_color: theme::persimmon_400(),
            disabled: self.chrome.disabled,
            readonly: self.chrome.readonly,
        };
        let s = state.read(cx);
        let line = window.rem_size() * LINE_HEIGHT_REM;
        let padding = self.editor_padding.unwrap_or(Edges {
            top: css_px(size.py()),
            right: css_px(size.px()),
            bottom: css_px(size.py()),
            left: css_px(size.px()),
        });
        let vertical = padding.top + padding.bottom;
        let min = line * s.rows.to_f32_lossy() + vertical;
        let max = s
            .auto_grow
            .map(|(_, max)| line * max.to_f32_lossy() + vertical);
        let fixed_height = self.chrome.style.size.height.is_some();
        let scroll = s.scroll.clone();
        let mut lines = div().w_full().child(editor);
        lines.style().padding = gpui::EdgesRefinement {
            top: Some(padding.top.into()),
            right: Some(padding.right.into()),
            bottom: Some(padding.bottom.into()),
            left: Some(padding.left.into()),
        };
        let area = div()
            .id(("textarea-scroll", state.entity_id()))
            .relative()
            .w_full()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&scroll)
            .when(!fixed_height, |el| {
                el.min_h(min).when_some(max, |el, max| el.max_h(max))
            })
            .when(fixed_height, |el| el.h_full())
            .on_mouse_down(gpui::MouseButton::Left, {
                let state = state.clone();
                move |event, window, cx| {
                    state.update(cx, |state, cx| {
                        state.click_below_text(event.position, window, cx)
                    });
                }
            })
            .child(lines)
            .child(crate::scroll::Scrollbar::vertical(&scroll));
        let mut el = frame(
            ("textarea", state.entity_id()).into(),
            &state,
            &self.chrome,
            window,
            cx,
        )
        .flex_col()
        .child(area);
        el.style().refine(&self.chrome.style);
        el.children(edit_menu(&state, cx))
    }
}
