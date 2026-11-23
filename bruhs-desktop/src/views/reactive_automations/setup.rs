//! One review and one atomic command activate a complete automation.
use super::app_poll::{app_poll_source, ask_before_kind, AppPollEditor};
use super::*;
use bruhs_models::apps::{describe_app_poll, FROM_NEWER_SOURCE};
#[path = "task_datetime_picker.rs"]
mod task_datetime_picker;
#[path = "task_schedule.rs"]
mod task_schedule;
use bruhs_models::crew::{
    access_summary, BruhLook, BruhStatus, COMMAND_SOURCE_ACCESS, RAIL_FACE_PX,
};
use bruhs_ui::zoom::px;

pub(super) struct AutomationEditor {
    pub(super) scheduled_form: bool,
    schedule_focus: task_schedule::ScheduleFocus,
    date_time_picker: task_datetime_picker::SchedulePicker,
    task_enabled: bool,
    task_name: Entity<InputState>,
    once_instant: Option<bruhs_models::EpochMs>,
    pub(super) picker: task_choices::TaskPickerState,
    pub(super) is_new: bool,
    pub(super) host_menu_open: bool,
    pub(super) host_request: Option<uuid::Uuid>,
    pub(super) filters: Vec<webhook_filters::FilterRow>,
    dedupe: webhook_dedupe::DedupeFields,
    pub(super) security: webhook_security::SecurityFields,
    pub(super) webhook_url: Option<Entity<task_webhook_url::TaskWebhookUrl>>,
    webhook_prompt: Option<Entity<TextareaState>>,
    pub(super) open_filter: Option<usize>,
    pub(super) task_thread_open: bool,
    pub(super) checkout: task_checkout::CheckoutEditor,
    agent: AgentEditor,
    source: SourceEditor,
    subscription: SubscriptionEditor,
    /// The App source's fields, kept while another type is chosen.
    poll: AppPollEditor,
    review: Option<AutomationSetup>,
    advanced: bool,
    /// Opened on a bruh's page: the routine runs as that bruh, so its agent
    /// fields are hidden and the agent is sent exactly as saved.
    agent_locked: bool,
}

impl AutomationEditor {
    fn signing_secret(&self, setup: &AutomationSetup, cx: &App) -> Option<WebhookSigningSecret> {
        setup.subscription.webhook_security.signature.as_ref()?;
        self.security.replacement(cx)
    }

    pub(super) fn task_title(&self) -> &'static str {
        if self.is_new {
            "New task"
        } else {
            "Edit task"
        }
    }

    pub(super) fn task_save_label(&self) -> &'static str {
        if self.is_new {
            "Create task"
        } else {
            "Save task"
        }
    }

    fn owner_prompt(&self) -> &Entity<TextareaState> {
        if self.agent_locked {
            &self.subscription.instructions
        } else {
            &self.agent.instructions
        }
    }

    fn task_prompt(&self) -> &Entity<TextareaState> {
        if self.source.kind == "webhook" {
            if let Some(prompt) = &self.webhook_prompt {
                return prompt;
            }
        }
        self.owner_prompt()
    }

    fn start_webhook_prompt(&mut self, window: &mut Window, cx: &mut App) {
        if self.webhook_prompt.is_none() {
            let text = value(self.owner_prompt(), cx);
            self.webhook_prompt = Some(edit_area(
                if text.trim().is_empty() {
                    DEFAULT_WEBHOOK_PROMPT
                } else {
                    &text
                },
                window,
                cx,
            ));
        }
    }

    fn select_trigger(&mut self, kind: &'static str, window: &mut Window, cx: &mut App) {
        if self.source.kind == kind {
            return;
        }
        self.security.encoding_open = false;
        if self.scheduled_form {
            self.date_time_picker.close();
            self.date_time_picker
                .use_text_entry(kind == "calendar" && value(&self.source.times, cx).contains(','));
            if self.source.kind == "webhook" && self.webhook_prompt.is_some() {
                let text = value(self.task_prompt(), cx);
                self.owner_prompt()
                    .update(cx, |input, cx| input.set_value(text, window, cx));
            }
            if kind == "webhook"
                && (self.webhook_prompt.is_some()
                    || self.source.source.kind != SourceKind::Webhook
                    || self.is_new)
            {
                self.webhook_prompt = None;
                self.start_webhook_prompt(window, cx);
            }
            if kind == "once" && value(&self.source.value, cx).is_empty() {
                self.seed_once(window, cx);
            }
            self.source.kind = kind;
            return;
        }
        self.source.kind = kind;
        self.source.value = edit_field(
            if kind == "app_event" {
                "meeting.completed"
            } else {
                ""
            },
            kind == "command",
            window,
            cx,
        );
        if kind == "schedule" {
            self.source
                .interval
                .update(cx, |input, cx| input.set_value("3600", window, cx));
        }
    }

    pub(super) fn first_task_field(&self, cx: &App) -> FocusHandle {
        self.task_name.focus_handle(cx)
    }

    pub(super) fn shared_task_choices(&self, selected_model: Option<&str>) -> (Div, Div) {
        let project = div().child(field("Project", &self.agent.path, false, true));
        let model = if let Some(model) = selected_model {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(muted("Model"))
                .child(muted(model.to_owned()))
        } else {
            div().child(field("Model", &self.agent.model, false, true))
        };
        (project, model)
    }

    pub(super) fn task_model(&self, cx: &App) -> String {
        value(&self.agent.model, cx)
    }

    pub(super) fn set_task_model(&self, model: &str, window: &mut Window, cx: &mut App) {
        self.agent
            .model
            .update(cx, |input, cx| input.set_value(model, window, cx));
    }

    pub(super) fn project_path(&self, cx: &App) -> String {
        value(&self.agent.path, cx)
    }
    pub(super) fn set_project_path(&self, project: &str, window: &mut Window, cx: &mut App) {
        self.agent
            .path
            .update(cx, |input, cx| input.set_value(project, window, cx));
    }

    pub(super) fn source_mut(&mut self) -> &mut SourceEditor {
        &mut self.source
    }
    fn schedule_uses_minutes(&self) -> bool {
        self.scheduled_form
            && !matches!(&self.source.source.kind,
            SourceKind::Schedule { every_ms } if *every_ms < 60_000 || *every_ms > task_schedule::MAX_EXACT_MS)
    }

    pub(super) fn task_conversation(&self) -> TaskConversation {
        self.subscription.subscription.task_conversation
    }
    pub(super) fn subscription_mut(&mut self) -> &mut SubscriptionEditor {
        &mut self.subscription
    }
    pub(super) fn poll_mut(&mut self) -> &mut AppPollEditor {
        &mut self.poll
    }
    #[cfg(test)]
    pub(super) fn review(&self) -> Option<&AutomationSetup> {
        self.review.as_ref()
    }
    /// The bruh this routine is locked to, when it was opened on its page.
    pub(super) fn locked_agent(&self) -> Option<&str> {
        self.agent_locked.then_some(self.agent.agent.id.as_str())
    }
}

/// What the setup form says about the agent.
pub(super) struct AgentFields {
    pub(super) name: String,
    pub(super) purpose: String,
    pub(super) folder: String,
    pub(super) model: String,
    pub(super) memory: String,
}

/// `agent` with the form's fields applied (web `with_agent_fields`). A
/// routine made on a bruh's page runs as that bruh: its agent stays exactly
/// as saved, so a routine never renames it, rewrites its job, moves its
/// folder, switches its model past the access check or turns it on.
pub(super) fn with_agent_fields(
    mut agent: ReactiveAgent,
    fields: AgentFields,
    agent_locked: bool,
) -> Result<ReactiveAgent, String> {
    if agent_locked {
        return Ok(agent);
    }
    if fields.name.trim().is_empty()
        || fields.purpose.trim().is_empty()
        || fields.folder.trim().is_empty()
    {
        return Err("Enter a name, instructions, and a project folder.".into());
    }
    bruhs_models::agent::ModelId::try_from(fields.model.clone())
        .map_err(|error| error.to_string())?;
    agent.name = fields.name;
    agent.instructions = fields.purpose;
    agent.project_path = fields.folder;
    agent.model = fields.model;
    agent.memory = fields.memory;
    agent.enabled = true;
    Ok(agent)
}

