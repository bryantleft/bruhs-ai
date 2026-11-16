//! Project data stays in this view until an explicit composer attachment or connection grant.
use crate::{
    api,
    state::{ActiveThreadId, ComposerDraft, ThreadsSignal},
};
use bruhs_models::database::*;
use leptos::prelude::*;
use uuid::Uuid;

const CONTROL: &str="bruhs-control min-h-control w-full min-w-0 rounded-grape border border-longan-700 bg-longan-950 px-2 py-1 text-[16px] text-rambutan-200 outline-none focus:border-persimmon-400 md:text-[13px]";
const BUTTON: &str="bruhs-control min-h-control rounded-grape border border-longan-700 px-2 py-1 text-label-sm text-rambutan-300 hover:bg-longan-800 focus-visible:outline focus-visible:outline-persimmon-400 disabled:opacity-40";

#[derive(Clone, Copy)]
struct Ui {
    project: Memo<Option<String>>,
    connections: RwSignal<Vec<DatabaseConnection>>,
    current: RwSignal<Option<DatabaseConnection>>,
    tables: RwSignal<Vec<DatabaseTable>>,
    table: RwSignal<Option<DatabaseTable>>,
    result: RwSignal<Option<DatabaseResult>>,
    result_is_table: RwSignal<bool>,
    selected: RwSignal<Vec<usize>>,
    detail: RwSignal<Option<usize>>,
    offset: RwSignal<u64>,
    previous_offsets: RwSignal<Vec<u64>>,
    last_browse: RwSignal<Option<BrowseDatabase>>,
    sql: RwSignal<String>,
    filter: RwSignal<String>,
    filter_column: RwSignal<String>,
    sort: RwSignal<String>,
    descending: RwSignal<bool>,
    error: RwSignal<Option<String>>,
    notice: RwSignal<Option<String>>,
    busy: RwSignal<bool>,
    request: RwSignal<Option<Uuid>>,
    generation: RwSignal<u64>,
    alive: RwSignal<bool>,
}
impl Ui {
    fn begin(self) -> Option<u64> {
        if self.busy.get_untracked() {
            return None;
        }
        self.busy.set(true);
        self.error.set(None);
        self.notice.set(None);
        self.generation
            .update(|value| *value = value.wrapping_add(1));
        Some(self.generation.get_untracked())
    }
    fn finish(self, generation: u64) -> bool {
        if self.alive.try_get_untracked() != Some(true)
            || self.generation.try_get_untracked() != Some(generation)
        {
            return false;
        }
        self.busy.set(false);
        self.request.set(None);
        true
    }
    fn clear_results(self) {
        self.result.set(None);
        self.selected.set(Vec::new());
        self.detail.set(None);
    }
    fn cancel(self) {
        let id = self.request.get_untracked();
        self.generation
            .update(|value| *value = value.wrapping_add(1));
        self.busy.set(false);
        self.request.set(None);
        if let Some(id) = id {
            leptos::task::spawn_local(async move {
                let _ = api::cancel_database_query(id).await;
            });
        }
    }
    fn reload(self) {
        let Some(project) = self.project.get_untracked() else {
            return;
        };
        let Some(generation) = self.begin() else {
            return;
        };
        leptos::task::spawn_local(async move {
            let result = api::databases(&project).await;
            if !self.finish(generation) {
                return;
            }
            match result {
                Ok(list) => {
                    let current = self.current.get_untracked().and_then(|current| {
                        list.iter().find(|value| value.id == current.id).cloned()
                    });
                    self.connections.set(list);
                    if let Some(current) = current {
                        self.choose(current);
                    } else {
                        self.current.set(None);
                        self.tables.set(Vec::new());
                        self.table.set(None);
                        self.clear_results();
                    }
                }
                Err(error) => self.error.set(Some(error.to_string())),
            }
        });
    }
    fn choose(self, connection: DatabaseConnection) {
        self.cancel();
        self.current.set(Some(connection.clone()));
        self.table.set(None);
        self.tables.set(Vec::new());
        self.offset.set(0);
        self.previous_offsets.set(Vec::new());
        self.clear_results();
        let Some(project) = self.project.get_untracked() else {
            return;
        };
        let Some(generation) = self.begin() else {
            return;
        };
        leptos::task::spawn_local(async move {
            let result = api::database_tables(&project, &connection).await;
            if !self.finish(generation) {
                return;
            }
            match result {
                Ok(tables) => self.tables.set(tables),
                Err(error) => self.error.set(Some(error.to_string())),
            }
        });
    }
    fn query(self, sql: String) {
        let (Some(project), Some(connection)) =
            (self.project.get_untracked(), self.current.get_untracked())
        else {
            return;
        };
        let Some(generation) = self.begin() else {
            return;
        };
        let id = Uuid::new_v4();
        self.request.set(Some(id));
        self.clear_results();
        self.result_is_table.set(false);
        leptos::task::spawn_local(async move {
            let result = api::database_query(&ProjectDatabaseQuery {
                project_path: project,
                query: DatabaseQuery {
                    id: connection.id,
                    revision: connection.revision,
                    sql,
                    request_id: id,
                },
            })
            .await;
            if !self.finish(generation) {
                return;
            }
            match result {
                Ok(result) => self.result.set(Some(result)),
                Err(error) => self.error.set(Some(error.to_string())),
            }
        });
    }
    fn page_ready(self) -> bool {
        self.last_browse.get().is_some_and(|request| {
            browse_criteria_match(
                &request,
                &self.filter_column.get(),
                &self.filter.get(),
                &self.sort.get(),
                self.descending.get(),
            )
        })
    }
    fn browse(self) {
        self.browse_or_describe(false);
    }
    fn browse_or_describe(self, describe: bool) {
        let (Some(project), Some(connection), Some(table)) = (
            self.project.get_untracked(),
            self.current.get_untracked(),
            self.table.get_untracked(),
        ) else {
            return;
        };
        let Some(generation) = self.begin() else {
            return;
        };
        self.clear_results();
        let request = ProjectDatabaseBrowse {
            project_path: project,
            browse: BrowseDatabase {
                request_id: uuid::Uuid::new_v4(),
                id: connection.id,
                revision: connection.revision,
                schema: table.schema,
                table: table.name,
                offset: self.offset.get_untracked(),
                sort_column: Some(self.sort.get_untracked()).filter(|value| !value.is_empty()),
                descending: self.descending.get_untracked(),
                filter_column: Some(self.filter_column.get_untracked())
                    .filter(|value| !value.is_empty()),
                filter: self.filter.get_untracked(),
            },
        };
        if !describe {
            self.last_browse.set(Some(request.browse.clone()));
        }
        self.request.set(Some(request.browse.request_id));
        self.result_is_table.set(!describe);
        leptos::task::spawn_local(async move {
            let result = if describe {
                api::database_describe(&request).await
            } else {
                api::database_browse(&request).await
            };
            if !self.finish(generation) {
                return;
            }
            match result {
                Ok(result) => self.result.set(Some(result)),
                Err(error) => self.error.set(Some(error.to_string())),
            }
        });
    }
}

