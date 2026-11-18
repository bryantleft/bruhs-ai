//! The knowledge page's record picker (web `RecordPicker`): connect a record
//! or cite a saved source by searching for it. Search, paging, keys and
//! wording come from `bruhs_models::logic::record_picker`, shared with web.

use bruhs_ui::input::{Input, InputEvent, InputState};
use bruhs_ui::zoom::px;

use super::*;
use bruhs_models::logic::record_picker::{
    match_count_label, navigate, page_label, page_start, RecordIndex, PAGE_SIZE, PICKER_EMPTY,
    PICKER_KEY_HINT, PICKER_SEARCH_PLACEHOLDER,
};
use bruhs_ui::press::on_press_view;

/// Web `sm:max-w-xl`.
const RECORD_PICKER_WIDTH_PX: f32 = 576.0;
/// Web `text-body-sm` rows and trigger, `text-label-sm` meta lines.
const RECORD_PICKER_ROW_TEXT_PX: f32 = 13.0;
const RECORD_PICKER_META_TEXT_PX: f32 = 12.0;
/// Web `max-w-24` on a row's group.
const RECORD_PICKER_GROUP_MAX_WIDTH_PX: f32 = 96.0;
const RECORD_PICKER_RADIUS_PX: f32 = 8.0;
const RECORD_PICKER_HIGHLIGHT_BG_OPACITY: f32 = 0.15;
const RECORD_PICKER_HIGHLIGHT_RING_OPACITY: f32 = 0.30;
const RECORD_PICKER_TRIGGER_BG_OPACITY: f32 = 0.05;
const RECORD_PICKER_TRIGGER_HOVER_BG_OPACITY: f32 = 0.07;
const RECORD_PICKER_TRIGGER_RING_OPACITY: f32 = 0.10;

/// What the picker adds to the open document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RecordPickerTarget {
    Link,
    Citation,
}

impl RecordPickerTarget {
    /// The trigger's and the list's accessible name (web `label`).
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Link => "Connect a record",
            Self::Citation => "Cite a saved source",
        }
    }

    /// The heading beside the close button (web `search_label`).
    fn search_label(self) -> &'static str {
        match self {
            Self::Link => "Search records",
            Self::Citation => "Search sources",
        }
    }

    /// The trigger's text (web `placeholder`).
    fn placeholder(self) -> &'static str {
        match self {
            Self::Link => "Connect task, meeting, person…",
            Self::Citation => "Add source…",
        }
    }

    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Link => "link",
            Self::Citation => "citation",
        }
    }
}

/// The picker's search field and result list scroll, made once.
pub(super) struct RecordPickerUi {
    input: Entity<InputState>,
    scroll: ScrollHandle,
}

impl RecordPickerUi {
    pub(super) fn new(
        window: &mut Window,
        cx: &mut Context<'_, Workspace>,
        subscriptions: &mut Vec<Subscription>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(PICKER_SEARCH_PLACEHOLDER));
        subscriptions.push(cx.subscribe_in(
            &input,
            window,
            |this: &mut Workspace, _input, event: &InputEvent, _window, cx| {
                // A new query starts at its first match, like web.
                if matches!(event, InputEvent::Change) {
                    this.set_record_picker_highlight(0, cx);
                }
            },
        ));
        Self {
            input,
            scroll: ScrollHandle::new(),
        }
    }
}

/// One record the picker can add.
struct RecordChoice {
    value: RecordValue,
    label: String,
    group: &'static str,
}

enum RecordValue {
    Link(KnowledgeLink),
    Citation(String),
}

impl Workspace {
    /// Open the picker for the document on screen, remembering the trigger
    /// so focus returns to it when the picker closes.
    pub(super) fn open_record_picker(
        &mut self,
        target: RecordPickerTarget,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if !self.modal_start_focus_handle.contains_focused(window, cx) {
            self.palette_return_focus = window
                .focused(cx)
                .map(|focus| (self.active_view.clone(), focus));
        }
        self.open_top_layer(TopLayer::RecordPicker {
            target,
            highlighted: 0,
        });
        let input = self.record_picker.input.clone();
        input.update(cx, |input, cx| input.set_value("", window, cx));
        cx.defer_in(window, move |_this, window, cx| {
            input.update(cx, |input, cx| input.focus(window, cx));
        });
        cx.notify();
    }

