//! How the Export file tab draws: the summary and the rows scroll, and
//! the buttons and both lines under them stay in view (I1), so focus and
//! feedback are never below the fold.

use super::*;
use crate::views::web_button::{web_button, ButtonState, WebButtonSize, WebButtonVariant};
use crate::views::workspace::import_ui::live_line::{
    live_line, LiveKind, EXPORT_ERROR_LIVE, EXPORT_STATUS_LIVE,
};
use bruhs_models::imports::{
    export_memory_from, memory_found_line, MoveInStart, MoveInTab, USE_AS_MEMORY,
};
use bruhs_ui::press::on_press_view;
use bruhs_ui::zoom::px;

/// Web `max-h-64`.
const EXPORT_LIST_MAX_HEIGHT_PX: f32 = 256.0;
/// Web `px-4`: the list reaches the dialog's edges, as the web's `-mx-4`.
const EXPORT_EDGE_PX: f32 = 16.0;
/// Web `bg-blueberry-600/20 inset-ring-blueberry-500/30`: the selection.
const EXPORT_SELECTED_BG_OPACITY: f32 = 0.20;
const EXPORT_SELECTED_RING_OPACITY: f32 = 0.30;
/// Web `focus:inset-ring-persimmon-500/60` on the search field.
const EXPORT_SEARCH_FOCUS_OPACITY: f32 = 0.60;
/// Web `aria-disabled:opacity-50`: a button that waits keeps its focus.
const EXPORT_WAITING_OPACITY: f32 = 0.50;

/// One line of the tab's text, findable by its words in tests.
fn export_text(kind: &'static str, text: String, color: Hsla) -> Div {
    let selector = format!("desktop-import-export-{kind}:{text}");
    div()
        .debug_selector(move || selector)
        .flex_shrink_0()
        .font_family(theme::UI_SMALL_FONT_FAMILY)
        .font_weight(FontWeight::MEDIUM)
        .text_size(px(IMPORT_LABEL_SM_TEXT_SIZE_PX))
        .text_color(color)
        .child(text)
}