fn webhook_prompt_help() -> Div {
    muted("Use {{body}} for the full text or {{body.action}} for a JSON or URL-encoded form field. {{headers.name}} and {{query.name}} select individual values. {{request}} includes the full request.")
}

impl ReactiveAutomations {
    fn render_task_prompt(&self, editor: &AutomationEditor, cx: &Context<'_, Self>) -> Div {
        let body = div()
            .debug_selector(|| "scheduled-task-prompt".into())
            .flex()
            .flex_col()
            .gap_1()
            .child(field("Prompt", editor.task_prompt(), true, self.busy));
        if editor.source.kind != "webhook" {
            return body;
        }
        if editor.webhook_prompt.is_some() {
            return body.child(webhook_prompt_help());
        }
        body.child(
            control(
                "enable-webhook-prompt",
                "Use request placeholders",
                self.busy,
            )
            .on_activate(cx, |this, window, cx| {
                if let Some(Editor::Automation(editor)) = &mut this.editor {
                    editor.start_webhook_prompt(window, cx);
                    editor.task_prompt().focus_handle(cx).focus(window, cx);
                }
                cx.notify();
            }),
        )
    }

    pub(super) fn open_automation(
        &mut self,
        setup: Option<AutomationSetup>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.open_automation_as(setup, None, window, cx);
    }
    /// A routine on `agent_id`'s page: a new one starts from that bruh, and
    /// either way the bruh is never rewritten.
    pub(super) fn open_routine(
        &mut self,
        setup: Option<AutomationSetup>,
        agent_id: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(bruh) = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.agents.iter().find(|agent| agent.id == agent_id))
            .cloned()
        else {
            return;
        };
        self.open_automation_as(setup, Some(bruh), window, cx);
    }
    fn open_automation_as(
        &mut self,
        setup: Option<AutomationSetup>,
        bruh: Option<ReactiveAgent>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy || self.snapshot.is_none() {
            return;
        }
        let is_new = setup.is_none();
        let agent_locked = bruh.is_some();
        let seed = setup.as_ref().map(|s| s.agent.clone()).or(bruh);
        self.open_agent(seed, window, cx);
        let Some(Editor::Agent(agent)) = self.editor.take() else {
            return;
        };
        self.open_source(setup.as_ref().map(|s| s.source.clone()), window, cx);
        let Some(Editor::Source(mut source)) = self.editor.take() else {
            return;
        };
        let source_text = value(&source.value, cx);
        source.value = edit_field(&source_text, source.kind == "command", window, cx);
        self.open_subscription(setup.as_ref().map(|s| s.subscription.clone()), window, cx);
        let Some(Editor::Subscription(mut subscription)) = self.editor.take() else {
            return;
        };
        if is_new {
            source.source.name.clear();
            subscription.subscription.agent_id = agent.agent.id.clone();
            subscription.subscription.source_id = source.source.id.clone();
            subscription.subscription.auto_run = true;
            subscription.instructions.update(cx,|input,cx|input.set_value("Follow the automation's purpose for this event. Prepare a draft and flag uncertainty. Never perform writes or payments.",window,cx));
        }
        let poll = AppPollEditor::new(&source.source.kind, window, cx);
        self.editor = Some(Editor::Automation(Box::new(AutomationEditor {
            scheduled_form: false,
            task_name: edit_input(
                setup.as_ref().map_or("", AutomationSetup::title),
                window,
                cx,
            ),
            schedule_focus: task_schedule::ScheduleFocus::new(cx),
            date_time_picker: task_datetime_picker::SchedulePicker::new(cx),
            task_enabled: setup.as_ref().is_none_or(AutomationSetup::enabled),
            once_instant: None,
            picker: task_choices::TaskPickerState::new(is_new, window, cx),
            is_new,
            host_menu_open: false,
            host_request: None,
            open_filter: None,
            task_thread_open: false,
            checkout: task_checkout::CheckoutEditor::new(
                &subscription.subscription.task_checkout,
                window,
                cx,
            ),
            webhook_url: None,
            webhook_prompt: subscription
                .subscription
                .webhook_prompt
                .as_deref()
                .map(|text| edit_area(text, window, cx)),
            security: webhook_security::SecurityFields::new(
                &subscription.subscription.webhook_security,
                window,
                cx,
            ),
            dedupe: webhook_dedupe::DedupeFields::new(
                &subscription.subscription.webhook_dedupe,
                window,
                cx,
            ),
            filters: subscription
                .subscription
                .webhook_filters
                .iter()
                .map(|filter| webhook_filters::FilterRow::new(Some(filter), window, cx))
                .collect(),
            agent,
            source,
            subscription,
            poll,
            review: None,
            advanced: false,
            agent_locked,
        })));
        self.error = None;
        // The App source offers the owner's accounts as they are now.
        self.load_poll_apps(cx);
        cx.notify();
    }
    pub(super) fn review_automation(&mut self, cx: &mut Context<'_, Self>) {
        let Some(Editor::Automation(editor)) = &self.editor else {
            return;
        };
        if editor.host_request.is_some() {
            return;
        }
        let mut setup = match editor.build(self.app_source_block(), self.poll_apps.listing(), cx) {
            Ok(setup) => setup,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        self.error = None;
        if editor.scheduled_form {
            // Other tasks keep their original clock or webhook when this task changes its trigger.
            if setup.source != editor.source.source
                && self.snapshot.as_ref().is_some_and(|snapshot| {
                    snapshot.subscriptions.iter().any(|sub| {
                        sub.id != setup.subscription.id && sub.source_id == setup.source.id
                    })
                })
            {
                setup.source.id = uuid::Uuid::new_v4().to_string();
                setup.subscription.source_id.clone_from(&setup.source.id);
            }
            let destination = editor.picker.chosen_thread.as_ref();
            if let Some(choice) = destination {
                // A paired host resolves the selected agent from its own records at save time.
                if self.linked_task.is_none() {
                    let saved_agent = self.snapshot.iter().flat_map(|s| &s.agents).find(|a| {
                        a.id == choice.agent_id && a.project_path == setup.agent.project_path
                    });
                    if let Some(agent) = saved_agent {
                        setup.agent = agent.clone();
                        if let Some(model) = &choice.desktop_model {
                            setup.agent.model.clone_from(model);
                        }
                    } else if let Some(model) = &choice.desktop_model {
                        setup.agent.id.clone_from(&choice.agent_id);
                        setup.agent.name.clone_from(&choice.title);
                        setup.agent.model.clone_from(model);
                        setup.agent.instructions =
                            "Continue the selected desktop conversation.".into();
                        setup.agent.memory.clear();
                    } else {
                        self.error = Some(
                            "The destination thread is no longer available in this project.".into(),
                        );
                        cx.notify();
                        return;
                    };
                    setup.subscription.agent_id.clone_from(&setup.agent.id);
                }
                if !editor.agent_locked {
                    let mut prompt = value(editor.owner_prompt(), cx);
                    if !setup.subscription.instructions.trim().is_empty()
                        && setup.subscription.instructions != prompt
                    {
                        prompt.push_str("\n\n");
                        prompt.push_str(&setup.subscription.instructions);
                    }
                    setup.subscription.instructions = prompt;
                }
            }
            if editor.is_new
                || setup.agent.model != editor.agent.agent.model
                || setup.agent.project_path != editor.agent.agent.project_path
            {
                let model = destination
                    .and_then(|choice| {
                        choice
                            .desktop_model
                            .as_ref()
                            .or(choice.agent_model.as_ref())
                    })
                    .unwrap_or(&setup.agent.model);
                if let Some(problem) = editor
                    .picker
                    .model_problem(&setup.agent.project_path, model)
                {
                    self.error = Some(problem.to_owned());
                    cx.notify();
                    return;
                }
            }
            let signing_secret = editor.signing_secret(&setup, cx);
            let destination = editor.picker.destination_edit(editor.task_conversation());
            if self.linked_task.is_some() {
                self.save_linked_task(setup, editor.task_enabled, signing_secret, destination, cx);
                return;
            }
            let command = if destination.is_none()
                && signing_secret.is_none()
                && editor.task_enabled == setup.enabled()
            {
                apply(ReactiveChange::SaveAutomation {
                    automation: Box::new(setup),
                })
            } else {
                ReactiveCommand::SaveScheduledTask {
                    destination,
                    automation: Box::new(setup),
                    enabled: editor.task_enabled,
                    signing_secret,
                }
            };
            self.send(command, cx);
        } else if let Some(Editor::Automation(editor)) = &mut self.editor {
            editor.review = Some(setup);
        }
        cx.notify();
    }
    pub(super) fn render_automation_editor(
        &self,
        editor: &AutomationEditor,
        cx: &Context<'_, Self>,
    ) -> Div {
        self.render_automation_form(editor, None, cx)
    }

    pub(super) fn render_automation_form(
        &self,
        editor: &AutomationEditor,
        choices: Option<(Div, Div, Div)>,
        cx: &Context<'_, Self>,
    ) -> Div {
        if let Some(setup) = &editor.review {
            let mut body = card().child(heading("Review automation"));
            let level =
                agent_run_level(self.snapshot.as_ref(), &setup.agent.id, &setup.agent.model);
            let account = self.poll_label(&setup.source.kind);
            for (label, text) in review_fields_with(setup, level, account.as_deref()) {
                body = body.child(
                    div().flex().flex_col().gap_1().child(muted(label)).child(
                        div()
                            .text_size(px(13.0))
                            .line_height(relative(1.5))
                            .text_color(theme::rambutan_200())
                            .child(text),
                    ),
                );
            }
            body=body.child(muted(event_policy_line(setup.subscription.auto_run, level))).child(muted("The local server must be running. This enables the complete automation in one saved change."));
            return body.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(self.command(
                        "automation-enable".into(),
                        "Enable automation",
                        match editor.signing_secret(setup, cx) {
                            Some(secret) => ReactiveCommand::SaveScheduledTask {
                                destination: None,
                                automation: Box::new(setup.clone()),
                                enabled: setup.enabled(),
                                signing_secret: Some(secret),
                            },
                            None => apply(ReactiveChange::SaveAutomation {
                                automation: Box::new(setup.clone()),
                            }),
                        },
                        cx,
                    ))
                    .child(
                        control("automation-back", "Back to edit", self.busy).on_activate(
                            cx,
                            |this, _, cx| {
                                if let Some(Editor::Automation(editor)) = &mut this.editor {
                                    editor.review = None;
                                }
                                cx.notify();
                            },
                        ),
                    )
                    .child(
                        control("automation-cancel", "Cancel setup", self.busy).on_activate(
                            cx,
                            |this, _, cx| {
                                this.editor = None;
                                cx.notify();
                            },
                        ),
                    ),
            );
        }
        let in_dialog = choices.is_some();
        let title = if editor.scheduled_form {
            editor.task_title()
        } else {
            "Set up an automation"
        };
        let mut body = if in_dialog {
            div().flex().flex_col().gap_3()
        } else {
            card().child(heading(title))
        };
        if !editor.scheduled_form {
            body = body.child(muted("Describe the outcome and choose what starts it. Nothing is saved or enabled until you review the complete automation."));
        }
        let choices = if let Some((host, project, model)) = choices {
            body = body.child(host);
            Some((project, model))
        } else {
            None
        };
        if in_dialog || editor.subscription.subscription.task_title.is_some() {
            body = body.child(field("Name", &editor.task_name, false, self.busy));
        } else if !editor.agent_locked {
            body = body.child(field(
                "Automation name",
                &editor.agent.name,
                false,
                self.busy,
            ));
        }
        if let Some((project, model)) = choices {
            body = body.child(self.render_task_conversation(editor, cx));
            if let Some(choice) = &editor.picker.chosen_thread {
                if let Some(agent) = self
                    .snapshot
                    .iter()
                    .flat_map(|s| &s.agents)
                    .find(|a| a.id == choice.agent_id)
                {
                    body = body.child(muted(format!("Runs as {}", agent.name)));
                }
            } else if editor.agent_locked {
                body = body.child(muted(format!("Runs as {}", editor.agent.agent.name)));
            }
            if !editor.task_conversation().is_agent() && editor.picker.displayed_thread().is_none()
            {
                let (workspace, details) = self.render_task_checkout(editor, cx);
                body = body
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_start()
                            .gap_3()
                            .child(div().flex_1().min_w(px(200.)).child(project))
                            .child(div().flex_1().min_w(px(200.)).child(workspace)),
                    )
                    .children(details);
            } else {
                body = body.child(project);
            }
            body = body.child(self.render_task_prompt(editor, cx)).child(model);
        } else if editor.agent_locked {
            body = body.child(
                muted("This routine runs as this bruh. Change its job and model in Settings.")
                    .debug_selector(|| "automation-runs-as-bruh".to_string()),
            );
        } else {
            body = body
                .child(field(
                    "What should this automation do?",
                    &editor.agent.instructions,
                    true,
                    self.busy,
                ))
                .child(field(
                    "Automation working folder",
                    &editor.agent.path,
                    false,
                    self.busy,
                ))
                .child(field("Model", &editor.agent.model, false, self.busy));
        }
        if !in_dialog && editor.source.kind == "webhook" {
            if let Some(prompt) = &editor.webhook_prompt {
                body = body
                    .child(field("Webhook prompt", prompt, true, self.busy))
                    .child(webhook_prompt_help());
            }
        }
        if editor.scheduled_form {
            if !in_dialog {
                body = body.child(self.render_task_conversation(editor, cx));
                if !editor.task_conversation().is_agent()
                    && editor.picker.displayed_thread().is_none()
                {
                    let (workspace, details) = self.render_task_checkout(editor, cx);
                    body = body.child(workspace).children(details);
                }
            }
            body = body.child(self.schedule_selector(editor, cx));
        } else {
            body = body
                .child(muted("Starts when"))
                .child(
                    div().flex().flex_wrap().gap_2().children(
                        super::app_poll::starts_when_options(self.apps_ready(), editor.source.kind)
                            .into_iter()
                            .map(|(kind, label)| {
                                control(
                                    SharedString::from(format!("automation-kind-{kind}")),
                                    label,
                                    self.busy,
                                )
                                .aria_selected(editor.source.kind == kind)
                                .when(editor.source.kind == kind, |button| {
                                    button.bg(theme::blueberry_500().opacity(0.15))
                                })
                                .on_activate(
                                    cx,
                                    move |this, window, cx| {
                                        if let Some(Editor::Automation(editor)) = &mut this.editor {
                                            if !ask_before_kind(&mut editor.source, kind) {
                                                editor.select_trigger(kind, window, cx);
                                            }
                                        }
                                        cx.notify();
                                    },
                                )
                            }),
                    ),
                )
                .children(self.render_newer_source_confirm(&editor.source, cx));
        }
        body=match editor.source.kind {
            "calendar" | "once" | "schedule" if editor.scheduled_form => body.child(self.schedule_timing_fields(editor, cx)),
            "folder"=>body.child(field("Folder to watch",&editor.source.value,false,self.busy)).child(field("File pattern",&editor.source.pattern,false,self.busy)),
            "http"=>body.child(field("Website URL",&editor.source.value,false,self.busy)),
            "command"=>body.child(field("Command to run",&editor.source.value,true,self.busy)).child(muted("This command runs on your machine once you enable the automation.")),
            "once"=>body.child(field("Run at (YYYY-MM-DD HH:MM)", &editor.source.value, false, self.busy)),
            "schedule"=>body.child(field("Run every, in seconds",&editor.source.interval,false,self.busy)),
            "app_event"=>body.child(div().flex().flex_wrap().gap_2().children([("meeting.completed","Meeting completed"),("meeting.transcribed","Final transcript updated"),("meeting.summary","Summary ready")].into_iter().map(|(topic,label)|{let selected=value(&editor.source.value,cx)==topic;control(SharedString::from(format!("automation-topic-{topic}")),label,self.busy).aria_selected(selected).when(selected,|button|button.bg(theme::blueberry_500().opacity(0.15))).on_activate(cx,move |this,window,cx|{if let Some(Editor::Automation(editor))=&mut this.editor{editor.source.value.set_value(topic,window,cx);}cx.notify();})}))),
            "calendar"=>body.child(self.render_calendar_fields(&editor.source, cx)),
            "webhook"=>body.child(if editor.scheduled_form && self.snapshot.as_ref().is_some_and(|s| s.can_address_task_webhooks) {
                match &editor.webhook_url {
                    Some(address) => address.clone().into_any_element(),
                    None => muted("The webhook URL appears after you save.").into_any_element(),
                }
            } else {
                muted("Open the saved source in Automations to reveal its endpoint and token.").into_any_element()
            }).child(editor.security.age(self.busy)).child(self.render_webhook_filters(editor,cx)).child(editor.dedupe.render(self.busy)).child(self.render_webhook_security(&editor.security,cx)),
            "app_poll"=>body.child(self.render_app_poll(&editor.poll, cx)),
            "other"=>body.child(muted(FROM_NEWER_SOURCE)),
            _=>body.child(muted("Device bridges can publish through the local API. Use a watched folder for FTP uploads.")),
        };
        if editor.scheduled_form {
            body = body.child(self.render_task_enabled(editor, cx));
        }
        body = body.child(
            control("automation-advanced-options", "Advanced options", self.busy)
                .aria_expanded(editor.advanced)
                .on_activate(cx, |this, _, cx| {
                    if let Some(Editor::Automation(editor)) = &mut this.editor {
                        editor.advanced = !editor.advanced;
                    }
                    cx.notify();
                }),
        );
        if editor.advanced {
            let mut advanced = card()
                .when(!editor.agent_locked, |advanced| {
                    advanced.child(field(
                        "Automation memory",
                        &editor.agent.memory,
                        true,
                        self.busy,
                    ))
                })
                .when(
                    !(in_dialog && editor.agent_locked)
                        && !(editor.source.kind == "webhook" && editor.webhook_prompt.is_some()),
                    |advanced| {
                        advanced.child(field(
                            "Event-specific instructions",
                            &editor.subscription.instructions,
                            true,
                            self.busy,
                        ))
                    },
                )
                .child(
                    control(
                        "automation-auto-run",
                        &super::auto_run_control(
                            editor.subscription.subscription.auto_run,
                            agent_run_level(
                                self.snapshot.as_ref(),
                                &editor.subscription.subscription.agent_id,
                                &value(&editor.agent.model, cx),
                            ),
                        ),
                        self.busy,
                    )
                    .on_activate(cx, |this, _, cx| {
                        if let Some(Editor::Automation(editor)) = &mut this.editor {
                            editor.subscription.subscription.auto_run =
                                !editor.subscription.subscription.auto_run;
                        }
                        cx.notify();
                    }),
                );
            if matches!(editor.source.kind, "folder" | "http" | "command") {
                advanced = advanced.child(field(
                    "Check interval in seconds",
                    &editor.source.interval,
                    false,
                    self.busy,
                ));
            }
            if editor.scheduled_form && editor.source.kind == "calendar" {
                advanced = advanced
                    .child(muted(
                        "For multiple runs each day, separate the times with commas.",
                    ))
                    .child(field(
                        "Time zone",
                        &editor.source.time_zone,
                        false,
                        self.busy,
                    ));
            }
            if editor.source.kind == "folder" {
                advanced = advanced
                    .child(field(
                        "File settle time in seconds",
                        &editor.source.settle,
                        false,
                        self.busy,
                    ))
                    .child(
                        control(
                            "automation-recursive",
                            if editor.source.recursive {
                                "Include subfolders: On"
                            } else {
                                "Include subfolders: Off"
                            },
                            self.busy,
                        )
                        .on_activate(cx, |this, _, cx| {
                            if let Some(Editor::Automation(editor)) = &mut this.editor {
                                editor.source.recursive = !editor.source.recursive;
                            }
                            cx.notify();
                        }),
                    );
            }
            if editor.source.kind == "command" {
                advanced = advanced
                    .child(field(
                        "Command working folder, if different",
                        &editor.source.cwd,
                        false,
                        self.busy,
                    ))
                    .child(field(
                        "Command timeout in seconds",
                        &editor.source.timeout,
                        false,
                        self.busy,
                    ));
            }
            advanced = advanced
                .child(field(
                    "Debounce in seconds",
                    &editor.subscription.debounce,
                    false,
                    self.busy,
                ))
                .child(field(
                    "Minimum gap between runs in seconds",
                    &editor.subscription.throttle,
                    false,
                    self.busy,
                ))
                .child(self.render_busy_choice(editor.subscription.subscription.on_busy, cx));
            body = body.child(advanced);
        }
        if in_dialog {
            return body;
        }
        if editor.scheduled_form {
            if let Some(error) = self.error.as_ref().or(self.load_error.as_ref()) {
                body = body.child(
                    muted(error.clone())
                        .id("task-save-error")
                        .debug_selector(|| "task-save-error".to_owned())
                        .role(Role::Alert)
                        .text_color(theme::lychee_300()),
                );
            }
        }
        body.child(
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .child(
                    control_as(
                        "automation-review",
                        match (editor.scheduled_form, editor.is_new) {
                            (true, true) => "Create task",
                            (true, false) => "Save task",
                            (false, _) => "Review automation",
                        },
                        self.busy,
                        true,
                    )
                    .on_activate(cx, |this, _, cx| this.review_automation(cx)),
                )
                .child(
                    control(
                        "automation-cancel",
                        if editor.scheduled_form {
                            "Cancel"
                        } else {
                            "Cancel setup"
                        },
                        self.busy,
                    )
                    .on_activate(cx, |this, _, cx| {
                        this.editor = None;
                        cx.notify();
                    }),
                ),
        )
    }
    /// The automation cards: every one, or only `agent`'s routines on its
    /// bruh page, where the bruh's memory has its own tab.
    pub(super) fn render_automation_cards_for(
        &self,
        agent: Option<&str>,
        cx: &Context<'_, Self>,
    ) -> Div {
        let mut body = div().flex().flex_col().gap_3();
        let Some(snapshot) = &self.snapshot else {
            return body;
        };
        let mut shown_agents = HashSet::new();
        let setups: Vec<AutomationSetup> = snapshot
            .subscriptions
            .iter()
            .filter(|subscription| agent.is_none_or(|id| subscription.agent_id == id))
            .filter_map(|subscription| AutomationSetup::from_subscription(snapshot, subscription))
            .collect();
        if setups.is_empty() {
            body=body.child(muted("No automations yet. Start with a scan folder, a meeting event, or a repeating check."));
        }
        let routines = agent.map(str::to_string);
        for setup in setups {
            let active = setup.enabled();
            let edit = setup.clone();
            let chat_id = setup.agent.id.clone();
            let (look, face_status) = snapshot
                .bruhs
                .iter()
                .find(|bruh| bruh.agent_id == setup.agent.id)
                .map_or_else(
                    || (BruhLook::default(), BruhStatus::Asleep),
                    |bruh| (bruh.look.clone(), bruh.status),
                );
            let mut row = card()
                .child(
                    div()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(crate::views::crew_face::bruh_face(
                            SharedString::from(format!("automation-{}", setup.subscription.id)),
                            RAIL_FACE_PX,
                            look,
                            face_status,
                            cx,
                        ))
                        .child(heading(setup.title().to_owned())),
                )
                .child(muted(setup.agent.instructions.clone()))
                .child(muted(source_summary(
                    &setup.source.kind,
                    self.poll_label(&setup.source.kind).as_deref(),
                )))
                .child(muted(automation_status(
                    &setup,
                    agent_run_level(self.snapshot.as_ref(), &setup.agent.id, &setup.agent.model),
                )))
                .when(setup.subscription.on_busy == BusyPolicy::Restart, |row| {
                    row.child(muted(format!(
                        "While the agent is busy: {}",
                        super::editor_fields::BUSY_RESTART_LABEL
                    )))
                });
            if let Some(run) = snapshot
                .runs
                .iter()
                .find(|run| run.subscription_id.as_deref() == Some(setup.subscription.id.as_str()))
            {
                row = row.child(muted(format!("Latest activity: {}", run_label(run.status))));
            }
            if let Some(error) = snapshot
                .source_statuses
                .iter()
                .find(|status| status.source_id == setup.source.id)
                .and_then(|status| status.error.as_ref())
            {
                row = row.child(
                    div()
                        .text_size(px(12.0))
                        .text_color(theme::lychee_300())
                        .child(error.clone()),
                );
            }
            row = row.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(self.command(
                        format!("automation-toggle-{}", setup.subscription.id),
                        if active {
                            "Pause automation"
                        } else {
                            "Resume automation"
                        },
                        ReactiveCommand::SetAutomationEnabled {
                            id: setup.subscription.id.clone(),
                            enabled: !active,
                        },
                        cx,
                    ))
                    .child(
                        control(
                            SharedString::from(format!(
                                "automation-edit-{}",
                                setup.subscription.id
                            )),
                            "Edit automation",
                            self.busy,
                        )
                        .on_activate(cx, {
                            let routines = routines.clone();
                            move |this, window, cx| match &routines {
                                Some(agent_id) => {
                                    this.open_routine(Some(edit.clone()), agent_id, window, cx)
                                }
                                None => this.open_automation(Some(edit.clone()), window, cx),
                            }
                        }),
                    )
                    .child(
                        control(
                            SharedString::from(format!(
                                "automation-chat-{}",
                                setup.subscription.id
                            )),
                            "Chat",
                            self.busy,
                        )
                        .on_activate(cx, {
                            let on_page = routines.is_some();
                            move |this, window, cx| {
                                // On a bruh's page, Chat is the page's own tab.
                                if on_page {
                                    this.set_bruh_tab(super::bruh_page::BruhTab::Chat, cx);
                                    return;
                                }
                                this.editor = Some(Editor::Chat {
                                    agent_id: chat_id.clone(),
                                    message: edit_area("", window, cx),
                                });
                                cx.notify();
                            }
                        }),
                    ),
            );
            if routines.is_none() && shown_agents.insert(setup.agent.id.clone()) {
                row = row.child(self.render_agent_records(&setup.agent.id, cx));
            }
            body = body.child(row);
        }
        body
    }
}

