//! The status and error lines of Export file, Move in and By prompt, as a
//! screen reader hears them: each is a live node with a stable id, a role
//! and a live setting, present while empty so the first line put in it is
//! announced, as on the web (`role="status"` and `role="alert"` with
//! `empty:sr-only`). The chat's reader (`chat/screen_reader.rs`) sets the
//! same live property; here it is written with the node's own properties,
//! so a test can read it back.

use gpui::accesskit::{Live, Node};
use gpui::{
    div, App, Bounds, Div, Element, ElementId, GlobalElementId, InspectorElementId,
    InteractiveElement as _, IntoElement, LayoutId, ParentElement as _, Pixels, Role, Stateful,
    StatefulInteractiveElement as _, Styled as _, Window,
};

/// The live lines' ids: Export file's status and error, an agent tab's
/// error, By prompt's status and error.
pub(in crate::views::workspace) const EXPORT_STATUS_LIVE: &str =
    "desktop-import-export-status-live";
pub(in crate::views::workspace) const EXPORT_ERROR_LIVE: &str = "desktop-import-export-error-live";
pub(in crate::views::workspace) const MOVE_IN_STATUS_LIVE: &str = "desktop-move-in-status-live";
pub(in crate::views::workspace) const MOVE_IN_ERROR_LIVE: &str = "desktop-move-in-error-live";

/// Which kind of line: progress and outcomes, or refusals and failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::views::workspace) enum LiveKind {
    /// Read when the reader is free: web `role="status"`.
    Status,
    /// Read at once: web `role="alert"`.
    Alert,
}

impl LiveKind {
    fn role(self) -> Role {
        match self {
            Self::Status => Role::Status,
            Self::Alert => Role::Alert,
        }
    }

    fn live(self) -> Live {
        match self {
            Self::Status => Live::Polite,
            Self::Alert => Live::Assertive,
        }
    }
}

/// A live line: the drawn line inside a node a screen reader follows.
pub(in crate::views::workspace) struct LiveLine {
    element: Stateful<Div>,
    live: Live,
    drawn: bool,
}

impl LiveLine {
    /// Whether the line has text to draw; an empty one takes no room.
    pub(in crate::views::workspace) fn drawn(&self) -> bool {
        self.drawn
    }
}

/// The live line `id`, drawing `text` with `draw` while there is any. With
/// no text the node stays, out of the flow, so the dialog's layout is the
/// same as without it and the next line is still announced.
pub(in crate::views::workspace) fn live_line(
    id: &'static str,
    kind: LiveKind,
    text: Option<String>,
    draw: impl FnOnce(String) -> Div,
) -> LiveLine {
    let element = div()
        .id(id)
        .debug_selector(move || id.to_string())
        .role(kind.role())
        .min_w_0();
    let text = text.filter(|text| !text.is_empty());
    let drawn = text.is_some();
    let element = match text {
        Some(text) => element.aria_label(text.clone()).child(draw(text)),
        None => element.absolute(),
    };
    LiveLine {
        element,
        live: kind.live(),
        drawn,
    }
}

impl IntoElement for LiveLine {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for LiveLine {
    type RequestLayoutState = <Stateful<Div> as Element>::RequestLayoutState;
    type PrepaintState = <Stateful<Div> as Element>::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        Element::id(&self.element)
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        self.element.source_location()
    }

    fn a11y_role(&self) -> Option<Role> {
        self.element.a11y_role()
    }

    fn write_a11y_info(&self, node: &mut Node) {
        self.element.write_a11y_info(node);
        node.set_live(self.live);
    }

    fn a11y_synthetic_children(
        &mut self,
        prepaint: &mut Self::PrepaintState,
        builder: &mut gpui::A11ySubtreeBuilder<'_>,
    ) {
        Element::a11y_synthetic_children(&mut self.element, prepaint, builder);
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.element.request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.element
            .prepaint(id, inspector_id, bounds, request_layout, window, cx)
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.element.paint(
            id,
            inspector_id,
            bounds,
            request_layout,
            prepaint,
            window,
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The node a screen reader gets for `line`.
    fn node(line: &LiveLine) -> Node {
        let role = line.a11y_role().expect("a live line is in the tree");
        let mut node = Node::new(role);
        line.write_a11y_info(&mut node);
        node
    }

    #[test]
    fn status_lines_are_polite_and_errors_assertive() {
        let status = live_line(
            "test-status",
            LiveKind::Status,
            Some("Imported 8 chats.".into()),
            |text| div().child(text),
        );
        assert_eq!(status.id(), Some(ElementId::from("test-status")));
        let read = node(&status);
        assert_eq!(read.role(), Role::Status);
        assert_eq!(read.live(), Some(Live::Polite));
        assert_eq!(read.label(), Some("Imported 8 chats."));
        assert!(status.drawn());

        let error = live_line(
            "test-error",
            LiveKind::Alert,
            Some("That isn't an export.".into()),
            |text| div().child(text),
        );
        let read = node(&error);
        assert_eq!(read.role(), Role::Alert);
        assert_eq!(read.live(), Some(Live::Assertive));
        assert_eq!(read.label(), Some("That isn't an export."));
    }

    #[test]
    fn an_empty_line_stays_in_the_tree_with_its_live_setting() {
        for (kind, role, live) in [
            (LiveKind::Status, Role::Status, Live::Polite),
            (LiveKind::Alert, Role::Alert, Live::Assertive),
        ] {
            for text in [None, Some(String::new())] {
                let mut drew = false;
                let line = live_line("test-empty", kind, text, |text| {
                    drew = true;
                    div().child(text)
                });
                assert!(!drew, "nothing is drawn for an empty line");
                assert!(!line.drawn());
                let read = node(&line);
                assert_eq!(read.role(), role);
                assert_eq!(read.live(), Some(live));
                assert_eq!(read.label(), None);
            }
        }
    }
}
