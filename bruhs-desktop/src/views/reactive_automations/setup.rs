//! One review and one atomic command activate a complete automation.
use super::*;

pub(super) struct AutomationEditor {
    agent: AgentEditor,
    source: SourceEditor,
    subscription: SubscriptionEditor,
    review: Option<AutomationSetup>,
    advanced: bool,
}

impl AutomationEditor {
    pub(super) fn source_mut(&mut self) -> &mut SourceEditor {
        &mut self.source
    }
    pub(super) fn subscription_mut(&mut self) -> &mut SubscriptionEditor {
        &mut self.subscription
    }
}

impl ReactiveAutomations {
    pub(super) fn open_automation(
        &mut self,
        setup: Option<AutomationSetup>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy || self.snapshot.is_none() {
            return;
        }
        let is_new = setup.is_none();
        self.open_agent(setup.as_ref().map(|s| s.agent.clone()), window, cx);
        let Some(Editor::Agent(agent)) = self.editor.take() else {
            return;
        };
        self.open_source(setup.as_ref().map(|s| s.source.clone()), window, cx);
        let Some(Editor::Source(mut source)) = self.editor.take() else {
            return;
        };
        let source_text = value(&source.value, cx);
        source.value = edit_input(&source_text, source.kind == "command", window, cx);
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
        self.editor = Some(Editor::Automation(Box::new(AutomationEditor {
            agent,
            source,
            subscription,
            review: None,
            advanced: false,
        })));
        self.error = None;
        cx.notify();
    }
    pub(super) fn review_automation(&mut self, cx: &mut Context<'_, Self>) {
        let result = match self.editor.as_ref() {
            Some(Editor::Automation(editor)) => editor.build(cx),
            _ => return,
        };
        match result {
            Ok(setup) => {
                if let Some(Editor::Automation(editor)) = &mut self.editor {
                    editor.review = Some(setup);
                }
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }
    pub(super) fn render_automation_editor(
        &self,
        editor: &AutomationEditor,
        cx: &Context<'_, Self>,
    ) -> Div {
        if let Some(setup) = &editor.review {
            let mut body = card().child(heading("Review automation"));
            for (label, text) in review_fields(setup) {
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
            body=body.child(muted(if setup.subscription.auto_run{"Runs read-only drafts automatically. Writes still use workflow review."}else{"Each event waits for review before a read-only draft runs. Writes still use workflow review."})).child(muted("The local server must be running. This enables the complete automation in one saved change."));
            return body.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(self.command(
                        "automation-enable".into(),
                        "Enable automation",
                        apply(ReactiveChange::SaveAutomation {
                            automation: Box::new(setup.clone()),
                        }),
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
        let mut body=card().child(heading("Set up an automation")).child(muted("Describe the outcome and choose what starts it. Nothing is saved or enabled until you review the complete automation."))
            .child(field("Automation name",&editor.agent.name,false,self.busy)).child(field("What should this automation do?",&editor.agent.instructions,true,self.busy)).child(field("Automation working folder",&editor.agent.path,false,self.busy)).child(field("Model",&editor.agent.model,false,self.busy)).child(muted("Starts when"))
            .child(div().flex().flex_wrap().gap_2().children([("folder","A file arrives"),("http","A website changes"),("command","A command reports a change"),("schedule","On a repeating schedule"),("calendar","At set times of day"),("webhook","A webhook arrives"),("app_event","A meeting event occurs"),("custom","A custom event arrives")].into_iter().map(|(kind,label)|control(SharedString::from(format!("automation-kind-{kind}")),label,self.busy).aria_selected(editor.source.kind==kind).when(editor.source.kind==kind,|button|button.bg(theme::blueberry_500().opacity(0.15))).on_activate(cx,move |this,window,cx|{if let Some(Editor::Automation(editor))=&mut this.editor{editor.source.kind=kind;editor.source.value=edit_input(if kind=="app_event"{"meeting.completed"}else{""},kind=="command",window,cx);if kind=="schedule"{editor.source.interval.update(cx,|input,cx|input.set_value("3600",window,cx));}}cx.notify();}))));
        body=match editor.source.kind {
            "folder"=>body.child(field("Folder to watch",&editor.source.value,false,self.busy)).child(field("File pattern",&editor.source.pattern,false,self.busy)),
            "http"=>body.child(field("Website URL",&editor.source.value,false,self.busy)),
            "command"=>body.child(field("Command to run",&editor.source.value,true,self.busy)).child(muted("This command runs on your machine once you enable the automation.")),
            "schedule"=>body.child(field("Run every, in seconds",&editor.source.interval,false,self.busy)),
            "app_event"=>body.child(div().flex().flex_wrap().gap_2().children([("meeting.completed","Meeting completed"),("meeting.transcribed","Final transcript updated"),("meeting.summary","Summary ready")].into_iter().map(|(topic,label)|{let selected=value(&editor.source.value,cx)==topic;control(SharedString::from(format!("automation-topic-{topic}")),label,self.busy).aria_selected(selected).when(selected,|button|button.bg(theme::blueberry_500().opacity(0.15))).on_activate(cx,move |this,window,cx|{if let Some(Editor::Automation(editor))=&mut this.editor{editor.source.value.update(cx,|input,cx|input.set_value(topic,window,cx));}cx.notify();})}))),
            "calendar"=>body.child(self.render_calendar_fields(&editor.source, cx)),
            "webhook"=>body.child(muted("A private endpoint is created when you enable this automation. Reveal its token in Advanced settings.")),
            _=>body.child(muted("Device bridges can publish through the local API. Use a watched folder for FTP uploads.")),
        };
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
                .child(field(
                    "Automation memory",
                    &editor.agent.memory,
                    true,
                    self.busy,
                ))
                .child(field(
                    "Event-specific instructions",
                    &editor.subscription.instructions,
                    true,
                    self.busy,
                ))
                .child(
                    control(
                        "automation-auto-run",
                        if editor.subscription.subscription.auto_run {
                            "Automatic read-only drafts: On"
                        } else {
                            "Review each event before running"
                        },
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
        body.child(
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .child(
                    control("automation-review", "Review automation", self.busy)
                        .on_activate(cx, |this, _, cx| this.review_automation(cx)),
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
        )
    }
    pub(super) fn render_automation_cards(&self, cx: &Context<'_, Self>) -> Div {
        let mut body = div().flex().flex_col().gap_3();
        let Some(snapshot) = &self.snapshot else {
            return body;
        };
        let mut shown_agents = HashSet::new();
        if snapshot.subscriptions.is_empty() {
            body=body.child(muted("No automations yet. Start with a scan folder, a meeting event, or a repeating check."));
        }
        for setup in snapshot
            .subscriptions
            .iter()
            .filter_map(|subscription| AutomationSetup::from_subscription(snapshot, subscription))
        {
            let active = setup.enabled();
            let edit = setup.clone();
            let chat_id = setup.agent.id.clone();
            let mut row = card()
                .child(heading(setup.agent.name.clone()))
                .child(muted(setup.agent.instructions.clone()))
                .child(muted(source_summary(&setup.source.kind)))
                .child(muted(automation_status(&setup)))
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
                        .on_activate(cx, move |this, window, cx| {
                            this.open_automation(Some(edit.clone()), window, cx)
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
                        .on_activate(cx, move |this, window, cx| {
                            this.editor = Some(Editor::Chat {
                                agent_id: chat_id.clone(),
                                message: edit_input("", true, window, cx),
                            });
                            cx.notify();
                        }),
                    ),
            );
            if shown_agents.insert(setup.agent.id.clone()) {
                row = row.child(self.render_agent_records(&setup.agent.id, cx));
            }
            body = body.child(row);
        }
        body
    }
}

impl AutomationEditor {
    fn build(&self, cx: &App) -> Result<AutomationSetup, String> {
        let mut agent = self.agent.agent.clone();
        agent.name = value(&self.agent.name, cx);
        agent.instructions = value(&self.agent.instructions, cx);
        agent.project_path = value(&self.agent.path, cx);
        agent.model = value(&self.agent.model, cx);
        agent.memory = value(&self.agent.memory, cx);
        agent.enabled = true;
        if agent.name.trim().is_empty()
            || agent.instructions.trim().is_empty()
            || agent.project_path.trim().is_empty()
        {
            return Err(
                "Enter an automation name, purpose, and working folder before reviewing.".into(),
            );
        }
        bruhs_models::agent::ModelId::try_from(agent.model.clone())
            .map_err(|error| error.to_string())?;
        let mut source = self.source.source.clone();
        if source.name.trim().is_empty() {
            source.name = format!("{} source", agent.name);
        }
        source.enabled = true;
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
            "schedule" => SourceKind::Schedule {
                every_ms: interval?,
            },
            "calendar" => super::editor_fields::calendar_kind(
                &value(&self.source.times, cx),
                &self.source.weekdays,
                &value(&self.source.time_zone, cx),
            )?,
            "webhook" => SourceKind::Webhook,
            "app_event" => SourceKind::AppEvent { topic: text },
            "custom" => SourceKind::Custom,
            _ => return Err("Choose a supported source type.".into()),
        };
        let mut subscription = self.subscription.subscription.clone();
        subscription.instructions = value(&self.subscription.instructions, cx);
        subscription.debounce_ms = milliseconds(&value(&self.subscription.debounce, cx), true)?;
        subscription.throttle_ms = milliseconds(&value(&self.subscription.throttle, cx), true)?;
        subscription.enabled = true;
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
fn source_summary(kind: &SourceKind) -> String {
    match kind {
        SourceKind::Folder { path, pattern, .. } => format!("Folder: {path} ({pattern})"),
        SourceKind::Http { url, .. } => format!("Website: {url}"),
        SourceKind::Command { command, .. } => format!("Command: {command}"),
        SourceKind::Schedule { every_ms } => format!("Every {} seconds", seconds(*every_ms)),
        SourceKind::Calendar {
            times,
            weekdays,
            time_zone,
        } => describe_calendar(times, weekdays, time_zone),
        SourceKind::Webhook => "Authenticated webhook".into(),
        SourceKind::AppEvent { topic } => format!("App event: {topic}"),
        SourceKind::Custom => "Custom events from your integrations".into(),
    }
}
fn automation_status(setup: &AutomationSetup) -> &'static str {
    if !setup.subscription.enabled {
        "Paused"
    } else if !setup.agent.enabled {
        "Paused: agent disabled"
    } else if !setup.source.enabled {
        "Paused: source disabled"
    } else if setup.subscription.auto_run {
        "Enabled: automatic read-only drafts"
    } else {
        "Enabled: review each event"
    }
}
fn source_details(kind: &SourceKind) -> String {
    match kind {
        SourceKind::Folder{path,pattern,recursive,settle_ms,poll_ms}=>format!("Folder: {path}\nFiles: {pattern}\nSubfolders: {}\nFile settle time: {} seconds\nCheck interval: {} seconds",if *recursive{"Included"}else{"Excluded"},seconds(*settle_ms),seconds(*poll_ms)),
        SourceKind::Http{url,poll_ms}=>format!("URL: {url}\nCheck interval: {} seconds",seconds(*poll_ms)),
        SourceKind::Command{command,cwd,poll_ms,timeout_ms}=>format!("Command: {command}\nWorking folder: {cwd}\nCheck interval: {} seconds\nTimeout: {} seconds\nSource command access: Runs with the local server account's permissions. The agent's read-only tool policy does not restrict this command.",seconds(*poll_ms),seconds(*timeout_ms)),
        SourceKind::Schedule{every_ms}=>format!("Repeat every {} seconds",seconds(*every_ms)),
        SourceKind::Calendar{times,weekdays,time_zone}=>describe_calendar(times,weekdays,time_zone),
        SourceKind::Webhook=>"Authenticated incoming webhook. The private token is available after enabling.".into(),
        SourceKind::AppEvent{topic}=>format!("App event: {topic}"),
        SourceKind::Custom=>"Events published by your integrations through the local API.".into(),
    }
}

pub(super) fn review_fields(setup: &AutomationSetup) -> Vec<(&'static str, String)> {
    vec![
        ("Name", setup.agent.name.clone()),
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
            if setup.subscription.auto_run {
                "Automatic read-only drafts"
            } else {
                "Review each event before running a read-only draft"
            }
            .into(),
        ),
        ("Starts when", source_summary(&setup.source.kind)),
        (
            "Complete source settings",
            source_details(&setup.source.kind),
        ),
        ("Working folder", setup.agent.project_path.clone()),
        ("Model", setup.agent.model.clone()),
        (
            if matches!(setup.source.kind, SourceKind::Command { .. }) {
                "Agent access"
            } else {
                "Access"
            },
            "Read-only drafts. No direct writes or payments.".into(),
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
    ]
}

#[cfg(test)]
mod tests {
    use super::{review_fields, AutomationSetup, Editor, ReactiveAutomations, ReactiveSnapshot};
    use crate::theme;
    use gpui::AppContext as _;
    #[gpui::test]
    fn review_builds_a_complete_unsaved_automation_without_sending_partial_commands(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_component::init(cx);
            theme::apply_bruhs_theme(cx);
        });
        let holder = std::rc::Rc::new(std::cell::RefCell::new(None));
        let capture = holder.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let panel = cx.new(ReactiveAutomations::new);
            *capture.borrow_mut() = Some(panel.clone());
            gpui_component::Root::new(panel, window, cx)
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
            editor
                .source
                .value
                .update(cx, |input, cx| input.set_value("/tmp/scans", window, cx));
            panel.review_automation(cx);
            let Some(Editor::Automation(editor)) = &panel.editor else {
                panic!("automation editor");
            };
            let setup: &AutomationSetup = editor.review.as_ref().unwrap();
            assert!(setup.enabled());
            assert!(setup.subscription.auto_run);
            assert_eq!(setup.agent.id, setup.subscription.agent_id);
            assert_eq!(setup.source.id, setup.subscription.source_id);
            assert!(review_fields(setup)
                .iter()
                .any(|(label, value)| *label == "Access" && value.contains("Read-only")));
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
}
