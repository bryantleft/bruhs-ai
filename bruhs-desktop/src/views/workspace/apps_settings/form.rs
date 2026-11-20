//! Settings, Apps' inline forms: Connect (Name, Used by, the app's values,
//! a Get a token link, then Test and connect) and Replace token (the app's
//! secrets only). The rules are `bruhs_models::apps`' (ruling F17): the
//! fields in page order, the checked request, the copy, as on the web.
//!
//! A token or key field is masked, never filled, is read and emptied in one
//! step on submit, and is emptied on cancel and whenever the form closes.

use super::super::vault_settings::{bruh_value, button, field, labelled, panel, small};
use super::*;
use bruhs_models::apps::{
    connect_request, form_fields, replace_fields, replace_request, AppConnect, AppParam,
    AppReplace, ConnectForm, APP_NAME_HINT, APP_SCOPE_OPTIONS, GET_TOKEN, KEEP_KEY_HINT,
    TEST_AND_CONNECT,
};
use bruhs_models::vault::{
    option_for, option_key, FormScope, CANCEL, CHOOSE_BRUH, FIELD_BRUH, FIELD_NAME, FIELD_USED_BY,
};
use bruhs_ui::input::InputContentType;
use std::collections::BTreeMap;

const SCOPE_DROPDOWN: &str = "desktop-apps-used-by";
const BRUH_DROPDOWN: &str = "desktop-apps-bruh";

/// A connect or a replace, ready to send. Each holds a token: neither is
/// `Clone`.
enum Request {
    Connect(AppConnect),
    Replace(String, AppReplace),
}

/// The dropdown that is open in the Connect form, if any.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FormDropdown {
    UsedBy,
    Bruh,
}

/// The open form. Secret fields are masked and start empty; the rest hold
/// what the owner typed.
pub(in crate::views::workspace) struct AppFormState {
    /// Which submit an answer is for, so a late answer never closes a newer
    /// form.
    pub(super) serial: u64,
    pub(in crate::views::workspace) open: Open,
    app: &'static AppConnector,
    scope: FormScope,
    bruh: String,
    pub(super) dropdown: Option<FormDropdown>,
    pub(super) error: Option<String>,
    /// Focus inside the form, so closing it knows to move focus.
    focus: FocusHandle,
    /// Connect's Name; a Replace token form has none.
    pub(in crate::views::workspace) name: Option<Entity<InputState>>,
    /// The app's values in page order, each with its field.
    pub(in crate::views::workspace) params: Vec<(&'static AppParam, Entity<InputState>)>,
    _subscriptions: Vec<Subscription>,
}

impl AppFormState {
    /// The field for value `id`, if the form asks for it.
    pub(in crate::views::workspace) fn param(&self, id: &str) -> Option<&Entity<InputState>> {
        self.params
            .iter()
            .find(|(param, _)| param.id == id)
            .map(|(_, input)| input)
    }

    /// The first field: Name on Connect, the token's first secret on
    /// Replace.
    fn first_field(&self) -> Option<&Entity<InputState>> {
        self.name
            .as_ref()
            .or_else(|| self.params.first().map(|(_, input)| input))
    }

    fn secrets(&self) -> impl Iterator<Item = &Entity<InputState>> {
        self.params
            .iter()
            .filter(|(param, _)| param.secret)
            .map(|(_, input)| input)
    }
}

impl AppsSettings {
    pub(super) fn open_connect(
        &mut self,
        app: &'static str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.blocked() {
            return;
        }
        self.notice = None;
        self.open_form(Open::Connect(app), window, cx);
    }

    /// `BRUHS_DESKTOP_SMOKE_SCENE=apps-connect:<app id>`: that app's Connect
    /// form, once the listing is in (same-size captures). False when there
    /// is no such app or no form opened.
    #[cfg(debug_assertions)]
    pub(in crate::views::workspace) fn smoke_open_connect(
        &mut self,
        app_id: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        let Some(app) = bruhs_models::apps::connector(app_id) else {
            return false;
        };
        self.open_connect(app.id, window, cx);
        self.form.is_some()
    }

    pub(super) fn open_replace(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.wait().is_some() {
            return;
        }
        if !self.listing.accounts.iter().any(|account| account.id == id) {
            return;
        }
        self.notice = None;
        self.asking = None;
        self.tested.remove(&id);
        self.open_form(Open::Replace(id), window, cx);
    }