impl Workspace {
    /// A tab button: it keeps its Tab stop while it waits, so focus never
    /// drops out of the dialog, and does nothing until the wait is over.
    fn export_button(
        label: &'static str,
        focus: &FocusHandle,
        waiting: bool,
        cx: &Context<'_, Self>,
        handler: impl Fn(&mut Self, &mut Window, &mut Context<'_, Self>) + 'static,
    ) -> Stateful<Div> {
        let el = web_button(
            SharedString::from(format!("desktop-import-export-{label}")),
            WebButtonVariant::Outline,
            WebButtonSize::Small,
            ButtonState::Enabled,
        )
        .debug_selector(move || format!("desktop-import-export-button:{label}"))
        .track_focus(focus)
        .flex_shrink_0()
        .min_h(px(theme::CONTROL_PX))
        .px_3()
        .border_color(theme::longan_700())
        .text_color(theme::rambutan_200())
        .aria_label(label)
        .when(waiting, |el| {
            el.opacity(EXPORT_WAITING_OPACITY).cursor_not_allowed()
        })
        .child(label);
        on_press_view(el, cx, handler)
    }

    /// The tab's body under the dialog's header. `rows` are
    /// [`Self::visible_export_rows`]; `left` is whether any of the batch's
    /// chats isn't here yet.
    pub(in crate::views::workspace) fn render_export_tab(
        &self,
        rows: &[ImportCandidate],
        left: bool,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let export = &self.import_picker.export;
        let Some(ui) = &export.ui else {
            return div().into_any_element();
        };
        let busy = self.export_busy();
        // The help or the summary and the rows scroll in a short window.
        let scroll = div()
            .id("desktop-import-export")
            .debug_selector(|| "desktop-import-export".to_string())
            .min_h_0()
            .flex_shrink_1()
            .overflow_y_scroll()
            .track_scroll(&export.body_scroll)
            .flex()
            .flex_col()
            .gap_2()
            .px_4()
            .pt_3();
        // The buttons and both lines stay in view under them (I1).
        let actions = div()
            .debug_selector(|| "desktop-import-export-actions".to_string())
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_2()
            .px_4()
            .pt_2()
            .pb_3();
        let (scroll, actions) = match export.batch() {
            None => (
                scroll
                    .children(
                        [EXPORT_HELP_CHATGPT, EXPORT_HELP_CLAUDE, EXPORT_HELP_GROK]
                            .into_iter()
                            .map(|line| {
                                export_text("help", line.to_owned(), theme::rambutan_300())
                            }),
                    )
                    .child(export_text(
                        "warn",
                        GROK_BEST_EFFORT.to_owned(),
                        theme::durian_300(),
                    )),
                actions.child(div().flex().child(Self::export_button(
                    CHOOSE_EXPORT,
                    &ui.choose,
                    export.reading() || busy,
                    cx,
                    |this, window, cx| this.choose_export_files(window, cx),
                ))),
            ),
            Some(batch) => {
                let scroll =
                    scroll.children(export_lines(batch).into_iter().map(|(line, warn)| {
                        if warn {
                            export_text("warn", line, theme::durian_300())
                        } else {
                            export_text("line", line, theme::rambutan_200())
                        }
                    }));
                let scroll = scroll.children(Self::render_export_memory(batch, cx));
                let scroll = if batch.candidates.is_empty() {
                    scroll
                } else if left {
                    scroll
                        .child(self.render_export_search(ui, window, cx))
                        .child(self.render_export_list(ui, rows, cx))
                } else {
                    scroll.child(export_text(
                        "line",
                        ALREADY_HERE.to_owned(),
                        theme::rambutan_200(),
                    ))
                };
                let buttons = div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(!rows.is_empty(), |el| {
                        el.child(Self::export_button(
                            IMPORT_ALL,
                            &ui.all,
                            busy,
                            cx,
                            |this, window, cx| this.import_all_exports(window, cx),
                        ))
                    })
                    .child(Self::export_button(
                        CHOOSE_ANOTHER,
                        &ui.another,
                        busy,
                        cx,
                        |this, window, cx| this.choose_export_files(window, cx),
                    ));
                (scroll, actions.child(buttons))
            }
        };
        // Live lines, there while empty, so a screen reader hears each
        // line put in them (web `role="status"` and `role="alert"`).
        let actions = actions
            .child(live_line(
                EXPORT_STATUS_LIVE,
                LiveKind::Status,
                export.status.clone(),
                |line| export_text("status", line, theme::rambutan_300()),
            ))
            .child(live_line(
                EXPORT_ERROR_LIVE,
                LiveKind::Alert,
                export.error.clone(),
                |line| export_text("error", line, theme::lychee_300()),
            ));
        div()
            .min_h_0()
            .flex_shrink_1()
            .flex()
            .flex_col()
            .child(scroll.vertical_scrollbar(&export.body_scroll))
            .child(actions)
            .into_any_element()
    }

    /// The export's memory of the owner, each offered as a bruh's memory:
    /// it opens Move in at By prompt with the text and where it came from
    /// (D14). The batch goes when this dialog closes; the text travels.
    fn render_export_memory(batch: &ImportBatch, cx: &Context<'_, Self>) -> Vec<Div> {
        batch
            .memory
            .iter()
            .filter(|suggestion| !suggestion.text.trim().is_empty())
            .enumerate()
            .map(|(index, suggestion)| {
                let source = suggestion.source;
                let text = suggestion.text.clone();
                let button = web_button(
                    ("desktop-import-export-use-memory", index),
                    WebButtonVariant::Outline,
                    WebButtonSize::Small,
                    ButtonState::Enabled,
                )
                .debug_selector(move || format!("desktop-import-export-button:{USE_AS_MEMORY}"))
                .flex_shrink_0()
                .min_h(px(theme::CONTROL_PX))
                .px_3()
                .border_color(theme::longan_700())
                .text_color(theme::rambutan_200())
                .focus_visible(|el| el.border_color(theme::persimmon_400()))
                .aria_label(USE_AS_MEMORY)
                .child(USE_AS_MEMORY);
                let button = on_press_view(button, cx, move |this, window, cx| {
                    let start = MoveInStart {
                        tab: MoveInTab::ByPrompt,
                        text: Some(text.clone()),
                        from: Some(export_memory_from(source)),
                    };
                    this.open_move_in(start, window, cx);
                });
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        export_text("memory", memory_found_line(source), theme::rambutan_200())
                            .flex_1()
                            .min_w_0(),
                    )
                    .child(button)
            })
            .collect()
    }

