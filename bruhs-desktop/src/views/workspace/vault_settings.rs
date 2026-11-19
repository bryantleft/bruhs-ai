//! Settings, Vault: the service connections, then every vault entry's
//! metadata from the server, and one inline form that creates or changes an
//! entry. It matches the web's `components/vault_settings.rs`: the same
//! fields in the same order, the same copy and states, and Delete asks
//! first.
//!
//! The server owns every value (D21): the page lists and changes the vault
//! through `/api/vault`, so the process that saves a value is the one that
//! reads it later. Values are write-only. Their fields are masked, never
//! filled, and emptied on submit, on cancel and when the form goes away.
//! Every rule and line of copy is `bruhs_models::vault`'s.
use super::*;
use crate::app_state::spawn_tokio;
use crate::server_client::ServerError;
use bruhs_models::crew::ServiceNotice;
use bruhs_models::vault::{
    bruh_choice_label, delete_entry_confirm, fixed_hint_after, form_create, form_fields, form_for,
    form_update, kind_label, matches_search, option_for, option_key, row_label, vault_row,
    wait_reason, FormField, FormKind, FormScope, FormSource, LoginFields, LoginMeta, VaultBusy,
    VaultCreate, VaultEntry, VaultForm, VaultKind, VaultListing, VaultUpdate, CANCEL, CHECK,
    CHOOSE_BRUH, DELETE, EDIT, EMPTY, ENTRY_UPDATED, FIELD_BRUH, FIELD_KIND, FIELD_NAME,
    FIELD_PASSWORD, FIELD_PASSWORD_REF, FIELD_PROJECT, FIELD_REFERENCE, FIELD_REMOVE_TOTP,
    FIELD_SIGN_IN, FIELD_STORED_IN, FIELD_TOTP, FIELD_TOTP_REF, FIELD_USED_BY, FIELD_USERNAME,
    FIELD_USERNAME_REF, FIELD_VALUE, FIXED_HINT, KEEP, KEEP_HINT, KIND_OPTIONS, LOADING,
    LOAD_RETRY, NEW_ENTRY, NO_BRUHS, NO_MATCH, OPEN_FROM_OWN_ADDRESS, OP_MISSING_HINT,
    REFERENCE_HINT, REFRESH, SAVE_ENTRY, SCOPE_OPTIONS, SEARCH_PLACEHOLDER, SOURCE_OPTIONS,
    STATE_NEEDS_ATTENTION, TOTP_HINT,
};
use bruhs_ui::checkbox::Checkbox;
use bruhs_ui::input::InputContentType;
use std::collections::HashMap;

/// Web `text-label-sm`.
const SMALL_TEXT_PX: f32 = 12.0;
/// Web `FIELD`: 14px desktop input text, 10px inset.
const FIELD_TEXT_PX: f32 = theme::APP_INPUT_TEXT_SIZE_PX;
const FIELD_X_PADDING_PX: f32 = 10.0;
/// Web `rounded-grape` and the form's `bg-longan-950/60` with a
/// `longan-700/50` ring.
const PANEL_RADIUS_PX: f32 = 10.0;
const PANEL_FILL_OPACITY: f32 = 0.6;
const PANEL_RING_OPACITY: f32 = 0.5;

const KIND_DROPDOWN: &str = "desktop-vault-kind";
const SCOPE_DROPDOWN: &str = "desktop-vault-used-by";
const BRUH_DROPDOWN: &str = "desktop-vault-bruh";
const SOURCE_DROPDOWN: &str = "desktop-vault-stored-in";

/// The dropdown takes `'static` values; each bruh id is kept once for the
/// life of the app, so the set grows only with the crew.
fn bruh_value(id: &str) -> &'static str {
    static VALUES: std::sync::OnceLock<Mutex<HashSet<&'static str>>> = std::sync::OnceLock::new();
    if id.is_empty() {
        return "";
    }
    let mut values = VALUES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(value) = values.get(id) {
        return value;
    }
    let value: &'static str = Box::leak(id.to_owned().into_boxed_str());
    values.insert(value);
    value
}

#[derive(Clone, PartialEq)]
enum Load {
    /// Not asked yet: the first draw asks.
    Idle,
    Loading,
    Ready,
    Failed(String),
    /// An older server: no `/api/vault`.
    ServingOld,
    /// The server refused this client as not the owner's app (ruling F26).
    OwnerOnly,
}

/// Which form is open: a new entry's, or one entry's edit.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) enum Open {
    New,
    Entry(String),
}

/// A create or an update, ready to send. Holds values: neither is `Clone`.
enum Request {
    Create(VaultCreate),
    Update(String, VaultUpdate),
}

/// The dropdown that is open in the form, if any.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FormDropdown {
    Kind,
    UsedBy,
    Bruh,
    StoredIn,
}

/// One row's buttons that focus returns to.
pub(super) struct RowFocus {
    edit: FocusHandle,
    delete: FocusHandle,
    pub(super) keep: FocusHandle,
}