    fn open_form(&mut self, open: Open, window: &mut Window, cx: &mut Context<'_, Self>) {
        let app_id = match &open {
            Open::Connect(app) => Some((*app).to_owned()),
            Open::Replace(id) => self
                .listing
                .accounts
                .iter()
                .find(|account| &account.id == id)
                .map(|account| account.app.clone()),
        };
        let Some(app) = app_id.and_then(|id| bruhs_models::apps::connector(&id)) else {
            return;
        };
        self.close_form(window, cx);
        self.next_serial = self.next_serial.wrapping_add(1);
        let form = self.build_form(open, app, window, cx);
        let first = form.first_field().cloned();
        self.form = Some(form);
        if let Some(first) = first {
            first.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    fn build_form(
        &mut self,
        open: Open,
        app: &'static AppConnector,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AppFormState {
        let mut subscriptions = Vec::new();
        let mut text = |masked: bool| {
            // Nothing is ever filled in: a token can't come back, and the
            // other values start empty as on the web.
            let input = cx.new(|cx| InputState::new(window, cx).masked(masked));
            // Enter in any one-line field submits, like the web form's
            // submit.
            subscriptions.push(cx.subscribe_in(
                &input,
                window,
                |this: &mut Self, _, event: &InputEvent, window, cx| {
                    if matches!(
                        event,
                        InputEvent::PressEnter {
                            secondary: false,
                            ..
                        }
                    ) {
                        this.submit(window, cx);
                    }
                },
            ));
            input
        };
        let (name, fields) = match &open {
            Open::Connect(_) => (Some(text(false)), form_fields(app)),
            Open::Replace(_) => (None, replace_fields(app)),
        };
        let params = fields
            .into_iter()
            .map(|param| (param, text(param.secret)))
            .collect();
        AppFormState {
            serial: self.next_serial,
            open,
            app,
            scope: FormScope::Global,
            bruh: String::new(),
            dropdown: None,
            error: None,
            focus: cx.focus_handle(),
            name,
            params,
            _subscriptions: subscriptions,
        }
    }

    /// Drop the form, emptying its secret fields first.
    pub(super) fn close_form(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some(form) = self.form.take() {
            for input in form.secrets() {
                input.update(cx, |input, cx| input.set_value("", window, cx));
            }
        }
    }

    /// Cancel: empty the tokens and close, with focus back on the button
    /// that opened the form. A running change keeps the form.
    fn cancel(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy != VaultBusy::Idle {
            return;
        }
        let Some(open) = self.form.as_ref().map(|form| form.open.clone()) else {
            return;
        };
        self.close_form(window, cx);
        self.focus_after_close(&open, window, cx);
        cx.notify();
    }

    fn focus_after_close(&mut self, open: &Open, window: &mut Window, cx: &mut Context<'_, Self>) {
        let target = match open {
            Open::Connect(app) => self.connect_focus.get(app).cloned(),
            Open::Replace(id) => Some(self.row_focus(id, cx).replace.clone()),
        };
        if let Some(target) = target {
            target.focus(window, cx);
        }
    }

    pub(super) fn submit(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy != VaultBusy::Idle {
            return;
        }
        let Some(form) = &self.form else {
            return;
        };
        let (open, app, serial) = (form.open.clone(), form.app, form.serial);
        let name = form
            .name
            .as_ref()
            .map(|input| input.read(cx).value().to_string())
            .unwrap_or_default();
        let (scope, bruh) = (form.scope, form.bruh.clone());
        let params = form.params.clone();
        // Each value is read, and each secret emptied in the same step.
        let mut values = BTreeMap::new();
        for (param, input) in params {
            let value = input.read(cx).value().to_string();
            if param.secret {
                input.update(cx, |input, cx| input.set_value("", window, cx));
            }
            values.insert(param.id, value);
        }
        let request = match &open {
            Open::Connect(_) => {
                let typed = ConnectForm {
                    app: app.id,
                    name,
                    scope,
                    bruh,
                    values,
                };
                let request = connect_request(&typed).map(Request::Connect);
                drop(typed);
                request
            }
            Open::Replace(id) => {
                let request = replace_request(app, &values)
                    .map(|replace| Request::Replace(id.clone(), replace));
                drop(values);
                request
            }
        };
        let Some(form) = self.form.as_mut() else {
            return;
        };
        let request = match request {
            Ok(request) => request,
            Err(message) => {
                form.error = Some(message.into());
                cx.notify();
                return;
            }
        };
        form.error = None;
        form.dropdown = None;
        self.busy = VaultBusy::Changing;
        self.notice = None;
        self.hold_focus_on_page(window, cx);
        self.record(match &request {
            Request::Connect(_) => format!("POST {}", crate::server_client::apps::ACCOUNTS_PATH),
            Request::Replace(id, _) => {
                format!("PUT {}", crate::server_client::apps::app_account_path(id))
            }
        });
        cx.notify();
        // Tests never send. The request holds a token, so it goes at once.
        if cfg!(test) {
            drop(request);
            return;
        }
        self.send(request, serial, window, cx);
    }

    /// Send a connect or a replace; its answer comes back to `answered`.
    fn send(
        &mut self,
        request: Request,
        serial: u64,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let client = self.client.clone();
        let operation = spawn_tokio(cx, async move {
            let result = match &request {
                Request::Connect(connect) => client.connect_app(connect).await,
                Request::Replace(id, replace) => client.replace_app_token(id, replace).await,
            };
            drop(request);
            result
        });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = operation.await;
            // The page or its window may be gone by now.
            let _ = this.update_in(cx, |this, window, cx| {
                this.answered(serial, result, window, cx);
            });
        }));
    }

    /// The server answered a connect or a replace.
    pub(super) fn answered(
        &mut self,
        serial: u64,
        result: Result<AppAccount, ServerError>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.busy = VaultBusy::Idle;
        let this_form = self
            .form
            .as_ref()
            .filter(|form| form.serial == serial)
            .map(|form| form.open.clone());
        // Focus moves only from inside the page: the answer can land after
        // Settings closed.
        let held = self.page_focus.contains_focused(window, cx);
        match result {
            Ok(account) => {
                let line = connected_as(&review_text(&account.account));
                self.tested.remove(&account.id);
                match self
                    .listing
                    .accounts
                    .iter_mut()
                    .find(|row| row.id == account.id)
                {
                    Some(row) => *row = account,
                    None => self.listing.accounts.push(account),
                }
                self.notice = Some(Notice::Done(line));
                cx.emit(super::CredentialsChanged);
                if let Some(open) = this_form {
                    self.close_form(window, cx);
                    if held {
                        self.focus_after_close(&open, window, cx);
                    }
                }
            }
            Err(error) if error.vault_may_have_landed() => self.unanswered(window, cx),
            Err(error) => {
                if let Some(form) = self.form.as_mut().filter(|form| form.serial == serial) {
                    form.error = Some(error.vault_message());
                }
            }
        }
        cx.notify();
    }

    fn chose(&mut self, which: FormDropdown, choice: &'static str, cx: &mut Context<'_, Self>) {
        let no_bruhs = self.listing.bruhs.is_empty();
        let Some(form) = self.form.as_mut() else {
            return;
        };
        form.dropdown = None;
        match which {
            FormDropdown::UsedBy => {
                let scope = option_for(&APP_SCOPE_OPTIONS, choice);
                // One bruh needs a bruh to choose (web: the option is off).
                if !(scope == FormScope::Bruh && no_bruhs) {
                    form.scope = scope;
                }
            }
            FormDropdown::Bruh => form.bruh = choice.to_owned(),
        }
        cx.notify();
    }

    fn render_dropdown(
        &self,
        which: FormDropdown,
        value: &'static str,
        options: Vec<DropdownOption>,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let open = self
            .form
            .as_ref()
            .is_some_and(|form| form.dropdown == Some(which));
        let (id, label) = match which {
            FormDropdown::UsedBy => (SCOPE_DROPDOWN, FIELD_USED_BY),
            FormDropdown::Bruh => (BRUH_DROPDOWN, FIELD_BRUH),
        };
        let entity = cx.entity().downgrade();
        let toggle = entity.clone();
        let on_toggle: ToggleHandler = Rc::new(move |_, cx| {
            let _ = toggle.update(cx, |this, cx| {
                if let Some(form) = this.form.as_mut() {
                    form.dropdown = (form.dropdown != Some(which)).then_some(which);
                }
                cx.notify();
            });
        });
        let on_select: SelectHandler = Rc::new(move |choice, _, cx| {
            let _ = entity.update(cx, |this, cx| this.chose(which, choice, cx));
        });
        labelled(
            label,
            dropdown(
                id,
                label,
                value,
                vec![DropdownGroup {
                    label: None,
                    mark: None,
                    options,
                }],
                Listbox::open_if(open),
                on_toggle,
                on_select,
            ),
        )
        .into_any_element()
    }

    /// Name and Used by, with the bruh picker for One bruh.
    fn render_owner_fields(
        &self,
        form: &AppFormState,
        name: &Entity<InputState>,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let no_bruhs = self.listing.bruhs.is_empty();
        let scopes = APP_SCOPE_OPTIONS
            .iter()
            .map(|(scope, key, text)| DropdownOption {
                value: key,
                label: (*text).to_owned(),
                description: (*scope == FormScope::Bruh && no_bruhs)
                    .then(|| NO_BRUHS_FOR_APPS.to_owned()),
            })
            .collect();
        let bruhs = std::iter::once(DropdownOption {
            value: "",
            label: CHOOSE_BRUH.to_owned(),
            description: None,
        })
        .chain(self.listing.bruhs.iter().map(|bruh| DropdownOption {
            value: bruh_value(&bruh.agent_id),
            label: review_text(&bruh.name),
            description: None,
        }))
        .collect();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(labelled(FIELD_NAME, field(name, FIELD_NAME, false, None)))
                    .child(small(APP_NAME_HINT, theme::rambutan_500())),
            )
            .child(self.render_dropdown(
                FormDropdown::UsedBy,
                option_key(&APP_SCOPE_OPTIONS, form.scope),
                scopes,
                cx,
            ))
            .when(no_bruhs, |el| {
                el.child(small(NO_BRUHS_FOR_APPS, theme::rambutan_500()))
            })
            .when(form.scope == FormScope::Bruh, |el| {
                el.child(self.render_dropdown(
                    FormDropdown::Bruh,
                    bruh_value(&form.bruh),
                    bruhs,
                    cx,
                ))
            })
            .into_any_element()
    }

    /// One of the app's values: masked for a secret, with its hint under
    /// it.
    fn render_param(param: &'static AppParam, input: &Entity<InputState>) -> AnyElement {
        let content = if param.secret {
            Some(InputContentType::Password)
        } else if param.id == "email" {
            Some(InputContentType::Email)
        } else {
            None
        };
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(labelled(
                param.label,
                field(input, param.label, false, content),
            ))
            .when(!param.hint.is_empty(), |el| {
                el.child(small(param.hint, theme::rambutan_500()))
            })
            .into_any_element()
    }

    pub(super) fn render_form(&self, form: &AppFormState, cx: &Context<'_, Self>) -> AnyElement {
        let waiting = self.wait().is_some();
        let replacing = matches!(form.open, Open::Replace(_));
        let keeps_key = replacing && form.param("key").is_some();
        let url = form.app.token_help_url;
        panel()
            .id("desktop-apps-form")
            .debug_selector(|| "desktop-apps-form".into())
            .role(Role::Form)
            .aria_label(if replacing {
                format!("{REPLACE_TOKEN}: {}", form.app.name)
            } else {
                format!("{CONNECT} {}", form.app.name)
            })
            .track_focus(&form.focus)
            // Escape cancels the form before it can close Settings.
            .on_action(cx.listener(|this, _: &CloseOverlays, window, cx| {
                this.cancel(window, cx);
            }))
            .children(
                form.name
                    .as_ref()
                    .map(|name| self.render_owner_fields(form, name, cx)),
            )
            .children(
                form.params
                    .iter()
                    .map(|(param, input)| Self::render_param(param, input)),
            )
            .when(keeps_key, |el| {
                el.child(small(KEEP_KEY_HINT, theme::rambutan_500()))
            })
            .when_some(form.error.clone(), |el, message| {
                el.child(
                    small(message, theme::lychee_300())
                        .id("desktop-apps-form-error")
                        .debug_selector(|| "desktop-apps-form-error".into())
                        .role(Role::Alert),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(on_press_view(
                        button(
                            "desktop-apps-submit",
                            "desktop-apps-submit".into(),
                            if replacing {
                                REPLACE_TOKEN
                            } else {
                                TEST_AND_CONNECT
                            },
                            WebButtonVariant::Outline,
                            waiting,
                        ),
                        cx,
                        |this, window, cx| this.submit(window, cx),
                    ))
                    .child(on_press_view(
                        button(
                            "desktop-apps-token-help",
                            "desktop-apps-token-help".into(),
                            GET_TOKEN,
                            WebButtonVariant::Outline,
                            false,
                        )
                        .role(Role::Link)
                        .text_color(theme::blueberry_300()),
                        cx,
                        move |_, _, cx| cx.open_url(url),
                    ))
                    .child(on_press_view(
                        button(
                            "desktop-apps-cancel",
                            "desktop-apps-cancel".into(),
                            CANCEL,
                            WebButtonVariant::Outline,
                            waiting,
                        ),
                        cx,
                        |this, window, cx| this.cancel(window, cx),
                    )),
            )
            .into_any_element()
    }
}
