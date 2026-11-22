//! Place a native language picker on the header line of each fenced block.
//! Position it after text layout so it follows soft wrapping, zoom and scrolling.
use super::{
    code_fence::{language, Fence, LANGUAGES},
    TextareaState,
};
use crate::dropdown::{saved_option_value, Dropdown, DropdownGroup, DropdownOption, Listbox};
use gpui::*;
use std::rc::Rc;

// Dropdown keys are static. Keep arbitrary names in the label and document only.
const CUSTOM: &str = " ";

pub(super) fn header(
    field: &Entity<TextareaState>,
    fence: &Fence,
    index: usize,
    interactive: bool,
    cx: &App,
) -> impl IntoElement {
    let start = fence.header.start;
    let state = field.read(cx);
    let tag = &state.core.text[fence.tag.clone()];
    let current = language(tag);
    let value = current.map_or(CUSTOM, |l| l.id);
    let label = current.map_or(tag, |l| l.label);
    let content = if interactive {
        let query = state.language_query(start);
        let id = saved_option_value(&format!("composer-code-language-{index}"));
        let choices = if query.is_some() {
            let custom = current.is_none().then(|| DropdownOption {
                value: CUSTOM,
                label: tag.to_owned(),
                description: None,
            });
            custom
                .into_iter()
                .chain(LANGUAGES.iter().map(|l| DropdownOption {
                    value: l.id,
                    label: l.label.to_owned(),
                    description: None,
                }))
                .collect()
        } else {
            Vec::new()
        };
        let toggle = field.downgrade();
        let select = field.downgrade();
        let mut menu = Dropdown::new(
            id,
            &format!("Code language: {label}"),
            value,
            vec![DropdownGroup {
                label: None,
                mark: None,
                options: choices,
            }],
            if query.is_some() {
                Listbox::Popover
            } else {
                Listbox::Closed
            },
            Rc::new(move |window, cx| {
                let _ = toggle.update(cx, |field, cx| {
                    field.toggle_language_menu(start, window, cx)
                });
            }),
            Rc::new(move |id, window, cx| {
                let _ = select.update(cx, |field, cx| {
                    field.choose_language(start, (id != CUSTOM).then_some(id), window, cx)
                });
            }),
        )
        .compact()
        .start_at_selection()
        .selection_label(label.to_owned(), None)
        .option_mark(crate::language_icons::element)
        .empty_label("No matching languages.");
        if let Some(query) = query {
            menu = menu.search_field(query, "Search languages");
        }
        div()
            .id(("fence-picker", index))
            .occlude()
            .child(menu)
            .into_any_element()
    } else {
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(crate::language_icons::element(value))
            .text_color(crate::theme::rambutan_400())
            .child(label.to_owned())
            .into_any_element()
    };
    FenceHeader {
        field: field.clone(),
        start,
        content,
    }
}

struct FenceHeader {
    field: Entity<TextareaState>,
    start: usize,
    content: AnyElement,
}
impl IntoElement for FenceHeader {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for FenceHeader {
    type RequestLayoutState = ();
    type PrepaintState = bool;
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let child = self.content.request_layout(window, cx);
        let style = Style {
            position: Position::Absolute,
            ..Style::default()
        };
        (window.request_layout(style, [child], cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let Some(drawn) = &self.field.read(cx).drawn else {
            return false;
        };
        let Some(origin) = drawn
            .layout
            .position_for_index(drawn.map.to_display(self.start))
        else {
            return false;
        };
        let origin = origin + point(crate::zoom::px(6.), crate::zoom::px(2.));
        if !window
            .content_mask()
            .bounds
            .intersects(&Bounds::new(origin, bounds.size))
        {
            return false;
        }
        window.with_element_offset(origin - bounds.origin, |window| {
            self.content.prepaint(window, cx)
        });
        true
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        visible: &mut bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        if *visible {
            self.content.paint(window, cx);
        }
    }
}