/// The inline form. Value fields are masked and start empty; the rest hold
/// metadata only.
pub(super) struct VaultFormState {
    /// Which submit a save answers, so a late answer never closes a newer
    /// form.
    serial: u64,
    /// The entry being edited.
    entry: Option<VaultEntry>,
    kind: FormKind,
    scope: FormScope,
    source: FormSource,
    bruh: String,
    remove_totp: bool,
    dropdown: Option<FormDropdown>,
    error: Option<String>,
    /// Focus inside the form, so closing it knows to move focus.
    focus: FocusHandle,
    pub(super) name: Entity<InputState>,
    project: Entity<InputState>,
    pub(super) url: Entity<InputState>,
    pub(super) username: Entity<InputState>,
    username_ref: Entity<InputState>,
    password_ref: Entity<InputState>,
    totp_ref: Entity<InputState>,
    reference: Entity<InputState>,
    value: Entity<InputState>,
    pub(super) password: Entity<InputState>,
    totp: Entity<InputState>,
    /// Read-only Kind and Used by on an edit.
    pub(super) kind_text: Entity<InputState>,
    pub(super) scope_text: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl VaultFormState {
    fn editing(&self) -> bool {
        self.entry.is_some()
    }

    fn has_totp(&self) -> bool {
        matches!(
            &self.entry,
            Some(VaultEntry {
                kind: VaultKind::Login(LoginMeta {
                    fields: LoginFields::Saved { totp: true, .. },
                    ..
                }),
                ..
            })
        )
    }

    fn fields(&self) -> Vec<FormField> {
        form_fields(self.kind, self.source)
    }

    fn secrets(&self) -> [&Entity<InputState>; 3] {
        [&self.value, &self.password, &self.totp]
    }

    /// The text field `field` draws, if it is one (the rest are
    /// dropdowns).
    fn input_for(&self, field: FormField) -> Option<&Entity<InputState>> {
        Some(match field {
            FormField::Kind | FormField::UsedBy | FormField::StoredIn => return None,
            FormField::Name => &self.name,
            FormField::SignIn => &self.url,
            FormField::Username => &self.username,
            FormField::Password => &self.password,
            FormField::Totp => &self.totp,
            FormField::UsernameRef => &self.username_ref,
            FormField::PasswordRef => &self.password_ref,
            FormField::TotpRef => &self.totp_ref,
            FormField::Value => &self.value,
            FormField::Reference => &self.reference,
        })
    }

    /// The first text field the owner can change. The dropdowns keep
    /// their own focus, so a new entry starts on Name, not Kind.
    fn first_field(&self) -> &Entity<InputState> {
        let editing = self.editing();
        self.fields()
            .into_iter()
            .filter(|field| !(editing && field.fixed_on_edit()))
            .find_map(|field| self.input_for(field))
            .unwrap_or(&self.name)
    }
}

pub(super) struct VaultSettings {
    client: ServerClient,
    connections: Entity<crate::views::service_connections::ServiceConnections>,
    listing: VaultListing,
    load: Load,
    busy: VaultBusy,
    notice: Option<&'static str>,
    search: Entity<InputState>,
    _search_subscription: Subscription,
    pub(super) form: Option<(Open, VaultFormState)>,
    /// The row whose Delete is asking.
    asking: Option<String>,
    /// What a row's last Check said: whether it passed, and its message.
    checked: HashMap<String, (bool, String)>,
    /// A row's failed Check or Delete.
    row_errors: HashMap<String, String>,
    pub(super) new_focus: FocusHandle,
    /// Focus anywhere on the page, so an answer knows it may move it.
    page_focus: FocusHandle,
    pub(super) rows_focus: HashMap<String, RowFocus>,
    next_serial: u64,
    task: Option<Task<()>>,
    /// Every request, as `METHOD path`; tests never reach a server.
    #[cfg(test)]
    vault_requests: Vec<String>,
    /// Every name the desktop's own thread forms were matched again for.
    #[cfg(test)]
    rematched: Vec<String>,
    /// The last create or update the page would send, held for a test.
    #[cfg(test)]
    last_request: Option<Request>,
}

impl VaultSettings {
    pub(super) fn new(
        client: ServerClient,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(SEARCH_PLACEHOLDER));
        let subscription = cx.subscribe_in(&search, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::Change) {
                this.searched(window, cx);
            }
        });
        Self {
            client,
            connections: cx.new(|cx| {
                crate::views::service_connections::ServiceConnections::new(None, None, cx)
            }),
            listing: VaultListing::default(),
            load: Load::Idle,
            busy: VaultBusy::Idle,
            notice: None,
            search,
            _search_subscription: subscription,
            form: None,
            asking: None,
            checked: HashMap::new(),
            row_errors: HashMap::new(),
            new_focus: cx.focus_handle().tab_stop(true),
            page_focus: cx.focus_handle(),
            rows_focus: HashMap::new(),
            next_serial: 0,
            task: None,
            #[cfg(test)]
            vault_requests: Vec::new(),
            #[cfg(test)]
            rematched: Vec::new(),
            #[cfg(test)]
            last_request: None,
        }
    }

    /// A form or a Delete question is open.
    pub(super) fn has_editor(&self) -> bool {
        self.form.is_some() || self.asking.is_some()
    }

    /// Close the form, emptying its values, and any Delete question.
    pub(super) fn clear_editor(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.close_form(window, cx);
        self.asking = None;
        cx.notify();
    }

    /// Whether Settings hiding has anything here to drop: a form, a
    /// question or a search.
    pub(super) fn holds_visit(&self, cx: &App) -> bool {
        self.has_editor() || !self.search.read(cx).value().is_empty()
    }

    /// Settings hid. The form and question close and the search goes, so
    /// the next visit starts with every row, as the web's page does.
    pub(super) fn left(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.clear_editor(window, cx);
        if !self.search.read(cx).value().is_empty() {
            self.search
                .update(cx, |search, cx| search.set_value("", window, cx));
        }
    }

    fn record(&mut self, request: String) {
        #[cfg(test)]
        self.vault_requests.push(request);
        #[cfg(not(test))]
        drop(request);
    }

    /// A press that starts a wait disables its own button, whose focus
    /// would then point at nothing and leave every key dead. Focus on the
    /// page moves to the page itself (no Tab stop; Tab goes on from it), and
    /// a text field keeps it.
    fn hold_focus_on_page(&self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.page_focus.contains_focused(window, cx) && !window.has_focused_input(cx) {
            self.page_focus.focus(window, cx);
        }
    }

    /// Ask the server for the listing again. Closes any form, as the web
    /// does, and waits while a change runs.
    pub(super) fn refresh(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy != VaultBusy::Idle {
            return;
        }
        self.connections
            .update(cx, |connections, cx| connections.refresh(cx));
        self.clear_editor(window, cx);
        self.notice = None;
        self.checked.clear();
        self.row_errors.clear();
        self.load = Load::Loading;
        self.busy = VaultBusy::Changing;
        self.hold_focus_on_page(window, cx);
        self.record("GET /api/vault".into());
        cx.notify();
        if cfg!(test) {
            return;
        }
        let client = self.client.clone();
        let operation = spawn_tokio(cx, async move { client.vault_listing().await });
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = operation.await;
            // The page may be gone by now.
            let _ = this.update(cx, |this, cx| this.listing_answered(result, cx));
        }));
    }

    fn listing_answered(
        &mut self,
        result: Result<VaultListing, ServerError>,
        cx: &mut Context<'_, Self>,
    ) {
        let (listing, load) = match result {
            Ok(listing) => (listing, Load::Ready),
            Err(error) if error.is_not_found() => (VaultListing::default(), Load::ServingOld),
            Err(error) if is_forbidden(&error) => (VaultListing::default(), Load::OwnerOnly),
            Err(error) => (VaultListing::default(), Load::Failed(error.vault_message())),
        };
        self.listing = listing;
        self.load = load;
        self.busy = VaultBusy::Idle;
        cx.notify();
    }

    /// The listing's state line, if it has one (web `#vault-status`).
    pub(super) fn status_line(&self) -> Option<String> {
        match &self.load {
            Load::Idle => None,
            Load::Loading => Some(LOADING.into()),
            Load::Failed(message) => Some(format!("{message} {LOAD_RETRY}")),
            Load::ServingOld => Some(ServiceNotice::ServingOld.message().into()),
            Load::OwnerOnly => Some(OPEN_FROM_OWN_ADDRESS.into()),
            Load::Ready => self.listing.entries.is_empty().then(|| EMPTY.into()),
        }
    }

    fn wait(&self) -> Option<&'static str> {
        wait_reason(self.busy, self.load == Load::Loading)
    }

    /// New entry also waits for a listing it can add to.
    fn new_blocked(&self) -> bool {
        self.wait().is_some() || self.load != Load::Ready
    }

    /// Each entry's row, as Task 1 reads it.
    #[cfg(test)]
    pub(super) fn rows(&self) -> Vec<bruhs_models::vault::VaultRow> {
        self.listing
            .entries
            .iter()
            .map(|entry| vault_row(entry, &self.listing.bruhs))
            .collect()
    }

    fn visible(&self, cx: &App) -> Vec<&VaultEntry> {
        let query = self.search.read(cx).value();
        self.listing
            .entries
            .iter()
            .filter(|entry| matches_search(entry, &self.listing.bruhs, &query))
            .collect()
    }

    /// A search hides rows: a form or question on a hidden row closes, as
    /// its row goes away on the web.
    fn searched(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let shown: HashSet<String> = self
            .visible(cx)
            .into_iter()
            .map(|entry| entry.id.clone())
            .collect();
        if matches!(&self.form, Some((Open::Entry(id), _)) if !shown.contains(id)) {
            self.close_form(window, cx);
        }
        if self.asking.as_ref().is_some_and(|id| !shown.contains(id)) {
            self.asking = None;
        }
        cx.notify();
    }

    /// The question Delete asks, while it asks.
    pub(super) fn confirm_line(&self) -> Option<String> {
        let id = self.asking.as_ref()?;
        self.listing
            .entries
            .iter()
            .find(|entry| &entry.id == id)
            .map(|entry| delete_entry_confirm(&entry.name))
    }

    fn row_focus(&mut self, id: &str, cx: &mut Context<'_, Self>) -> &RowFocus {
        self.rows_focus
            .entry(id.to_owned())
            .or_insert_with(|| RowFocus {
                edit: cx.focus_handle().tab_stop(true),
                delete: cx.focus_handle().tab_stop(true),
                keep: cx.focus_handle().tab_stop(true),
            })
    }

    // ── The form ────────────────────────────────────────────────────────

    fn open_new(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.new_blocked() {
            return;
        }
        self.notice = None;
        self.asking = None;
        self.open_form(Open::New, None, window, cx);
    }

    /// `BRUHS_DESKTOP_SMOKE_SCENE=vault-new` / `vault-new-1password`: New
    /// entry open on a login stored in the keychain or in 1Password, for
    /// the same-size comparison with the web. False while the page waits.
    #[cfg(any(test, debug_assertions))]
    pub(super) fn smoke_open_new(
        &mut self,
        source: FormSource,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        self.open_new(window, cx);
        let Some((_, form)) = self.form.as_mut() else {
            return false;
        };
        form.kind = FormKind::Login;
        form.source = source;
        self.reshaped(window, cx);
        cx.notify();
        true
    }

    fn open_edit(&mut self, id: String, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.wait().is_some() {
            return;
        }
        let Some(entry) = self.listing.entries.iter().find(|e| e.id == id).cloned() else {
            return;
        };
        self.notice = None;
        self.asking = None;
        self.checked.remove(&id);
        self.open_form(Open::Entry(id), Some(entry), window, cx);
    }

    fn open_form(
        &mut self,
        open: Open,
        entry: Option<VaultEntry>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.close_form(window, cx);
        self.next_serial = self.next_serial.wrapping_add(1);
        let form = self.build_form(entry, window, cx);
        let first = form.first_field().clone();
        self.form = Some((open, form));
        first.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn build_form(
        &mut self,
        entry: Option<VaultEntry>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> VaultFormState {
        // Metadata only: `form_for` leaves every value empty.
        let start = entry.as_ref().map(form_for).unwrap_or_default();
        let scope_line = entry
            .as_ref()
            .map(|entry| vault_row(entry, &self.listing.bruhs).scope)
            .unwrap_or_default();
        let mut subscriptions = Vec::new();
        let mut text = |value: &str, masked: bool| {
            let input = cx.new(|cx| {
                let input = InputState::new(window, cx).masked(masked);
                if value.is_empty() {
                    input
                } else {
                    input.default_value(value.to_owned())
                }
            });
            // Enter in any one-line field saves, like the web form's submit.
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
        let name = text(&start.name, false);
        let project = text(&start.project, false);
        let url = text(&start.url, false);
        let username = text(&start.username, false);
        let username_ref = text(&start.username_ref, false);
        let password_ref = text(&start.password_ref, false);
        let totp_ref = text(&start.totp_ref, false);
        let reference = text(&start.reference, false);
        // Values: masked, and never filled.
        let value = text("", true);
        let password = text("", true);
        let totp = text("", true);
        let kind_text = text(kind_label(start.kind), false);
        let scope_text = text(&scope_line, false);
        VaultFormState {
            serial: self.next_serial,
            kind: start.kind,
            scope: start.scope,
            source: start.source,
            bruh: start.bruh.clone(),
            remove_totp: false,
            dropdown: None,
            error: None,
            focus: cx.focus_handle(),
            entry,
            name,
            project,
            url,
            username,
            username_ref,
            password_ref,
            totp_ref,
            reference,
            value,
            password,
            totp,
            kind_text,
            scope_text,
            _subscriptions: subscriptions,
        }
    }

    /// Drop the form, emptying its value fields first.
    fn close_form(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some((_, form)) = self.form.take() {
            for input in form.secrets() {
                input.update(cx, |input, cx| input.set_value("", window, cx));
            }
        }
    }

    /// Cancel: empty the values and close, with focus back where the form
    /// was opened from. A running save keeps the form.
    fn cancel(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy != VaultBusy::Idle {
            return;
        }
        let Some((open, _)) = &self.form else {
            return;
        };
        let open = open.clone();
        self.close_form(window, cx);
        // Closing drops what an earlier Check said, so the row shows the
        // entry's own state (web `close`).
        if let Open::Entry(id) = &open {
            self.checked.remove(id);
        }
        self.focus_after_close(&open, window, cx);
        cx.notify();
    }

    fn focus_after_close(&mut self, open: &Open, window: &mut Window, cx: &mut Context<'_, Self>) {
        let target = match open {
            Open::New => self.new_focus.clone(),
            Open::Entry(id) => self.row_focus(id, cx).edit.clone(),
        };
        target.focus(window, cx);
    }

    /// Hide the value fields a new kind or source no longer draws, emptied.
    fn reshaped(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some((_, form)) = &self.form else {
            return;
        };
        let fields = form.fields();
        let hidden: Vec<_> = FormField::ALL
            .into_iter()
            .filter(|field| field.holds_value() && !fields.contains(field))
            .filter_map(|field| form.input_for(field).cloned())
            .collect();
        for input in hidden {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy != VaultBusy::Idle {
            return;
        }
        let Some((_, form)) = &self.form else {
            return;
        };
        let read = |input: &Entity<InputState>| input.read(cx).value().to_string();
        let mut vault_form = VaultForm {
            name: read(&form.name),
            kind: form.kind,
            scope: form.scope,
            project: read(&form.project),
            bruh: form.bruh.clone(),
            url: read(&form.url),
            source: form.source,
            username: read(&form.username),
            remove_totp: form.remove_totp,
            reference: read(&form.reference),
            username_ref: read(&form.username_ref),
            password_ref: read(&form.password_ref),
            totp_ref: read(&form.totp_ref),
            ..VaultForm::default()
        };
        let fields = form.fields();
        let secrets = [form.value.clone(), form.password.clone(), form.totp.clone()];
        let entry = form.entry.clone();
        let serial = form.serial;
        // Each value field is read and emptied in one step, shown or not;
        // only the ones the form draws are sent.
        let [value, password, totp] = secrets.map(|input| {
            let value = input.read(cx).value().to_string();
            input.update(cx, |input, cx| input.set_value("", window, cx));
            value
        });
        if fields.contains(&FormField::Value) {
            vault_form.value = value;
        }
        if fields.contains(&FormField::Password) {
            vault_form.password = password;
        }
        if fields.contains(&FormField::Totp) {
            vault_form.totp = totp;
        }
        let request = match &entry {
            None => form_create(&vault_form).map(Request::Create),
            Some(entry) => form_update(&vault_form, entry)
                .map(|update| Request::Update(entry.id.clone(), update)),
        };
        drop(vault_form);
        let Some((_, form)) = self.form.as_mut() else {
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
            Request::Create(_) => "POST /api/vault".into(),
            Request::Update(id, _) => {
                format!("PUT {}", crate::server_client::vault::vault_entry_path(id))
            }
        });
        cx.notify();
        if cfg!(test) {
            self.hold_for_test(request);
            return;
        }
        self.send(request, serial, window, cx);
    }

    /// Tests never send: they keep the request to look at.
    fn hold_for_test(&mut self, request: Request) {
        #[cfg(test)]
        {
            self.last_request = Some(request);
        }
        #[cfg(not(test))]
        drop(request);
    }

    /// Send a create or update; its answer comes back to `saved`.
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
                Request::Create(create) => client.create_vault_entry(create).await,
                Request::Update(id, update) => client.update_vault_entry(id, update).await,
            };
            drop(request);
            result
        });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = operation.await;
            // The page or its window may be gone by now.
            let _ = this.update_in(cx, |this, window, cx| {
                this.saved(serial, result, window, cx)
            });
        }));
    }

    fn saved(
        &mut self,
        serial: u64,
        result: Result<VaultEntry, ServerError>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.busy = VaultBusy::Idle;
        let this_form = self
            .form
            .as_ref()
            .filter(|(_, form)| form.serial == serial)
            .map(|(open, _)| open.clone());
        // Focus moves only from inside the page: the answer can land after
        // Settings closed.
        let held = self.page_focus.contains_focused(window, cx);
        match result {
            Ok(entry) => {
                let entry_name = entry.name.clone();
                match self
                    .listing
                    .entries
                    .iter_mut()
                    .find(|row| row.id == entry.id)
                {
                    Some(row) => *row = entry,
                    None => self.listing.entries.push(entry),
                }
                self.rematch(entry_name, None, cx);
                self.notice = Some(ENTRY_UPDATED);
                if let Some(open) = this_form {
                    if let Open::Entry(id) = &open {
                        self.checked.remove(id);
                    }
                    self.close_form(window, cx);
                    if held {
                        self.focus_after_close(&open, window, cx);
                    }
                }
            }
            Err(error) => {
                if let Some((_, form)) = self.form.as_mut().filter(|(_, f)| f.serial == serial) {
                    form.error = Some(error.vault_message());
                }
            }
        }
        cx.notify();
    }

    // ── Rows ────────────────────────────────────────────────────────────

    fn ask_delete(&mut self, id: String, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.wait().is_some() {
            return;
        }
        self.row_errors.remove(&id);
        let keep = self.row_focus(&id, cx).keep.clone();
        self.asking = Some(id);
        keep.focus(window, cx);
        cx.notify();
    }

    fn keep(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(id) = self.asking.take() else {
            return;
        };
        let delete = self.row_focus(&id, cx).delete.clone();
        delete.focus(window, cx);
        cx.notify();
    }

    fn delete(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy != VaultBusy::Idle {
            return;
        }
        let Some(id) = self.asking.clone() else {
            return;
        };
        self.busy = VaultBusy::Changing;
        self.notice = None;
        self.row_errors.remove(&id);
        self.hold_focus_on_page(window, cx);
        self.record(format!(
            "DELETE {}",
            crate::server_client::vault::vault_entry_path(&id)
        ));
        cx.notify();
        if cfg!(test) {
            return;
        }
        let client = self.client.clone();
        let target = id.clone();
        let operation = spawn_tokio(cx, async move { client.delete_vault_entry(&target).await });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = operation.await;
            let _ = this.update_in(cx, |this, window, cx| this.deleted(id, result, window, cx));
        }));
    }

    /// The server answered a Delete. Gone either way on a 404.
    fn deleted(
        &mut self,
        id: String,
        result: Result<(), ServerError>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.busy = VaultBusy::Idle;
        match result {
            Ok(()) => self.forget(&id, window, cx),
            Err(error) if error.is_not_found() => self.forget(&id, window, cx),
            Err(error) => {
                self.row_errors.insert(id, error.vault_message());
            }
        }
        cx.notify();
    }

    /// An entry is gone: its row, its form and its question go, the
    /// desktop's own thread forms match again, and focus goes to New entry
    /// if it was on the page. The answer can land after Settings closed,
    /// and focus elsewhere stays where it is.
    fn forget(&mut self, id: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        let held = self.page_focus.contains_focused(window, cx);
        if let Some(name) = self
            .listing
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.name.clone())
        {
            self.rematch(name, Some(id.to_owned()), cx);
        }
        if matches!(&self.form, Some((Open::Entry(open), _)) if open == id) {
            self.close_form(window, cx);
        }
        if self.asking.as_deref() == Some(id) {
            self.asking = None;
        }
        self.listing.entries.retain(|entry| entry.id != id);
        self.checked.remove(id);
        self.row_errors.remove(id);
        self.rows_focus.remove(id);
        if held {
            self.new_focus.focus(window, cx);
        }
    }

    /// The vault changed through the server, another process: match the
    /// desktop's own open thread forms for `name` again, as the server's
    /// manager does for its own (D20). `deleted` names an entry that is
    /// gone, whose last failure is dropped.
    fn rematch(&mut self, name: String, deleted: Option<String>, cx: &mut Context<'_, Self>) {
        #[cfg(test)]
        self.rematched.push(name.clone());
        let Some(state) = cx.try_global::<DesktopState>() else {
            return;
        };
        let manager = state.app.secrets().clone();
        spawn_tokio(cx, async move {
            if let Some(id) = &deleted {
                manager.forget_attention([id.as_str()]);
            }
            manager.refresh_saved_requests(&name).await;
        })
        .detach();
    }

    fn check(&mut self, id: String, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy != VaultBusy::Idle {
            return;
        }
        self.busy = VaultBusy::Checking;
        self.notice = None;
        self.checked.remove(&id);
        self.row_errors.remove(&id);
        self.hold_focus_on_page(window, cx);
        self.record(format!(
            "POST {}",
            crate::server_client::vault::vault_check_path(&id)
        ));
        cx.notify();
        if cfg!(test) {
            return;
        }
        let client = self.client.clone();
        let target = id.clone();
        let operation = spawn_tokio(cx, async move { client.check_vault_entry(&target).await });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = operation.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.check_answered(id, result, window, cx);
            });
        }));
    }

    /// The server answered a Check.
    fn check_answered(
        &mut self,
        id: String,
        result: Result<bruhs_models::vault::VaultCheck, ServerError>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.busy = VaultBusy::Idle;
        match result {
            Ok(check) => {
                if check.ok {
                    if let Some(entry) =
                        self.listing.entries.iter_mut().find(|entry| entry.id == id)
                    {
                        entry.attention = None;
                    }
                }
                self.checked.insert(id, (check.ok, check.message));
            }
            Err(error) if error.is_not_found() => self.forget(&id, window, cx),
            Err(error) => {
                self.row_errors.insert(id, error.vault_message());
            }
        }
        cx.notify();
    }

    #[cfg(test)]
    pub(super) fn vault_requests_for_test(&self) -> Vec<String> {
        self.vault_requests.clone()
    }

    /// Show `listing` as if the server had sent it.
    #[cfg(test)]
    pub(super) fn show_listing_for_test(
        &mut self,
        listing: VaultListing,
        cx: &mut Context<'_, Self>,
    ) {
        self.answer_listing_for_test(Ok(listing), cx);
    }

    /// Answer the listing request as the server would.
    #[cfg(test)]
    pub(super) fn answer_listing_for_test(
        &mut self,
        result: Result<VaultListing, ServerError>,
        cx: &mut Context<'_, Self>,
    ) {
        self.listing_answered(result, cx);
    }
}

