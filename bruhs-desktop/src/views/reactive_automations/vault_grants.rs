//! The vault entries a bruh's Settings grant it (D3; web
//! `crew/vault_grants.rs`): its own entries as lines, and one switch per
//! shared entry. The picks live in the settings form, so Save sends them
//! with the rest through `settings_commands_for`, and only when they
//! changed. The listing is read when Settings opens and forgotten when it
//! closes; it holds names and kinds, never a value.
use super::bruh_page::{page_button, page_switch, small_text};
use super::create_bruh::hint;
use super::*;
use bruhs_models::crew::{
    usable_by, vault_choice_label, vault_choices, vault_own_line, VAULT_GRANTS_HINT,
    VAULT_GRANTS_LABEL, VAULT_GRANTS_NONE,
};
use bruhs_models::vault::{
    vault_row, VaultListing, LOADING, LOAD_RETRY, OPEN_FROM_OWN_ADDRESS, REFRESH,
};
use std::sync::Mutex;

/// One row of the section: an entry the bruh owns, or a shared one it can
/// be granted.
#[derive(Debug, Clone, PartialEq, Eq)]
enum GrantRow {
    Own { id: String, line: String },
    Shared { id: String, label: String, on: bool },
}

/// The rows for bruh `agent_id` with `grants` picked: its own entries,
/// then the shared ones, each in `vault_choices`' order. Names and kinds
/// read as on the Vault page.
fn grant_rows(listing: &VaultListing, agent_id: &str, grants: &[String]) -> Vec<GrantRow> {
    let choices = vault_choices(&listing.entries, agent_id);
    let own = choices.own.iter().map(|entry| GrantRow::Own {
        id: entry.id.clone(),
        line: vault_own_line(&vault_row(entry, &listing.bruhs).name),
    });
    let shared = choices.shared.iter().map(|entry| GrantRow::Shared {
        id: entry.id.clone(),
        label: vault_choice_label(entry),
        on: usable_by(entry, agent_id, grants),
    });
    own.chain(shared).collect()
}

/// Adds or removes one grant, keeping the set sorted as the service stores
/// it, so toggling back to the stored set leaves Save off.
pub(super) fn toggle(grants: &mut Vec<String>, id: &str, on: bool) {
    grants.retain(|grant| grant != id);
    if on {
        grants.push(id.to_owned());
        grants.sort();
    }
}

/// A switch's id is `'static`; each one is kept once for the life of the
/// app, so the set grows only with the vault.
fn grant_selector(id: &str) -> &'static str {
    static SELECTORS: std::sync::OnceLock<Mutex<HashSet<&'static str>>> =
        std::sync::OnceLock::new();
    let selector = format!("desktop-bruh-vault-grant-{id}");
    let mut selectors = SELECTORS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(selector) = selectors.get(selector.as_str()) {
        return selector;
    }
    let selector: &'static str = Box::leak(selector.into_boxed_str());
    selectors.insert(selector);
    selector
}

#[derive(Clone, Default, PartialEq)]
enum Load {
    /// Not asked yet: Settings asks once it shows.
    #[default]
    Idle,
    Loading,
    Ready(VaultListing),
    Failed(String),
    /// An older server: no `/api/vault`.
    ServingOld,
    /// The server refused this client as not the owner's app (ruling F26).
    OwnerOnly,
}

/// The listing behind the section, while a bruh's Settings show.
#[derive(Default)]
pub(crate) struct VaultGrantsState {
    load: Load,
    /// The listing request in flight; dropping it drops the answer.
    task: Option<Task<()>>,
    /// Every listing asked for, as `METHOD path`; tests never reach a
    /// server.
    #[cfg(test)]
    requests: Vec<String>,
}

impl VaultGrantsState {
    /// Settings closed: forget the listing and any answer still coming.
    pub(super) fn reset(&mut self) {
        self.load = Load::Idle;
        self.task = None;
    }

    pub(super) fn idle(&self) -> bool {
        self.load == Load::Idle
    }

    fn answered(&mut self, result: Result<VaultListing, ServerError>) {
        self.task = None;
        self.load = match result {
            Ok(listing) => Load::Ready(listing),
            Err(error) if error.is_not_found() => Load::ServingOld,
            Err(ServerError::Status { code, .. }) if code == reqwest::StatusCode::FORBIDDEN => {
                Load::OwnerOnly
            }
            Err(error) => Load::Failed(error.vault_message()),
        };
    }
}

