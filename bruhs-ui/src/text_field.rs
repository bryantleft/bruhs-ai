//! One handle for a text field that is single-line or multi-line.
//!
//! bruhs-ui keeps the two as separate types: [`InputState`] renders
//! with [`Input`], and [`TextareaState`] (which has rows and auto-grow) renders
//! with [`Textarea`]. Forms that choose per field hold a [`TextField`], so
//! their read, write and render helpers take either kind.

use crate::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, SharedString, Styled,
    Subscription, Window,
};

/// A single-line or multi-line text field.
#[derive(Clone)]
pub enum TextField {
    Line(Entity<InputState>),
    Area(Entity<TextareaState>),
}

impl TextField {
    /// A new, empty field: multi-line when `multi_line` is set.
    pub fn new(multi_line: bool, window: &mut Window, cx: &mut App) -> Self {
        if multi_line {
            Self::Area(cx.new(|cx| TextareaState::new(window, cx)))
        } else {
            Self::Line(cx.new(|cx| InputState::new(window, cx)))
        }
    }

    pub fn is_multi_line(&self) -> bool {
        matches!(self, Self::Area(_))
    }

    pub fn value(&self, cx: &App) -> SharedString {
        match self {
            Self::Line(state) => state.read(cx).value(),
            Self::Area(state) => state.read(cx).value(),
        }
    }

    pub fn set_value(&self, value: impl Into<SharedString>, window: &mut Window, cx: &mut App) {
        let value: SharedString = value.into();
        match self {
            Self::Line(state) => state.update(cx, |state, cx| state.set_value(value, window, cx)),
            Self::Area(state) => state.update(cx, |state, cx| state.set_value(value, window, cx)),
        }
    }

    pub fn set_placeholder(
        &self,
        placeholder: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let placeholder: SharedString = placeholder.into();
        match self {
            Self::Line(state) => state.update(cx, |state, cx| {
                state.set_placeholder(placeholder, window, cx)
            }),
            Self::Area(state) => state.update(cx, |state, cx| {
                state.set_placeholder(placeholder, window, cx)
            }),
        }
    }

    pub fn focus_handle(&self, cx: &App) -> FocusHandle {
        match self {
            Self::Line(state) => state.focus_handle(cx),
            Self::Area(state) => state.focus_handle(cx),
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        match self {
            Self::Line(state) => state.update(cx, |state, cx| state.focus(window, cx)),
            Self::Area(state) => state.update(cx, |state, cx| state.focus(window, cx)),
        }
    }

    /// Calls `on_event` with each of the field's [`InputEvent`]s.
    pub fn subscribe<T: 'static>(
        &self,
        cx: &mut Context<'_, T>,
        mut on_event: impl FnMut(&mut T, &InputEvent, &mut Context<'_, T>) + 'static,
    ) -> Subscription {
        match self {
            Self::Line(state) => {
                cx.subscribe(state, move |this, _, event, cx| on_event(this, event, cx))
            }
            Self::Area(state) => {
                cx.subscribe(state, move |this, _, event, cx| on_event(this, event, cx))
            }
        }
    }

    /// [`Self::subscribe`] for a handler that needs the window.
    pub fn subscribe_in<T: 'static>(
        &self,
        window: &Window,
        cx: &mut Context<'_, T>,
        mut on_event: impl FnMut(&mut T, &InputEvent, &mut Window, &mut Context<'_, T>) + 'static,
    ) -> Subscription {
        match self {
            Self::Line(state) => {
                cx.subscribe_in(state, window, move |this, _, event, window, cx| {
                    on_event(this, event, window, cx)
                })
            }
            Self::Area(state) => {
                cx.subscribe_in(state, window, move |this, _, event, window, cx| {
                    on_event(this, event, window, cx)
                })
            }
        }
    }

    /// The field's element, named `label` for assistive tech.
    pub fn element(&self, label: impl Into<SharedString>, disabled: bool) -> TextFieldElement {
        let label: SharedString = label.into();
        match self {
            Self::Line(state) => {
                TextFieldElement::Line(Input::new(state).aria_label(label).disabled(disabled))
            }
            Self::Area(state) => {
                TextFieldElement::Area(Textarea::new(state).aria_label(label).disabled(disabled))
            }
        }
    }
}

impl From<Entity<InputState>> for TextField {
    fn from(state: Entity<InputState>) -> Self {
        Self::Line(state)
    }
}

impl From<Entity<TextareaState>> for TextField {
    fn from(state: Entity<TextareaState>) -> Self {
        Self::Area(state)
    }
}

/// [`TextField::element`]: an [`Input`] or a [`Textarea`], styled alike.
pub enum TextFieldElement {
    Line(Input),
    Area(Textarea),
}

impl Styled for TextFieldElement {
    fn style(&mut self) -> &mut gpui::StyleRefinement {
        match self {
            Self::Line(input) => input.style(),
            Self::Area(textarea) => textarea.style(),
        }
    }
}

impl gpui::IntoElement for TextFieldElement {
    type Element = gpui::AnyElement;

    fn into_element(self) -> Self::Element {
        match self {
            Self::Line(input) => input.into_any_element(),
            Self::Area(textarea) => textarea.into_any_element(),
        }
    }
}

/// Read, write and render either kind of text field, for helpers that serve
/// both: `Entity<InputState>`, `Entity<TextareaState>` and [`TextField`].
pub trait TextInput {
    fn text_value(&self, cx: &App) -> SharedString;
    fn set_text_value(&self, value: SharedString, window: &mut Window, cx: &mut App);
    fn text_element(&self, label: SharedString, disabled: bool) -> TextFieldElement;
    fn text_focus_handle(&self, cx: &App) -> FocusHandle;
}

impl TextInput for Entity<InputState> {
    fn text_value(&self, cx: &App) -> SharedString {
        self.read(cx).value()
    }
    fn set_text_value(&self, value: SharedString, window: &mut Window, cx: &mut App) {
        self.update(cx, |state, cx| state.set_value(value, window, cx));
    }
    fn text_element(&self, label: SharedString, disabled: bool) -> TextFieldElement {
        TextFieldElement::Line(Input::new(self).aria_label(label).disabled(disabled))
    }
    fn text_focus_handle(&self, cx: &App) -> FocusHandle {
        self.focus_handle(cx)
    }
}

impl TextInput for Entity<TextareaState> {
    fn text_value(&self, cx: &App) -> SharedString {
        self.read(cx).value()
    }
    fn set_text_value(&self, value: SharedString, window: &mut Window, cx: &mut App) {
        self.update(cx, |state, cx| state.set_value(value, window, cx));
    }
    fn text_element(&self, label: SharedString, disabled: bool) -> TextFieldElement {
        TextFieldElement::Area(Textarea::new(self).aria_label(label).disabled(disabled))
    }
    fn text_focus_handle(&self, cx: &App) -> FocusHandle {
        self.focus_handle(cx)
    }
}

impl TextInput for TextField {
    fn text_value(&self, cx: &App) -> SharedString {
        self.value(cx)
    }
    fn set_text_value(&self, value: SharedString, window: &mut Window, cx: &mut App) {
        self.set_value(value, window, cx);
    }
    fn text_element(&self, label: SharedString, disabled: bool) -> TextFieldElement {
        self.element(label, disabled)
    }
    fn text_focus_handle(&self, cx: &App) -> FocusHandle {
        self.focus_handle(cx)
    }
}