    fn render_export_search(
        &self,
        ui: &ExportUi,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let focused = ui.search.focus_handle(cx).is_focused(window);
        div()
            .debug_selector(|| "desktop-import-export-search".to_string())
            .flex_shrink_0()
            .w_full()
            .rounded(px(IMPORT_RADIUS_PX))
            .bg(theme::longan_900())
            .border_1()
            .border_color(if focused {
                theme::persimmon_500().opacity(EXPORT_SEARCH_FOCUS_OPACITY)
            } else {
                theme::longan_700()
            })
            // Web `px-2.5`, less the 1px border; a small input in 2px
            // padding is the web's `py-1.5` field.
            .px(px(9.0))
            .py(px(2.0))
            .child(
                Input::new(&ui.search)
                    .small()
                    .aria_label(EXPORT_SEARCH)
                    .appearance(false)
                    .px_0()
                    .bordered(false)
                    .focus_bordered(false)
                    .cleanable(false)
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_size(px(IMPORT_LABEL_SM_TEXT_SIZE_PX)),
            )
            .into_any_element()
    }

    fn render_export_list(
        &self,
        ui: &ExportUi,
        rows: &[ImportCandidate],
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let export = &self.import_picker.export;
        if rows.is_empty() {
            return export_text("none", NO_MATCHING_CHATS.to_owned(), theme::rambutan_300())
                .py_2()
                .into_any_element();
        }
        let now = EpochMs::now();
        let shown = rows.len().min(MAX_ROWS_SHOWN);
        let selected = export.selected.min(shown - 1);
        div()
            .id("desktop-import-export-list")
            .debug_selector(|| "desktop-import-export-list".to_string())
            .track_focus(&ui.list)
            .role(Role::ListBox)
            .aria_label(EXPORT_TAB)
            .mx(px(-EXPORT_EDGE_PX))
            .flex_shrink_0()
            .max_h(px(EXPORT_LIST_MAX_HEIGHT_PX))
            .border_1()
            .border_color(gpui::transparent_black())
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            .overflow_y_scroll()
            .track_scroll(&export.scroll)
            .children(rows.iter().take(shown).enumerate().map(|(index, row)| {
                let chosen = index == selected;
                let key = import_key(row.source, &row.id);
                let detail = export_row_detail(row, now);
                let title = bruhs_models::logic::text::plain_title(&row.title).into_owned();
                let picked = row.clone();
                div()
                    .id(("desktop-import-export-row", index))
                    .debug_selector(move || format!("desktop-import-export-row-{key}"))
                    .role(Role::ListBoxOption)
                    .aria_label(title.clone())
                    .aria_selected(chosen)
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap_0p5()
                    .px(px(EXPORT_EDGE_PX - 1.0))
                    .py(px(7.0))
                    .border_1()
                    .cursor_pointer()
                    .map(|el| {
                        // Blueberry marks the selection (design rule).
                        if chosen {
                            el.bg(theme::blueberry_600().opacity(EXPORT_SELECTED_BG_OPACITY))
                                .border_color(
                                    theme::blueberry_500().opacity(EXPORT_SELECTED_RING_OPACITY),
                                )
                        } else {
                            el.border_color(gpui::transparent_black()).hover(|el| {
                                el.bg(theme::longan_700().opacity(IMPORT_HOVER_ROW_BG_OPACITY))
                            })
                        }
                    })
                    .on_hover(cx.listener(move |this, hovered, _window, cx| {
                        if *hovered && this.import_picker.export.selected != index {
                            this.import_picker.export.selected = index;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _event, window, cx| {
                        this.import_export_row(picked.clone(), window, cx);
                    }))
                    .child(
                        export_text("title", title, theme::rambutan_200())
                            .w_full()
                            .min_w_0()
                            .truncate(),
                    )
                    .child(
                        export_text("detail", detail, theme::rambutan_400())
                            .w_full()
                            .min_w_0()
                            .truncate(),
                    )
            }))
            .when(rows.len() > MAX_ROWS_SHOWN, |el| {
                el.child(
                    export_text(
                        "more",
                        rows_shown_line(MAX_ROWS_SHOWN, rows.len()),
                        theme::rambutan_400(),
                    )
                    .px_4()
                    .py_2(),
                )
            })
            .vertical_scrollbar(&export.scroll)
            .into_any_element()
    }
}