impl ReactiveAutomations {
    /// Read the vault listing for the open bruh's Settings.
    pub(super) fn load_bruh_vault(&mut self, cx: &mut Context<'_, Self>) {
        let client = self.client.clone();
        let Some(state) = self.bruh_vault_mut() else {
            return;
        };
        state.load = Load::Loading;
        #[cfg(test)]
        state.requests.push("GET /api/vault".into());
        cx.notify();
        if cfg!(test) {
            return;
        }
        let operation = spawn_tokio(cx, async move { client.vault_listing().await });
        let task = cx.spawn(async move |this, cx| {
            let result = operation.await;
            // The panel may be gone by now; a closed Settings or another
            // bruh's page dropped this task with its state.
            let _ = this.update(cx, |this, cx| this.bruh_vault_answered(result, cx));
        });
        if let Some(state) = self.bruh_vault_mut() {
            state.task = Some(task);
        }
    }

    fn bruh_vault_answered(
        &mut self,
        result: Result<VaultListing, ServerError>,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(state) = self.bruh_vault_mut() {
            state.answered(result);
            cx.notify();
        }
    }

    /// Answer the listing request as the server would.
    #[cfg(test)]
    pub(super) fn answer_bruh_vault_for_test(
        &mut self,
        result: Result<VaultListing, ServerError>,
        cx: &mut Context<'_, Self>,
    ) {
        self.bruh_vault_answered(result, cx);
    }

    #[cfg(test)]
    pub(super) fn bruh_vault_requests_for_test(&self) -> Vec<String> {
        self.bruh_vault()
            .map(|state| state.requests.clone())
            .unwrap_or_default()
    }
}

/// The section: label, hint and why it is off, then the listing's state or
/// its rows, 4px apart so neighbouring touch targets stay apart.
pub(crate) fn vault_grants(
    state: &VaultGrantsState,
    agent_id: &str,
    grants: &[String],
    disabled: Option<&'static str>,
    cx: &Context<'_, ReactiveAutomations>,
) -> impl IntoElement {
    let head = div()
        .min_w_0()
        .flex()
        .flex_col()
        .child(small_text(VAULT_GRANTS_LABEL, theme::rambutan_300()))
        .child(hint(VAULT_GRANTS_HINT))
        .when_some(disabled, |el, reason| {
            el.child(
                div()
                    .debug_selector(|| "desktop-bruh-vault-reason".to_string())
                    .child(hint(reason)),
            )
        });
    let notice = |text: &'static str| {
        div()
            .id("desktop-bruh-vault-notice")
            .debug_selector(|| "desktop-bruh-vault-notice".to_string())
            .role(Role::Status)
            .child(small_text(text, theme::durian_300()))
    };
    let body = match &state.load {
        Load::Idle => None,
        Load::Loading => Some(
            div()
                .id("desktop-bruh-vault-loading")
                .debug_selector(|| "desktop-bruh-vault-loading".to_string())
                .role(Role::Status)
                .child(hint(LOADING))
                .into_any_element(),
        ),
        Load::Failed(message) => Some(
            div()
                .min_w_0()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .id("desktop-bruh-vault-error")
                        .debug_selector(|| "desktop-bruh-vault-error".to_string())
                        .role(Role::Alert)
                        .min_w_0()
                        .child(small_text(
                            format!("{message} {LOAD_RETRY}"),
                            theme::lychee_300(),
                        )),
                )
                .child(page_button(
                    "desktop-bruh-vault-refresh",
                    REFRESH,
                    WebButtonVariant::Primary,
                    false,
                    cx,
                    |this, _, cx| this.load_bruh_vault(cx),
                ))
                .into_any_element(),
        ),
        // An older service already says so through `disabled`.
        Load::ServingOld => disabled
            .is_none()
            .then(|| notice(ServiceNotice::ServingOld.message()).into_any_element()),
        Load::OwnerOnly => Some(notice(OPEN_FROM_OWN_ADDRESS).into_any_element()),
        Load::Ready(listing) => {
            let rows = grant_rows(listing, agent_id, grants);
            let none = !rows
                .iter()
                .any(|row| matches!(row, GrantRow::Shared { .. }));
            let off = disabled.is_some();
            Some(
                div()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .children(rows.into_iter().map(|row| grant_row(row, off, cx))),
                    )
                    .when(none, |el| {
                        el.child(
                            div()
                                .debug_selector(|| "desktop-bruh-vault-none".to_string())
                                .child(hint(VAULT_GRANTS_NONE)),
                        )
                    })
                    .into_any_element(),
            )
        }
    };
    div()
        .id("desktop-bruh-vault")
        .debug_selector(|| "desktop-bruh-vault".to_string())
        .role(Role::Group)
        .aria_label(VAULT_GRANTS_LABEL)
        .min_w_0()
        .flex()
        .flex_col()
        .gap_1()
        .child(head)
        .children(body)
}