impl AutomationEditor {
    /// `blocked` says why an App source can't be saved, and `listing` is
    /// the accounts it may poll with, once read.
    fn build(
        &self,
        blocked: Option<&'static str>,
        listing: Option<&bruhs_models::apps::AppsListing>,
        cx: &App,
    ) -> Result<AutomationSetup, String> {
        let task_name =
            if self.scheduled_form || self.subscription.subscription.task_title.is_some() {
                let title = value(&self.task_name, cx).trim().to_owned();
                if title.is_empty() {
                    return Err("Enter a task name.".into());
                }
                Some(title)
            } else {
                None
            };
        let fields = AgentFields {
            name: if self.is_new {
                task_name
                    .clone()
                    .unwrap_or_else(|| value(&self.agent.name, cx))
            } else {
                value(&self.agent.name, cx)
            },
            purpose: if self.scheduled_form
                && self.source.kind == "webhook"
                && self.webhook_prompt.is_some()
            {
                if self.agent.agent.instructions.trim().is_empty() {
                    "Follow the task prompt supplied by the owner for each webhook delivery.".into()
                } else {
                    self.agent.agent.instructions.clone()
                }
            } else {
                value(&self.agent.instructions, cx)
            },
            folder: value(&self.agent.path, cx),
            model: value(&self.agent.model, cx),
            memory: value(&self.agent.memory, cx),
        };
        let mut agent = with_agent_fields(self.agent.agent.clone(), fields, self.agent_locked)?;
        let preserve_state = self.scheduled_form && !self.is_new;
        if preserve_state {
            agent.enabled = self.agent.agent.enabled;
        }
        let mut source = self.source.source.clone();
        if source.name.trim().is_empty() {
            source.name = format!("{} source", agent.name);
        }
        source.enabled = !preserve_state || self.source.source.enabled;
        let text = value(&self.source.value, cx);
        let interval = milliseconds(&value(&self.source.interval, cx), false);
        source.kind = match self.source.kind {
            "folder" => SourceKind::Folder {
                path: text,
                pattern: value(&self.source.pattern, cx),
                recursive: self.source.recursive,
                settle_ms: milliseconds(&value(&self.source.settle, cx), true)?,
                poll_ms: interval?,
            },
            "http" => SourceKind::Http {
                url: text,
                poll_ms: interval?,
            },
            "command" => {
                let cwd = value(&self.source.cwd, cx);
                SourceKind::Command {
                    command: text,
                    cwd: if cwd.trim().is_empty() {
                        agent.project_path.clone()
                    } else {
                        cwd
                    },
                    poll_ms: interval?,
                    timeout_ms: milliseconds(&value(&self.source.timeout, cx), false)?,
                }
            }
            "schedule" if self.schedule_uses_minutes() => SourceKind::Schedule {
                every_ms: task_schedule::interval_milliseconds(
                    &value(&self.source.interval, cx),
                    &self.source.source.kind,
                )?,
            },
            "schedule" => SourceKind::Schedule {
                every_ms: interval?,
            },
            "calendar" => super::editor_fields::calendar_kind(
                &self.calendar_times(cx),
                &self.source.weekdays,
                &value(&self.source.time_zone, cx),
            )?,
            "once" if self.scheduled_form => self.once_schedule(&text)?,
            "once" => once_source(&text)?,
            "webhook" => SourceKind::Webhook,
            "app_event" => SourceKind::AppEvent { topic: text },
            "custom" => SourceKind::Custom,
            "app_poll" => app_poll_source(&self.poll.form(cx), listing, blocked)?,
            // A newer Bruhs' source is never saved (ruling F3).
            "other" => return Err(bruhs_models::apps::FROM_NEWER_SOURCE.into()),
            _ => return Err("Choose a supported source type.".into()),
        };
        let mut subscription = self.subscription.subscription.clone();
        if let Some(name) = task_name {
            subscription.task_title = Some(name);
        }
        if !matches!(
            source.kind,
            SourceKind::Once { .. }
                | SourceKind::Calendar { .. }
                | SourceKind::Schedule { .. }
                | SourceKind::Webhook
        ) {
            subscription.task_conversation = TaskConversation::Agent;
        }
        if subscription.task_conversation.is_agent() {
            subscription.task_checkout = TaskCheckout::Project;
        } else if let Some(choice) = self.picker.displayed_thread() {
            subscription.task_checkout = choice.checkout.clone();
        } else if self.scheduled_form {
            subscription.task_checkout = self.checkout.build(cx)?;
        }
        subscription.webhook_prompt = if source.kind == SourceKind::Webhook {
            self.webhook_prompt.as_ref().map(|input| value(input, cx))
        } else {
            None
        };
        if subscription
            .webhook_prompt
            .as_ref()
            .is_some_and(|p| p.trim().is_empty())
        {
            return Err("Enter a webhook prompt.".into());
        }
        subscription.webhook_security = if source.kind == SourceKind::Webhook {
            self.security.build(cx)?
        } else {
            WebhookSecurity::default()
        };
        subscription.webhook_dedupe = if source.kind == SourceKind::Webhook {
            self.dedupe.build(cx)?
        } else {
            WebhookDedupe::default()
        };
        subscription.webhook_filters = if source.kind == SourceKind::Webhook {
            self.filters
                .iter()
                .enumerate()
                .map(|(index, row)| {
                    row.build(cx)
                        .map_err(|error| format!("Filter {} · {error}", index + 1))
                })
                .collect::<Result<_, _>>()?
        } else {
            Vec::new()
        };
        subscription.instructions = value(&self.subscription.instructions, cx);
        subscription.debounce_ms = milliseconds(&value(&self.subscription.debounce, cx), true)?;
        subscription.throttle_ms = milliseconds(&value(&self.subscription.throttle, cx), true)?;
        subscription.enabled = !preserve_state || self.subscription.subscription.enabled;
        Ok(AutomationSetup {
            agent,
            source,
            subscription,
        })
    }
}
/// Agents whose memory already shows on an automation card.
pub(super) fn automation_agent_ids(snapshot: &ReactiveSnapshot) -> HashSet<String> {
    snapshot
        .subscriptions
        .iter()
        .filter_map(|subscription| AutomationSetup::from_subscription(snapshot, subscription))
        .map(|setup| setup.agent.id)
        .collect()
}
/// `account` names a new mail routine's mailbox.
pub(super) fn source_summary(kind: &SourceKind, account: Option<&str>) -> String {
    match kind {
        SourceKind::Folder { path, pattern, .. } => format!("Folder: {path} ({pattern})"),
        SourceKind::Http { url, .. } => format!("Website: {url}"),
        SourceKind::Command { command, .. } => format!("Command: {command}"),
        SourceKind::Once { due_at } => format!("Once · {}", local_time(*due_at)),
        SourceKind::Schedule { every_ms } => format!("Every {} seconds", seconds(*every_ms)),
        SourceKind::Calendar {
            times,
            weekdays,
            time_zone,
        } => describe_calendar(times, weekdays, time_zone),
        SourceKind::Webhook => "Authenticated webhook".into(),
        SourceKind::AppEvent { topic } => format!("App event: {topic}"),
        SourceKind::Custom => "Custom events from your integrations".into(),
        SourceKind::AppPoll { query, .. } => describe_app_poll(query, account),
        SourceKind::Other => FROM_NEWER_SOURCE.into(),
    }
}
/// `level` is the level the automation's agent runs at.
fn automation_status(setup: &AutomationSetup, level: AccessLevel) -> String {
    if !setup.subscription.enabled {
        "Paused".into()
    } else if !setup.agent.enabled {
        "Paused: agent disabled".into()
    } else if !setup.source.enabled {
        "Paused: source disabled".into()
    } else if setup.subscription.auto_run {
        format!("Enabled: {}", event_policy(true, level))
    } else {
        "Enabled: review each event".into()
    }
}
fn source_details(kind: &SourceKind, account: Option<&str>) -> String {
    match kind {
        SourceKind::Folder{path,pattern,recursive,settle_ms,poll_ms}=>format!("Folder: {path}\nFiles: {pattern}\nSubfolders: {}\nFile settle time: {} seconds\nCheck interval: {} seconds",if *recursive{"Included"}else{"Excluded"},seconds(*settle_ms),seconds(*poll_ms)),
        SourceKind::Http{url,poll_ms}=>format!("URL: {url}\nCheck interval: {} seconds",seconds(*poll_ms)),
        SourceKind::Command{command,cwd,poll_ms,timeout_ms}=>format!("Command: {command}\nWorking folder: {cwd}\nCheck interval: {} seconds\nTimeout: {} seconds\n{COMMAND_SOURCE_ACCESS}",seconds(*poll_ms),seconds(*timeout_ms)),
        SourceKind::Once {due_at} => format!("Once · {}", local_time(*due_at)),
        SourceKind::Schedule{every_ms}=>format!("Repeat every {} seconds",seconds(*every_ms)),
        SourceKind::Calendar{times,weekdays,time_zone}=>describe_calendar(times,weekdays,time_zone),
        SourceKind::Webhook=>"Authenticated incoming webhook. The source row provides its endpoint and token after saving.".into(),
        SourceKind::AppEvent{topic}=>format!("App event: {topic}"),
        SourceKind::Custom=>"Events published by your integrations through the local API.".into(),
        SourceKind::AppPoll{query,poll_ms,..}=>format!("{}\nCheck interval: {} seconds",describe_app_poll(query,account),seconds(*poll_ms)),
        SourceKind::Other=>FROM_NEWER_SOURCE.into(),
    }
}

