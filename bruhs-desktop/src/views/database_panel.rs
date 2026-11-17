//! Native controls for the same server-owned connections and read policy as the web.
use super::web_button::{
    is_web_button_activation_key, web_button, ButtonState, WebButtonSize, WebButtonVariant,
};
use crate::{
    app_state::spawn_tokio,
    server_client::{ServerClient, ServerError},
    theme,
};
use bruhs_models::{database::*, num::ToFloatLossy};
use gpui::{prelude::*, *};
use gpui_component::zoom::px;
use gpui_component::{
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
};
use std::{collections::BTreeSet, future::Future, rc::Rc};
use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct DatabaseAttachment {
    pub project_path: String,
    pub text: String,
}
struct Editor {
    previous: Option<DatabaseConnection>,
    postgres: bool,
    name: Entity<InputState>,
    path: Entity<InputState>,
    host: Entity<InputState>,
    port: Entity<InputState>,
    database: Entity<InputState>,
    user: Entity<InputState>,
    password: Entity<InputState>,
}
pub(crate) struct DatabasePanel {
    pub project_path: String,
    client: ServerClient,
    connections: Vec<DatabaseConnection>,
    current: Option<DatabaseConnection>,
    tables: Vec<DatabaseTable>,
    table: Option<DatabaseTable>,
    result: Option<DatabaseResult>,
    result_is_table: bool,
    selected: BTreeSet<usize>,
    detail: Option<usize>,
    offset: u64,
    previous_offsets: Vec<u64>,
    last_browse: Option<BrowseDatabase>,
    sql: Entity<InputState>,
    filter_column: Entity<InputState>,
    filter: Entity<InputState>,
    sort: Entity<InputState>,
    descending: bool,
    editor: Option<Editor>,
    error: Option<String>,
    notice: Option<String>,
    busy: bool,
    generation: u64,
    request_id: Option<Uuid>,
    request: Option<Task<()>>,
    input_subscriptions: Vec<Subscription>,
}
impl EventEmitter<DatabaseAttachment> for DatabasePanel {}
fn input(
    value: &str,
    multiline: bool,
    secret: bool,
    window: &mut Window,
    cx: &mut App,
) -> Entity<InputState> {
    cx.new(|cx| {
        let mut input = InputState::new(window, cx).multi_line(multiline);
        if secret {
            input = input.masked(true);
        }
        input.set_value(value, window, cx);
        input
    })
}
fn value(input: &Entity<InputState>, cx: &App) -> String {
    input.read(cx).value().to_string()
}
fn field(label: &'static str, input: &Entity<InputState>, multiline: bool, disabled: bool) -> Div {
    div()
        .min_w_0()
        .w_full()
        .flex()
        .flex_col()
        .gap_1()
        .child(text(label))
        .child(
            Styled::h(
                Input::new(input).aria_label(label).disabled(disabled),
                px(if multiline { 96.0 } else { 32.0 }),
            )
            .text_size(px(14.0)),
        )
}
fn text(label: impl Into<SharedString>) -> Div {
    div()
        .min_w_0()
        .text_size(px(12.0))
        .line_height(relative(1.5))
        .text_color(theme::rambutan_400())
        .child(label.into())
}
fn control(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    disabled: bool,
) -> Stateful<Div> {
    let label = label.into();
    let debug = label.clone();
    web_button(
        id,
        WebButtonVariant::Outline,
        WebButtonSize::Small,
        ButtonState::disabled_if(disabled),
    )
    .aria_label(label.clone())
    .debug_selector(move || format!("database-{debug}"))
    .child(label)
}
trait Action {
    fn activate(
        self,
        cx: &Context<'_, DatabasePanel>,
        f: impl Fn(&mut DatabasePanel, &mut Window, &mut Context<'_, DatabasePanel>) + 'static,
    ) -> Self;
}
impl Action for Stateful<Div> {
    fn activate(
        self,
        cx: &Context<'_, DatabasePanel>,
        f: impl Fn(&mut DatabasePanel, &mut Window, &mut Context<'_, DatabasePanel>) + 'static,
    ) -> Self {
        let f = Rc::new(f);
        let click = Rc::clone(&f);
        self.on_click(cx.listener(move |this, _, window, cx| click(this, window, cx)))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if is_web_button_activation_key(event.keystroke.key.as_str()) {
                    window.prevent_default();
                    cx.stop_propagation();
                    f(this, window, cx);
                }
            }))
    }
}
impl DatabasePanel {
    pub fn new(project_path: String, window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let client =
            ServerClient::from_env().unwrap_or_else(|error| ServerClient::unavailable(&error));
        Self::with_client(project_path, client, window, cx)
    }
    fn with_client(
        project_path: String,
        client: ServerClient,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let mut panel = Self {
            project_path,
            client,
            connections: Vec::new(),
            current: None,
            tables: Vec::new(),
            table: None,
            result: None,
            result_is_table: false,
            selected: BTreeSet::new(),
            detail: None,
            offset: 0,
            previous_offsets: Vec::new(),
            last_browse: None,
            sql: input("SELECT 1", true, false, window, cx),
            filter_column: input("", false, false, window, cx),
            filter: input("", false, false, window, cx),
            sort: input("", false, false, window, cx),
            descending: false,
            editor: None,
            error: None,
            notice: None,
            busy: false,
            generation: 0,
            request_id: None,
            request: None,
            input_subscriptions: Vec::new(),
        };
        for input in [&panel.filter_column, &panel.filter, &panel.sort] {
            panel
                .input_subscriptions
                .push(cx.subscribe(input, |_, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                }));
        }
        panel.reload(cx);
        panel
    }
    fn perform<T: Send + 'static>(
        &mut self,
        operation: impl Future<Output = Result<T, ServerError>> + Send + 'static,
        apply: impl FnOnce(&mut Self, T, &mut Context<'_, Self>) + 'static,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        self.notice = None;
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let operation = spawn_tokio(cx, operation);
        self.request = Some(cx.spawn(async move |this, cx| {
            let result = operation.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.busy = false;
                this.request_id = None;
                match result {
                    Ok(result) => apply(this, result, cx),
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn clear_results(&mut self) {
        self.result = None;
        self.selected.clear();
        self.detail = None;
    }
    fn reload(&mut self, cx: &mut Context<'_, Self>) {
        let client = self.client.clone();
        let project = self.project_path.clone();
        self.perform(
            async move { client.databases(&project).await },
            |this, list, cx| {
                let current = this
                    .current
                    .as_ref()
                    .and_then(|current| list.iter().find(|value| value.id == current.id).cloned());
                this.connections = list;
                if let Some(current) = current {
                    this.choose(current, cx);
                } else {
                    this.current = None;
                    this.table = None;
                    this.tables.clear();
                    this.clear_results();
                }
            },
            cx,
        );
    }
    fn cancel(&mut self, cx: &mut Context<'_, Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.busy = false;
        if let Some(id) = self.request_id.take() {
            let client = self.client.clone();
            spawn_tokio(cx, async move {
                let _ = client.cancel_database_query(id).await;
            })
            .detach();
        }
        self.notice = Some("Query cancelled.".into());
        cx.notify();
    }
    fn choose(&mut self, connection: DatabaseConnection, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        self.current = Some(connection.clone());
        self.tables.clear();
        self.table = None;
        self.offset = 0;
        self.previous_offsets.clear();
        self.clear_results();
        let client = self.client.clone();
        let project = self.project_path.clone();
        self.perform(
            async move { client.database_tables(&project, &connection).await },
            |this, tables, _| this.tables = tables,
            cx,
        );
    }
    fn query(&mut self, sql: String, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        let Some(connection) = &self.current else {
            return;
        };
        let id = Uuid::new_v4();
        let request = ProjectDatabaseQuery {
            project_path: self.project_path.clone(),
            query: DatabaseQuery {
                id: connection.id,
                revision: connection.revision,
                sql,
                request_id: id,
            },
        };
        self.clear_results();
        self.result_is_table = false;
        self.request_id = Some(id);
        let client = self.client.clone();
        self.perform(
            async move { client.database_query(&request).await },
            |this, result, _| this.result = Some(result),
            cx,
        );
    }
    fn browse(&mut self, describe: bool, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        let (Some(connection), Some(table)) = (&self.current, &self.table) else {
            return;
        };
        let request = ProjectDatabaseBrowse {
            project_path: self.project_path.clone(),
            browse: BrowseDatabase {
                request_id: uuid::Uuid::new_v4(),
                id: connection.id,
                revision: connection.revision,
                schema: table.schema.clone(),
                table: table.name.clone(),
                offset: self.offset,
                sort_column: Some(value(&self.sort, cx)).filter(|value| !value.is_empty()),
                descending: self.descending,
                filter_column: Some(value(&self.filter_column, cx))
                    .filter(|value| !value.is_empty()),
                filter: value(&self.filter, cx),
            },
        };
        self.clear_results();
        self.result_is_table = !describe;
        if !describe {
            self.last_browse = Some(request.browse.clone());
        }
        self.request_id = Some(request.browse.request_id);
        let client = self.client.clone();
        self.perform(
            async move {
                if describe {
                    client.database_describe(&request).await
                } else {
                    client.database_browse(&request).await
                }
            },
            |this, result, _| this.result = Some(result),
            cx,
        );
    }
    fn edit(
        &mut self,
        previous: Option<DatabaseConnection>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy {
            return;
        }
        let mut path = String::new();
        let mut host = "127.0.0.1".to_string();
        let mut port = "5432".to_string();
        let mut database = String::new();
        let mut user = String::new();
        let mut postgres = false;
        if let Some(connection) = &previous {
            match &connection.target {
                DatabaseTarget::Sqlite { path: sqlite } => path = sqlite.clone(),
                DatabaseTarget::Postgres {
                    host: h,
                    port: p,
                    database: d,
                    user: u,
                } => {
                    postgres = true;
                    host = h.clone();
                    port = p.to_string();
                    database = d.clone();
                    user = u.clone();
                }
            }
        }
        self.editor = Some(Editor {
            name: input(
                previous.as_ref().map_or("", |value| value.name.as_str()),
                false,
                false,
                window,
                cx,
            ),
            previous,
            postgres,
            path: input(&path, false, false, window, cx),
            host: input(&host, false, false, window, cx),
            port: input(&port, false, false, window, cx),
            database: input(&database, false, false, window, cx),
            user: input(&user, false, false, window, cx),
            password: input("", false, true, window, cx),
        });
        cx.notify();
    }
    fn save(&mut self, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        let Some(editor) = &self.editor else {
            return;
        };
        let target = if editor.postgres {
            let Ok(port) = value(&editor.port, cx).parse() else {
                self.error = Some("Enter a valid Postgres port.".into());
                cx.notify();
                return;
            };
            DatabaseTarget::Postgres {
                host: value(&editor.host, cx),
                port,
                database: value(&editor.database, cx),
                user: value(&editor.user, cx),
            }
        } else {
            DatabaseTarget::Sqlite {
                path: value(&editor.path, cx),
            }
        };
        let request = SaveDatabase {
            id: editor
                .previous
                .as_ref()
                .map_or_else(Uuid::new_v4, |value| value.id),
            project_path: self.project_path.clone(),
            name: value(&editor.name, cx),
            target,
            expected_revision: editor.previous.as_ref().map(|value| value.revision),
            password: Some(value(&editor.password, cx))
                .filter(|value| editor.postgres && !value.is_empty()),
        };
        let client = self.client.clone();
        self.perform(
            async move { client.save_database(&request).await },
            |this, saved, cx| {
                this.editor = None;
                this.connections.retain(|value| value.id != saved.id);
                this.connections.push(saved.clone());
                this.choose(saved, cx);
            },
            cx,
        );
    }
    fn grant(&mut self, cx: &mut Context<'_, Self>) {
        let Some(connection) = &self.current else {
            return;
        };
        let request = DatabaseGrant {
            id: connection.id,
            expected_revision: connection.revision,
            enabled: !connection.agent_access,
        };
        let client = self.client.clone();
        self.perform(
            async move { client.database_grant(&request).await },
            |this, saved, _| {
                this.clear_results();
                if let Some(connection) = this
                    .connections
                    .iter_mut()
                    .find(|value| value.id == saved.id)
                {
                    *connection = saved.clone();
                }
                this.current = Some(saved);
            },
            cx,
        );
    }
    fn attach(&mut self, cx: &mut Context<'_, Self>) {
        let Some(result) = &self.result else {
            return;
        };
        match selected_result_text(result, &self.selected.iter().copied().collect::<Vec<_>>()) {
            Ok(text) => {
                cx.emit(DatabaseAttachment {
                    project_path: self.project_path.clone(),
                    text,
                });
                self.notice = Some(
                    "Selected rows added to the current draft. Review and send when ready.".into(),
                );
            }
            Err(error) => self.error = Some(error.into()),
        }
        cx.notify();
    }
    fn page_ready(&self, cx: &App) -> bool {
        self.last_browse.as_ref().is_some_and(|request| {
            browse_criteria_match(
                request,
                &value(&self.filter_column, cx),
                &value(&self.filter, cx),
                &value(&self.sort, cx),
                self.descending,
            )
        })
    }
    fn render_results(&self, cx: &Context<'_, Self>) -> Option<AnyElement> {
        let result = self.result.as_ref()?;
        let mut rows = div()
            .flex()
            .flex_col()
            .gap_2()
            .min_w_0()
            .child(text(format!(
                "{} rows · {} ms{}",
                result.rows.len(),
                result.elapsed_ms,
                if result.truncated {
                    " · Partial result; narrow the query or use the next table page"
                } else {
                    ""
                }
            )))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(control("db-select-all", "Select all rows", false).activate(
                        cx,
                        |this, _, cx| {
                            if let Some(result) = &this.result {
                                if this.selected.len() == result.rows.len() {
                                    this.selected.clear();
                                } else {
                                    this.selected = (0..result.rows.len()).collect();
                                }
                            }
                            cx.notify();
                        },
                    ))
                    .child(
                        control(
                            "db-attach",
                            "Add selected to draft",
                            self.selected.is_empty(),
                        )
                        .activate(cx, |this, _, cx| this.attach(cx)),
                    ),
            );
        let grid_width = 156.0 + result.columns.len().to_f32_lossy() * 160.0;
        let header = div()
            .min_w(px(grid_width))
            .flex()
            .child(text("Select / row").w(px(156.0)).flex_shrink_0())
            .children(result.columns.iter().map(|column| {
                text(column.name.clone())
                    .w(px(160.0))
                    .flex_shrink_0()
                    .truncate()
            }));
        rows =
            rows.child(
                div()
                    .id("db-result-grid")
                    .w_full()
                    .min_w_0()
                    .overflow_x_scrollbar()
                    .child(header)
                    .child(
                        uniform_list(
                            "database-result-rows",
                            result.rows.len(),
                            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                                range
                                    .map(|index| {
                                        let Some(result) = &this.result else {
                                            return div().into_any_element();
                                        };
                                        let Some(row) = result.rows.get(index) else {
                                            return div().into_any_element();
                                        };
                                        div()
                                            .h(px(40.0))
                                            .min_w(px(
                                                156.0 + result.columns.len().to_f32_lossy() * 160.0
                                            ))
                                            .flex()
                                            .items_center()
                                            .child(
                                                control(
                                                    ("db-select", index),
                                                    format!(
                                                        "{} row {}",
                                                        if this.selected.contains(&index) {
                                                            "Deselect"
                                                        } else {
                                                            "Select"
                                                        },
                                                        index + 1
                                                    ),
                                                    false,
                                                )
                                                .role(Role::CheckBox)
                                                .aria_toggled(if this.selected.contains(&index) {
                                                    Toggled::True
                                                } else {
                                                    Toggled::False
                                                })
                                                .activate(cx, move |this, _, cx| {
                                                    if !this.selected.remove(&index) {
                                                        this.selected.insert(index);
                                                    }
                                                    cx.notify();
                                                }),
                                            )
                                            .child(
                                                control(
                                                    ("db-detail", index),
                                                    format!("Row {}", index + 1),
                                                    false,
                                                )
                                                .activate(cx, move |this, _, cx| {
                                                    this.detail = Some(index);
                                                    cx.notify();
                                                }),
                                            )
                                            .children(row.iter().map(|value| {
                                                text(value.clone().unwrap_or_else(|| "NULL".into()))
                                                    .w(px(160.0))
                                                    .flex_shrink_0()
                                                    .truncate()
                                            }))
                                            .into_any_element()
                                    })
                                    .collect::<Vec<_>>()
                            }),
                        )
                        .h(px(240.0))
                        .min_w(px(grid_width)),
                    ),
            );
        if self.result_is_table {
            rows = rows.child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        control(
                            "db-previous",
                            "Previous rows",
                            self.offset == 0 || self.busy || !self.page_ready(cx),
                        )
                        .activate(cx, |this, _, cx| {
                            if this.offset > 0 && !this.busy && this.page_ready(cx) {
                                if let Some(offset) = this.previous_offsets.pop() {
                                    this.offset = offset;
                                }
                                this.browse(false, cx);
                            }
                        }),
                    )
                    .child(
                        control(
                            "db-next",
                            "Next rows",
                            result.next_offset.is_none() || self.busy || !self.page_ready(cx),
                        )
                        .activate(cx, |this, _, cx| {
                            if !this.busy
                                && this.page_ready(cx)
                                && this
                                    .result
                                    .as_ref()
                                    .is_some_and(|result| result.next_offset.is_some())
                            {
                                if let Some(offset) =
                                    this.result.as_ref().and_then(|result| result.next_offset)
                                {
                                    this.previous_offsets.push(this.offset);
                                    this.offset = offset;
                                }
                                this.browse(false, cx);
                            }
                        }),
                    ),
            );
        }
        if let Some(index) = self.detail {
            if let Some(row) = result.rows.get(index) {
                rows = rows.child(
                    div()
                        .p_3()
                        .border_1()
                        .border_color(theme::longan_700())
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(control("db-close-detail", "Close detail", false).activate(
                            cx,
                            |this, _, cx| {
                                this.detail = None;
                                cx.notify();
                            },
                        ))
                        .children(result.columns.iter().zip(row).map(|(column, value)| {
                            div().child(text(column.name.clone())).child(
                                text(value.clone().unwrap_or_else(|| "NULL".into()))
                                    .text_color(theme::rambutan_200())
                                    .whitespace_normal(),
                            )
                        })),
                );
            }
        }
        Some(rows.into_any_element())
    }
}
impl Render for DatabasePanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let mut body = div()
            .id("native-database-panel")
            .size_full()
            .overflow_y_scrollbar()
            .p_3()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(text("Databases").flex_1().text_color(theme::rambutan_100()))
                    .child(
                        control("db-refresh", "Refresh", self.busy)
                            .activate(cx, |this, _, cx| this.reload(cx)),
                    )
                    .child(
                        control("db-connect", "Connect", self.busy)
                            .activate(cx, |this, window, cx| this.edit(None, window, cx)),
                    ),
            )
            .child(text(
                "Read-only · Results stay here until you attach rows. Agent access starts off.",
            ));
        if let Some(error) = &self.error {
            body = body.child(
                text(error.clone())
                    .id("database-error")
                    .role(Role::Alert)
                    .text_color(theme::persimmon_300()),
            );
        }
        if let Some(notice) = &self.notice {
            body = body.child(
                text(notice.clone())
                    .id("database-notice")
                    .role(Role::Status),
            );
        }
        if let Some(editor) = &self.editor {
            let mut form = div()
                .p_3()
                .border_1()
                .border_color(theme::longan_700())
                .rounded(px(8.0))
                .flex()
                .flex_col()
                .gap_2()
                .child(field("Connection name", &editor.name, false, self.busy))
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            control("db-sqlite", "SQLite file", self.busy)
                                .aria_selected(!editor.postgres)
                                .activate(cx, |this, _, cx| {
                                    if !this.busy {
                                        if let Some(editor) = &mut this.editor {
                                            editor.postgres = false;
                                        }
                                        cx.notify();
                                    }
                                }),
                        )
                        .child(
                            control("db-postgres", "Local Postgres", self.busy)
                                .aria_selected(editor.postgres)
                                .activate(cx, |this, _, cx| {
                                    if !this.busy {
                                        if let Some(editor) = &mut this.editor {
                                            editor.postgres = true;
                                        }
                                        cx.notify();
                                    }
                                }),
                        ),
                );
            if editor.postgres {
                form = form
                    .child(field("Host", &editor.host, false, self.busy))
                    .child(field("Port", &editor.port, false, self.busy))
                    .child(field("Database", &editor.database, false, self.busy))
                    .child(field("User", &editor.user, false, self.busy))
                    .child(field(
                        "Password (blank keeps saved value)",
                        &editor.password,
                        false,
                        self.busy,
                    ))
                    .child(text(
                        "Saved in the native OS credential store. Local loopback connections only.",
                    ));
            } else {
                form = form.child(field(
                    "SQLite file on this machine",
                    &editor.path,
                    false,
                    self.busy,
                ));
            }
            body = body.child(
                form.child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            control("db-save", "Save connection", self.busy)
                                .activate(cx, |this, _, cx| this.save(cx)),
                        )
                        .child(control("db-editor-cancel", "Cancel", self.busy).activate(
                            cx,
                            |this, _, cx| {
                                if !this.busy {
                                    this.editor = None;
                                    cx.notify();
                                }
                            },
                        )),
                ),
            );
        }
        body = body.child(
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .children(
                    self.connections
                        .iter()
                        .enumerate()
                        .map(|(index, connection)| {
                            let connection = connection.clone();
                            control(("db-connection", index), connection.name.clone(), self.busy)
                                .aria_selected(
                                    self.current
                                        .as_ref()
                                        .is_some_and(|current| current.id == connection.id),
                                )
                                .activate(cx, move |this, _, cx| {
                                    this.choose(connection.clone(), cx)
                                })
                        }),
                ),
        );
        if let Some(connection) = &self.current {
            body = body.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        control("db-edit", "Edit connection", self.busy)
                            .activate(cx, |this, window, cx| {
                                this.edit(this.current.clone(), window, cx)
                            }),
                    )
                    .child(
                        control(
                            "db-agent-access",
                            if connection.agent_access {
                                "Agent access: on"
                            } else {
                                "Agent access: off"
                            },
                            self.busy,
                        )
                        .role(Role::Switch)
                        .aria_toggled(if connection.agent_access {
                            Toggled::True
                        } else {
                            Toggled::False
                        })
                        .activate(cx, |this, _, cx| this.grant(cx)),
                    ),
            );
            body = body.child(text("Schema / table")).child(
                div()
                    .id("db-tables")
                    .max_h(px(128.0))
                    .overflow_y_scrollbar()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(self.tables.iter().enumerate().map(|(index, table)| {
                        let table = table.clone();
                        control(
                            ("db-table", index),
                            format!("{}.{}", table.schema, table.name),
                            self.busy,
                        )
                        .activate(cx, move |this, _, cx| {
                            if this.busy {
                                return;
                            }
                            this.table = Some(table.clone());
                            this.offset = 0;
                            this.previous_offsets.clear();
                            this.browse(false, cx);
                        })
                    })),
            );
            if self.table.is_some() {
                body = body
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_2()
                            .child(control("db-browse", "Browse rows", self.busy).activate(
                                cx,
                                |this, _, cx| {
                                    this.offset = 0;
                                    this.previous_offsets.clear();
                                    this.browse(false, cx);
                                },
                            ))
                            .child(
                                control("db-describe", "Describe columns", self.busy)
                                    .activate(cx, |this, _, cx| this.browse(true, cx)),
                            ),
                    )
                    .child(field(
                        "Filter column",
                        &self.filter_column,
                        false,
                        self.busy,
                    ))
                    .child(field("Contains text", &self.filter, false, self.busy))
                    .child(field("Sort column", &self.sort, false, self.busy))
                    .child(
                        control("db-descending", "Descending", self.busy)
                            .role(Role::CheckBox)
                            .aria_toggled(if self.descending {
                                Toggled::True
                            } else {
                                Toggled::False
                            })
                            .activate(cx, |this, _, cx| {
                                if !this.busy {
                                    this.descending = !this.descending;
                                    cx.notify();
                                }
                            }),
                    );
            }
            body = body
                .child(field("Read-only SQL", &self.sql, true, self.busy))
                .child(
                    control("db-run", "Run query", self.busy)
                        .activate(cx, |this, _, cx| this.query(value(&this.sql, cx), cx)),
                );
        }
        if self.busy {
            body = body
                .child(text("Working…").id("database-loading").role(Role::Status))
                .when(self.request_id.is_some(), |body| {
                    body.child(
                        control("db-cancel", "Cancel query", false)
                            .activate(cx, |this, _, cx| this.cancel(cx)),
                    )
                });
        }
        body.when_some(self.render_results(cx), |body, results| body.child(results))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        value, DatabaseAttachment, DatabaseColumn, DatabasePanel, DatabaseResult, ServerClient,
        Uuid,
    };
    use crate::theme;
    use bruhs_models::database::{
        BrowseDatabase, DatabaseConnection, DatabaseTable, DatabaseTarget,
    };
    use gpui::{px, size, AppContext, Modifiers, TestAppContext};
    use std::rc::Rc;
    #[gpui::test]
    fn database_native_controls_keep_data_private_until_selected(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            theme::apply_bruhs_theme(cx);
            cx.set_global(crate::app_state::DesktopState::new().unwrap());
        });
        let holder = Rc::new(std::cell::RefCell::new(None));
        let capture = Rc::clone(&holder);
        let (_, cx) = cx.add_window_view(|window, cx| {
            let client = ServerClient::new("http://127.0.0.1:0", "http://127.0.0.1:0").unwrap();
            let panel = cx.new(|cx| {
                DatabasePanel::with_client("/synthetic/project".into(), client, window, cx)
            });
            *capture.borrow_mut() = Some(panel.clone());
            gpui_component::Root::new(panel, window, cx)
        });
        cx.simulate_resize(size(px(640.0), px(1100.0)));
        let panel = holder.borrow().clone().unwrap();
        let attached = Rc::new(std::cell::RefCell::new(Vec::<DatabaseAttachment>::new()));
        let capture = Rc::clone(&attached);
        let subscription = cx.update(|_, cx| {
            cx.subscribe(&panel, move |_, event: &DatabaseAttachment, _| {
                capture.borrow_mut().push(event.clone())
            })
        });
        panel.update_in(cx, |panel, window, cx| {
            panel.generation = panel.generation.wrapping_add(1);
            panel.busy = false;
            panel.error = None;
            panel.edit(None, window, cx);
            assert!(!panel.editor.as_ref().unwrap().postgres);
            let editor = panel.editor.as_ref().unwrap();
            editor
                .name
                .update(cx, |input, cx| input.set_value("Fixture", window, cx));
            editor.path.update(cx, |input, cx| {
                input.set_value("/synthetic/db.sqlite3", window, cx)
            });
            let old_name = value(&editor.name, cx);
            panel.notice = Some("A view refresh".into());
            assert_eq!(value(&panel.editor.as_ref().unwrap().name, cx), old_name);
            panel.editor = None;
            panel.result = Some(DatabaseResult {
                connection_id: Uuid::new_v4(),
                revision: 1,
                columns: vec![DatabaseColumn {
                    name: "name".into(),
                    data_type: "text".into(),
                }],
                rows: vec![vec![Some("Ada".into())], vec![Some("Lin".into())]],
                truncated: false,
                elapsed_ms: 1,
                next_offset: None,
            });
            cx.notify();
        });
        cx.run_until_parked();
        assert!(attached.borrow().is_empty());
        let select = cx
            .debug_bounds("database-Select row 1")
            .expect("row selection control");
        cx.simulate_click(select.center(), Modifiers::default());
        cx.run_until_parked();
        let attach = cx
            .debug_bounds("database-Add selected to draft")
            .expect("explicit attachment control");
        cx.simulate_click(attach.center(), Modifiers::default());
        cx.run_until_parked();
        assert_eq!(attached.borrow().len(), 1);
        assert!(attached.borrow()[0].text.contains("Ada"));
        assert!(!attached.borrow()[0].text.contains("Lin"));
        assert_eq!(attached.borrow()[0].project_path, "/synthetic/project");
        panel.update_in(cx, |panel, _, cx| {
            assert_eq!(panel.selected.len(), 1);
            assert!(panel.notice.as_deref().unwrap().contains("current draft"));
            cx.notify();
        });
        // Exercise real native controls with a partial page whose cursor is below ROW_LIMIT.
        cx.simulate_resize(size(px(640.0), px(1600.0)));
        let id = Uuid::new_v4();
        let partial = DatabaseResult {
            connection_id: id,
            revision: 1,
            columns: vec![DatabaseColumn {
                name: "name".into(),
                data_type: "text".into(),
            }],
            rows: (1..=15)
                .map(|row| vec![Some(format!("Page row {row}"))])
                .collect(),
            truncated: true,
            elapsed_ms: 1,
            next_offset: Some(15),
        };
        panel.update_in(cx, |panel, _, cx| {
            panel.generation = panel.generation.wrapping_add(1);
            panel.busy = false;
            panel.error = None;
            panel.notice = None;
            panel.current = Some(DatabaseConnection {
                id,
                project_path: panel.project_path.clone(),
                name: "Fixture".into(),
                target: DatabaseTarget::Sqlite {
                    path: "/synthetic/data.sqlite3".into(),
                },
                revision: 1,
                agent_access: false,
                password_saved: false,
                credential_version: Uuid::new_v4(),
            });
            panel.table = Some(DatabaseTable {
                schema: "main".into(),
                name: "customers".into(),
                kind: "table".into(),
            });
            panel.result = Some(partial.clone());
            panel.result_is_table = true;
            panel.last_browse = Some(BrowseDatabase {
                request_id: Uuid::new_v4(),
                id,
                revision: 1,
                schema: "main".into(),
                table: "customers".into(),
                offset: 0,
                sort_column: None,
                descending: false,
                filter_column: None,
                filter: String::new(),
            });
            cx.notify();
        });
        cx.run_until_parked();
        let next = cx
            .debug_bounds("database-Next rows")
            .expect("next table page");
        cx.simulate_click(next.center(), Modifiers::default());
        cx.run_until_parked();
        panel.update_in(cx, |panel, _, cx| {
            assert_eq!(panel.offset, 15);
            assert_eq!(panel.last_browse.as_ref().unwrap().offset, 15);
            assert_eq!(panel.previous_offsets, vec![0]);
            panel.generation = panel.generation.wrapping_add(1);
            panel.busy = false;
            panel.error = None;
            panel.result = Some(partial.clone());
            cx.notify();
        });
        cx.run_until_parked();
        let previous = cx
            .debug_bounds("database-Previous rows")
            .expect("previous table page");
        cx.simulate_click(previous.center(), Modifiers::default());
        cx.run_until_parked();
        panel.update_in(cx, |panel, window, cx| {
            assert_eq!(panel.offset, 0);
            assert!(panel.previous_offsets.is_empty());
            panel.generation = panel.generation.wrapping_add(1);
            panel.busy = false;
            panel.error = None;
            panel.result = Some(partial.clone());
            panel
                .sort
                .update(cx, |input, cx| input.set_value("name", window, cx));
            cx.notify();
        });
        cx.run_until_parked();
        let next = cx.debug_bounds("database-Next rows").unwrap();
        cx.simulate_click(next.center(), Modifiers::default());
        cx.run_until_parked();
        panel.update_in(cx, |panel, _, _| {
            assert_eq!(panel.offset, 0);
            assert_eq!(panel.last_browse.as_ref().unwrap().sort_column, None);
        });
        let browse = cx
            .debug_bounds("database-Browse rows")
            .expect("restart with changed criteria");
        cx.simulate_click(browse.center(), Modifiers::default());
        cx.run_until_parked();
        panel.update_in(cx, |panel, _, _| {
            assert_eq!(panel.offset, 0);
            assert_eq!(
                panel.last_browse.as_ref().unwrap().sort_column.as_deref(),
                Some("name")
            );
        });
        drop(subscription);
    }
}
