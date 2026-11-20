//! Bounded library pages share keyboard controls and retain their full records.
use super::*;
use bruhs_ui::press::on_press_view;
use bruhs_ui::zoom::px;

#[derive(Clone, Copy)]
pub(super) enum LibraryPage {
    Tasks,
    TaskColumn(usize),
    Documents,
    Sources,
    MeetingTasks,
}

impl LibraryPage {
    fn key(self) -> String {
        match self {
            Self::Tasks => "task-list".into(),
            Self::TaskColumn(column) => format!("task-column-{column}"),
            Self::Documents => "knowledge-documents".into(),
            Self::Sources => "knowledge-sources".into(),
            Self::MeetingTasks => "meeting-tasks".into(),
        }
    }
}

#[derive(Default)]
pub(super) struct LibraryPages {
    pub tasks: usize,
    pub columns: [usize; 4],
    pub documents: usize,
    pub sources: usize,
    pub meeting_tasks: usize,
    pub task_scroll: ScrollHandle,
    pub column_scroll: [ScrollHandle; 4],
    pub knowledge_scroll: ScrollHandle,
}

impl LibraryPages {
    pub fn start(&self, target: LibraryPage, total: usize, size: usize) -> usize {
        let page = match target {
            LibraryPage::Tasks => self.tasks,
            LibraryPage::TaskColumn(column) => self.columns[column],
            LibraryPage::Documents => self.documents,
            LibraryPage::Sources => self.sources,
            LibraryPage::MeetingTasks => self.meeting_tasks,
        };
        page.min(total.saturating_sub(1) / size) * size
    }

    pub fn select(&mut self, target: LibraryPage, page: usize) {
        match target {
            LibraryPage::Tasks => {
                self.tasks = page;
                self.task_scroll.set_offset(point(px(0.0), px(0.0)));
            }
            LibraryPage::TaskColumn(column) => {
                self.columns[column] = page;
                self.column_scroll[column].set_offset(point(px(0.0), px(0.0)));
            }
            LibraryPage::Documents => {
                self.documents = page;
                // The top, so the search box above the documents shows too.
                self.knowledge_scroll.scroll_to_top_of_item(0);
            }
            LibraryPage::Sources => {
                self.sources = page;
                // After the search box and the documents.
                self.knowledge_scroll.scroll_to_top_of_item(2);
            }
            LibraryPage::MeetingTasks => self.meeting_tasks = page,
        }
    }
}

impl Workspace {
    fn select_library_page(
        &mut self,
        target: LibraryPage,
        page: usize,
        cx: &mut Context<'_, Self>,
    ) {
        self.library_pages.select(target, page);
        if matches!(target, LibraryPage::MeetingTasks) {
            if let Some(view) = self.meeting_transcript_view.borrow().as_ref() {
                view.invalidate_footer();
            }
        }
        cx.notify();
    }

    pub(super) fn render_library_pages(
        &self,
        target: LibraryPage,
        total: usize,
        size: usize,
        label: &str,
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        if total <= size {
            return None;
        }
        let start = self.library_pages.start(target, total, size);
        let key = target.key();
        let debug_key = key.clone();
        Some(
            div()
                .id(SharedString::from(format!("{key}-pages")))
                .debug_selector(move || format!("desktop-{debug_key}-pages"))
                .role(Role::Navigation)
                .aria_label(format!("{label} pages"))
                .flex_shrink_0()
                .border_t_1()
                .border_color(theme::longan_800())
                .px_2()
                .py_1()
                .child(
                    div()
                        .id("page-status")
                        .role(Role::Status)
                        .text_center()
                        .font_features(theme::tabular_nums())
                        .text_size(px(11.0))
                        .text_color(theme::rambutan_400())
                        .child(format!(
                            "{} to {} of {total}",
                            start + 1,
                            (start + size).min(total)
                        )),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(self.library_page_button(
                            target,
                            start / size,
                            false,
                            start == 0,
                            label,
                            cx,
                        ))
                        .child(self.library_page_button(
                            target,
                            start / size,
                            true,
                            start + size >= total,
                            label,
                            cx,
                        )),
                )
                .into_any_element(),
        )
    }

    fn library_page_button(
        &self,
        target: LibraryPage,
        page: usize,
        next: bool,
        disabled: bool,
        label: &str,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let direction = if next { "next" } else { "previous" };
        let id = format!("desktop-{}-{direction}", target.key());
        let debug_id = id.clone();
        let next_page = if next {
            page + 1
        } else {
            page.saturating_sub(1)
        };
        web_button(
            SharedString::from(id),
            WebButtonVariant::Ghost,
            WebButtonSize::Small,
            ButtonState::disabled_if(disabled),
        )
        .debug_selector(move || debug_id)
        .aria_label(format!("{direction} {label} page"))
        .min_h(px(theme::CONTROL_PX))
        .when(!disabled, |button| {
            on_press_view(button, cx, move |this, _, cx| {
                this.select_library_page(target, next_page, cx);
            })
        })
        .child(if next { "Next" } else { "Previous" })
        .into_any_element()
    }
}