/// `level` is the level the automation's agent runs at.
pub(super) fn review_fields(
    setup: &AutomationSetup,
    level: AccessLevel,
) -> Vec<(&'static str, String)> {
    review_fields_with(setup, level, None)
}

/// [`review_fields`], with `account` naming a new mail routine's mailbox.
fn review_fields_with(
    setup: &AutomationSetup,
    level: AccessLevel,
    account: Option<&str>,
) -> Vec<(&'static str, String)> {
    let mut fields = vec![
        ("Name", setup.title().to_owned()),
        ("Purpose", setup.agent.instructions.clone()),
        ("Event source name", setup.source.name.clone()),
        (
            "Agent state",
            if setup.agent.enabled {
                "Enabled"
            } else {
                "Paused"
            }
            .into(),
        ),
        (
            "Source state",
            if setup.source.enabled {
                "Enabled"
            } else {
                "Paused"
            }
            .into(),
        ),
        (
            "Automation state",
            if setup.subscription.enabled {
                "Enabled"
            } else {
                "Paused"
            }
            .into(),
        ),
        (
            "Event policy",
            event_policy_line(setup.subscription.auto_run, level),
        ),
        ("Starts when", source_summary(&setup.source.kind, account)),
        (
            "Complete source settings",
            source_details(&setup.source.kind, account),
        ),
        ("Working folder", setup.agent.project_path.clone()),
        ("Model", setup.agent.model.clone()),
        (
            if matches!(setup.source.kind, SourceKind::Command { .. }) {
                "Agent access"
            } else {
                "Access"
            },
            access_summary(level).into(),
        ),
        (
            "Persistent memory",
            if setup.agent.memory.is_empty() {
                "None".into()
            } else {
                setup.agent.memory.clone()
            },
        ),
        (
            "Event instructions",
            setup.subscription.instructions.clone(),
        ),
        (
            "Debounce",
            format!("{} seconds", seconds(setup.subscription.debounce_ms)),
        ),
        (
            "Minimum gap",
            format!("{} seconds", seconds(setup.subscription.throttle_ms)),
        ),
    ];
    if !setup.subscription.task_conversation.is_agent() {
        fields.push((
            "Thread",
            setup.subscription.task_conversation.label().into(),
        ));
    }
    if let Some(prompt) = &setup.subscription.webhook_prompt {
        fields.push(("Webhook prompt", prompt.clone()));
    }
    if let Some(sig) = &setup.subscription.webhook_security.signature {
        fields.push(("Require signature", format!("HMAC-SHA256 · {}", sig.header)));
    }
    if let Some(age) = setup.subscription.webhook_security.max_delivery_age_minutes {
        fields.push(("Skip requests older than", format!("{age} minutes")));
    }
    let dedupe = &setup.subscription.webhook_dedupe;
    if !dedupe.replay_key.is_empty() {
        fields.push(("Duplicate request key", dedupe.replay_key.clone()));
    }
    if !dedupe.coalesce_key.is_empty() {
        fields.push(("Coalesce key", dedupe.coalesce_key.clone()));
        fields.push((
            "Coalesce window",
            format!("{} seconds", dedupe.coalesce_window_seconds.unwrap_or(120)),
        ));
    }
    if !setup.subscription.webhook_filters.is_empty() {
        let filters = setup
            .subscription
            .webhook_filters
            .iter()
            .map(|filter| {
                let test = match &filter.test {
                    WebhookTest::Equals(value) => format!("equals {value}"),
                    WebhookTest::NotEquals(value) => format!("does not equal {value}"),
                    WebhookTest::In(values) => format!("is one of {}", values.join(", ")),
                    WebhookTest::StartsWith(value) => format!("starts with {value}"),
                    WebhookTest::Exists => "exists".to_owned(),
                };
                format!("{} {test}", filter.path)
            })
            .collect::<Vec<_>>();
        fields.push(("Webhook filters", filters.join("\n")));
    }
    fields
}

