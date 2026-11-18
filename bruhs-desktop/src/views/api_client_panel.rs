//! Native controls for the shared project API host.
use super::web_button::{web_button, ButtonState, WebButtonSize, WebButtonVariant};
use crate::{
    app_state::{spawn_tokio, DesktopState},
    server_client::{ServerClient, ServerError},
    theme,
};
use bruhs_models::{api_client::*, num::ToFloatLossy};
use bruhs_ui::press::on_press_view;
use gpui::{prelude::*, *};
use gpui_component::zoom::px;
use gpui_component::{
    input::{Input, InputState},
    scroll::ScrollableElement,
};
use std::{collections::BTreeSet, future::Future, sync::Arc};
use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct ApiAttachment {
    pub project_path: String,
    pub text: String,
}
pub(crate) struct ApiClientPanel {
    pub project_path: String,
    client: ServerClient,
    runtime: Arc<tokio::runtime::Runtime>,
    snapshot: Option<ApiCollectionSnapshot>,
    id: Uuid,
    name: Entity<InputState>,
    method: Entity<InputState>,
    url: Entity<InputState>,
    headers: Entity<InputState>,
    body: Entity<InputState>,
    values: Entity<InputState>,
    secrets: Entity<InputState>,
    curl: Entity<InputState>,
    credential_name: Entity<InputState>,
    credential_value: Entity<InputState>,
    response: Option<ApiResponse>,
    response_tab: usize,
    expanded: BTreeSet<String>,
    proposals: Vec<ApiProposal>,
    reviewing: Option<Uuid>,
    error: Option<String>,
    notice: Option<String>,
    busy: bool,
    visible: bool,
    generation: u64,
    active: Option<Uuid>,
    request: Option<Task<()>>,
}
impl EventEmitter<ApiAttachment> for ApiClientPanel {}
impl Drop for ApiClientPanel {
    fn drop(&mut self) {
        if let Some(id) = self.active.take() {
            let client = self.client.clone();
            self.runtime.spawn(async move {
                let _ = client.cancel_api_request(id).await;
            });
        }
    }
}
fn input(
    value: &str,
    multiline: bool,
    secret: bool,
    window: &mut Window,
    cx: &mut App,
) -> Entity<InputState> {
    cx.new(|cx| {
        let mut state = InputState::new(window, cx).multi_line(multiline);
        if secret {
            state = state.masked(true);
        }
        state.set_value(value, window, cx);
        state
    })
}
fn value(input: &Entity<InputState>, cx: &App) -> String {
    input.read(cx).value().to_string()
}
fn set(
    input: &Entity<InputState>,
    value: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
) {
    input.update(cx, |input, cx| input.set_value(value, window, cx));
}
fn text(value: impl Into<SharedString>) -> Div {
    div()
        .min_w_0()
        .text_size(px(12.0))
        .line_height(relative(1.5))
        .text_color(theme::rambutan_400())
        .child(value.into())
}
fn field(label: &'static str, input: &Entity<InputState>, multiline: bool, disabled: bool) -> Div {
    div()
        .w_full()
        .min_w_0()
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
fn control(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    disabled: bool,
) -> Stateful<Div> {
    let label = label.into();
    let selector = label.clone();
    web_button(
        id,
        WebButtonVariant::Outline,
        WebButtonSize::Small,
        ButtonState::disabled_if(disabled),
    )
    .aria_label(label.clone())
    .debug_selector(move || format!("api-client-{selector}"))
    .child(label)
}
trait Action {
    fn activate(
        self,
        cx: &Context<'_, ApiClientPanel>,
        f: impl Fn(&mut ApiClientPanel, &mut Window, &mut Context<'_, ApiClientPanel>) + 'static,
    ) -> Self;
}
impl Action for Stateful<Div> {
    fn activate(
        self,
        cx: &Context<'_, ApiClientPanel>,
        f: impl Fn(&mut ApiClientPanel, &mut Window, &mut Context<'_, ApiClientPanel>) + 'static,
    ) -> Self {
        on_press_view(self, cx, f)
    }
}
impl ApiClientPanel {
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
        let request = ApiRequest::default();
        let runtime = Arc::clone(&cx.global::<DesktopState>().runtime);
        let mut panel = Self {
            project_path,
            client,
            runtime,
            snapshot: None,
            id: request.id,
            name: input(&request.name, false, false, window, cx),
            method: input(&request.method, false, false, window, cx),
            url: input(&request.url, false, false, window, cx),
            headers: input("", true, false, window, cx),
            body: input("", true, false, window, cx),
            values: input("", true, false, window, cx),
            secrets: input("", true, false, window, cx),
            curl: input("", true, false, window, cx),
            credential_name: input("", false, false, window, cx),
            credential_value: input("", false, true, window, cx),
            response: None,
            response_tab: 0,
            expanded: BTreeSet::from([String::new()]),
            proposals: Vec::new(),
            reviewing: None,
            error: None,
            notice: None,
            busy: false,
            visible: true,
            generation: 0,
            active: None,
            request: None,
        };
        panel.reload(window, cx);
        panel
    }
    fn perform<T: Send + 'static>(
        &mut self,
        operation: impl Future<Output = Result<T, ServerError>> + Send + 'static,
        apply: impl FnOnce(&mut Self, T, &mut Window, &mut Context<'_, Self>) + 'static,
        window: &mut Window,
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
        self.request = Some(cx.spawn_in(window, async move |this, cx| {
            let result = operation.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation {
                    return;
                }
                this.busy = false;
                this.active = None;
                match result {
                    Ok(result) => apply(this, result, window, cx),
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn choose(&mut self, request: ApiRequest, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        self.id = request.id;
        set(&self.name, request.name, window, cx);
        set(&self.method, request.method, window, cx);
        set(&self.url, request.url, window, cx);
        set(&self.headers, format_headers(&request.headers), window, cx);
        set(&self.body, request.body, window, cx);
        cx.notify();
    }
    fn draft(&self, cx: &App) -> Result<(ApiRequest, ApiEnvironment), String> {
        Ok((
            ApiRequest {
                id: self.id,
                name: value(&self.name, cx),
                method: value(&self.method, cx).trim().to_ascii_uppercase(),
                url: value(&self.url, cx),
                headers: parse_headers(&value(&self.headers, cx))?,
                body: value(&self.body, cx),
            },
            ApiEnvironment {
                values: parse_values(&value(&self.values, cx))?,
                secrets: parse_values(&value(&self.secrets, cx))?,
            },
        ))
    }
    fn reload(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let client = self.client.clone();
        let project = self.project_path.clone();
        self.perform(
            async move {
                tokio::try_join!(
                    client.api_collection(&project),
                    client.api_proposals(&project)
                )
            },
            |this, (snapshot, proposals), window, cx| {
                set(
                    &this.values,
                    format_values(&snapshot.collection.environment.values),
                    window,
                    cx,
                );
                set(
                    &this.secrets,
                    format_values(&snapshot.collection.environment.secrets),
                    window,
                    cx,
                );
                this.choose(
                    snapshot
                        .collection
                        .requests
                        .first()
                        .cloned()
                        .unwrap_or_default(),
                    window,
                    cx,
                );
                this.snapshot = Some(snapshot);
                this.proposals = proposals;
                this.notice = Some(
                    "Collection loaded. Ordinary environment values are plain repository content."
                        .into(),
                );
            },
            window,
            cx,
        );
    }
    fn refresh_proposals(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let client = self.client.clone();
        let project = self.project_path.clone();
        self.perform(
            async move { client.api_proposals(&project).await },
            |this, proposals, _, _| this.proposals = proposals,
            window,
            cx,
        );
    }
    fn save(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        let Some(snapshot) = self.snapshot.clone() else {
            self.error = Some(
                "Load the collection before saving; repair existing file errors first.".into(),
            );
            cx.notify();
            return;
        };
        let (request, environment) = match self.draft(cx) {
            Ok(draft) => draft,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        let mut collection = snapshot.collection;
        collection.requests.retain(|item| item.id != request.id);
        collection.requests.push(request);
        collection.environment = environment;
        let input = SaveApiCollection {
            project_path: self.project_path.clone(),
            expected_revision: snapshot.revision,
            collection,
        };
        let client = self.client.clone();
        self.perform(
            async move { client.save_api_collection(&input).await },
            |this, snapshot, _, _| {
                this.snapshot = Some(snapshot);
                this.notice =
                    Some("Saved .bruhs/api/collection.json. Review it in the project diff.".into());
            },
            window,
            cx,
        );
    }
    fn send(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        let (request, environment) = match self.draft(cx) {
            Ok(draft) => draft,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        let id = Uuid::new_v4();
        let execution = ApiExecution {
            project_path: self.project_path.clone(),
            request_id: id,
            request,
            environment,
        };
        let client = self.client.clone();
        self.response = None;
        self.perform(
            async move { client.send_api_request(&execution).await },
            |this, response, _, _| {
                this.response = Some(response);
                this.expanded = BTreeSet::from([String::new()]);
            },
            window,
            cx,
        );
        self.active = Some(id);
    }
    fn cancel(&mut self, cx: &mut Context<'_, Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.busy = false;
        if let Some(id) = self.active.take() {
            let client = self.client.clone();
            spawn_tokio(cx, async move {
                let _ = client.cancel_api_request(id).await;
            })
            .detach();
        }
        self.notice=Some("Request cancelled locally. A remote server may already have applied it; inspect its state before retrying.".into());
        cx.notify();
    }
    pub(super) fn set_visible(
        &mut self,
        visible: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        if !visible {
            if self.active.is_some() {
                self.cancel(cx);
            }
            set(&self.credential_value, "", window, cx);
        }
    }
    fn save_credential(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        let input = ApiCredentialInput {
            project_path: self.project_path.clone(),
            name: value(&self.credential_name, cx),
            value: value(&self.credential_value, cx),
        };
        let client = self.client.clone();
        self.perform(async move{client.save_api_credential(&input).await},|this,(),window,cx|{set(&this.credential_value,"",window,cx);this.notice=Some("Credential saved in the native keychain. Its value was cleared; add its name under Keychain references.".into());},window,cx);
    }
    fn decide(&mut self, id: Uuid, approve: bool, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        let decision = ApiProposalDecision {
            project_path: self.project_path.clone(),
            id,
            approve,
        };
        let client = self.client.clone();
        self.perform(
            async move { client.decide_api_proposal(&decision).await },
            |this, proposal, _, _| {
                match &proposal.state {
                    ApiProposalState::Finished { response } => {
                        this.response = Some(response.clone());
                        this.expanded = BTreeSet::from([String::new()]);
                        this.notice =
                            Some("Approved response is available to the proposing turn.".into());
                    }
                    ApiProposalState::Failed { error } => this.error = Some(error.clone()),
                    ApiProposalState::Declined => {
                        this.notice = Some("Request declined. Nothing was sent.".into())
                    }
                    _ => {}
                }
                if let Some(old) = this.proposals.iter_mut().find(|old| old.id == proposal.id) {
                    *old = proposal;
                }
            },
            window,
            cx,
        );
        if approve {
            self.active = Some(id);
        }
    }
    fn import(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy {
            return;
        }
        match import_curl(&value(&self.curl, cx)) {
            Ok(request) => {
                self.choose(request, window, cx);
                set(&self.curl, "", window, cx);
                self.error = None;
                self.notice=Some("Curl imported into this draft. Review variables and keychain references before sending or saving.".into());
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }
    fn attach(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(response) = &self.response {
            match response_attachment(response) {
                Ok(text) => {
                    cx.emit(ApiAttachment {
                        project_path: self.project_path.clone(),
                        text,
                    });
                    self.notice = Some(
                        "Response added to the current unsent draft. Review and send when ready."
                            .into(),
                    );
                }
                Err(error) => self.error = Some(error),
            }
        }
        cx.notify();
    }
    fn render_response(&self, cx: &Context<'_, Self>) -> Option<Div> {
        let response = self.response.as_ref()?;
        let mut section = div()
            .min_w_0()
            .w_full()
            .flex()
            .flex_col()
            .gap_2()
            .pt_3()
            .border_t_1()
            .border_color(theme::longan_700())
            .child(text(format!(
                "HTTP {} · {} ms · {} bytes{}",
                response.status,
                response.elapsed_ms,
                response.size_bytes,
                if response.redacted {
                    " · credentials redacted"
                } else {
                    ""
                }
            )))
            .child(text(
                "Response from the last send. HTML is displayed as text.",
            ));
        let mut tabs = div().flex().flex_wrap().gap_2();
        for (index, label) in [(0, "JSON tree"), (1, "Body"), (2, "Headers")] {
            tabs = tabs.child(
                control(("api-response-tab", index), label, false)
                    .role(Role::Tab)
                    .aria_selected(self.response_tab == index)
                    .activate(cx, move |this, _, cx| {
                        this.response_tab = index;
                        cx.notify();
                    }),
            );
        }
        section = section.child(tabs);
        section = section
            .child(match self.response_tab {
                1 => text(response.body.clone()).p_2().into_any_element(),
                2 => text(format_headers(&response.headers))
                    .p_2()
                    .into_any_element(),
                _ => match json_tree_lines(&response.body, &self.expanded) {
                    Err(error) => text(error).into_any_element(),
                    Ok(lines) => {
                        let lines = Arc::new(lines);
                        div()
                            .h(px(240.0))
                            .child(
                                uniform_list(
                                    "api-json-tree",
                                    lines.len(),
                                    cx.processor(
                                        move |_this, range: std::ops::Range<usize>, _, cx| {
                                            range
                                                .filter_map(|index| {
                                                    let line = lines.get(index)?;
                                                    let pointer = line.pointer.clone();
                                                    let expandable = line.expandable;
                                                    let marker = if expandable {
                                                        if line.expanded {
                                                            "▾ "
                                                        } else {
                                                            "▸ "
                                                        }
                                                    } else {
                                                        "  "
                                                    };
                                                    Some(
                                                        control(
                                                            ("api-json-line", index),
                                                            format!("{marker}{}", line.text),
                                                            !expandable,
                                                        )
                                                        .pl(px((line.depth.min(20) * 12)
                                                            .to_f32_lossy()))
                                                        .h(px(28.0))
                                                        .w_full()
                                                        .justify_start()
                                                        .activate(cx, move |this, _, cx| {
                                                            if expandable {
                                                                if !this.expanded.remove(&pointer) {
                                                                    this.expanded
                                                                        .insert(pointer.clone());
                                                                }
                                                                cx.notify();
                                                            }
                                                        })
                                                        .into_any_element(),
                                                    )
                                                })
                                                .collect()
                                        },
                                    ),
                                )
                                .size_full(),
                            )
                            .into_any_element()
                    }
                },
            })
            .child(
                control("api-attach", "Add response to draft", false)
                    .activate(cx, |this, _, cx| this.attach(cx)),
            );
        Some(section)
    }
    fn render_approvals(&self, cx: &Context<'_, Self>) -> Div {
        let mut section=div().min_w_0().w_full().flex().flex_col().gap_2().pt_3().border_t_1().border_color(theme::longan_700()).child(div().flex().flex_wrap().gap_2().child(text("Agent requests").flex_1()).child(control("api-refresh-approvals","Refresh approvals",self.busy).activate(cx,|this,window,cx|this.refresh_proposals(window,cx)))).child(text("Send once releases the reviewed response to its proposing turn. Approval cannot be reused."));
        if self.proposals.is_empty() {
            return section.child(text("No proposed requests."));
        }
        for proposal in &self.proposals {
            let id = proposal.id;
            section = section.child(
                control(
                    ElementId::Name(format!("api-proposal-{id}").into()),
                    format!("Review: {}", proposal.request.name),
                    self.busy,
                )
                .aria_selected(self.reviewing == Some(id))
                .activate(cx, move |this, _, cx| {
                    if !this.busy {
                        this.reviewing = Some(id);
                        cx.notify();
                    }
                }),
            );
        }
        if let Some(proposal) = self
            .proposals
            .iter()
            .find(|proposal| Some(proposal.id) == self.reviewing)
        {
            let id = proposal.id;
            let pending = matches!(proposal.state, ApiProposalState::Pending);
            let status = match &proposal.state {
                ApiProposalState::Pending => "Awaiting approval".to_string(),
                ApiProposalState::Running => "Sending".into(),
                ApiProposalState::Finished { response } => {
                    format!("Completed: HTTP {}", response.status)
                }
                ApiProposalState::Failed { error } => format!("Failed: {error}"),
                ApiProposalState::Declined => "Declined".into(),
            };
            section = section
                .child(text(status))
                .child(text(api_approval_review(proposal)).p_2());
            if pending {
                section = section.child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            control("api-approve", "Send once", self.busy)
                                .activate(cx, move |this, window, cx| {
                                    this.decide(id, true, window, cx)
                                }),
                        )
                        .child(
                            control("api-decline", "Decline", self.busy)
                                .activate(cx, move |this, window, cx| {
                                    this.decide(id, false, window, cx)
                                }),
                        ),
                );
            }
        }
        section
    }
}
impl Render for ApiClientPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let mut panel=div().id("native-api-client-panel").size_full().overflow_y_scrollbar().p_3().flex().flex_col().gap_3().child(div().flex().flex_wrap().gap_2().child(text("API client").flex_1().text_color(theme::rambutan_100())).child(control("api-reload","Reload collection",self.busy).activate(cx,|this,window,cx|this.reload(window,cx)))).child(text("Send from this machine. Responses stay here until attached. Agent requests require approval below."));
        if let Some(error) = &self.error {
            panel = panel.child(
                text(error.clone())
                    .id("api-error")
                    .role(Role::Alert)
                    .text_color(theme::persimmon_300()),
            );
        }
        if let Some(notice) = &self.notice {
            panel = panel.child(text(notice.clone()).id("api-notice").role(Role::Status));
        }
        let mut saved = div().flex().flex_wrap().gap_2().child(
            control("api-new", "New", self.busy).activate(cx, |this, window, cx| {
                this.choose(ApiRequest::default(), window, cx)
            }),
        );
        if let Some(snapshot) = &self.snapshot {
            for request in &snapshot.collection.requests {
                let request = request.clone();
                saved = saved.child(
                    control(
                        ElementId::Name(format!("api-saved-{}", request.id).into()),
                        request.name.clone(),
                        self.busy,
                    )
                    .aria_selected(self.id == request.id)
                    .activate(cx, move |this, window, cx| {
                        this.choose(request.clone(), window, cx)
                    }),
                );
            }
        }
        panel=panel.child(saved).child(field("Request name",&self.name,false,self.busy)).child(field("Method",&self.method,false,self.busy)).child(field("Request URL",&self.url,false,self.busy)).child(field("Headers (Name: value)",&self.headers,true,self.busy)).child(field("Body (raw or GraphQL JSON)",&self.body,true,self.busy)).child(field("Variables (name=value)",&self.values,true,self.busy)).child(field("Keychain references (variable=credential name)",&self.secrets,true,self.busy)).child(text("Use {{name}}. Keychain variables work in headers/body only. Ordinary values are plain repository text."))
        .child(div().flex().flex_wrap().gap_2().child(control("api-send","Send request",self.busy).activate(cx,|this,window,cx|this.send(window,cx))).child(control("api-save","Save request",self.busy).activate(cx,|this,window,cx|this.save(window,cx))));
        if self.busy {
            panel = panel.child(text("Working…").id("api-busy").role(Role::Status));
        }
        if self.active.is_some() {
            panel = panel.child(
                control("api-cancel", "Cancel request", false)
                    .activate(cx, |this, _, cx| this.cancel(cx)),
            );
        }
        panel = panel
            .child(field("Curl command", &self.curl, true, self.busy))
            .child(
                control("api-import", "Import into draft", self.busy)
                    .activate(cx, |this, window, cx| this.import(window, cx)),
            )
            .child(field(
                "New credential name",
                &self.credential_name,
                false,
                self.busy,
            ))
            .child(field(
                "New credential value",
                &self.credential_value,
                false,
                self.busy,
            ))
            .child(
                control("api-save-credential", "Save credential", self.busy)
                    .activate(cx, |this, window, cx| this.save_credential(window, cx)),
            );
        if let Some(response) = self.render_response(cx) {
            panel = panel.child(response);
        }
        panel.child(self.render_approvals(cx))
    }
}
#[cfg(test)]
mod tests;