    fn set_record_picker_highlight(&mut self, highlighted: usize, cx: &mut Context<'_, Self>) {
        let Some(TopLayer::RecordPicker {
            highlighted: current,
            ..
        }) = &mut self.top_layer
        else {
            return;
        };
        *current = highlighted;
        // The list holds one page, so the row sits at its page offset.
        self.record_picker
            .scroll
            .scroll_to_item(highlighted % PAGE_SIZE);
        cx.notify();
    }

    /// Web `link_options` and `citation_options`, in the same order.
    fn record_picker_choices(&self, target: RecordPickerTarget, cx: &App) -> Vec<RecordChoice> {
        let AppView::DocumentDetail { document_id } = &self.active_view else {
            return Vec::new();
        };
        match target {
            RecordPickerTarget::Link => self
                .knowledge_link_candidates(document_id, cx)
                .into_iter()
                .map(|link| RecordChoice {
                    group: link.kind.label(),
                    label: link.label.clone(),
                    value: RecordValue::Link(link),
                })
                .collect(),
            RecordPickerTarget::Citation => self
                .source_clips
                .iter()
                .map(|clip| RecordChoice {
                    value: RecordValue::Citation(clip.id.clone()),
                    label: clip.title.clone(),
                    group: "Sources",
                })
                .collect(),
        }
    }

    fn record_picker_matches(&self, choices: &[RecordChoice], cx: &App) -> Vec<usize> {
        let query = self.record_picker.input.read(cx).value().to_string();
        RecordIndex::new(
            choices
                .iter()
                .map(|choice| (choice.label.as_str(), Some(choice.group))),
        )
        .matching(&query)
    }

    /// Add the match at `position` and close, as a click or Enter does.
    fn choose_record(&mut self, position: usize, cx: &mut Context<'_, Self>) {
        let Some(TopLayer::RecordPicker { target, .. }) = self.top_layer else {
            return;
        };
        let choices = self.record_picker_choices(target, cx);
        let matches = self.record_picker_matches(&choices, cx);
        let Some(choice) = matches
            .get(position)
            .and_then(|index| choices.into_iter().nth(*index))
        else {
            return;
        };
        self.close_transient_surfaces();
        match choice.value {
            RecordValue::Link(link) => self.add_knowledge_link(link, cx),
            RecordValue::Citation(clip_id) => self.add_citation(&clip_id, cx),
        }
        cx.notify();
    }

    /// The trigger in a knowledge panel (web: a full-width field with the
    /// placeholder and a search icon).
    pub(super) fn render_record_picker_trigger(
        &self,
        target: RecordPickerTarget,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        div()
            .id(ElementId::Name(
                format!("desktop-record-picker-trigger-{}", target.key()).into(),
            ))
            .debug_selector(move || format!("desktop-record-picker-trigger-{}", target.key()))
            .w_full()
            .min_h(px(theme::CONTROL_PX))
            .px_3()
            .py_2()
            .rounded(px(RECORD_PICKER_RADIUS_PX))
            .bg(gpui::white().opacity(RECORD_PICKER_TRIGGER_BG_OPACITY))
            .border_1()
            .border_color(gpui::white().opacity(RECORD_PICKER_TRIGGER_RING_OPACITY))
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .text_size(px(RECORD_PICKER_ROW_TEXT_PX))
            .text_color(theme::rambutan_300())
            .role(Role::Button)
            .aria_label(target.label())
            .tab_index(0)
            .cursor_pointer()
            .hover(|el| el.bg(gpui::white().opacity(RECORD_PICKER_TRIGGER_HOVER_BG_OPACITY)))
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            .map(|el| {
                on_press_view(el, cx, move |this, window, cx| {
                    this.open_record_picker(target, window, cx);
                })
            })
            .child(div().min_w_0().truncate().child(target.placeholder()))
            .child(
                svg()
                    .path(assets::TOOL_SEARCH_ICON_PATH)
                    .size(px(theme::ICON_PX))
                    .flex_shrink_0()
                    .text_color(theme::rambutan_500()),
            )
            .into_any_element()
    }

