use super::*;
use bruhs_ui::dropdown::{dropdown, DropdownGroup, DropdownOption, Listbox};

// Each row keeps an independent keyboard focus target in the shared settings dropdown.
const IDS: [&str; MAX_WEBHOOK_FILTERS] = [
    "webhook-test-1",
    "webhook-test-2",
    "webhook-test-3",
    "webhook-test-4",
    "webhook-test-5",
    "webhook-test-6",
    "webhook-test-7",
    "webhook-test-8",
    "webhook-test-9",
    "webhook-test-10",
    "webhook-test-11",
    "webhook-test-12",
    "webhook-test-13",
    "webhook-test-14",
    "webhook-test-15",
    "webhook-test-16",
];
const TESTS: [(&str, &str); 5] = [
    ("equals", "Equals"),
    ("notEquals", "Does not equal"),
    ("in", "Is one of"),
    ("startsWith", "Starts with"),
    ("exists", "Exists"),
];

pub(super) struct FilterRow {
    path: TextField,
    test: &'static str,
    operand: TextField,
}
impl FilterRow {
    pub(super) fn new(filter: Option<&WebhookFilter>, window: &mut Window, cx: &mut App) -> Self {
        let (test, operand) = match filter.map(|f| &f.test) {
            Some(WebhookTest::Equals(v)) => ("equals", v.clone()),
            Some(WebhookTest::NotEquals(v)) => ("notEquals", v.clone()),
            Some(WebhookTest::StartsWith(v)) => ("startsWith", v.clone()),
            Some(WebhookTest::In(v)) => ("in", v.join(", ")),
            Some(WebhookTest::Exists) => ("exists", String::new()),
            None => ("equals", String::new()),
        };
        Self {
            path: edit_field(filter.map_or("", |f| &f.path), false, window, cx),
            test,
            operand: edit_field(&operand, false, window, cx),
        }
    }
    pub(super) fn build(&self, cx: &App) -> Result<WebhookFilter, String> {
        let value = value(&self.operand, cx);
        let test = match self.test {
            "equals" => WebhookTest::Equals(value),
            "notEquals" => WebhookTest::NotEquals(value),
            "startsWith" => WebhookTest::StartsWith(value),
            "in" => WebhookTest::In(
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect(),
            ),
            "exists" => WebhookTest::Exists,
            _ => return Err("Choose a filter test.".into()),
        };
        let filter = WebhookFilter {
            path: super::value(&self.path, cx).trim().into(),
            test,
        };
        filter.validate()?;
        Ok(filter)
    }
}

impl ReactiveAutomations {
    pub(super) fn render_webhook_filters(
        &self,
        editor: &setup::AutomationEditor,
        cx: &Context<'_, Self>,
    ) -> Div {
        let add = control(
            "webhook-add-filter",
            "Add filter",
            self.busy || editor.filters.len() >= MAX_WEBHOOK_FILTERS,
        )
        .on_activate(cx, |this, window, cx| {
            if let Some(Editor::Automation(editor)) = &mut this.editor {
                if editor.filters.len() < MAX_WEBHOOK_FILTERS {
                    editor.filters.push(FilterRow::new(None, window, cx));
                }
            }
            cx.notify();
        });
        let mut body = div().flex().flex_col().gap_2()
            .child(div().flex().items_center().justify_between().child(heading("Filters")).child(add))
            .child(muted("Run when every filter matches. Use a path such as body.action, headers.x-event or query.source."));
        for (index, row) in editor.filters.iter().take(MAX_WEBHOOK_FILTERS).enumerate() {
            let number = index + 1;
            let toggle = cx.weak_entity();
            let select = toggle.clone();
            let menu = dropdown(
                IDS[index],
                &format!("Filter {number} test"),
                row.test,
                vec![DropdownGroup {
                    label: None,
                    mark: None,
                    options: TESTS
                        .iter()
                        .map(|&(value, label)| DropdownOption {
                            value: value.into(),
                            label: label.to_string(),
                            description: None,
                        })
                        .collect(),
                }],
                if editor.open_filter == Some(index) {
                    Listbox::Popover
                } else {
                    Listbox::Closed
                },
                Rc::new(move |_, cx| {
                    let _ = toggle.update(cx, |this, cx| {
                        if let Some(Editor::Automation(e)) = &mut this.editor {
                            e.task_thread_open = false;
                            e.picker.open = None;
                            e.security.encoding_open = false;
                            e.checkout.close();
                            e.open_filter = (e.open_filter != Some(index)).then_some(index);
                        }
                        cx.notify();
                    });
                }),
                Rc::new(move |test, _, cx| {
                    let _ = select.update(cx, |this, cx| {
                        if let Some(Editor::Automation(e)) = &mut this.editor {
                            if let Some(row) = e.filters.get_mut(index) {
                                if let Some(&(known, _)) = TESTS.iter().find(|(k, _)| *k == test) {
                                    row.test = known;
                                }
                            }
                            e.open_filter = None;
                        }
                        cx.notify();
                    });
                }),
            );
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_end()
                            .gap_2()
                            .child(field(
                                format!("Filter {number} path"),
                                &row.path,
                                false,
                                self.busy,
                            ))
                            .child(
                                ghost_control(
                                    SharedString::from(format!("webhook-remove-{index}")),
                                    "Remove",
                                    self.busy,
                                )
                                .aria_label(format!("Remove filter {number}"))
                                .on_activate(
                                    cx,
                                    move |this, _, cx| {
                                        if let Some(Editor::Automation(e)) = &mut this.editor {
                                            if index < e.filters.len() {
                                                e.filters.remove(index);
                                            }
                                            e.open_filter = None;
                                        }
                                        cx.notify();
                                    },
                                ),
                            ),
                    )
                    .child(menu)
                    .when(row.test != "exists", |body| {
                        body.child(field(
                            format!("Filter {number} value"),
                            &row.operand,
                            false,
                            self.busy,
                        ))
                    })
                    .when(row.test == "in", |body| {
                        body.child(muted("Separate values with commas."))
                    }),
            );
        }
        body
    }
}