fn is_forbidden(error: &ServerError) -> bool {
    matches!(error, ServerError::Status { code, .. } if *code == reqwest::StatusCode::FORBIDDEN)
}

// ── Drawing ─────────────────────────────────────────────────────────────

fn small(text: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .min_w_0()
        .font_family(theme::UI_SMALL_FONT_FAMILY)
        .text_size(px(SMALL_TEXT_PX))
        .text_color(color)
        .child(text.into())
}

fn labelled(label: &'static str, control: impl IntoElement) -> Div {
    div()
        .min_w_0()
        .w_full()
        .flex()
        .flex_col()
        .gap_1()
        .child(small(label, theme::rambutan_300()))
        .child(control)
}

fn field(
    input: &Entity<InputState>,
    label: &'static str,
    readonly: bool,
    content: Option<InputContentType>,
) -> impl IntoElement {
    let mut element = Input::new(input).aria_label(label).readonly(readonly);
    if let Some(content) = content {
        element = element.content_type(content);
    }
    // The web's FIELD fill (`bg-longan-950`), not the kit's white tint,
    // which reads lighter than the form panel it sits on.
    Styled::h(element, px(theme::CONTROL_PX))
        .bg(theme::longan_950())
        .px(px(FIELD_X_PADDING_PX))
        .font_family(theme::CODE_FONT_FAMILY)
        .text_size(px(FIELD_TEXT_PX))
}