    pub(super) fn render_record_picker(
        &self,
        target: RecordPickerTarget,
        highlighted: usize,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let choices = self.record_picker_choices(target, cx);
        let matches = self.record_picker_matches(&choices, cx);
        let count = matches.len();
        let current = highlighted.min(count.saturating_sub(1));
        let start = page_start(count, current);
        let rows: Vec<AnyElement> = matches
            .iter()
            .enumerate()
            .skip(start)
            .take(PAGE_SIZE)
            .filter_map(|(position, index)| {
                let choice = choices.get(*index)?;
                Some(self.render_record_row(position, position == current, choice, cx))
            })
            .collect();
        let page_count = count.div_ceil(PAGE_SIZE);
        let page = current / PAGE_SIZE;
        div()
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .left_0()
            .occlude()
            .id("desktop-record-picker-overlay")
            .bg(theme::longan_950().opacity(DESKTOP_MODAL_OVERLAY_OPACITY))
            .flex()
            .items_start()
            .justify_center()
            .pt(px(DESKTOP_DIALOG_TOP_PADDING_PX))
            .on_click(cx.listener(|this, _ev, _window, cx| {
                this.close_transient_surfaces();
                cx.notify();
            }))
            .child(
                div()
                    .id("desktop-record-picker")
                    .debug_selector(|| "desktop-record-picker".to_string())
                    .role(Role::Dialog)
                    .aria_label(target.label())
                    .child(self.blur_scrim.backdrop())
                    .w(px(RECORD_PICKER_WIDTH_PX))
                    .max_w(relative(DESKTOP_DIALOG_MAX_WIDTH_RATIO))
                    .max_h(modal_ui::picker_max_height(window))
                    .rounded(px(DESKTOP_DIALOG_RADIUS_PX))
                    .border_1()
                    .border_color(theme::ring_modal())
                    .bg(theme::longan_800())
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .track_focus(&self.modal_start_focus_handle)
                    .tab_group()
                    .capture_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        let key = event.keystroke.key.as_str();
                        let step = navigate(current, count, key);
                        if event.keystroke.modifiers != Modifiers::default()
                            || !(matches!(key, "escape" | "enter") || step.is_some())
                        {
                            return;
                        }
                        let composing = this.record_picker.input.update(cx, |input, cx| {
                            input.marked_text_range(window, cx).is_some()
                        });
                        if composing {
                            return;
                        }
                        window.prevent_default();
                        cx.stop_propagation();
                        match (key, step) {
                            ("enter", _) => this.choose_record(current, cx),
                            ("escape", _) => {
                                this.close_transient_surfaces();
                                cx.notify();
                            }
                            (_, Some(next)) => this.set_record_picker_highlight(next, cx),
                            _ => {}
                        }
                    }))
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        this.trap_modal_tab_focus(event, window, cx);
                    }))
                    .on_click(cx.listener(|_this, _ev, _window, cx| {
                        cx.stop_propagation();
                    }))
                    .child(self.render_record_picker_header(target, cx))
                    .child(
                        div()
                            .flex_shrink_0()
                            .px_3()
                            .py_2()
                            .text_size(px(RECORD_PICKER_META_TEXT_PX))
                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                            .text_color(theme::rambutan_400())
                            .debug_selector(|| "desktop-record-picker-count".to_string())
                            .child(match_count_label(count)),
                    )
                    .child(
                        div()
                            .id("desktop-record-picker-list")
                            .debug_selector(|| "desktop-record-picker-list".to_string())
                            .role(Role::ListBox)
                            .aria_label(target.label())
                            .min_h_0()
                            .flex_1()
                            .overflow_y_scroll()
                            .track_scroll(&self.record_picker.scroll)
                            .px_1()
                            .pb_1()
                            .flex()
                            .flex_col()
                            .children(rows)
                            .when(count == 0, |el| {
                                el.child(
                                    div()
                                        .px_3()
                                        .py_5()
                                        .text_size(px(RECORD_PICKER_ROW_TEXT_PX))
                                        .text_color(theme::rambutan_400())
                                        .child(PICKER_EMPTY),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .border_t_1()
                            .border_color(theme::longan_700())
                            .px_3()
                            .py_2()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .text_size(px(RECORD_PICKER_META_TEXT_PX))
                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                            .text_color(theme::rambutan_400())
                            .child(PICKER_KEY_HINT)
                            .when(count > PAGE_SIZE, |el| {
                                el.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(self.render_record_page_button(
                                            "desktop-record-picker-previous",
                                            "Previous",
                                            "Previous results",
                                            page > 0,
                                            page.saturating_sub(1) * PAGE_SIZE,
                                            cx,
                                        ))
                                        .child(page_label(count, current))
                                        .child(self.render_record_page_button(
                                            "desktop-record-picker-next",
                                            "Next",
                                            "Next results",
                                            page + 1 < page_count,
                                            (page + 1) * PAGE_SIZE,
                                            cx,
                                        )),
                                )
                            }),
                    ),
            )
            .into_any_element()
    }

    fn render_record_picker_header(
        &self,
        target: RecordPickerTarget,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        div()
            .flex_shrink_0()
            .border_b_1()
            .border_color(theme::longan_700())
            .px_3()
            .pb_3()
            .child(
                div()
                    .min_h(px(theme::CONTROL_PX))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .text_size(px(theme::APP_UI_TEXT_SIZE_PX))
                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                            .text_color(theme::rambutan_200())
                            .child(target.search_label()),
                    )
                    .child(
                        div()
                            .id("desktop-record-picker-close")
                            .debug_selector(|| "desktop-record-picker-close".to_string())
                            .size(px(theme::CONTROL_PX))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(RECORD_PICKER_RADIUS_PX))
                            .border_1()
                            .border_color(gpui::transparent_black())
                            .text_color(theme::rambutan_400())
                            .role(Role::Button)
                            .aria_label("Close record picker")
                            .tab_index(0)
                            .cursor_pointer()
                            .hover(|el| el.bg(theme::longan_700()))
                            .focus_visible(|el| el.border_color(theme::persimmon_400()))
                            .map(|el| {
                                on_press_view(el, cx, |this, _window, cx| {
                                    this.close_transient_surfaces();
                                    cx.notify();
                                })
                            })
                            .child(
                                svg()
                                    .path(assets::CLOSE_ICON_PATH)
                                    .size(px(theme::ICON_PX))
                                    .text_color(theme::rambutan_400()),
                            ),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .rounded(px(RECORD_PICKER_RADIUS_PX))
                    .bg(gpui::white().opacity(RECORD_PICKER_TRIGGER_BG_OPACITY))
                    .border_1()
                    .border_color(gpui::white().opacity(RECORD_PICKER_TRIGGER_RING_OPACITY))
                    // Web `px-3` inside an inset ring; here the 1px border takes one.
                    .px(px(11.0))
                    .py_1()
                    .child(
                        Input::new(&self.record_picker.input)
                            .aria_label(target.search_label())
                            .appearance(false)
                            .px_0()
                            .bordered(false)
                            .focus_bordered(false)
                            .cleanable(false)
                            .text_size(px(theme::APP_INPUT_TEXT_SIZE_PX)),
                    ),
            )
            .into_any_element()
    }

    fn render_record_row(
        &self,
        position: usize,
        selected: bool,
        choice: &RecordChoice,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let label = choice.label.clone();
        div()
            .id(("desktop-record-picker-option", position))
            .debug_selector(move || format!("desktop-record-picker-option-{position}"))
            .role(Role::ListBoxOption)
            .aria_label(label.clone())
            .aria_selected(selected)
            .flex_shrink_0()
            .w_full()
            .min_h(px(theme::CONTROL_PX))
            .px_3()
            .py_2()
            .rounded(px(RECORD_PICKER_RADIUS_PX))
            .border_1()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .text_size(px(RECORD_PICKER_ROW_TEXT_PX))
            .cursor_pointer()
            .map(|el| {
                if selected {
                    el.bg(theme::blueberry_500().opacity(RECORD_PICKER_HIGHLIGHT_BG_OPACITY))
                        .border_color(
                            theme::blueberry_400().opacity(RECORD_PICKER_HIGHLIGHT_RING_OPACITY),
                        )
                        .text_color(theme::blueberry_200())
                } else {
                    el.border_color(gpui::transparent_black())
                        .text_color(theme::rambutan_200())
                        .hover(|el| el.bg(theme::longan_700()))
                }
            })
            .on_click(cx.listener(move |this, _ev, _window, cx| {
                cx.stop_propagation();
                this.choose_record(position, cx);
            }))
            .child(div().min_w_0().truncate().child(label))
            .child(
                div()
                    .max_w(px(RECORD_PICKER_GROUP_MAX_WIDTH_PX))
                    .flex_shrink_0()
                    .truncate()
                    .text_size(px(RECORD_PICKER_META_TEXT_PX))
                    .text_color(theme::rambutan_400())
                    .child(choice.group),
            )
            .into_any_element()
    }

    fn render_record_page_button(
        &self,
        id: &'static str,
        text: &'static str,
        label: &'static str,
        enabled: bool,
        highlight: usize,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        web_button(
            id,
            WebButtonVariant::Ghost,
            WebButtonSize::Small,
            ButtonState::disabled_if(!enabled),
        )
        .debug_selector(move || id.to_string())
        .min_h(px(theme::CONTROL_PX))
        .aria_label(label)
        .when(enabled, |el| {
            el.on_click(cx.listener(move |this, _ev, _window, cx| {
                cx.stop_propagation();
                this.set_record_picker_highlight(highlight, cx);
            }))
        })
        .child(text)
        .into_any_element()
    }
}