#[cfg(test)]
mod tests {
    use super::{
        review_fields, AccessLevel, AutomationSetup, Editor, ReactiveAutomations, ReactiveSnapshot,
    };
    use crate::theme;
    use gpui::AppContext as _;
    #[gpui::test]
    fn scheduled_host_change_keeps_an_incomplete_draft_and_discards_stale_requests(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            bruhs_ui::init(cx);
            theme::apply_bruhs_theme(cx);
        });
        let (origin, window) = cx.add_window_view(|_, cx| ReactiveAutomations::new(cx));
        let destination = window.update(|_, cx| cx.new(ReactiveAutomations::new));
        let first = uuid::Uuid::new_v4();
        let second = uuid::Uuid::new_v4();
        origin.update_in(window, |panel, w, cx| {
            panel.snapshot = Some(ReactiveSnapshot::default());
            panel.edit_scheduled_task(None, w, cx);
            let Some(Editor::Automation(e)) = &mut panel.editor else {
                panic!("new draft")
            };
            e.select_trigger("schedule", w, cx);
            e.source
                .interval
                .update(cx, |field, cx| field.set_value("unfinished", w, cx));
            e.set_task_model("first-host-model", w, cx);
            e.set_project_path("/first-host", w, cx);
            e.task_enabled = false;
            panel.select_task_host(Some(first), cx);
            panel.select_task_host(Some(second), cx);
            assert!(!panel.awaits_task_host(first));
            assert!(panel.awaits_task_host(second));
            panel.review_automation(cx);
            assert!(panel.sent_for_test().is_empty());
            assert!(panel.error.is_none());
        });
        destination.update_in(window, |panel, w, cx| {
            panel.receive_task_draft(&origin, "/second-host", "second-host-model", w, cx);
            let Some(Editor::Automation(e)) = &panel.editor else {
                panic!("transferred draft")
            };
            assert!(e.is_new);
            assert_eq!(e.project_path(cx), "/second-host");
            assert_eq!(e.task_model(cx), "second-host-model");
            assert_eq!(super::value(&e.source.interval, cx), "unfinished");
            assert!(!e.task_enabled);
            assert!(e.host_request.is_none());
        });
        origin.read_with(window, |p, _| assert!(!p.scheduled_editor_open()));
    }

    #[gpui::test]
    fn scheduled_editor_preserves_calendar_interval_and_once_values(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            bruhs_ui::init(cx);
            theme::apply_bruhs_theme(cx);
        });
        let holder = std::rc::Rc::new(std::cell::RefCell::new(None));
        let capture = holder.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let panel = cx.new(ReactiveAutomations::new);
            *capture.borrow_mut() = Some(panel.clone());
            bruhs_ui::Root::new(panel, window, cx)
        });
        let panel = holder.borrow().clone().unwrap();
        panel.update_in(cx, |panel, window, cx| {
            panel.snapshot = Some(ReactiveSnapshot::default());
            panel.loaded = true;
            panel.edit_scheduled_task(None, window, cx);
            let Some(Editor::Automation(editor)) = &mut panel.editor else {
                panic!("new task")
            };
            for (field, text) in [
                (&editor.task_name, "Interval"),
                (&editor.agent.path, "/tmp"),
            ] {
                field.update(cx, |input, cx| input.set_value(text, window, cx));
            }
            editor.agent.instructions.update(cx, |input, cx| {
                input.set_value("Summarize changes", window, cx)
            });
            assert!(matches!(editor.build(None, None, cx).unwrap().source.kind, super::SourceKind::Calendar {times, weekdays, ..}
                if times == ["09:00"] && weekdays == [1,2,3,4,5]));
            editor.select_trigger("once", window, cx);
            assert!(matches!(editor.build(None, None, cx).unwrap().source.kind, super::SourceKind::Once {due_at}
                if due_at.as_millis() >= bruhs_models::EpochMs::now().as_millis()));
            editor.source.value.set_value("2030-05-06 11:32", window, cx);
            editor.select_trigger("schedule", window, cx);
            assert!(editor.schedule_uses_minutes());
            assert_eq!(super::value(&editor.source.interval, cx), "15");
            let mut saved = editor.build(None, None, cx).unwrap();
            assert_eq!(
                saved.source.kind,
                super::SourceKind::Schedule { every_ms: 900_000 }
            );
            editor.source.interval.update(cx, |field,cx| field.set_value("0.5",window,cx));
            assert!(editor.build(None, None, cx).unwrap_err().contains("at least one minute"));
            editor.source.interval.update(cx, |field,cx| field.set_value("1.25",window,cx));
            assert_eq!(editor.build(None, None, cx).unwrap().source.kind, super::SourceKind::Schedule { every_ms: 75_000 });
            editor.select_trigger("once", window, cx);
            assert_eq!(super::value(&editor.source.value, cx), "2030-05-06 11:32");
            editor.select_trigger("schedule", window, cx);
            assert_eq!(super::value(&editor.source.interval, cx), "1.25");
            for (millis, units, value) in [(3_600_000, true, "60"), (90_000, true, "1.5"), (5_500, false, "5.500")] {
                saved.source.kind = super::SourceKind::Schedule { every_ms: millis };
                panel.edit_scheduled_task(Some(saved.clone()), window, cx);
                let Some(Editor::Automation(editor)) = &panel.editor else {
                    panic!("existing task")
                };
                assert_eq!(editor.schedule_uses_minutes(), units);
                assert_eq!(super::value(&editor.source.interval, cx), value);
                assert_eq!(editor.build(None, None, cx).unwrap().source.kind, saved.source.kind);
            }
            // Empty weekdays in older sources mean every day; keep the original wire form.
            saved.source.kind = super::SourceKind::Calendar {
                times: vec!["08:15".into(), "16:45".into()], weekdays: vec![], time_zone: "Europe/Paris".into(),
            };
            panel.edit_scheduled_task(Some(saved.clone()), window, cx);
            let Some(Editor::Automation(editor)) = &mut panel.editor else { panic!("calendar") };
            editor.select_trigger("schedule", window, cx);
            editor.select_trigger("calendar", window, cx);
            assert_eq!(editor.build(None, None, cx).unwrap().source.kind, saved.source.kind);
            let original = bruhs_models::EpochMs::new(1_800_000_001_234).unwrap();
            saved.source.kind = super::SourceKind::Once {due_at: original};
            panel.edit_scheduled_task(Some(saved.clone()), window, cx);
            let Some(Editor::Automation(editor)) = &mut panel.editor else { panic!("once") };
            editor.select_trigger("calendar", window, cx);
            editor.select_trigger("once", window, cx);
            assert_eq!(editor.build(None, None, cx).unwrap().source.kind, saved.source.kind);
            saved.source.kind = super::SourceKind::Webhook;
            panel.edit_scheduled_task(Some(saved), window, cx);
            let Some(Editor::Automation(editor)) = &mut panel.editor else { panic!("legacy webhook") };
            assert!(editor.build(None, None, cx).unwrap().subscription.webhook_prompt.is_none());
            editor.start_webhook_prompt(window, cx);
            assert_eq!(super::value(editor.task_prompt(), cx), "Summarize changes");
            editor.task_prompt().update(cx, |input, cx| input.set_value("Review {{body.title}}", window, cx));
            let converted = editor.build(None, None, cx).unwrap();
            panel.edit_scheduled_task(Some(converted), window, cx);
            let Some(Editor::Automation(editor)) = &panel.editor else { panic!("saved template") };
            assert_eq!(super::value(editor.task_prompt(), cx), "Review {{body.title}}");
            assert_eq!(editor.build(None, None, cx).unwrap().agent.instructions, "Summarize changes");
        });
    }

    #[gpui::test]
    fn scheduled_webhook_form_keeps_selected_filter_and_thread_mode(cx: &mut gpui::TestAppContext) {
        use super::{TaskConversation, WebhookFilter, WebhookTest};
        cx.update(|cx| {
            bruhs_ui::init(cx);
            theme::apply_bruhs_theme(cx);
        });
        let holder = std::rc::Rc::new(std::cell::RefCell::new(None));
        let capture = holder.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let panel = cx.new(ReactiveAutomations::new);
            *capture.borrow_mut() = Some(panel.clone());
            bruhs_ui::Root::new(panel, window, cx)
        });
        cx.simulate_resize(gpui::size(gpui::px(900.0), gpui::px(2600.0)));
        let panel = holder.borrow().clone().unwrap();
        panel.update_in(cx, |panel, window, cx| {
            panel.snapshot = Some(ReactiveSnapshot::default());
            panel.loaded = true;
            panel.open_automation(None, window, cx);
            let Some(Editor::Automation(editor)) = &mut panel.editor else {
                panic!("expected the task editor");
            };
            editor.scheduled_form = true;
            editor.select_trigger("webhook", window, cx);
            assert_eq!(
                super::value(editor.task_prompt(), cx),
                super::DEFAULT_WEBHOOK_PROMPT
            );
            editor.task_prompt().update(cx, |input, cx| {
                input.set_value("Summarize {{body.issue}}.", window, cx)
            });
            editor
                .task_name
                .update(cx, |input, cx| input.set_value("Review", window, cx));
            editor.agent.instructions.update(cx, |input, cx| {
                input.set_value("Summarize the event.", window, cx)
            });
            editor
                .agent
                .path
                .update(cx, |input, cx| input.set_value("/tmp", window, cx));
            for (input, text) in [
                (&editor.dedupe.replay, "{{headers.delivery-id}}"),
                (&editor.dedupe.coalesce, "{{body.id}}"),
                (&editor.dedupe.seconds, "9"),
            ] {
                input.update(cx, |input, cx| input.set_value(text, window, cx));
            }
            assert!(editor
                .build(None, None, cx)
                .unwrap_err()
                .contains("10 to 3600"));
            editor
                .dedupe
                .seconds
                .update(cx, |input, cx| input.set_value("30", window, cx));
            let original = WebhookFilter {
                path: "headers.source".into(),
                test: WebhookTest::Equals("original".into()),
            };
            editor.filters.push(super::webhook_filters::FilterRow::new(
                Some(&original),
                window,
                cx,
            ));
        });
        cx.run_until_parked();
        for selector in [
            "task-webhook-signature",
            "task-signature-encoding",
            "task-signature-encoding-option-base64",
        ] {
            let bounds = cx
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("missing {selector}"));
            cx.simulate_click(bounds.center(), gpui::Modifiers::default());
            cx.run_until_parked();
        }
        panel.update_in(cx, |panel, window, cx| {
            let Some(Editor::Automation(editor)) = &mut panel.editor else {
                panic!("expected the task editor")
            };
            assert!(editor
                .build(None, None, cx)
                .unwrap_err()
                .contains("shared signing secret"));
            editor.security.secret.update(cx, |input, cx| {
                input.set_value("fixture-secret", window, cx)
            });
            editor
                .security
                .age
                .update(cx, |input, cx| input.set_value("1441", window, cx));
            assert!(editor.build(None, None, cx).unwrap_err().contains("1440"));
            editor
                .security
                .age
                .update(cx, |input, cx| input.set_value("15", window, cx));
        });
        for selector in [
            "webhook-test-1",
            "webhook-test-1-option-exists",
            "schedule-conversation",
            "schedule-conversation-option-continue",
        ] {
            let bounds = cx
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("missing {selector}"));
            cx.simulate_click(bounds.center(), gpui::Modifiers::default());
            cx.run_until_parked();
        }
        for selector in [
            "task-checkout",
            "task-checkout-option-existing",
            "reactive-Create task",
        ] {
            let bounds = cx
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("missing {selector}"));
            cx.simulate_click(bounds.center(), gpui::Modifiers::default());
            cx.run_until_parked();
        }
        assert!(cx.debug_bounds("task-save-error").is_some());
        for selector in ["task-checkout", "task-checkout-option-project"] {
            let bounds = cx
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("missing {selector}"));
            cx.simulate_click(bounds.center(), gpui::Modifiers::default());
            cx.run_until_parked();
        }
        assert!(cx.debug_bounds("reactive-Create task").is_some());
        panel.read_with(cx, |panel, cx| {
            let Some(Editor::Automation(editor)) = &panel.editor else {
                panic!("expected the task review");
            };
            let reviewed = editor.build(None, None, cx).expect("valid task");
            assert_eq!(
                reviewed.subscription.webhook_prompt.as_deref(),
                Some("Summarize {{body.issue}}.")
            );
            assert!(!reviewed.agent.instructions.contains("{{body.issue}}"));
            let policy = &reviewed.subscription.webhook_security;
            assert_eq!(policy.max_delivery_age_minutes, Some(15));
            assert_eq!(
                policy.signature.as_ref().unwrap().encoding,
                super::WebhookSignatureEncoding::Base64
            );
            assert!(!serde_json::to_string(&reviewed)
                .unwrap()
                .contains("fixture-secret"));
            assert_eq!(
                reviewed.subscription.webhook_dedupe,
                super::WebhookDedupe {
                    replay_key: "{{headers.delivery-id}}".into(),
                    coalesce_key: "{{body.id}}".into(),
                    coalesce_window_seconds: Some(30),
                }
            );
            assert_eq!(
                reviewed.subscription.task_conversation,
                TaskConversation::Continue
            );
            assert_eq!(
                reviewed.subscription.webhook_filters,
                vec![WebhookFilter {
                    path: "headers.source".into(),
                    test: WebhookTest::Exists,
                }]
            );
        });
    }

    #[gpui::test]
    fn review_builds_a_complete_unsaved_automation_without_sending_partial_commands(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            bruhs_ui::init(cx);
            theme::apply_bruhs_theme(cx);
        });
        let holder = std::rc::Rc::new(std::cell::RefCell::new(None));
        let capture = holder.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let panel = cx.new(ReactiveAutomations::new);
            *capture.borrow_mut() = Some(panel.clone());
            bruhs_ui::Root::new(panel, window, cx)
        });
        let panel = holder.borrow().clone().unwrap();
        panel.update_in(cx, |panel, window, cx| {
            panel.snapshot = Some(ReactiveSnapshot::default());
            panel.loaded = true;
            panel.open_automation(None, window, cx);
            let Some(Editor::Automation(editor)) = &mut panel.editor else {
                panic!("automation editor");
            };
            editor.agent.name.update(cx, |input, cx| {
                input.set_value("Scan assistant", window, cx)
            });
            editor.agent.instructions.update(cx, |input, cx| {
                input.set_value(
                    "Extract payment details and prepare a reviewable draft.",
                    window,
                    cx,
                )
            });
            editor
                .agent
                .path
                .update(cx, |input, cx| input.set_value("/tmp", window, cx));
            editor.source.value.set_value("/tmp/scans", window, cx);
            panel.review_automation(cx);
            let Some(Editor::Automation(editor)) = &panel.editor else {
                panic!("automation editor");
            };
            let setup: &AutomationSetup = editor.review.as_ref().unwrap();
            assert!(setup.enabled());
            assert!(setup.subscription.auto_run);
            assert_eq!(setup.agent.id, setup.subscription.agent_id);
            assert_eq!(setup.source.id, setup.subscription.source_id);
            assert!(review_fields(setup, AccessLevel::ReadOnly)
                .iter()
                .any(|(label, value)| *label == "Access" && value.starts_with("Read only.")));
            let full = review_fields(setup, AccessLevel::Full);
            assert!(full
                .iter()
                .any(|(label, value)| *label == "Access" && value.starts_with("Full access.")));
            assert!(!full.iter().any(|(_, value)| value.contains("ead-only")));
            assert!(panel.snapshot.as_ref().unwrap().agents.is_empty());
            assert!(!panel.busy);
            let mut legacy = setup.clone();
            legacy.subscription.auto_run = false;
            legacy.subscription.enabled = false;
            panel.open_automation(Some(legacy), window, cx);
            panel.review_automation(cx);
            let Some(Editor::Automation(editor)) = &panel.editor else {
                panic!("automation editor");
            };
            assert!(!editor.review.as_ref().unwrap().subscription.auto_run);
            assert!(panel.snapshot.as_ref().unwrap().subscriptions.is_empty());
            assert!(!panel.busy);
        });
    }
    #[test]
    fn app_and_newer_sources_describe_themselves() {
        use super::{source_details, source_summary, SourceKind, FROM_NEWER_SOURCE};
        let app = SourceKind::AppPoll {
            app: "github".into(),
            account: "v1-g".into(),
            query: bruhs_models::apps::AppQuery::GithubPulls { repo: "o/r".into() },
            poll_ms: 60_000,
        };
        assert_eq!(
            source_summary(&app, None),
            "GitHub: new pull requests in o/r"
        );
        assert_eq!(
            source_details(&app, None),
            "GitHub: new pull requests in o/r\nCheck interval: 60 seconds"
        );
        let mail = SourceKind::AppPoll {
            app: "mail".into(),
            account: "m1".into(),
            query: bruhs_models::apps::AppQuery::NewMail,
            poll_ms: 300_000,
        };
        assert_eq!(source_summary(&mail, None), "Email: new mail in your mail");
        assert_eq!(
            source_summary(&mail, Some("me@x.test")),
            "Email: new mail in me@x.test"
        );
        assert_eq!(source_summary(&SourceKind::Other, None), FROM_NEWER_SOURCE);
        assert_eq!(source_details(&SourceKind::Other, None), FROM_NEWER_SOURCE);
    }
}
