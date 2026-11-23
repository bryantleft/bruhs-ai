//! Device-owned usage sources in Providers & updates.
use super::provider_editor::editor_button;
use super::*;
use bruhs_core::{
    secret_sources::{validate_reference, SecretReference},
    usage_hubs::{self as api, AccountUsage, Hub, ManagementKey},
};
use bruhs_ui::dropdown::saved_option_value;

struct AddForm {
    url: Entity<InputState>,
    key: Entity<InputState>,
    label: Entity<InputState>,
    reference: Entity<InputState>,
    source: Option<String>,
    _events: Vec<Subscription>,
}
#[derive(Clone)]
struct CreditChoice {
    hub: Hub,
    account: String,
    label: String,
    credit: api::ResetCredit,
}
enum CreditResult {
    Pending,
    Finished(String),
    Retry(String),
}
struct CreditAction {
    choice: CreditChoice,
    result: CreditResult,
}
enum Dialog {
    Add(AddForm),
    Remove(Hub),
    Redeem(CreditChoice),
}
type LinkedUsageReport = (String, Option<Result<Vec<AccountUsage>, String>>);

pub(super) struct UsageHubs {
    pub(super) open_workbench_usage: bool,
    rows: Vec<Hub>,
    device_accounts: Vec<AccountUsage>,
    linked: Vec<LinkedUsageReport>,
    linked_revision: u64,
    reports: BTreeMap<String, Result<Vec<AccountUsage>, String>>,
    generation: u64,
    loading: bool,
    pending: usize,
    loaded: bool,
    busy: bool,
    error: Option<String>,
    dialog: Option<Dialog>,
    credit_action: Option<CreditAction>,
    start: FocusHandle,
    end: FocusHandle,
    add: FocusHandle,
    cancel: FocusHandle,
    trigger: Option<FocusHandle>,
}
impl UsageHubs {
    pub(super) fn new(cx: &App) -> Self {
        Self {
            open_workbench_usage: false,
            rows: vec![],
            device_accounts: vec![],
            linked: Vec::new(),
            linked_revision: 0,
            reports: Default::default(),
            generation: 0,
            loading: false,
            pending: 0,
            loaded: false,
            busy: false,
            error: None,
            dialog: None,
            credit_action: None,
            start: cx.focus_handle(),
            end: cx.focus_handle(),
            add: cx.focus_handle().tab_stop(true),
            cancel: cx.focus_handle().tab_stop(true),
            trigger: None,
        }
    }
    fn account_groups(&self, linked_revision: u64) -> Vec<api::AccountGroup<'_>> {
        let mut readings = self
            .device_accounts
            .iter()
            .map(|account| api::Reading {
                origin: api::Origin::Device,
                account,
            })
            .collect::<Vec<_>>();
        for hub in &self.rows {
            if let Some(Ok(accounts)) = self.reports.get(&hub.url) {
                readings.extend(accounts.iter().map(|account| api::Reading {
                    origin: api::Origin::Hub(hub),
                    account,
                }));
            }
        }
        if linked_revision == self.linked_revision {
            for (name, result) in &self.linked {
                if let Some(Ok(accounts)) = result {
                    readings.extend(accounts.iter().map(|account| api::Reading {
                        origin: api::Origin::Environment(name),
                        account,
                    }));
                }
            }
        }
        api::group_accounts(readings)
    }

    pub(super) fn bounds(&self) -> Option<(FocusHandle, FocusHandle)> {
        self.dialog
            .as_ref()
            .map(|_| (self.start.clone(), self.end.clone()))
    }
    pub(super) fn close(&mut self) {
        self.dialog = None;
        self.trigger = None;
    }
}
impl Workspace {
    pub(super) fn load_usage_hubs(&mut self, cx: &mut Context<'_, Self>) {
        if self.usage_hubs.busy || self.usage_hubs.loading || self.usage_hubs.pending != 0 {
            return;
        }
        self.usage_hubs.loading = true;
        let generation = self.usage_hubs.generation;
        let task = cx.background_spawn(async { api::list() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |ws, cx| {
                if ws.usage_hubs.generation != generation {
                    return;
                }
                ws.usage_hubs.loading = false;
                match result {
                    Ok(rows) => {
                        ws.usage_hubs.error = None;
                        ws.probe_usage_hubs(rows, cx);
                    }
                    Err(e) => ws.usage_hubs.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn probe_usage_hubs(&mut self, rows: Vec<Hub>, cx: &mut Context<'_, Self>) {
        self.usage_hubs.generation += 1;
        self.usage_hubs.loaded = true;
        let generation = self.usage_hubs.generation;
        self.usage_hubs.reports.retain(|url, _| {
            rows.iter()
                .any(|new| &new.url == url && self.usage_hubs.rows.contains(new))
        });
        self.usage_hubs.rows = rows.clone();
        self.usage_hubs.pending = rows.len();
        let Some(runtime) = cx
            .try_global::<DesktopState>()
            .map(|state| state.runtime.clone())
        else {
            self.usage_hubs.pending = 0;
            self.usage_hubs.error = Some("The desktop usage service is unavailable.".into());
            return;
        };
        {
            self.usage_hubs.pending += 1;
            let probe = runtime.spawn(api::device_accounts());
            cx.spawn(async move |this, cx| {
                let accounts = probe.await.unwrap_or_default();
                let _ = this.update(cx, |ws, cx| {
                    if ws.usage_hubs.generation == generation {
                        ws.usage_hubs.pending -= 1;
                        ws.usage_hubs.device_accounts = accounts;
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        self.usage_hubs.linked.clear();
        self.usage_hubs.linked_revision = self.linked_projects.usage_revision();
        for (name, read) in self.linked_projects.usage_readers() {
            let index = self.usage_hubs.linked.len();
            self.usage_hubs.linked.push((name, None));
            self.usage_hubs.pending += 1;
            let task = runtime.spawn(read);
            cx.spawn(async move |this, cx| {
                let result = task
                    .await
                    .unwrap_or_else(|_| Err("The usage check was interrupted.".into()));
                let _ = this.update(cx, |ws, cx| {
                    if ws.usage_hubs.generation == generation {
                        ws.usage_hubs.pending -= 1;
                        ws.usage_hubs.linked[index].1 = Some(result);
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        for hub in rows {
            let id = hub.url.clone();
            let task =
                runtime.spawn(async move { api::report(&hub).await.map_err(|e| e.to_string()) });
            cx.spawn(async move |this, cx| {
                let result = task
                    .await
                    .unwrap_or_else(|_| Err("The hub usage check was interrupted.".into()));
                let _ = this.update(cx, |ws, cx| {
                    if ws.usage_hubs.generation == generation {
                        ws.usage_hubs.pending -= 1;
                        ws.usage_hubs.reports.insert(id, result);
                        cx.notify();
                    }
                });
            })
            .detach();
        }
    }

    fn open_hub(&mut self, previous: Option<Hub>, w: &mut Window, cx: &mut Context<'_, Self>) {
        if self.usage_hubs.busy {
            return;
        }
        self.usage_hubs.trigger = w.focused(cx);
        self.usage_hubs.error = None;
        self.usage_hubs.dialog = Some(match previous {
            Some(hub) => {
                self.usage_hubs.cancel.focus(w, cx);
                Dialog::Remove(hub)
            }
            None => {
                let sources = self.credential_settings.read(cx).secret_sources();
                sources.update(cx, |view, cx| view.reload(cx));
                let mut field = |placeholder: &'static str, masked| {
                    cx.new(|cx| {
                        InputState::new(w, cx)
                            .placeholder(placeholder)
                            .masked(masked)
                    })
                };
                let url = field("https://hub.example.ts.net:8318", false);
                let key = field("Not set", true);
                let label = field("Defaults to the hub’s host name", false);
                let reference = field("op://vault/item/field", false);
                let events = [&url, &key, &reference]
                    .into_iter()
                    .map(|i| {
                        cx.subscribe(i, |ws, _, e: &InputEvent, cx| {
                            if matches!(e, InputEvent::Change) {
                                ws.usage_hubs.error = None;
                                cx.notify();
                            }
                        })
                    })
                    .collect();
                url.update(cx, |i, cx| i.focus(w, cx));
                Dialog::Add(AddForm {
                    url,
                    key,
                    label,
                    reference,
                    source: None,
                    _events: events,
                })
            }
        });
        cx.notify();
    }
    pub(super) fn select_hub_key_source(&mut self, value: &str) {
        if let Some(Dialog::Add(form)) = &mut self.usage_hubs.dialog {
            form.source = value.strip_prefix("source:").map(str::to_owned);
        }
    }
    pub(super) fn dismiss_hub(&mut self, w: &mut Window, cx: &mut Context<'_, Self>) {
        if self.usage_hubs.busy {
            return;
        }
        if self.settings_open_dropdown().is_some() {
            self.set_settings_open_dropdown(None);
        } else {
            self.usage_hubs.dialog = None;
            self.usage_hubs.error = None;
            self.usage_hubs
                .trigger
                .take()
                .unwrap_or_else(|| self.usage_hubs.add.clone())
                .focus(w, cx);
        }
        cx.notify();
    }
    fn can_add_hub(&self, f: &AddForm, cx: &App) -> bool {
        api::normalize_url(&f.url.read(cx).value()).is_ok()
            && match &f.source {
                Some(id) => {
                    self.credential_settings
                        .read(cx)
                        .secret_sources()
                        .read(cx)
                        .sources()
                        .contains_key(id)
                        && validate_reference(f.reference.read(cx).value().trim()).is_ok()
                }
                None => !f.key.read(cx).value().trim().is_empty(),
            }
    }
    fn confirm_credit(&mut self, choice: CreditChoice, w: &mut Window, cx: &mut Context<'_, Self>) {
        if self.usage_hubs.busy {
            return;
        }
        self.usage_hubs.error = None;
        self.usage_hubs.trigger = w.focused(cx);
        self.set_settings_open_dropdown(None);
        self.usage_hubs.dialog = Some(Dialog::Redeem(choice));
        self.usage_hubs.cancel.focus(w, cx);
        cx.notify();
    }

    fn use_credit(&mut self, choice: CreditChoice, w: &mut Window, cx: &mut Context<'_, Self>) {
        self.dismiss_hub(w, cx);
        self.usage_hubs.busy = true;
        self.usage_hubs.credit_action = Some(CreditAction {
            choice: choice.clone(),
            result: CreditResult::Pending,
        });
        let job = crate::app_state::spawn_tokio(cx, async move {
            api::redeem(&choice.hub, &choice.credit).await
        });
        cx.spawn(async move |ws, cx| {
            let result = job.await;
            let _ = ws.update(cx, |ws, cx| {
                ws.usage_hubs.busy = false;
                let Some(action) = ws.usage_hubs.credit_action.as_mut() else {
                    return;
                };
                action.result = match result {
                    Ok(outcome) => {
                        let mut message = outcome.message.to_owned();
                        if outcome.cooldown_warning {
                            message.push_str(" The hub cooldown could not be cleared. Routing may resume when it expires.");
                        }
                        CreditResult::Finished(message)
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => {
                        CreditResult::Finished(e.to_string())
                    }
                    Err(e) => CreditResult::Retry(e.to_string()),
                };
                // A poll that began before redemption may still be in flight.
                ws.usage_hubs.generation += 1;
                ws.usage_hubs.loading = false;
                ws.usage_hubs.pending = 0;
                ws.load_usage_hubs(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn credit_controls(
        &self,
        hub: &Hub,
        id: &str,
        label: &str,
        bank: Option<&api::ResetCredits>,
        action: Option<&CreditAction>,
        cx: &Context<'_, Self>,
    ) -> Div {
        let mut controls = div().flex().flex_col().gap_1();
        let mut retry = None;
        if let Some(action) = action {
            let message = match &action.result {
                CreditResult::Pending => "Using reset credit…",
                CreditResult::Finished(s) => s,
                CreditResult::Retry(s) => {
                    retry = Some(action.choice.clone());
                    s
                }
            };
            controls = controls.child(
                div()
                    .id("credit-status")
                    .role(Role::Status)
                    .child(message.to_owned()),
            );
        }
        let mut line = div()
            .flex()
            .items_center()
            .flex_wrap()
            .justify_between()
            .gap_2();
        let mut choice = None;
        if let Some(bank) = bank {
            let mut summary = match bank.count {
                0 => "No reset credits banked".into(),
                1 => "1 reset credit banked".into(),
                n => format!("{n} reset credits banked"),
            };
            if let Some(credit) = &bank.next {
                let remaining = credit.expires_at.signed_duration_since(chrono::Utc::now());
                if remaining > chrono::Duration::zero() {
                    let time = super::thread_details::usage::countdown(remaining.num_seconds());
                    summary.push_str(&format!(" · next expires in {time}"));
                    choice = Some(CreditChoice {
                        hub: hub.clone(),
                        account: id.to_owned(),
                        label: label.to_owned(),
                        credit: credit.clone(),
                    });
                }
            }
            line = line.child(summary);
        }
        let label = if retry.is_some() {
            "Retry reset"
        } else {
            "Use reset"
        };
        if let Some(choice) = retry.or(choice) {
            line = line.child(editor_button(
                format!("usage-credit-{}-{}", choice.hub.url, choice.account),
                label,
                WebButtonVariant::Outline,
                self.usage_hubs.busy,
                cx,
                move |ws, w, cx| ws.confirm_credit(choice.clone(), w, cx),
            ));
        }
        controls.child(line)
    }

    fn submit_hub(&mut self, w: &mut Window, cx: &mut Context<'_, Self>) {
        if self.usage_hubs.busy {
            return;
        }
        let task = match &self.usage_hubs.dialog {
            Some(Dialog::Add(f)) if self.can_add_hub(f, cx) => {
                let url = f.url.read(cx).value().trim().to_owned();
                let label = f.label.read(cx).value().to_string();
                let previous = api::normalize_url(&url)
                    .ok()
                    .and_then(|url| {
                        self.usage_hubs
                            .rows
                            .iter()
                            .find(|row| row.url == url.as_str())
                    })
                    .cloned();
                let key = match &f.source {
                    Some(source) => ManagementKey::Reference(SecretReference {
                        source: source.clone(),
                        reference: f.reference.read(cx).value().trim().into(),
                    }),
                    None => ManagementKey::Device(f.key.read(cx).value().trim().to_owned().into()),
                };
                cx.background_spawn(async move { api::save(&url, &label, key, previous.as_ref()) })
            }
            Some(Dialog::Redeem(choice)) => {
                self.use_credit(choice.clone(), w, cx);
                return;
            }
            Some(Dialog::Remove(hub)) => {
                let hub = hub.clone();
                cx.background_spawn(async move { api::remove(&hub) })
            }
            _ => return,
        };
        self.usage_hubs.busy = true;
        self.usage_hubs.generation += 1;
        self.usage_hubs.loading = false;
        self.usage_hubs.pending = 0;
        cx.spawn_in(w, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |ws, w, cx| {
                ws.usage_hubs.busy = false;
                match result {
                    Ok(rows) => {
                        let displayed = ws.usage_hubs.dialog.is_some() && ws.settings_visible();
                        ws.usage_hubs.close();
                        ws.usage_hubs.error = None;
                        if displayed {
                            ws.usage_hubs.add.focus(w, cx);
                        }
                        ws.probe_usage_hubs(rows, cx);
                    }
                    Err(e) => ws.usage_hubs.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn pooled_usage_row(
        &self,
        index: usize,
        group: &api::AccountGroup<'_>,
        action: Option<&CreditAction>,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let sample = group.quota.account;
        let label = &group.readings[0].account.label;
        let mut title = if label == sample.provider {
            label.clone()
        } else {
            format!("{} · {label}", sample.provider)
        };
        if group.show_workspace {
            title.push_str(" · ");
            title.push_str(group.workspace.unwrap_or("Unspecified workspace"));
        }
        let mut sources = Vec::new();
        for reading in &group.readings {
            let label = reading.origin.label();
            if !sources.contains(&label) {
                sources.push(label);
            }
        }
        let mut row = div()
            .id(("usage-account", index))
            .flex()
            .flex_col()
            .gap_1()
            .px_4()
            .py_2()
            .text_xs()
            .text_color(theme::rambutan_400())
            .child(
                div()
                    .text_sm()
                    .text_color(theme::rambutan_100())
                    .child(title),
            )
            .child(sources.join(" · "));
        row = match &sample.windows {
            Err(e) => row.child(e.to_string()),
            Ok(w) if w.is_empty() => row.child("No quota windows reported."),
            Ok(windows) => row.children(windows.iter().map(|w| {
                super::thread_details::usage::window_row(w, chrono::Utc::now().timestamp())
            })),
        };
        if sample.windows.as_ref().is_ok_and(|w| !w.is_empty()) {
            for reading in &group.readings {
                match &reading.account.windows {
                    Err(e) => row = row.child(format!("{} · {e}", reading.origin.label())),
                    Ok(w) if matches!(reading.origin, api::Origin::Device) && w.is_empty() => {
                        row = row.child(format!("{} · No limits reported.", reading.origin.label()))
                    }
                    _ => {}
                }
            }
        }
        if let Some(api::Reading {
            origin: api::Origin::Hub(hub),
            account,
        }) = group.credits
        {
            row = row.child(self.credit_controls(
                hub,
                &account.id,
                &account.label,
                account.credits.as_ref(),
                action,
                cx,
            ));
        } else if let Some(a) = action {
            row = row.child(self.credit_controls(
                &a.choice.hub,
                &a.choice.account,
                &a.choice.label,
                None,
                Some(a),
                cx,
            ));
        }
        row.into_any_element()
    }

    pub(super) fn usage_hubs_card(&self, cx: &Context<'_, Self>) -> AnyElement {
        let state = &self.usage_hubs;
        let mut rows = Vec::new();
        let revision = self.linked_projects.usage_revision();
        let linked = if state.linked_revision == revision {state.linked.as_slice()} else {&[]};
        if linked.is_empty() && state.rows.is_empty() && state.device_accounts.is_empty() {
            rows.push(
                div()
                    .px_4()
                    .py_3()
                    .text_sm()
                    .text_color(theme::rambutan_400())
                    .child(if state.error.is_some() {
                        "Usage providers unavailable."
                    } else if state.loading || state.pending != 0 || !state.loaded {
                        "Checking account usage…"
                    } else {
                        "No subscription limits reported."
                    })
                    .into_any_element(),
            );
        }
        for hub in &state.rows {
            let target = hub.clone();
            let remove = editor_button(
                format!("usage-hub-remove-{}", hub.url),
                "Remove",
                WebButtonVariant::Outline,
                state.busy,
                cx,
                move |ws, w, cx| ws.open_hub(Some(target.clone()), w, cx),
            );
            rows.push(self.settings_row(
                format!("usage-hub-{}", hub.url),
                hub.label.clone(),
                format!("CLI Proxy · {}", hub.url),
                remove,
            ));
            let status = match state.reports.get(&hub.url) {
                None => Some("Checking account usage…".into()),
                Some(Err(e)) => Some(e.clone()),
                Some(Ok(accounts)) if accounts.is_empty() => {
                    Some("No enabled accounts report usage.".into())
                }
                Some(Ok(_)) => None,
            };
            if let Some(status) = status {
                rows.push(
                    div()
                        .px_4()
                        .py_2()
                        .text_xs()
                        .text_color(theme::rambutan_400())
                        .child(status)
                        .into_any_element(),
                );
            }
        }
        for (name, result) in linked {
            let status = match result {
                None => "Checking usage limits…",
                Some(Err(error)) => error,
                Some(Ok(accounts)) if accounts.is_empty() => "No accounts reported.",
                Some(Ok(_)) => "Linked environment",
            };
            rows.push(div().px_4().py_2().text_xs().text_color(theme::rambutan_400())
                .child(format!("{name} · {status}")).into_any_element());
        }
        let mut outstanding = state
            .credit_action
            .as_ref()
            .filter(|a| state.rows.contains(&a.choice.hub));
        for (index, group) in state.account_groups(revision).into_iter().enumerate() {
            let action = outstanding.take_if(|a| {
                group.readings.iter().any(|r| {
                    matches!(r.origin,api::Origin::Hub(h) if h==&a.choice.hub)
                        && r.account.id == a.choice.account
                })
            });
            rows.push(self.pooled_usage_row(index, &group, action, cx));
        }
        if let Some(action) = outstanding {
            rows.push(
                div()
                    .px_4()
                    .py_2()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_sm().child(format!(
                        "{} · {}",
                        action.choice.hub.label, action.choice.label
                    )))
                    .child(self.credit_controls(
                        &action.choice.hub,
                        &action.choice.account,
                        &action.choice.label,
                        None,
                        Some(action),
                        cx,
                    ))
                    .into_any_element(),
            );
        }
        if state.dialog.is_none() {
            if let Some(e) = &state.error {
                rows.push(
                    div()
                        .px_4()
                        .py_2()
                        .text_sm()
                        .text_color(theme::lychee_400())
                        .child(e.clone())
                        .into_any_element(),
                );
            }
        }
        let add = web_button(
            "usage-hub-add",
            WebButtonVariant::Outline,
            WebButtonSize::Small,
            ButtonState::disabled_if(state.busy),
        )
        .track_focus(&state.add)
        .aria_label("Add hub")
        .child("Add hub");
        let actions = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .child(editor_button(
                "usage-hub-view",
                "View usage",
                WebButtonVariant::Ghost,
                false,
                cx,
                |ws, window, cx| {
                    ws.usage_hubs.open_workbench_usage = true;
                    ws.activate_sidebar_view(AppView::Workbench, window, cx);
                },
            ))
            .child(editor_button(
                "usage-hub-refresh",
                "Refresh",
                WebButtonVariant::Ghost,
                state.busy || state.loading || state.pending != 0,
                cx,
                |ws, _, cx| ws.load_usage_hubs(cx),
            ))
            .child(on_press_view(add, cx, |ws, w, cx| ws.open_hub(None, w, cx)));
        let header = self.settings_row(
            "settings-row-usage-providers",
            "Usage providers",
            "",
            actions.into_any_element(),
        );
        self.settings_card_with_heading(header, rows)
    }
    pub(super) fn render_hub_dialog(
        &self,
        w: &Window,
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        let state = &self.usage_hubs;
        let dialog = state.dialog.as_ref()?;
        let confirming_credit = matches!(dialog, Dialog::Redeem(_));
        let mut body = div()
            .id("hub-fields")
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_3()
            .px_5()
            .py_4();
        let (title, submit, enabled, variant) = match dialog {
            Dialog::Add(f) => {
                body=body.child(div().text_sm().text_color(theme::rambutan_400()).child("Show quota for the accounts this hub pools. The management key stays on this computer."));
                let field = |label, input: &Entity<InputState>| {
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(div().text_sm().child(label))
                        .child(Input::new(input).aria_label(label).disabled(state.busy))
                };
                body = body.child(field("Hub URL", &f.url));
                let sources = self.credential_settings.read(cx).secret_sources();
                let sources = sources.read(cx).sources();
                let mut key = div().flex().gap_2().items_center();
                if !sources.is_empty() || f.source.is_some() {
                    let mut options = vec![DropdownOption {
                        value: "value",
                        label: "Value".into(),
                        description: None,
                    }];
                    for (id, s) in sources {
                        options.push(DropdownOption {
                            value: saved_option_value(&format!("source:{id}")),
                            label: s.name.clone(),
                            description: None,
                        });
                    }
                    let selected = f.source.as_ref().map_or("value", |s| {
                        saved_option_value(&format!("source:{s}"))
                    });
                    let id = "usage-hub-key-source";
                    let (toggle, select) = self.settings_dropdown_handlers(id, cx);
                    key = key.child(
                        bruhs_ui::dropdown::Dropdown::new(
                            id,
                            "Management key source",
                            selected,
                            vec![DropdownGroup {
                                label: None,
                                mark: None,
                                options,
                            }],
                            self.settings_listbox(id),
                            toggle,
                            select,
                        )
                        .icon_trigger(if f.source.is_some() {
                            "icons/shield.svg"
                        } else {
                            assets::TOOL_PENCIL_ICON_PATH
                        }),
                    );
                }
                let input = if f.source.is_some() {
                    &f.reference
                } else {
                    &f.key
                };
                body = body
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().text_sm().child("Management key"))
                            .child(
                                key.child(
                                    div().flex_1().min_w_0().child(
                                        Input::new(input)
                                            .aria_label("Management key")
                                            .disabled(state.busy),
                                    ),
                                ),
                            ),
                    )
                    .child(field("Label (optional)", &f.label));
                (
                    "Add a CLIProxyAPI hub".to_owned(),
                    "Add hub",
                    self.can_add_hub(f, cx),
                    WebButtonVariant::Primary,
                )
            }
            Dialog::Redeem(choice) => {
                body=body.child(div().text_sm().child(format!("{} · {}", choice.hub.label,choice.label)))
                    .child(div().text_sm().text_color(theme::rambutan_400()).child("Using one credit clears this account’s current rate-limit windows. This action is permanent."));
                (
                    "Use a reset credit?".into(),
                    "Use credit",
                    true,
                    WebButtonVariant::Primary,
                )
            }
            Dialog::Remove(hub) => {
                body =
                    body.child(div().text_sm().text_color(theme::rambutan_400()).child(
                        "This removes its accounts from the usage display on this computer.",
                    ));
                (
                    format!("Remove {}?", hub.label),
                    "Remove",
                    true,
                    WebButtonVariant::Destructive,
                )
            }
        };
        if let Some(e) = &state.error {
            body = body.child(
                div()
                    .id("hub-form-error")
                    .role(Role::Alert)
                    .text_sm()
                    .text_color(theme::lychee_400())
                    .child(e.clone()),
            );
        }
        let close = web_button(
            "usage-hub-close",
            WebButtonVariant::Ghost,
            WebButtonSize::Small,
            ButtonState::disabled_if(state.busy),
        )
        .aria_label("Close dialog")
        .child(super::updates_ui::icon_svg(
            assets::CLOSE_ICON_PATH,
            14.,
            theme::rambutan_400(),
        ));
        let cancel = web_button(
            "usage-hub-cancel",
            WebButtonVariant::Outline,
            WebButtonSize::Small,
            ButtonState::disabled_if(state.busy),
        )
        .track_focus(&state.cancel)
        .child("Cancel");
        let footer = div()
            .flex()
            .justify_end()
            .gap_2()
            .px_5()
            .py_3()
            .border_t_1()
            .border_color(theme::longan_700())
            .child(on_press_view(cancel, cx, Self::dismiss_hub))
            .child(editor_button(
                "usage-hub-submit",
                submit,
                variant,
                state.busy || !enabled,
                cx,
                Self::submit_hub,
            ));
        Some(
            div()
                .id("usage-hub-overlay")
                .absolute()
                .inset_0()
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme::longan_950().opacity(0.72))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .id("usage-hub-dialog")
                        .role(if confirming_credit {
                            Role::AlertDialog
                        } else {
                            Role::Dialog
                        })
                        .aria_label(title.clone())
                        .track_focus(&state.start)
                        .tab_group()
                        .on_key_down(cx.listener(Self::trap_modal_tab_focus))
                        .w(px(if confirming_credit { 560. } else { 470. }))
                        .max_w(w.viewport_size().width - px(32.))
                        .max_h(w.viewport_size().height - px(32.))
                        .flex()
                        .flex_col()
                        .overflow_hidden()
                        .rounded_xl()
                        .border_1()
                        .border_color(theme::longan_700())
                        .bg(theme::longan_900())
                        .child(
                            div()
                                .px_5()
                                .pt_4()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div()
                                        .text_lg()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(title),
                                )
                                .when(!confirming_credit, |header| {
                                    header.child(on_press_view(close, cx, Self::dismiss_hub))
                                }),
                        )
                        .child(body)
                        .child(footer)
                        .child(div().size(px(0.)).track_focus(&state.end)),
                )
                .into_any_element(),
        )
    }
}