fn button(
    id: impl Into<ElementId>,
    selector: String,
    label: &'static str,
    variant: WebButtonVariant,
    disabled: bool,
) -> Stateful<Div> {
    web_button(
        id,
        variant,
        WebButtonSize::Small,
        ButtonState::disabled_if(disabled),
    )
    .debug_selector(move || selector)
    .min_h(px(theme::CONTROL_PX))
    .flex_shrink_0()
    .aria_label(label)
    .child(label)
}

fn panel() -> Div {
    div()
        .mt_3()
        .p_3()
        .rounded(px(PANEL_RADIUS_PX))
        .bg(theme::longan_950().opacity(PANEL_FILL_OPACITY))
        .border_1()
        .border_color(theme::longan_700().opacity(PANEL_RING_OPACITY))
        .flex()
        .flex_col()
        .gap_3()
}

impl VaultSettings {
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
            .is_some_and(|(_, form)| form.dropdown == Some(which));
        let (id, label) = match which {
            FormDropdown::Kind => (KIND_DROPDOWN, FIELD_KIND),
            FormDropdown::UsedBy => (SCOPE_DROPDOWN, FIELD_USED_BY),
            FormDropdown::Bruh => (BRUH_DROPDOWN, FIELD_BRUH),
            FormDropdown::StoredIn => (SOURCE_DROPDOWN, FIELD_STORED_IN),
        };
        let entity = cx.entity().downgrade();
        let toggle = entity.clone();
        let on_toggle: ToggleHandler = Rc::new(move |_, cx| {
            let _ = toggle.update(cx, |this, cx| {
                if let Some((_, form)) = this.form.as_mut() {
                    form.dropdown = (form.dropdown != Some(which)).then_some(which);
                }
                cx.notify();
            });
        });
        let on_select: SelectHandler = Rc::new(move |choice, window, cx| {
            let _ = entity.update(cx, |this, cx| this.chose(which, choice, window, cx));
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

    fn chose(
        &mut self,
        which: FormDropdown,
        choice: &'static str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let no_bruhs = self.listing.bruhs.is_empty();
        let Some((_, form)) = self.form.as_mut() else {
            return;
        };
        form.dropdown = None;
        match which {
            FormDropdown::Kind => form.kind = option_for(&KIND_OPTIONS, choice),
            FormDropdown::UsedBy => {
                let scope = option_for(&SCOPE_OPTIONS, choice);
                // One bruh needs a bruh to choose (web: the option is off).
                if !(scope == FormScope::Bruh && no_bruhs) {
                    form.scope = scope;
                }
            }
            FormDropdown::Bruh => form.bruh = choice.to_owned(),
            FormDropdown::StoredIn => form.source = option_for(&SOURCE_OPTIONS, choice),
        }
        self.reshaped(window, cx);
        cx.notify();
    }

    fn options(table: &[(impl Copy, &'static str, &'static str)]) -> Vec<DropdownOption> {
        table
            .iter()
            .map(|(_, key, text)| DropdownOption {
                value: key,
                label: (*text).to_owned(),
                description: None,
            })
            .collect()
    }

    fn render_field(
        &self,
        form: &VaultFormState,
        which: FormField,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let editing = form.editing();
        match which {
            FormField::Kind if editing => {
                labelled(FIELD_KIND, field(&form.kind_text, FIELD_KIND, true, None))
                    .into_any_element()
            }
            FormField::Kind => self.render_dropdown(
                FormDropdown::Kind,
                option_key(&KIND_OPTIONS, form.kind),
                Self::options(&KIND_OPTIONS),
                cx,
            ),
            FormField::Name => labelled(FIELD_NAME, field(&form.name, FIELD_NAME, editing, None))
                .into_any_element(),
            FormField::UsedBy if editing => labelled(
                FIELD_USED_BY,
                field(&form.scope_text, FIELD_USED_BY, true, None),
            )
            .into_any_element(),
            FormField::UsedBy => {
                let no_bruhs = self.listing.bruhs.is_empty();
                let scopes = SCOPE_OPTIONS
                    .iter()
                    .map(|(scope, key, text)| DropdownOption {
                        value: key,
                        label: (*text).to_owned(),
                        description: (*scope == FormScope::Bruh && no_bruhs)
                            .then(|| NO_BRUHS.to_owned()),
                    })
                    .collect();
                let bruhs = std::iter::once(DropdownOption {
                    value: "",
                    label: CHOOSE_BRUH.to_owned(),
                    description: None,
                })
                .chain(self.listing.bruhs.iter().map(|bruh| DropdownOption {
                    value: bruh_value(&bruh.agent_id),
                    label: bruh_choice_label(bruh),
                    description: None,
                }))
                .collect();
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(self.render_dropdown(
                        FormDropdown::UsedBy,
                        option_key(&SCOPE_OPTIONS, form.scope),
                        scopes,
                        cx,
                    ))
                    .when(no_bruhs, |el| {
                        el.child(small(NO_BRUHS, theme::rambutan_500()))
                    })
                    .when(form.scope == FormScope::Project, |el| {
                        el.child(labelled(
                            FIELD_PROJECT,
                            field(&form.project, FIELD_PROJECT, false, None),
                        ))
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
            FormField::SignIn => labelled(
                FIELD_SIGN_IN,
                // Ruling F3: a login's site is fixed once it exists.
                field(
                    &form.url,
                    FIELD_SIGN_IN,
                    editing,
                    Some(InputContentType::Url),
                ),
            )
            .into_any_element(),
            FormField::StoredIn => self.render_dropdown(
                FormDropdown::StoredIn,
                option_key(&SOURCE_OPTIONS, form.source),
                Self::options(&SOURCE_OPTIONS),
                cx,
            ),
            FormField::Username => labelled(
                FIELD_USERNAME,
                field(
                    &form.username,
                    FIELD_USERNAME,
                    false,
                    Some(InputContentType::Username),
                ),
            )
            .into_any_element(),
            FormField::Password => labelled(
                FIELD_PASSWORD,
                field(
                    &form.password,
                    FIELD_PASSWORD,
                    false,
                    Some(InputContentType::Password),
                ),
            )
            .into_any_element(),
            FormField::Totp => {
                let remove = form.remove_totp;
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(labelled(
                        FIELD_TOTP,
                        field(
                            &form.totp,
                            FIELD_TOTP,
                            false,
                            Some(InputContentType::Password),
                        ),
                    ))
                    .child(small(TOTP_HINT, theme::rambutan_500()))
                    .when(editing && form.has_totp(), |el| {
                        let entity = cx.entity().downgrade();
                        el.child(
                            div()
                                .min_h(px(theme::CONTROL_PX))
                                .flex()
                                .items_center()
                                .child(
                                    div()
                                        .debug_selector(|| "desktop-vault-remove-totp".into())
                                        .child(
                                            Checkbox::new("desktop-vault-remove-totp")
                                                .label(FIELD_REMOVE_TOTP)
                                                .checked(remove)
                                                .on_click(move |checked, _, cx| {
                                                    let checked = *checked;
                                                    let _ = entity.update(cx, |this, cx| {
                                                        if let Some((_, form)) = this.form.as_mut()
                                                        {
                                                            form.remove_totp = checked;
                                                        }
                                                        cx.notify();
                                                    });
                                                }),
                                        ),
                                ),
                        )
                    })
                    .into_any_element()
            }
            FormField::UsernameRef => labelled(
                FIELD_USERNAME_REF,
                field(&form.username_ref, FIELD_USERNAME_REF, false, None),
            )
            .into_any_element(),
            FormField::PasswordRef => labelled(
                FIELD_PASSWORD_REF,
                field(&form.password_ref, FIELD_PASSWORD_REF, false, None),
            )
            .into_any_element(),
            FormField::TotpRef => div()
                .flex()
                .flex_col()
                .gap_1()
                .child(labelled(
                    FIELD_TOTP_REF,
                    field(&form.totp_ref, FIELD_TOTP_REF, false, None),
                ))
                .child(small(REFERENCE_HINT, theme::rambutan_500()))
                .into_any_element(),
            FormField::Value => labelled(
                FIELD_VALUE,
                field(
                    &form.value,
                    FIELD_VALUE,
                    false,
                    Some(InputContentType::Password),
                ),
            )
            .into_any_element(),
            FormField::Reference => div()
                .flex()
                .flex_col()
                .gap_1()
                .child(labelled(
                    FIELD_REFERENCE,
                    field(&form.reference, FIELD_REFERENCE, false, None),
                ))
                .child(small(REFERENCE_HINT, theme::rambutan_500()))
                .into_any_element(),
        }
    }

    fn render_form(&self, form: &VaultFormState, cx: &Context<'_, Self>) -> AnyElement {
        let waiting = self.wait().is_some();
        panel()
            .id("desktop-vault-form")
            .debug_selector(|| "desktop-vault-form".into())
            .track_focus(&form.focus)
            // Escape cancels the form before it can close Settings.
            .on_action(cx.listener(|this, _: &CloseOverlays, window, cx| {
                this.cancel(window, cx);
            }))
            .children({
                let fields = form.fields();
                let hint_after = fixed_hint_after(&fields).filter(|_| form.editing());
                fields.into_iter().map(move |which| {
                    let drawn = self.render_field(form, which, cx);
                    if hint_after == Some(which) {
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(drawn)
                            .child(
                                small(FIXED_HINT, theme::rambutan_500())
                                    .debug_selector(|| "desktop-vault-fixed-hint".into()),
                            )
                            .into_any_element()
                    } else {
                        drawn
                    }
                })
            })
            .when_some(form.error.clone(), |el, message| {
                el.child(
                    small(message, theme::lychee_300())
                        .id("desktop-vault-form-error")
                        .role(Role::Alert),
                )
            })
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(on_press_view(
                        button(
                            "desktop-vault-save",
                            "desktop-vault-save".into(),
                            SAVE_ENTRY,
                            WebButtonVariant::Outline,
                            waiting,
                        ),
                        cx,
                        |this, window, cx| this.submit(window, cx),
                    ))
                    .child(on_press_view(
                        button(
                            "desktop-vault-cancel",
                            "desktop-vault-cancel".into(),
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

    fn render_row(&self, entry: &VaultEntry, cx: &Context<'_, Self>) -> AnyElement {
        let row = vault_row(entry, &self.listing.bruhs);
        let id = entry.id.clone();
        let waiting = self.wait().is_some();
        let asking = self.asking.as_deref() == Some(id.as_str());
        let editing = matches!(&self.form, Some((Open::Entry(open), _)) if *open == id);
        let checked = self.checked.get(&id).cloned();
        let focus = self.rows_focus.get(&id);
        // An earlier Check's line stands in for the stored attention.
        let attention = row.attention.filter(|_| checked.is_none());
        let stuck = row.state == STATE_NEEDS_ATTENTION;
        let info = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_size(px(theme::APP_UI_TEXT_SIZE_PX))
                    .text_color(theme::rambutan_100())
                    .child(row.name.clone()),
            )
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(small(row.kind, theme::rambutan_300()))
                    .child(small(row.scope.clone(), theme::rambutan_300())),
            )
            .when_some(row.detail.clone(), |el, detail| {
                el.child(small(detail, theme::rambutan_400()))
            })
            .child(small(
                row.state,
                if stuck {
                    theme::durian_300()
                } else {
                    theme::rambutan_400()
                },
            ))
            .when_some(attention, |el, attention| {
                let selector = format!("desktop-vault-attention-{id}");
                el.child(small(attention, theme::durian_300()).debug_selector(move || selector))
            });
        let edit = {
            let id = id.clone();
            let mut edit = button(
                SharedString::from(format!("desktop-vault-edit-{id}")),
                format!("desktop-vault-edit-{id}"),
                EDIT,
                WebButtonVariant::Outline,
                waiting,
            )
            .aria_expanded(editing);
            if let Some(focus) = focus {
                edit = edit.track_focus(&focus.edit);
            }
            on_press_view(edit, cx, move |this, window, cx| {
                this.open_edit(id.clone(), window, cx);
            })
        };
        let check = {
            let id = id.clone();
            on_press_view(
                button(
                    SharedString::from(format!("desktop-vault-check-{id}")),
                    format!("desktop-vault-check-{id}"),
                    CHECK,
                    WebButtonVariant::Outline,
                    waiting,
                ),
                cx,
                move |this, window, cx| this.check(id.clone(), window, cx),
            )
        };
        let delete = (!asking).then(|| {
            let id = id.clone();
            let mut delete = button(
                SharedString::from(format!("desktop-vault-delete-{id}")),
                format!("desktop-vault-delete-{id}"),
                DELETE,
                WebButtonVariant::Outline,
                waiting,
            );
            if let Some(focus) = focus {
                delete = delete.track_focus(&focus.delete);
            }
            on_press_view(delete, cx, move |this, window, cx| {
                this.ask_delete(id.clone(), window, cx);
            })
        });
        let confirm = asking.then(|| {
            let mut keep = button(
                "desktop-vault-keep",
                "desktop-vault-keep".into(),
                KEEP,
                WebButtonVariant::Outline,
                waiting,
            );
            if let Some(focus) = focus {
                keep = keep.track_focus(&focus.keep);
            }
            let mut question = panel().id("desktop-vault-question").role(Role::Group);
            // Named by the question, as the web's group is (BC M4).
            if let Some(line) = self.confirm_line() {
                question = question.aria_label(line);
            }
            question
                .on_action(cx.listener(|this, _: &CloseOverlays, window, cx| {
                    this.keep(window, cx);
                }))
                .child(
                    div()
                        .text_size(px(theme::APP_BODY_TEXT_SIZE_PX))
                        .text_color(theme::rambutan_200())
                        .children(self.confirm_line()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(on_press_view(
                            button(
                                "desktop-vault-confirm",
                                "desktop-vault-confirm".into(),
                                DELETE,
                                WebButtonVariant::Destructive,
                                waiting,
                            ),
                            cx,
                            |this, window, cx| this.delete(window, cx),
                        ))
                        .child(on_press_view(keep, cx, |this, window, cx| {
                            this.keep(window, cx);
                        })),
                )
        });
        div()
            .id(SharedString::from(format!("desktop-vault-row-{id}")))
            .debug_selector({
                let selector = format!("desktop-vault-row-{id}");
                move || selector
            })
            .role(Role::Group)
            .aria_label(row_label(&entry.name))
            .min_w_0()
            .py_3()
            .border_b_1()
            .border_color(theme::longan_800())
            .flex()
            .flex_col()
            .child(
                div().flex().items_start().gap_3().child(info).child(
                    div()
                        .flex_shrink_0()
                        .flex()
                        .gap_2()
                        .child(edit)
                        .child(check)
                        .children(delete),
                ),
            )
            .children(confirm)
            .when(editing, |el| {
                el.children(
                    self.form
                        .as_ref()
                        .map(|(_, form)| self.render_form(form, cx)),
                )
            })
            .when_some(checked, |el, (ok, message)| {
                el.child(
                    small(
                        message,
                        if ok {
                            theme::pandan_300()
                        } else {
                            theme::durian_300()
                        },
                    )
                    .mt_2()
                    .id(SharedString::from(format!("desktop-vault-checked-{id}")))
                    .role(Role::Status),
                )
            })
            .when_some(self.row_errors.get(&id).cloned(), |el, message| {
                el.child(
                    small(message, theme::lychee_300())
                        .mt_2()
                        .id(SharedString::from(format!("desktop-vault-error-{id}")))
                        .role(Role::Alert),
                )
            })
            .into_any_element()
    }
}

impl Render for VaultSettings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        if self.load == Load::Idle {
            self.refresh(window, cx);
        }
        // Every row's buttons keep their focus handles while it shows.
        let ids: Vec<String> = self.listing.entries.iter().map(|e| e.id.clone()).collect();
        self.rows_focus.retain(|id, _| ids.contains(id));
        for id in &ids {
            self.row_focus(id, cx);
        }
        let wait = self.wait();
        let new_blocked = self.new_blocked();
        let new_open = matches!(&self.form, Some((Open::New, _)));
        let visible = self.visible(cx);
        let rows: Vec<AnyElement> = visible
            .iter()
            .map(|entry| self.render_row(entry, cx))
            .collect();
        let no_match = rows.is_empty() && !self.listing.entries.is_empty();
        let status = self.status_line().map(|line| {
            let color = match &self.load {
                Load::Failed(_) => theme::lychee_300(),
                Load::ServingOld | Load::OwnerOnly => theme::durian_300(),
                _ => theme::rambutan_400(),
            };
            let notice = matches!(self.load, Load::ServingOld | Load::OwnerOnly);
            small(line, color)
                .id("desktop-vault-status")
                .role(if matches!(self.load, Load::Failed(_)) {
                    Role::Alert
                } else {
                    Role::Status
                })
                .when(notice, |el| {
                    el.debug_selector(|| "desktop-vault-notice".into())
                })
        });
        let new_entry = on_press_view(
            button(
                "desktop-vault-new",
                "desktop-vault-new".into(),
                NEW_ENTRY,
                WebButtonVariant::Outline,
                new_blocked,
            )
            .track_focus(&self.new_focus)
            .aria_expanded(new_open),
            cx,
            |this, window, cx| this.open_new(window, cx),
        );
        let refresh = on_press_view(
            button(
                "desktop-vault-refresh",
                "desktop-vault-refresh".into(),
                REFRESH,
                WebButtonVariant::Outline,
                wait.is_some(),
            ),
            cx,
            |this, window, cx| this.refresh(window, cx),
        );
        div()
            .id("desktop-vault")
            .track_focus(&self.page_focus)
            .p_4()
            .min_w_0()
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_size(px(SMALL_TEXT_PX))
            .text_color(theme::rambutan_200())
            .flex()
            .flex_col()
            .gap_2()
            .child(self.connections.clone())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(field(
                        &self.search,
                        SEARCH_PLACEHOLDER,
                        false,
                        None,
                    )))
                    .child(new_entry)
                    .child(refresh),
            )
            .child(small(KEEP_HINT, theme::rambutan_400()))
            .when(
                self.load == Load::Ready && !self.listing.one_password,
                |el| el.child(small(OP_MISSING_HINT, theme::rambutan_400())),
            )
            .children(status)
            // A load says so in the status line above.
            .when_some(wait.filter(|_| self.load != Load::Loading), |el, reason| {
                el.child(
                    small(reason, theme::rambutan_400())
                        .id("desktop-vault-wait")
                        .role(Role::Status),
                )
            })
            .when_some(self.notice, |el, message| {
                el.child(
                    small(message, theme::pandan_300())
                        .id("desktop-vault-saved")
                        .role(Role::Status),
                )
            })
            .when(new_open, |el| {
                el.children(
                    self.form
                        .as_ref()
                        .map(|(_, form)| self.render_form(form, cx)),
                )
            })
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .children(rows)
                    .when(no_match, |el| {
                        el.child(small(NO_MATCH, theme::rambutan_400()).py_3())
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{Open, Request, VaultFormState, VaultSettings};
    use crate::server_client::{ServerClient, ServerError};
    use crate::{app_state::DesktopState, theme};
    use bruhs_models::crew::ServiceNotice;
    use bruhs_models::secrets::SavedCredentialState;
    use bruhs_models::vault::{
        delete_entry_confirm, vault_row, CredentialScope, LoginFields, LoginInput, LoginMeta,
        TotpInput, VaultAttention, VaultBruh, VaultBusy, VaultEntry, VaultKind, VaultListing,
        VaultSecretInput, VaultUpdate, ENTRY_UPDATED, OPEN_FROM_OWN_ADDRESS,
    };
    use bruhs_models::vault::{FormKind, FormSource};
    use gpui::{
        AppContext as _, Entity, FocusHandle, Focusable as _, Modifiers, TestAppContext,
        VisualTestContext,
    };
    use std::{cell::RefCell, rc::Rc};

    fn listing() -> VaultListing {
        VaultListing {
            entries: vec![
                VaultEntry {
                    id: "v1-shop".into(),
                    name: "Shop".into(),
                    scope: CredentialScope::Global,
                    kind: VaultKind::Login(LoginMeta {
                        url: "https://shop.test/in".into(),
                        fields: LoginFields::Saved {
                            username: "me".into(),
                            totp: true,
                        },
                    }),
                    state: SavedCredentialState::Saved,
                    attention: Some(VaultAttention::OnePasswordNeedsYou),
                },
                VaultEntry {
                    id: "v1-deploy".into(),
                    name: "DEPLOY_KEY".into(),
                    scope: CredentialScope::Bruh {
                        agent_id: "ash".into(),
                    },
                    kind: VaultKind::Value,
                    state: SavedCredentialState::Saved,
                    attention: None,
                },
                VaultEntry {
                    id: "v1-token".into(),
                    name: "Token".into(),
                    scope: CredentialScope::Project {
                        path: "/w/app".into(),
                    },
                    kind: VaultKind::OnePassword {
                        reference: "op://P/Token/credential".into(),
                    },
                    state: SavedCredentialState::Saved,
                    attention: None,
                },
            ],
            bruhs: vec![VaultBruh {
                agent_id: "ash".into(),
                name: "Ash".into(),
            }],
            one_password: true,
        }
    }

    /// The page alone in a window, its listing as `listing` (or none yet).
    fn page(
        cx: &mut TestAppContext,
        listing: Option<VaultListing>,
    ) -> (Entity<VaultSettings>, &mut VisualTestContext) {
        cx.update(|cx| {
            bruhs_ui::init(cx);
            theme::apply_bruhs_theme(cx);
            cx.set_global(DesktopState::new().unwrap());
        });
        let capture = Rc::new(RefCell::new(None));
        let output = capture.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let client = ServerClient::new("http://127.0.0.1:9", "http://127.0.0.1:9").unwrap();
            let view = cx.new(|cx| VaultSettings::new(client, window, cx));
            *output.borrow_mut() = Some(view.clone());
            bruhs_ui::Root::new(view, window, cx)
        });
        let view = capture.borrow().clone().unwrap();
        // The service connections sit above the vault: room for every row
        // and its form, so clicks land on screen.
        cx.simulate_resize(gpui::size(gpui::px(900.0), gpui::px(2400.0)));
        cx.run_until_parked();
        if let Some(listing) = listing {
            view.update(cx, |view, cx| view.show_listing_for_test(listing, cx));
            cx.run_until_parked();
        }
        (view, cx)
    }

    fn requests(view: &Entity<VaultSettings>, cx: &mut VisualTestContext) -> Vec<String> {
        view.read_with(cx, |view, _| view.vault_requests_for_test())
    }

    fn rendered(cx: &mut VisualTestContext, selector: &str) -> bool {
        let selector: &'static str = Box::leak(selector.to_owned().into_boxed_str());
        cx.debug_bounds(selector).is_some()
    }

    fn click(cx: &mut VisualTestContext, selector: &str) {
        let selector: &'static str = Box::leak(selector.to_owned().into_boxed_str());
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not drawn"));
        cx.simulate_click(bounds.center(), Modifiers::default());
        cx.run_until_parked();
    }

    /// A key press as the app gets it: gpui clicks a focused button on key up.
    fn press(cx: &mut VisualTestContext, key: &'static str) {
        let keystroke = gpui::Keystroke::parse(key).unwrap();
        cx.simulate_event(gpui::KeyDownEvent {
            keystroke: keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.simulate_event(gpui::KeyUpEvent { keystroke });
        cx.run_until_parked();
    }

    fn focused(cx: &mut VisualTestContext) -> Option<FocusHandle> {
        cx.update(|window, cx| window.focused(cx))
    }

    fn form_input(
        view: &Entity<VaultSettings>,
        cx: &mut VisualTestContext,
        pick: impl Fn(&VaultFormState) -> &Entity<bruhs_ui::input::InputState>,
    ) -> Entity<bruhs_ui::input::InputState> {
        view.read_with(cx, |view, _| {
            let (_, form) = view.form.as_ref().expect("a form is open");
            pick(form).clone()
        })
    }

    /// Tab until `target` has focus, as the owner would.
    fn tab_to(cx: &mut VisualTestContext, target: &Entity<bruhs_ui::input::InputState>) {
        let handle = cx.update(|_, cx| target.focus_handle(cx));
        for _ in 0..4 {
            press(cx, "tab");
            if focused(cx).as_ref() == Some(&handle) {
                return;
            }
        }
        panic!("Tab never reached the field");
    }

    #[gpui::test]
    fn the_vault_lists_entries_without_values(cx: &mut TestAppContext) {
        let (view, cx) = page(cx, Some(listing()));
        assert_eq!(requests(&view, cx), ["GET /api/vault"]);
        let fixture = listing();
        for entry in &fixture.entries {
            assert!(rendered(cx, &format!("desktop-vault-row-{}", entry.id)));
            assert!(rendered(cx, &format!("desktop-vault-check-{}", entry.id)));
            assert!(rendered(cx, &format!("desktop-vault-delete-{}", entry.id)));
        }
        let rows = view.read_with(cx, |view, _| view.rows());
        let expected: Vec<_> = fixture
            .entries
            .iter()
            .map(|entry| vault_row(entry, &fixture.bruhs))
            .collect();
        assert_eq!(rows, expected);
        assert_eq!(
            rows[0].detail.as_deref(),
            Some("https://shop.test as me, with a one-time code")
        );
        assert_eq!(rows[1].scope, "Ash's own");
        assert!(rendered(cx, "desktop-vault-attention-v1-shop"));
        assert!(!rendered(cx, "desktop-vault-attention-v1-deploy"));
        assert!(!rendered(cx, "desktop-vault-attention-v1-token"));
        assert!(!rendered(cx, "desktop-vault-form"));
    }

    #[gpui::test]
    fn the_smoke_scene_opens_new_entry_stored_where_asked(cx: &mut TestAppContext) {
        let (view, cx) = page(cx, Some(listing()));
        for source in [FormSource::OnePassword, FormSource::Saved] {
            let opened = cx.update(|window, cx| {
                view.update(cx, |view, cx| view.smoke_open_new(source, window, cx))
            });
            assert!(opened);
            assert!(rendered(cx, "desktop-vault-form"));
            view.read_with(cx, |view, _| {
                let (open, form) = view.form.as_ref().expect("a form is open");
                assert!(matches!(open, Open::New));
                assert_eq!((form.kind, form.source), (FormKind::Login, source));
            });
        }
    }

    #[gpui::test]
    fn new_entry_by_keyboard_builds_a_saved_login(cx: &mut TestAppContext) {
        let (view, cx) = page(cx, Some(listing()));
        let new_entry = view.read_with(cx, |view, _| view.new_focus.clone());
        cx.update(|window, cx| new_entry.focus(window, cx));
        press(cx, "enter");
        assert!(rendered(cx, "desktop-vault-form"));
        let name = form_input(&view, cx, |form| &form.name);
        let name_focus = cx.update(|_, cx| name.focus_handle(cx));
        assert_eq!(focused(cx), Some(name_focus), "the form starts on Name");
        cx.simulate_input("Shop");
        let url = form_input(&view, cx, |form| &form.url);
        tab_to(cx, &url);
        cx.simulate_input("https://shop.test/in");
        let username = form_input(&view, cx, |form| &form.username);
        tab_to(cx, &username);
        cx.simulate_input("me@shop.test");
        let password = form_input(&view, cx, |form| &form.password);
        tab_to(cx, &password);
        cx.simulate_input("hunter2-canary");
        assert_eq!(
            cx.update(|_, cx| password.read(cx).value().to_string()),
            "hunter2-canary"
        );
        press(cx, "enter");
        let sent = requests(&view, cx);
        assert_eq!(
            sent.iter().filter(|line| line.starts_with("POST")).count(),
            1
        );
        assert_eq!(sent.last().map(String::as_str), Some("POST /api/vault"));
        assert!(!sent.concat().contains("hunter2"), "{sent:?}");
        assert!(
            cx.update(|_, cx| password.read(cx).value().is_empty()),
            "the password is cleared on submit"
        );
    }

    #[gpui::test]
    fn editing_keeps_name_kind_and_scope_fixed(cx: &mut TestAppContext) {
        let (view, cx) = page(cx, Some(listing()));
        click(cx, "desktop-vault-edit-v1-shop");
        assert!(rendered(cx, "desktop-vault-form"));
        assert!(rendered(cx, "desktop-vault-fixed-hint"));
        view.read_with(cx, |view, _| {
            assert!(matches!(&view.form, Some((Open::Entry(id), _)) if id == "v1-shop"));
        });
        let (name, kind, scope, password, url) = view.read_with(cx, |view, _| {
            let (_, form) = view.form.as_ref().unwrap();
            (
                form.name.clone(),
                form.kind_text.clone(),
                form.scope_text.clone(),
                form.password.clone(),
                form.url.clone(),
            )
        });
        cx.update(|_, cx| {
            assert_eq!(name.read(cx).value().as_ref(), "Shop");
            assert_eq!(kind.read(cx).value().as_ref(), "Login");
            assert_eq!(scope.read(cx).value().as_ref(), "All projects");
            assert!(password.read(cx).value().is_empty(), "no prefill");
            assert_eq!(url.read(cx).value().as_ref(), "https://shop.test/in");
        });
        // The first field the owner can change takes focus: Sign-in page is
        // fixed too (ruling F3), so Username.
        let username = view.read_with(cx, |view, _| view.form.as_ref().unwrap().1.username.clone());
        let username_focus = cx.update(|_, cx| username.focus_handle(cx));
        assert_eq!(focused(cx), Some(username_focus));
        // Name and Sign-in page are read only: typing changes nothing.
        for (input, stored) in [(&name, "Shop"), (&url, "https://shop.test/in")] {
            cx.update(|window, cx| input.update(cx, |input, cx| input.focus(window, cx)));
            cx.run_until_parked();
            cx.simulate_input("x");
            assert_eq!(
                cx.update(|_, cx| input.read(cx).value().to_string()),
                stored
            );
        }
        // Cancel clears what was typed and closes the form.
        let password_focus = cx.update(|_, cx| password.focus_handle(cx));
        cx.update(|window, cx| window.focus(&password_focus, cx));
        cx.simulate_input("typed-canary");
        click(cx, "desktop-vault-cancel");
        assert!(cx.update(|_, cx| password.read(cx).value().is_empty()));
        assert!(!rendered(cx, "desktop-vault-form"));
        let edit = view.read_with(cx, |view, _| view.rows_focus["v1-shop"].edit.clone());
        assert_eq!(focused(cx), Some(edit), "Cancel returns to the row's Edit");
        assert!(!requests(&view, cx)
            .iter()
            .any(|line| line.starts_with("PUT")));
    }

    #[gpui::test]
    fn delete_asks_first(cx: &mut TestAppContext) {
        let (view, cx) = page(cx, Some(listing()));
        click(cx, "desktop-vault-delete-v1-deploy");
        assert!(rendered(cx, "desktop-vault-confirm"));
        assert_eq!(
            view.read_with(cx, |view, _| view.confirm_line()),
            Some(delete_entry_confirm("DEPLOY_KEY"))
        );
        let keep = view.read_with(cx, |view, _| view.rows_focus["v1-deploy"].keep.clone());
        assert_eq!(focused(cx), Some(keep), "Keep, the safe choice, has focus");
        click(cx, "desktop-vault-keep");
        assert!(!rendered(cx, "desktop-vault-confirm"));
        let delete = view.read_with(cx, |view, _| view.rows_focus["v1-deploy"].delete.clone());
        assert_eq!(focused(cx), Some(delete), "Keep returns to Delete");
        assert_eq!(
            requests(&view, cx),
            ["GET /api/vault"],
            "Keep sends nothing"
        );
        click(cx, "desktop-vault-delete-v1-deploy");
        click(cx, "desktop-vault-confirm");
        assert_eq!(
            requests(&view, cx).last().map(String::as_str),
            Some("DELETE /api/vault/v1-deploy")
        );
    }

    #[gpui::test]
    fn an_older_server_shows_the_service_notice(cx: &mut TestAppContext) {
        let (view, cx) = page(cx, None);
        let not_found = ServerError::Status {
            request: "GET /api/vault".into(),
            code: reqwest::StatusCode::NOT_FOUND,
            body: String::new(),
        };
        view.update(cx, |view, cx| {
            view.answer_listing_for_test(Err(not_found), cx)
        });
        cx.run_until_parked();
        assert!(rendered(cx, "desktop-vault-notice"));
        assert_eq!(
            view.read_with(cx, |view, _| view.status_line()),
            Some(ServiceNotice::ServingOld.message().to_string())
        );
        // New entry waits for a listing it can add to.
        click(cx, "desktop-vault-new");
        assert!(!rendered(cx, "desktop-vault-form"));
        assert!(view.read_with(cx, |view, _| view.rows().is_empty()));
    }

    #[gpui::test]
    fn escape_cancels_the_form_and_focus_returns_to_new_entry(cx: &mut TestAppContext) {
        let (view, cx) = page(cx, Some(listing()));
        cx.update(|_, cx| {
            cx.bind_keys([gpui::KeyBinding::new(
                "escape",
                bruhs_ui::actions::CloseOverlays,
                None,
            )]);
        });
        click(cx, "desktop-vault-new");
        let password = form_input(&view, cx, |form| &form.password);
        let handle = cx.update(|_, cx| password.focus_handle(cx));
        cx.update(|window, cx| window.focus(&handle, cx));
        cx.run_until_parked();
        cx.simulate_input("escape-canary");
        press(cx, "escape");
        assert!(
            !rendered(cx, "desktop-vault-form"),
            "Escape closes the form"
        );
        assert!(cx.update(|_, cx| password.read(cx).value().is_empty()));
        let new_entry = view.read_with(cx, |view, _| view.new_focus.clone());
        assert_eq!(focused(cx), Some(new_entry));
    }

    #[gpui::test]
    fn a_late_save_lands_after_the_form_closed(cx: &mut TestAppContext) {
        let (view, cx) = page(cx, Some(listing()));
        click(cx, "desktop-vault-edit-v1-deploy");
        click(cx, "desktop-vault-save");
        assert_eq!(
            requests(&view, cx).last().map(String::as_str),
            Some("PUT /api/vault/v1-deploy")
        );
        let serial = view.read_with(cx, |view, _| view.form.as_ref().unwrap().1.serial);
        // Settings closes while the save runs.
        cx.update(|window, cx| view.update(cx, |view, cx| view.clear_editor(window, cx)));
        let mut saved = listing().entries.remove(1);
        saved.attention = Some(bruhs_models::vault::VaultAttention::KeychainNeedsYou);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.saved(serial, Ok(saved.clone()), window, cx)
            });
        });
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert!(view.form.is_none());
            assert!(view.busy == VaultBusy::Idle);
            assert_eq!(view.notice, Some(ENTRY_UPDATED));
            assert_eq!(view.listing.entries[1], saved);
        });
    }

    fn deleted_answer(
        view: &Entity<VaultSettings>,
        cx: &mut VisualTestContext,
        id: &str,
        result: Result<(), ServerError>,
    ) {
        let id = id.to_owned();
        cx.update(|window, cx| view.update(cx, |view, cx| view.deleted(id, result, window, cx)));
        cx.run_until_parked();
    }

    #[gpui::test]
    fn a_page_on_another_address_shows_why(cx: &mut TestAppContext) {
        let (view, cx) = page(cx, None);
        let forbidden = ServerError::Status {
            request: "GET /api/vault".into(),
            code: reqwest::StatusCode::FORBIDDEN,
            body: String::new(),
        };
        view.update(cx, |view, cx| {
            view.answer_listing_for_test(Err(forbidden), cx)
        });
        cx.run_until_parked();
        assert!(rendered(cx, "desktop-vault-notice"));
        assert_eq!(
            view.read_with(cx, |view, _| view.status_line()),
            Some(OPEN_FROM_OWN_ADDRESS.to_string())
        );
        click(cx, "desktop-vault-new");
        assert!(!rendered(cx, "desktop-vault-form"), "New entry stays off");
    }

    #[gpui::test]
    fn a_kind_change_empties_the_values_it_hides(cx: &mut TestAppContext) {
        let (view, cx) = page(cx, Some(listing()));
        click(cx, "desktop-vault-new");
        let password = form_input(&view, cx, |form| &form.password);
        let handle = cx.update(|_, cx| password.focus_handle(cx));
        cx.update(|window, cx| window.focus(&handle, cx));
        cx.simulate_input("hidden-canary");
        click(cx, "desktop-vault-kind");
        click(cx, "desktop-vault-kind-option-value");
        assert!(
            cx.update(|_, cx| password.read(cx).value().is_empty()),
            "a Key has no password field, so its text goes"
        );
        // The Key's own Value field is drawn and empty.
        let value = form_input(&view, cx, |form| &form.value);
        assert!(cx.update(|_, cx| value.read(cx).value().is_empty()));
    }

    #[gpui::test]
    fn remove_one_time_code_reaches_the_update(cx: &mut TestAppContext) {
        let (view, cx) = page(cx, Some(listing()));
        click(cx, "desktop-vault-edit-v1-shop");
        click(cx, "desktop-vault-remove-totp");
        click(cx, "desktop-vault-save");
        assert_eq!(
            requests(&view, cx).last().map(String::as_str),
            Some("PUT /api/vault/v1-shop")
        );
        view.read_with(cx, |view, _| {
            assert!(matches!(
                &view.last_request,
                Some(Request::Update(
                    id,
                    VaultUpdate {
                        secret: VaultSecretInput::Login {
                            url,
                            fields: LoginInput::Saved {
                                password: None,
                                totp: TotpInput::Remove,
                                ..
                            },
                        },
                    },
                )) if id == "v1-shop" && url == "https://shop.test/in"
            ));
        });
    }

    /// D20: the desktop's own thread forms match again after a change made
    /// through the server.
    #[gpui::test]
    fn changes_match_the_desktops_thread_forms_again(cx: &mut TestAppContext) {
        let (view, cx) = page(cx, Some(listing()));
        let saved = listing().entries.remove(1);
        cx.update(|window, cx| {
            view.update(cx, |view, cx| view.saved(0, Ok(saved), window, cx));
        });
        click(cx, "desktop-vault-delete-v1-token");
        click(cx, "desktop-vault-confirm");
        // The pressed Confirm turns off; focus holds on the page meanwhile.
        let page_focus = view.read_with(cx, |view, _| view.page_focus.clone());
        assert_eq!(focused(cx), Some(page_focus));
        deleted_answer(&view, cx, "v1-token", Ok(()));
        let gone = ServerError::Status {
            request: "POST /api/vault/v1-shop/check".into(),
            code: reqwest::StatusCode::NOT_FOUND,
            body: String::new(),
        };
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.check_answered("v1-shop".into(), Err(gone), window, cx);
            });
        });
        view.read_with(cx, |view, _| {
            assert_eq!(view.rematched, ["DEPLOY_KEY", "Token", "Shop"]);
            assert!(view
                .listing
                .entries
                .iter()
                .all(|entry| entry.id == "v1-deploy"));
        });
    }

    /// An answer that lands after Settings closed leaves focus alone.
    /// BC M3: as the web's page starts afresh on each visit, leaving
    /// Settings drops the search, so every row shows next time.
    #[gpui::test]
    fn leaving_settings_drops_the_search(cx: &mut TestAppContext) {
        let (view, cx) = page(cx, Some(listing()));
        let search = view.read_with(cx, |view, _| view.search.clone());
        cx.update(|window, cx| {
            search.update(cx, |search, cx| search.set_value("shop", window, cx));
            view.update(cx, |view, cx| view.searched(window, cx));
        });
        cx.run_until_parked();
        assert!(rendered(cx, "desktop-vault-row-v1-shop"));
        assert!(!rendered(cx, "desktop-vault-row-v1-deploy"));
        assert!(view.read_with(cx, |view, cx| view.holds_visit(cx)));
        cx.update(|window, cx| view.update(cx, |view, cx| view.left(window, cx)));
        cx.run_until_parked();
        assert!(search.read_with(cx, |search, _| search.value().is_empty()));
        assert!(rendered(cx, "desktop-vault-row-v1-deploy"));
        assert!(!view.read_with(cx, |view, cx| view.holds_visit(cx)));
    }

    #[gpui::test]
    fn a_late_delete_answer_leaves_focus_where_it_is(cx: &mut TestAppContext) {
        let (view, cx) = page(cx, Some(listing()));
        click(cx, "desktop-vault-delete-v1-deploy");
        click(cx, "desktop-vault-confirm");
        // Settings closes and focus leaves the page (the test window has
        // nothing else to hold it, so it holds none).
        cx.update(|window, cx| view.update(cx, |view, cx| view.clear_editor(window, cx)));
        cx.update(|window, cx| {
            let elsewhere = cx.focus_handle();
            window.focus(&elsewhere, cx);
        });
        cx.run_until_parked();
        let before = focused(cx);
        let new_entry = view.read_with(cx, |view, _| view.new_focus.clone());
        deleted_answer(&view, cx, "v1-deploy", Ok(()));
        assert_eq!(focused(cx), before, "focus stays where the owner left it");
        assert_ne!(focused(cx), Some(new_entry.clone()));
        view.read_with(cx, |view, _| {
            assert!(!view
                .listing
                .entries
                .iter()
                .any(|entry| entry.id == "v1-deploy"));
        });
        // On the page, the same answer moves focus to New entry.
        click(cx, "desktop-vault-delete-v1-token");
        click(cx, "desktop-vault-confirm");
        deleted_answer(&view, cx, "v1-token", Ok(()));
        assert_eq!(focused(cx), Some(new_entry));
    }
}