fn grant_row(row: GrantRow, off: bool, cx: &Context<'_, ReactiveAutomations>) -> Div {
    let line = div().min_h(px(theme::CONTROL_PX)).flex().items_center();
    match row {
        GrantRow::Own { id, line: text } => line
            .debug_selector(move || format!("desktop-bruh-vault-own-{id}"))
            .child(small_text(text, theme::rambutan_300())),
        GrantRow::Shared { id, label, on } => line
            .justify_between()
            .gap_3()
            .child(small_text(label.clone(), theme::rambutan_200()))
            .child(page_switch(
                grant_selector(&id),
                label,
                on,
                off,
                cx,
                move |this, _, cx| this.toggle_bruh_grant(&id, !on, cx),
            )),
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings gpui's `test`, which `#[test]` would expand.
    use super::{grant_rows, grant_selector, toggle, GrantRow};
    use bruhs_models::secrets::SavedCredentialState;
    use bruhs_models::vault::{
        CredentialScope, LoginFields, LoginMeta, VaultEntry, VaultKind, VaultListing,
    };

    fn entry(id: &str, name: &str, scope: CredentialScope, kind: VaultKind) -> VaultEntry {
        VaultEntry {
            id: id.into(),
            name: name.into(),
            scope,
            kind,
            state: SavedCredentialState::Saved,
            attention: None,
        }
    }

    fn login() -> VaultKind {
        VaultKind::Login(LoginMeta {
            url: "https://github.com/login".into(),
            fields: LoginFields::Saved {
                username: "me".into(),
                totp: false,
            },
        })
    }

    fn bruh(agent_id: &str) -> CredentialScope {
        CredentialScope::Bruh {
            agent_id: agent_id.into(),
        }
    }

    /// The same rows as the web's `grant_rows`.
    #[test]
    fn vault_grants_list_own_entries_then_shared_switches() {
        let listing = VaultListing {
            entries: vec![
                entry("v1-g", "GitHub", CredentialScope::Global, login()),
                entry("v1-s", "Shop", bruh("a"), login()),
                entry("v1-k", "Deploy", CredentialScope::Global, VaultKind::Value),
                entry("v1-o", "Other", bruh("b"), VaultKind::Value),
                entry(
                    "v1-p",
                    "Project",
                    CredentialScope::Project {
                        path: "/work".into(),
                    },
                    VaultKind::Value,
                ),
            ],
            ..VaultListing::default()
        };
        assert_eq!(
            grant_rows(&listing, "a", &["v1-g".into()]),
            vec![
                GrantRow::Own {
                    id: "v1-s".into(),
                    line: "Shop: its own".into(),
                },
                GrantRow::Shared {
                    id: "v1-k".into(),
                    label: "Deploy, Key".into(),
                    on: false,
                },
                GrantRow::Shared {
                    id: "v1-g".into(),
                    label: "GitHub, Login".into(),
                    on: true,
                },
            ]
        );
    }

    #[test]
    fn vault_grants_toggle_keeps_the_stored_order() {
        let mut grants = vec!["v1-k".to_string()];
        toggle(&mut grants, "v1-g", true);
        assert_eq!(grants, ["v1-g", "v1-k"]);
        toggle(&mut grants, "v1-g", true);
        assert_eq!(grants, ["v1-g", "v1-k"]);
        toggle(&mut grants, "v1-g", false);
        assert_eq!(grants, ["v1-k"]);
        assert_eq!(grant_selector("v1-k"), "desktop-bruh-vault-grant-v1-k");
        assert!(std::ptr::eq(grant_selector("v1-k"), grant_selector("v1-k")));
    }
}