#[component]
fn Field(
    label: &'static str,
    value: RwSignal<String>,
    #[prop(default = false)] secret: bool,
) -> impl IntoView {
    view! {<label class="flex min-w-0 flex-col gap-1 text-label-sm text-rambutan-400"><span>{label}</span><input class=CONTROL type=if secret {"password"}else{"text"} autocomplete=if secret {"new-password"}else{"off"} prop:value=move || value.get() on:input=move |event|value.set(event_target_value(&event)) /></label>}
}

#[component]
pub fn DatabasePanel() -> impl IntoView {
    let threads = expect_context::<ThreadsSignal>().0;
    let active = expect_context::<ActiveThreadId>().0;
    let project = Memo::new(move |_| {
        threads.track();
        active.track();
        super::files_panel::project_path()
    });
    let ui = Ui {
        project,
        connections: RwSignal::new(Vec::new()),
        current: RwSignal::new(None),
        tables: RwSignal::new(Vec::new()),
        table: RwSignal::new(None),
        result: RwSignal::new(None),
        result_is_table: RwSignal::new(false),
        selected: RwSignal::new(Vec::new()),
        detail: RwSignal::new(None),
        offset: RwSignal::new(0),
        previous_offsets: RwSignal::new(Vec::new()),
        last_browse: RwSignal::new(None),
        sql: RwSignal::new("SELECT 1".into()),
        filter: RwSignal::new(String::new()),
        filter_column: RwSignal::new(String::new()),
        sort: RwSignal::new(String::new()),
        descending: RwSignal::new(false),
        error: RwSignal::new(None),
        notice: RwSignal::new(None),
        busy: RwSignal::new(false),
        request: RwSignal::new(None),
        generation: RwSignal::new(0),
        alive: RwSignal::new(true),
    };
    let editor = RwSignal::new(false);
    let editing = RwSignal::new(None::<DatabaseConnection>);
    let name = RwSignal::new(String::new());
    let engine = RwSignal::new("sqlite".to_string());
    let path = RwSignal::new(String::new());
    let host = RwSignal::new("127.0.0.1".into());
    let port = RwSignal::new("5432".into());
    let database = RwSignal::new(String::new());
    let user = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    Effect::new(move |_| {
        project.track();
        ui.cancel();
        ui.current.set(None);
        ui.tables.set(Vec::new());
        ui.table.set(None);
        ui.connections.set(Vec::new());
        ui.clear_results();
        editor.set(false);
        password.set(String::new());
        ui.reload();
    });
    on_cleanup(move || {
        ui.cancel();
        ui.alive.set(false);
        password.set(String::new());
    });
    let open_editor = move |connection: Option<DatabaseConnection>| {
        ui.error.set(None);
        password.set(String::new());
        name.set(
            connection
                .as_ref()
                .map_or_else(String::new, |value| value.name.clone()),
        );
        if let Some(connection) = &connection {
            match &connection.target {
                DatabaseTarget::Sqlite { path: sqlite } => {
                    engine.set("sqlite".into());
                    path.set(sqlite.clone());
                }
                DatabaseTarget::Postgres {
                    host: pg,
                    port: p,
                    database: db,
                    user: u,
                } => {
                    engine.set("postgres".into());
                    host.set(pg.clone());
                    port.set(p.to_string());
                    database.set(db.clone());
                    user.set(u.clone());
                }
            }
        }
        editing.set(connection);
        editor.set(true);
    };
    let save = move |_| {
        let Some(project) = project.get_untracked() else {
            return;
        };
        let target = if engine.get_untracked() == "sqlite" {
            DatabaseTarget::Sqlite {
                path: path.get_untracked(),
            }
        } else {
            let Ok(port) = port.get_untracked().parse() else {
                ui.error.set(Some("Enter a valid Postgres port.".into()));
                return;
            };
            DatabaseTarget::Postgres {
                host: host.get_untracked(),
                port,
                database: database.get_untracked(),
                user: user.get_untracked(),
            }
        };
        let Some(generation) = ui.begin() else {
            return;
        };
        let previous = editing.get_untracked();
        let request = SaveDatabase {
            id: previous
                .as_ref()
                .map_or_else(Uuid::new_v4, |value| value.id),
            project_path: project,
            name: name.get_untracked(),
            target,
            expected_revision: previous.map(|value| value.revision),
            password: Some(password.get_untracked())
                .filter(|value| !value.is_empty() && engine.get_untracked() == "postgres"),
        };
        leptos::task::spawn_local(async move {
            let result = api::save_database(&request).await;
            if !ui.finish(generation) {
                return;
            }
            match result {
                Ok(connection) => {
                    password.set(String::new());
                    editor.set(false);
                    ui.connections.update(|list| {
                        list.retain(|value| value.id != connection.id);
                        list.push(connection.clone());
                    });
                    ui.choose(connection);
                }
                Err(error) => ui.error.set(Some(error.to_string())),
            }
        });
    };
    let draft = expect_context::<ComposerDraft>().0;
    let attach = move |_| {
        let Some(result) = ui.result.get_untracked() else {
            return;
        };
        match selected_result_text(&result, &ui.selected.get_untracked()) {
            Ok(text) => {
                draft.update(|draft| {
                    if !draft.is_empty() {
                        draft.push_str("\n\n");
                    }
                    draft.push_str(&text);
                });
                ui.notice.set(Some(
                    "Selected rows added to the current draft. Review and send when ready.".into(),
                ));
            }
            Err(error) => ui.error.set(Some(error.into())),
        }
    };
    view! {
        <section data-database-panel class="flex h-full min-h-0 flex-col text-label-sm" aria-label="Databases">
            <div class="min-h-0 flex-1 space-y-3 overflow-y-auto p-3">
                <div class="flex flex-wrap items-center gap-2"><h2 class="min-w-0 flex-1 font-medium text-rambutan-200">"Databases"</h2><button type="button" class=BUTTON disabled=move ||ui.busy.get() on:click=move |_|ui.reload()>"Refresh"</button><button type="button" class=BUTTON disabled=move ||project.get().is_none()||ui.busy.get() on:click=move |_|open_editor(None)>"Connect"</button></div>
                <p class="text-rambutan-500">"Read-only · Results stay here until you attach rows. Agent access starts off."</p>
                {move ||project.get().is_none().then(||view!{<p>"Open a project to connect a database."</p>})}
                {move ||ui.error.get().map(|error|view!{<p role="alert" class="break-words text-persimmon-300">{error}</p>})}
                {move ||ui.notice.get().map(|notice|view!{<p role="status" class="text-rambutan-300">{notice}</p>})}
                <Show when=move ||editor.get()>
                    <fieldset disabled=move ||ui.busy.get() class="space-y-3 rounded-grape border border-longan-700 p-3" aria-label="Database connection">
                        <Field label="Connection name" value=name />
                        <label class="flex flex-col gap-1">"Engine"<select aria-label="Engine" class=CONTROL prop:value=move ||engine.get() on:change=move |event|engine.set(event_target_value(&event))><option value="sqlite">"SQLite file"</option><option value="postgres">"Local Postgres"</option></select></label>
                        <Show when=move ||engine.get()=="sqlite" fallback=move ||view!{<div class="grid min-w-0 grid-cols-2 gap-3"><Field label="Host" value=host /><Field label="Port" value=port /><Field label="Database" value=database /><Field label="User" value=user /></div><Field label="Password (blank keeps saved value)" value=password secret=true /><p class="text-rambutan-500">"Saved in the native OS credential store. Local loopback connections only."</p>}><Field label="SQLite file on this machine" value=path /></Show>
                        <div class="flex gap-2"><button type="button" class=BUTTON disabled=move ||ui.busy.get() on:click=save>"Save connection"</button><button type="button" class=BUTTON on:click=move |_|{editor.set(false);password.set(String::new());}>"Cancel"</button></div>
                    </fieldset>
                </Show>
                <div class="flex flex-wrap gap-2" aria-label="Saved database connections">
                    {move ||ui.connections.get().into_iter().map(|connection|{let title=connection.name.clone();let id=connection.id;view!{<button type="button" class=BUTTON aria-pressed=move ||(ui.current.get().as_ref().map(|value|value.id)==Some(id)).to_string() disabled=move ||ui.busy.get() on:click=move |_|ui.choose(connection.clone())>{title}</button>}}).collect_view()}
                </div>
                <Show when=move ||ui.current.get().is_some()>
                    <div class="flex flex-wrap items-center gap-2"><button type="button" class=BUTTON disabled=move ||ui.busy.get() on:click=move |_|open_editor(ui.current.get_untracked())>"Edit connection"</button><button type="button" role="switch" aria-label="Allow agents to read this connection" aria-checked=move ||ui.current.get().is_some_and(|value|value.agent_access).to_string() class=BUTTON disabled=move ||ui.busy.get() on:click=move |_|{
                        let Some(connection)=ui.current.get_untracked() else {return;};let Some(generation)=ui.begin() else {return;};
                        leptos::task::spawn_local(async move {let result=api::database_grant(&DatabaseGrant{id:connection.id,expected_revision:connection.revision,enabled:!connection.agent_access}).await;if !ui.finish(generation){return;}match result{Ok(saved)=>{ui.clear_results();ui.current.set(Some(saved.clone()));ui.connections.update(|list|{if let Some(old)=list.iter_mut().find(|value|value.id==saved.id){*old=saved;}});},Err(error)=>ui.error.set(Some(error.to_string()))}});
                    }>{move ||if ui.current.get().is_some_and(|value|value.agent_access){"Agent access: on"}else{"Agent access: off"}}</button></div>
                    <label class="flex flex-col gap-1">"Schema / table"<select aria-label="Schema / table" disabled=move ||ui.busy.get() class=CONTROL prop:value=move ||ui.table.get().and_then(|selected|ui.tables.with(|tables|tables.iter().position(|table|table==&selected))).map_or_else(String::new,|index|index.to_string()) on:change=move |event|{let value=event_target_value(&event);ui.table.set(value.parse::<usize>().ok().and_then(|index|ui.tables.with_untracked(|tables|tables.get(index).cloned())));ui.offset.set(0);ui.previous_offsets.set(Vec::new());ui.filter.set(String::new());ui.filter_column.set(String::new());ui.sort.set(String::new());ui.browse();}><option value="">"Choose a table or view"</option>{move ||ui.tables.get().into_iter().enumerate().map(|(index,table)|{let label=format!("{}.{}",table.schema,table.name);view!{<option value=index.to_string()>{label}</option>}}).collect_view()}</select></label>
                    <Show when=move ||ui.table.get().is_some()>
                        <div class="flex flex-wrap gap-2"><button type="button" class=BUTTON disabled=move ||ui.busy.get() on:click=move |_|{ui.offset.set(0);ui.previous_offsets.set(Vec::new());ui.browse();}>"Browse rows"</button><button type="button" class=BUTTON disabled=move ||ui.busy.get() on:click=move |_|{ui.browse_or_describe(true);}>"Describe columns"</button></div>
                        <fieldset disabled=move ||ui.busy.get() class="grid grid-cols-2 gap-2"><Field label="Filter column" value=ui.filter_column /><Field label="Contains text" value=ui.filter /><Field label="Sort column" value=ui.sort /><label class="flex items-end gap-2 pb-2"><input type="checkbox" prop:checked=move ||ui.descending.get() on:change=move |event|ui.descending.set(event_target_checked(&event)) />"Descending"</label></fieldset>
                    </Show>
                    <label class="flex flex-col gap-1">"Read-only SQL"<textarea disabled=move ||ui.busy.get() class=CONTROL rows=4 prop:value=move ||ui.sql.get() on:input=move |event|ui.sql.set(event_target_value(&event))></textarea></label>
                    <div class="flex flex-wrap gap-2"><button type="button" class=BUTTON disabled=move ||ui.busy.get() on:click=move |_|ui.query(ui.sql.get_untracked())>"Run query"</button><Show when=move ||ui.busy.get()><span role="status" class="self-center text-rambutan-500">"Working…"</span><Show when=move ||ui.request.get().is_some()><button type="button" class=BUTTON on:click=move |_|{ui.cancel();ui.notice.set(Some("Query cancelled.".into()));}>"Cancel query"</button></Show></Show></div>
                </Show>
                {move ||ui.result.get().map(|result|{
                    let rows=result.rows.clone();let columns=result.columns.clone();
                    view!{<div class="space-y-2" aria-label="Query results"><p class="text-rambutan-500">{format!("{} rows · {} ms{}",rows.len(),result.elapsed_ms,if result.truncated{" · Partial result: narrow the query or use the next table page"}else{""})}</p>
                        <div class="flex flex-wrap gap-2"><button type="button" class=BUTTON on:click=move |_|{if ui.selected.get_untracked().len()==rows.len(){ui.selected.set(Vec::new());}else{ui.selected.set((0..rows.len()).collect());}}>"Select all rows"</button><button type="button" class=BUTTON disabled=move ||ui.selected.get().is_empty() on:click=attach>"Add selected to draft"</button></div>
                        <div class="max-h-80 max-w-full overflow-auto rounded-grape border border-longan-700"><table class="w-full border-collapse text-left text-label-sm"><thead><tr><th class="sticky top-0 bg-longan-900 p-2">"Select"</th>{columns.iter().map(|column|view!{<th class="sticky top-0 whitespace-nowrap bg-longan-900 p-2" title=column.data_type.clone()>{column.name.clone()}</th>}).collect_view()}</tr></thead><tbody>{result.rows.into_iter().enumerate().map(|(index,row)|view!{<tr><td class="border-t border-longan-800 p-2"><label class="bruhs-control inline-flex min-h-control min-w-control items-center justify-center"><input type="checkbox" aria-label=format!("Select row {}",index+1) prop:checked=move ||ui.selected.get().contains(&index) on:change=move |event|ui.selected.update(|selected|{if event_target_checked(&event){if !selected.contains(&index){selected.push(index);}}else{selected.retain(|value|*value!=index);}}) /></label><button type="button" class=BUTTON aria-label=format!("Inspect row {}",index+1) on:click=move |_|ui.detail.set(Some(index))>"Detail"</button></td>{row.into_iter().map(|cell|view!{<td class="max-w-64 truncate border-t border-longan-800 p-2" title=cell.unwrap_or_else(||"NULL".into())>{cell.clone().unwrap_or_else(||"NULL".into())}</td>}).collect_view()}</tr>}).collect_view()}</tbody></table></div>
                        <Show when=move ||ui.result_is_table.get()><Show when=move ||!ui.page_ready()><p class="text-rambutan-500">"Filters changed. Choose Browse rows to start from the first page."</p></Show><div class="flex gap-2"><button type="button" class=BUTTON disabled=move ||ui.offset.get()==0||ui.busy.get()||!ui.page_ready() on:click=move |_|{if let Some(offset)=ui.previous_offsets.try_update(|offsets|offsets.pop()).flatten(){ui.offset.set(offset);}ui.browse();}>"Previous rows"</button><button type="button" class=BUTTON disabled=move ||result.next_offset.is_none()||ui.busy.get()||!ui.page_ready() on:click=move |_|{if let Some(offset)=ui.result.get_untracked().and_then(|result|result.next_offset){ui.previous_offsets.update(|offsets|offsets.push(ui.offset.get_untracked()));ui.offset.set(offset);}ui.browse();}>"Next rows"</button></div></Show>
                    </div>}
                })}
                {move ||ui.detail.get().and_then(|index|ui.result.get().and_then(|result|result.rows.get(index).cloned().map(|row|(result.columns,row)))).map(|(columns,row)|view!{<div class="space-y-2 rounded-grape border border-longan-700 p-3" aria-label="Row detail"><div class="flex justify-between"><h3>"Row detail"</h3><button type="button" class=BUTTON on:click=move |_|ui.detail.set(None)>"Close detail"</button></div>{columns.into_iter().zip(row).map(|(column,value)|view!{<div><div class="text-rambutan-500">{column.name}</div><pre class="whitespace-pre-wrap break-all text-rambutan-200">{value.unwrap_or_else(||"NULL".into())}</pre></div>}).collect_view()}</div>})}
            </div>
        </section>
    }
}
