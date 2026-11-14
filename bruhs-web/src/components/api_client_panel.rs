//! Project request drafts and one-use approvals. Responses are inert text.
use crate::{
    api,
    state::{ActiveThreadId, ApiClientDraft, ApiClientSession, ComposerDraft, ThreadsSignal},
};
use bruhs_models::api_client::*;
use leptos::prelude::*;
use std::collections::BTreeSet;
use uuid::Uuid;
const CONTROL:&str="min-h-8 w-full min-w-0 rounded-grape border border-longan-700 bg-longan-950 px-2 py-1 text-[16px] text-rambutan-200 outline-none focus:border-persimmon-400 max-md:min-h-11 md:text-[13px]";
const BUTTON:&str="min-h-8 rounded-grape border border-longan-700 px-2 py-1 text-label-sm text-rambutan-300 hover:bg-longan-800 focus-visible:outline focus-visible:outline-persimmon-400 disabled:opacity-40 max-md:min-h-11";
#[derive(Clone, Copy)]
struct Ui {
    project: Memo<Option<String>>,
    scope: StoredValue<Option<String>>,
    snapshot: RwSignal<Option<ApiCollectionSnapshot>>,
    request_id: RwSignal<Uuid>,
    name: RwSignal<String>,
    method: RwSignal<String>,
    url: RwSignal<String>,
    headers: RwSignal<String>,
    body: RwSignal<String>,
    values: RwSignal<String>,
    secrets: RwSignal<String>,
    curl: RwSignal<String>,
    response: RwSignal<Option<ApiResponse>>,
    proposals: RwSignal<Vec<ApiProposal>>,
    expanded: RwSignal<BTreeSet<String>>,
    response_tab: RwSignal<&'static str>,
    error: RwSignal<Option<String>>,
    notice: RwSignal<Option<String>>,
    busy: RwSignal<bool>,
    active: RwSignal<Option<Uuid>>,
    generation: RwSignal<u64>,
    alive: RwSignal<bool>,
}
impl Ui {
    fn remember(self, session: StoredValue<Option<ApiClientDraft>>) {
        let Some(project_path) = self.scope.try_get_value().flatten() else {
            return;
        };
        session.try_set_value(Some(ApiClientDraft {
            project_path,
            snapshot: self.snapshot.get_untracked(),
            id: self.request_id.get_untracked(),
            name: self.name.get_untracked(),
            method: self.method.get_untracked(),
            url: self.url.get_untracked(),
            headers: self.headers.get_untracked(),
            body: self.body.get_untracked(),
            values: self.values.get_untracked(),
            secrets: self.secrets.get_untracked(),
            curl: self.curl.get_untracked(),
            response: self.response.get_untracked(),
            response_tab: self.response_tab.get_untracked(),
            expanded: self.expanded.get_untracked(),
            error: self.error.get_untracked(),
            notice: self.notice.get_untracked(),
        }));
    }
    fn current_project(self) -> Option<String> {
        let project = self.scope.get_value()?;
        if self.project.get_untracked().as_deref() != Some(project.as_str()) {
            self.error.set(Some("The working folder changed. Reopen API before sending, saving or attaching this draft.".into()));
            return None;
        }
        Some(project)
    }
    fn restore(self, draft: ApiClientDraft) {
        self.snapshot.set(draft.snapshot);
        self.request_id.set(draft.id);
        self.name.set(draft.name);
        self.method.set(draft.method);
        self.url.set(draft.url);
        self.headers.set(draft.headers);
        self.body.set(draft.body);
        self.values.set(draft.values);
        self.secrets.set(draft.secrets);
        self.curl.set(draft.curl);
        self.response.set(draft.response);
        self.response_tab.set(draft.response_tab);
        self.expanded.set(draft.expanded);
        self.error.set(draft.error);
        self.notice.set(draft.notice);
    }
    fn begin(self) -> Option<u64> {
        if self.busy.get_untracked() {
            return None;
        }
        self.busy.set(true);
        self.error.set(None);
        self.notice.set(None);
        self.generation.update(|n| *n = n.wrapping_add(1));
        Some(self.generation.get_untracked())
    }
    fn finish(self, generation: u64) -> bool {
        if self.alive.try_get_untracked() != Some(true)
            || self.generation.try_get_untracked() != Some(generation)
        {
            return false;
        }
        self.busy.set(false);
        self.active.set(None);
        true
    }
    fn choose(self, request: ApiRequest) {
        self.request_id.set(request.id);
        self.name.set(request.name);
        self.method.set(request.method);
        self.url.set(request.url);
        self.headers.set(format_headers(&request.headers));
        self.body.set(request.body);
    }
    fn draft(self) -> Result<(ApiRequest, ApiEnvironment), String> {
        Ok((
            ApiRequest {
                id: self.request_id.get_untracked(),
                name: self.name.get_untracked(),
                method: self.method.get_untracked(),
                url: self.url.get_untracked(),
                headers: parse_headers(&self.headers.get_untracked())?,
                body: self.body.get_untracked(),
            },
            ApiEnvironment {
                values: parse_values(&self.values.get_untracked())?,
                secrets: parse_values(&self.secrets.get_untracked())?,
            },
        ))
    }
    fn reload(self) {
        let Some(project) = self.current_project() else {
            return;
        };
        let Some(generation) = self.begin() else {
            return;
        };
        leptos::task::spawn_local(async move {
            let result = api::api_collection(&project).await;
            if !self.finish(generation) {
                return;
            }
            match result {
                Ok(snapshot) => {
                    self.values
                        .set(format_values(&snapshot.collection.environment.values));
                    self.secrets
                        .set(format_values(&snapshot.collection.environment.secrets));
                    self.choose(
                        snapshot
                            .collection
                            .requests
                            .first()
                            .cloned()
                            .unwrap_or_default(),
                    );
                    self.snapshot.set(Some(snapshot));
                    self.notice.set(Some("Collection loaded. Ordinary environment values are plain repository content.".into()));
                }
                Err(error) => self.error.set(Some(error.to_string())),
            }
            self.refresh_proposals();
        });
    }
    fn refresh_proposals(self) {
        let Some(project) = self.current_project() else {
            return;
        };
        let generation = self.generation.get_untracked();
        leptos::task::spawn_local(async move {
            let result = api::api_proposals(&project).await;
            if self.alive.try_get_untracked() != Some(true)
                || self.generation.try_get_untracked() != Some(generation)
            {
                return;
            }
            match result {
                Ok(proposals) => self.proposals.set(proposals),
                Err(error) => self.error.set(Some(error.to_string())),
            }
        });
    }
    fn save(self) {
        let Some(project_path) = self.current_project() else {
            return;
        };
        let Some(snapshot) = self.snapshot.get_untracked() else {
            self.error.set(Some(
                "Load the collection before saving; repair existing file errors first.".into(),
            ));
            return;
        };
        let (request, environment) = match self.draft() {
            Ok(draft) => draft,
            Err(error) => {
                self.error.set(Some(error));
                return;
            }
        };
        let Some(generation) = self.begin() else {
            return;
        };
        let mut collection = snapshot.collection;
        collection.requests.retain(|item| item.id != request.id);
        collection.requests.push(request);
        collection.environment = environment;
        leptos::task::spawn_local(async move {
            let result = api::save_api_collection(&SaveApiCollection {
                project_path,
                expected_revision: snapshot.revision,
                collection,
            })
            .await;
            if !self.finish(generation) {
                return;
            }
            match result {
                Ok(snapshot) => {
                    self.snapshot.set(Some(snapshot));
                    self.notice.set(Some(
                        "Saved .bruhs/api/collection.json. Review it in the project diff.".into(),
                    ));
                }
                Err(error) => self.error.set(Some(error.to_string())),
            }
        });
    }
    fn send(self) {
        let Some(project_path) = self.current_project() else {
            return;
        };
        let (request, environment) = match self.draft() {
            Ok(draft) => draft,
            Err(error) => {
                self.error.set(Some(error));
                return;
            }
        };
        let Some(generation) = self.begin() else {
            return;
        };
        let request_id = Uuid::new_v4();
        self.active.set(Some(request_id));
        self.response.set(None);
        leptos::task::spawn_local(async move {
            let result = api::send_api_request(&ApiExecution {
                project_path,
                request_id,
                request,
                environment,
            })
            .await;
            if !self.finish(generation) {
                return;
            }
            match result {
                Ok(response) => {
                    self.response.set(Some(response));
                    self.expanded.set(BTreeSet::from([String::new()]));
                }
                Err(error) => self.error.set(Some(error.to_string())),
            }
        });
    }
    fn decide(self, id: Uuid, approve: bool) {
        let Some(project_path) = self.current_project() else {
            return;
        };
        let Some(generation) = self.begin() else {
            return;
        };
        if approve {
            self.active.set(Some(id));
            self.response.set(None);
        }
        leptos::task::spawn_local(async move {
            let result = api::decide_api_proposal(&ApiProposalDecision {
                project_path,
                id,
                approve,
            })
            .await;
            if !self.finish(generation) {
                return;
            }
            match result {
                Ok(proposal) => {
                    match &proposal.state {
                        ApiProposalState::Finished { response } => {
                            self.response.set(Some(response.clone()));
                            self.expanded.set(BTreeSet::from([String::new()]));
                            self.notice.set(Some(
                                "Approved response is available to the proposing turn.".into(),
                            ));
                        }
                        ApiProposalState::Failed { error } => self.error.set(Some(error.clone())),
                        ApiProposalState::Declined => self
                            .notice
                            .set(Some("Request declined. Nothing was sent.".into())),
                        _ => {}
                    }
                    self.proposals.update(|list| {
                        if let Some(old) = list.iter_mut().find(|old| old.id == id) {
                            *old = proposal;
                        }
                    });
                }
                Err(error) => self.error.set(Some(error.to_string())),
            }
        });
    }
    fn cancel(self) {
        let id = self.active.get_untracked();
        self.generation.update(|n| *n = n.wrapping_add(1));
        self.busy.set(false);
        self.active.set(None);
        if let Some(id) = id {
            leptos::task::spawn_local(async move {
                if let Err(error) = api::cancel_api_request(id).await {
                    if self.alive.try_get_untracked() == Some(true) {
                        self.error.set(Some(error.to_string()));
                    }
                }
            });
        }
    }
}
#[component]
pub fn ApiClientPanel() -> impl IntoView {
    let session = expect_context::<ApiClientSession>().0;
    let threads = expect_context::<ThreadsSignal>().0;
    let active = expect_context::<ActiveThreadId>().0;
    let project = Memo::new(move |_| {
        threads.track();
        active.track();
        super::files_panel::project_path()
    });
    let request = ApiRequest::default();
    let ui = Ui {
        project,
        scope: StoredValue::new(None),
        snapshot: RwSignal::new(None),
        request_id: RwSignal::new(request.id),
        name: RwSignal::new(request.name),
        method: RwSignal::new(request.method),
        url: RwSignal::new(request.url),
        headers: RwSignal::new(String::new()),
        body: RwSignal::new(String::new()),
        values: RwSignal::new(String::new()),
        secrets: RwSignal::new(String::new()),
        curl: RwSignal::new(String::new()),
        response: RwSignal::new(None),
        proposals: RwSignal::new(Vec::new()),
        expanded: RwSignal::new(BTreeSet::from([String::new()])),
        response_tab: RwSignal::new("json"),
        error: RwSignal::new(None),
        notice: RwSignal::new(None),
        busy: RwSignal::new(false),
        active: RwSignal::new(None),
        generation: RwSignal::new(0),
        alive: RwSignal::new(true),
    };
    Effect::new(move |_| {
        let current = project.get();
        ui.remember(session);
        ui.cancel();
        ui.scope.set_value(current.clone());
        if let Some(draft) = session
            .get_value()
            .filter(|draft| Some(&draft.project_path) == current.as_ref())
        {
            ui.restore(draft);
            if ui.snapshot.get_untracked().is_none() && ui.error.get_untracked().is_none() {
                ui.reload();
                return;
            }
            ui.refresh_proposals();
            return;
        }
        ui.snapshot.set(None);
        ui.proposals.set(Vec::new());
        ui.response.set(None);
        ui.choose(ApiRequest::default());
        ui.values.set(String::new());
        ui.secrets.set(String::new());
        ui.reload();
    });
    on_cleanup(move || {
        if ui.active.get_untracked().is_some() {
            ui.notice.set(Some("Request cancelled when the panel closed. A remote server may already have applied it; inspect its state before retrying.".into()));
        }
        ui.cancel();
        ui.remember(session);
        ui.alive.set(false);
    });
    view! {<section data-api-client-panel class="flex h-full min-h-0 flex-col text-label-sm" aria-label="API client"><div class="min-h-0 flex-1 space-y-3 overflow-y-auto p-3">
        <div class="flex flex-wrap items-center gap-2"><h2 class="min-w-0 flex-1 text-rambutan-200">"API client"</h2><button type="button" class=BUTTON disabled=move||ui.busy.get() on:click=move |_|ui.reload()>"Reload collection"</button></div>
        <p class="text-rambutan-500">"Send from this machine. Responses stay here until attached. Agent requests require approval below."</p>
        {move||project.get().is_none().then(||view!{<p>"Open a project to use its API collection."</p>})}
        {move||ui.error.get().map(|error|view!{<p role="alert" class="break-words text-persimmon-300">{error}</p>})}
        {move||ui.notice.get().map(|notice|view!{<p role="status" class="break-words text-rambutan-300">{notice}</p>})}
        <fieldset class="min-w-0 space-y-3" disabled=move||ui.busy.get()||project.get().is_none()>
            <div class="flex min-w-0 gap-2"><label class="min-w-0 flex-1">"Saved request"<select class=CONTROL aria-label="Saved request" prop:value=move||ui.request_id.get().to_string() on:change=move|event|{let id=event_target_value(&event);if let Some(request)=ui.snapshot.get_untracked().and_then(|snapshot|snapshot.collection.requests.into_iter().find(|request|request.id.to_string()==id)){ui.choose(request);}}><option value="">"Choose a saved request"</option>{move||ui.snapshot.get().map(|snapshot|snapshot.collection.requests.into_iter().map(|request|view!{<option value=request.id.to_string()>{request.name}</option>}).collect_view())}</select></label><button type="button" class=BUTTON on:click=move |_|ui.choose(ApiRequest::default())>"New"</button></div>
            <ApiField label="Request name" value=ui.name />
            <div class="flex min-w-0 items-end gap-2"><label class="w-24 shrink-0">"Method"<select class=CONTROL aria-label="Method" prop:value=move||ui.method.get() on:change=move|event|ui.method.set(event_target_value(&event))>{["GET","POST","PUT","PATCH","DELETE","HEAD","OPTIONS"].into_iter().map(|method|view!{<option value=method>{method}</option>}).collect_view()}</select></label><div class="min-w-0 flex-1"><ApiField label="Request URL" value=ui.url /></div></div>
            <ApiField label="Headers (Name: value)" value=ui.headers multiline=true />
            <ApiField label="Body (raw or GraphQL JSON)" value=ui.body multiline=true />
            <details><summary class="cursor-pointer py-2 text-rambutan-300">"Environment and keychain"</summary><div class="space-y-3 py-2"><ApiField label="Variables (name=value)" value=ui.values multiline=true /><ApiField label="Keychain references (variable=credential name)" value=ui.secrets multiline=true /><p class="text-rambutan-500">"Use {{name}} in the URL, headers or body. Keychain variables work in headers/body only. Save a new secret below; Settings → Credentials manages saved entries. Ordinary values are saved as plain repository text."</p><Credential ui=ui /></div></details>
            <div class="flex flex-wrap gap-2"><button type="button" class=BUTTON on:click=move |_|ui.send()>"Send request"</button><button type="button" class=BUTTON on:click=move |_|ui.save()>"Save request"</button></div>
            <details><summary class="cursor-pointer py-2 text-rambutan-300">"Import curl"</summary><ApiField label="Curl command" value=ui.curl multiline=true /><button type="button" class=BUTTON on:click=move |_|match import_curl(&ui.curl.get_untracked()){Ok(request)=>{ui.choose(request);ui.curl.set(String::new());ui.notice.set(Some("Curl imported into this draft. Review variables and keychain references before sending or saving.".into()));ui.error.set(None);},Err(error)=>ui.error.set(Some(error))}>"Import into draft"</button></details>
        </fieldset>
        <Show when=move||ui.busy.get()><p role="status">"Working…"</p></Show>
        <Show when=move||ui.active.get().is_some()><button type="button" class=BUTTON on:click=move |_|{ui.cancel();ui.notice.set(Some("Request cancelled locally. A remote server may already have applied it; inspect its state before retrying.".into()));}>"Cancel request"</button></Show>
        <Response ui=ui />
        <Approvals ui=ui />
    </div></section>}
}
#[component]
fn Response(ui: Ui) -> impl IntoView {
    let draft = expect_context::<ComposerDraft>().0;
    view! {<Show when=move||ui.response.get().is_some()><section aria-label="API response" class="min-w-0 space-y-2 border-t border-longan-700 pt-3">
        <h3 class="break-words text-rambutan-200">{move||ui.response.get().map(|response|format!("HTTP {} · {} ms · {} bytes{}",response.status,response.elapsed_ms,response.size_bytes,if response.redacted{" · credentials redacted"}else{""}))}</h3><p class="text-rambutan-500">"Response from the last send. HTML is displayed as text."</p>
        <div class="flex flex-wrap gap-2" role="tablist" aria-label="Response format">{["json","body","headers"].into_iter().map(move|tab|view!{<button type="button" role="tab" class=BUTTON aria-selected=move||(ui.response_tab.get()==tab).to_string() on:click=move |_|ui.response_tab.set(tab)>{match tab{"json"=>"JSON tree","body"=>"Body",_=>"Headers"}}</button>}).collect_view()}</div>
        {move||{let Some(response)=ui.response.get()else{return ().into_any();};match ui.response_tab.get(){
            "headers"=>view!{<pre class="max-h-80 overflow-auto whitespace-pre-wrap break-all rounded-grape border border-longan-700 p-2">{format_headers(&response.headers)}</pre>}.into_any(),
            "body"=>view!{<pre class="max-h-80 overflow-auto whitespace-pre-wrap break-all rounded-grape border border-longan-700 p-2">{response.body}</pre>}.into_any(),
            _=>match json_tree_lines(&response.body,&ui.expanded.get()){Err(error)=>view!{<p class="text-rambutan-500">{error}</p>}.into_any(),Ok(lines)=>view!{<div class="max-h-80 min-w-0 overflow-auto rounded-grape border border-longan-700 p-2" aria-label="JSON tree">{lines.into_iter().map(move|line|{let pointer=line.pointer;let padding=format!("padding-left:{}px",line.depth*12);let marker=if line.expandable{if line.expanded{"▾ "}else{"▸ "}}else{"  "};view!{<button type="button" class="block min-h-8 w-full whitespace-pre-wrap break-all text-left text-rambutan-300 max-md:min-h-11" style=padding aria-expanded=line.expandable.then(||line.expanded.to_string()) disabled=!line.expandable on:click=move |_|ui.expanded.update(|expanded|{if !expanded.remove(&pointer){expanded.insert(pointer.clone());}})>{format!("{marker}{}",line.text)}</button>}}).collect_view()}</div>}.into_any()}
        }}}
        <button type="button" class=BUTTON on:click=move |_|{if ui.current_project().is_none(){return;}
                if let Some(response)=ui.response.get_untracked(){match response_attachment(&response){Ok(text)=>{draft.update(|draft|{if !draft.is_empty(){draft.push_str("\n\n");}draft.push_str(&text);});ui.notice.set(Some("Response added to the current unsent draft. Review and send when ready.".into()));},Err(error)=>ui.error.set(Some(error))}}}>"Add response to draft"</button>
    </section></Show>}
}
#[component]
fn Approvals(ui: Ui) -> impl IntoView {
    view! {<section aria-label="Agent request approvals" class="min-w-0 space-y-2 border-t border-longan-700 pt-3"><div class="flex flex-wrap items-center gap-2"><h3 class="flex-1 text-rambutan-200">"Agent requests"</h3><button type="button" class=BUTTON disabled=move||ui.busy.get() on:click=move |_|ui.refresh_proposals()>"Refresh approvals"</button></div><p class="text-rambutan-500">"Review each request. Send once releases its response to that proposing turn. Approval cannot be reused."</p>
        {move||{let proposals=ui.proposals.get();if proposals.is_empty(){return view!{<p class="text-rambutan-500">"No proposed requests."</p>}.into_any();}proposals.into_iter().map(move|proposal|{
            let id=proposal.id;let pending=matches!(proposal.state,ApiProposalState::Pending);
            let review=api_approval_review(&proposal);
            let status=match proposal.state{ApiProposalState::Pending=>"Awaiting approval".to_string(),ApiProposalState::Running=>"Sending".into(),ApiProposalState::Finished{response}=>format!("Completed: HTTP {}",response.status),ApiProposalState::Failed{error}=>format!("Failed: {error}"),ApiProposalState::Declined=>"Declined".into()};
            view!{<article class="min-w-0 space-y-2 rounded-grape border border-longan-700 p-2"><h4 class="break-words text-rambutan-200">{proposal.request.name}</h4><p>{status}</p><pre class="max-h-80 overflow-auto whitespace-pre-wrap break-all" aria-label="Request approval details">{review}</pre>{pending.then(||view!{<div class="flex gap-2"><button type="button" class=BUTTON disabled=move||ui.busy.get() on:click=move |_|ui.decide(id,true)>"Send once"</button><button type="button" class=BUTTON disabled=move||ui.busy.get() on:click=move |_|ui.decide(id,false)>"Decline"</button></div>})}</article>}
        }).collect_view().into_any()}}
    </section>}
}
#[component]
fn ApiField(
    label: &'static str,
    value: RwSignal<String>,
    #[prop(default = false)] multiline: bool,
    #[prop(default = false)] secret: bool,
) -> impl IntoView {
    view! {<label class="flex min-w-0 flex-col gap-1 text-rambutan-400">{label}{if multiline{view!{<textarea class=CONTROL rows=3 aria-label=label prop:value=move||value.get() on:input=move|event|value.set(event_target_value(&event))/>}.into_any()}else{view!{<input class=CONTROL type=if secret{"password"}else{"text"} autocomplete=if secret{"new-password"}else{"off"} aria-label=label prop:value=move||value.get() on:input=move|event|value.set(event_target_value(&event))/>}.into_any()}}</label>}
}

#[component]
fn Credential(ui: Ui) -> impl IntoView {
    let name = RwSignal::new(String::new());
    let value = RwSignal::new(String::new());
    Effect::new(move |_| {
        ui.project.track();
        name.set(String::new());
        value.set(String::new());
    });
    on_cleanup(move || value.set(String::new()));
    view! {<div class="space-y-2 rounded-grape border border-longan-700 p-2"><ApiField label="New credential name" value=name /><ApiField label="New credential value" value=value secret=true /><button type="button" class=BUTTON on:click=move |_|{
        let Some(project_path)=ui.current_project()else{return;};let Some(generation)=ui.begin()else{return;};let input=ApiCredentialInput{project_path,name:name.get_untracked(),value:value.get_untracked()};
        leptos::task::spawn_local(async move{let result=api::save_api_credential(&input).await;if !ui.finish(generation){return;}match result{Ok(())=>{value.set(String::new());ui.notice.set(Some("Credential saved in the native keychain. Its value was cleared; add its name under Keychain references.".into()));},Err(error)=>ui.error.set(Some(error.to_string()))}});
    }>"Save credential"</button></div>}
}
