//! The Vault page's form and rows, shared by web and desktop so neither
//! invents a rule or a line of copy.

use super::{
    CredentialScope, LoginFields, LoginInput, LoginMeta, TotpInput, VaultBruh, VaultCreate,
    VaultEntry, VaultKind, VaultSecretInput, VaultUpdate, FIELD_KIND, FIELD_NAME, FIELD_PASSWORD,
    FIELD_PASSWORD_REF, FIELD_REFERENCE, FIELD_SIGN_IN, FIELD_STORED_IN, FIELD_TOTP,
    FIELD_TOTP_REF, FIELD_USED_BY, FIELD_USERNAME, FIELD_USERNAME_REF, FIELD_VALUE, FROM_NEWER,
    KIND_APP, KIND_KEY, KIND_LOGIN, KIND_ONE_PASSWORD, KIND_OTHER, LOADING, MANAGE_IN_APPS,
    SCOPE_BRUH, SCOPE_GLOBAL, SCOPE_PROJECT, SOURCE_ONE_PASSWORD, SOURCE_SAVED,
    STATE_IN_ONE_PASSWORD, STATE_NEEDS_ATTENTION, STATE_SAVED, WAIT_FOR_CHANGE, WAIT_FOR_CHECK,
};
use crate::secrets::{review_text, SavedCredentialState};

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum FormKind {
    #[default]
    Login,
    Value,
    OnePassword,
}

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum FormScope {
    #[default]
    Global,
    Project,
    Bruh,
}

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum FormSource {
    #[default]
    Saved,
    OnePassword,
}

/// The Kind choices, in page order: the choice, its option key and its
/// label.
pub const KIND_OPTIONS: [(FormKind, &str, &str); 3] = [
    (FormKind::Login, "login", KIND_LOGIN),
    (FormKind::Value, "value", KIND_KEY),
    (FormKind::OnePassword, "one_password", KIND_ONE_PASSWORD),
];
/// The Used by choices, in page order.
pub const SCOPE_OPTIONS: [(FormScope, &str, &str); 3] = [
    (FormScope::Global, "global", SCOPE_GLOBAL),
    (FormScope::Project, "project", SCOPE_PROJECT),
    (FormScope::Bruh, "bruh", SCOPE_BRUH),
];
/// The Stored in choices, in page order.
pub const SOURCE_OPTIONS: [(FormSource, &str, &str); 2] = [
    (FormSource::Saved, "saved", SOURCE_SAVED),
    (FormSource::OnePassword, "one_password", SOURCE_ONE_PASSWORD),
];

/// The option key for `choice` in `options`.
#[must_use]
pub fn option_key<T: PartialEq + Copy>(
    options: &[(T, &'static str, &'static str)],
    choice: T,
) -> &'static str {
    options
        .iter()
        .find(|(option, ..)| *option == choice)
        .map_or("", |(_, key, _)| key)
}

/// The choice for an option key; an unknown key is the default choice.
#[must_use]
pub fn option_for<T: Copy + Default>(options: &[(T, &str, &str)], key: &str) -> T {
    options
        .iter()
        .find(|(_, option, _)| *option == key)
        .map_or_else(T::default, |(choice, ..)| *choice)
}

/// One field of the Vault form, in page order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormField {
    Kind,
    Name,
    UsedBy,
    SignIn,
    StoredIn,
    Username,
    Password,
    Totp,
    UsernameRef,
    PasswordRef,
    TotpRef,
    Value,
    Reference,
}

impl FormField {
    pub const ALL: [Self; 13] = [
        Self::Kind,
        Self::Name,
        Self::UsedBy,
        Self::SignIn,
        Self::StoredIn,
        Self::Username,
        Self::Password,
        Self::Totp,
        Self::UsernameRef,
        Self::PasswordRef,
        Self::TotpRef,
        Self::Value,
        Self::Reference,
    ];

    /// Fixed once the entry exists, so an edit draws it read only beside
    /// [`super::FIXED_HINT`]: the name, kind and scope (D2), and a login's
    /// sign-in page, whose origin is where its password may be filled
    /// (ruling F3).
    #[must_use]
    pub fn fixed_on_edit(self) -> bool {
        matches!(self, Self::Kind | Self::Name | Self::UsedBy | Self::SignIn)
    }

    /// The field's label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Kind => FIELD_KIND,
            Self::Name => FIELD_NAME,
            Self::UsedBy => FIELD_USED_BY,
            Self::SignIn => FIELD_SIGN_IN,
            Self::StoredIn => FIELD_STORED_IN,
            Self::Username => FIELD_USERNAME,
            Self::Password => FIELD_PASSWORD,
            Self::Totp => FIELD_TOTP,
            Self::UsernameRef => FIELD_USERNAME_REF,
            Self::PasswordRef => FIELD_PASSWORD_REF,
            Self::TotpRef => FIELD_TOTP_REF,
            Self::Value => FIELD_VALUE,
            Self::Reference => FIELD_REFERENCE,
        }
    }

    /// Holds a value: masked, never filled, emptied on submit and close.
    #[must_use]
    pub fn holds_value(self) -> bool {
        matches!(self, Self::Password | Self::Totp | Self::Value)
    }
}

/// The fields a form for `kind` shows, in page order. An edit shows the
/// same ones, with the [`FormField::fixed_on_edit`] ones read only.
#[must_use]
pub fn form_fields(kind: FormKind, source: FormSource) -> Vec<FormField> {
    use FormField::{
        Kind, Name, Password, PasswordRef, Reference, SignIn, StoredIn, Totp, TotpRef, UsedBy,
        Username, UsernameRef, Value,
    };
    let mut fields = vec![Kind, Name, UsedBy];
    match (kind, source) {
        (FormKind::Login, FormSource::Saved) => {
            fields.extend([SignIn, StoredIn, Username, Password, Totp]);
        }
        (FormKind::Login, FormSource::OnePassword) => {
            fields.extend([SignIn, StoredIn, UsernameRef, PasswordRef, TotpRef]);
        }
        (FormKind::Value, _) => fields.push(Value),
        (FormKind::OnePassword, _) => fields.push(Reference),
    }
    fields
}

/// The fixed field an edit draws [`super::FIXED_HINT`] under: the last
/// one the form shows.
#[must_use]
pub fn fixed_hint_after(fields: &[FormField]) -> Option<FormField> {
    fields
        .iter()
        .rev()
        .copied()
        .find(|field| field.fixed_on_edit())
}

/// What the Vault page is waiting on, if anything.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VaultBusy {
    #[default]
    Idle,
    Changing,
    Checking,
}

/// Why the page's buttons are disabled right now, if they are: the
/// listing loading, a check, or a change.
#[must_use]
pub fn wait_reason(busy: VaultBusy, loading: bool) -> Option<&'static str> {
    match busy {
        VaultBusy::Idle => None,
        _ if loading => Some(LOADING),
        VaultBusy::Checking => Some(WAIT_FOR_CHECK),
        VaultBusy::Changing => Some(WAIT_FOR_CHANGE),
    }
}

/// What the owner typed. Holds values: deliberately no `Debug` or `Clone`.
/// `project` is the folder for a Project scope and `bruh` the agent id for
/// a Bruh scope.
#[derive(Default)]
pub struct VaultForm {
    pub name: String,
    pub kind: FormKind,
    pub scope: FormScope,
    pub project: String,
    pub bruh: String,
    pub value: String,
    pub url: String,
    pub source: FormSource,
    pub username: String,
    pub password: String,
    pub totp: String,
    pub remove_totp: bool,
    pub reference: String,
    pub username_ref: String,
    pub password_ref: String,
    pub totp_ref: String,
}

#[must_use]
pub fn kind_label(kind: FormKind) -> &'static str {
    match kind {
        FormKind::Login => KIND_LOGIN,
        FormKind::Value => KIND_KEY,
        FormKind::OnePassword => KIND_ONE_PASSWORD,
    }
}

/// A new entry from the form, checked ([`VaultCreate::validate`]).
pub fn form_create(form: &VaultForm) -> Result<VaultCreate, &'static str> {
    let scope = match form.scope {
        FormScope::Global => CredentialScope::Global,
        FormScope::Project => CredentialScope::Project {
            path: form.project.trim().into(),
        },
        FormScope::Bruh => CredentialScope::Bruh {
            agent_id: form.bruh.trim().into(),
        },
    };
    let mut create = VaultCreate {
        name: form.name.trim().into(),
        scope,
        secret: form_secret(form, false),
    };
    create.validate()?;
    Ok(create)
}

/// A change to `existing` from the form, checked against its kind
/// ([`VaultUpdate::validate`]). An empty value or password keeps the stored
/// one, as does an empty setup key unless Remove one-time code is on. A
/// login keeps its stored sign-in page (ruling F3): the form draws it read
/// only, and the body never carries another.
pub fn form_update(form: &VaultForm, existing: &VaultEntry) -> Result<VaultUpdate, &'static str> {
    let mut secret = form_secret(form, true);
    if let (VaultSecretInput::Login { url, .. }, VaultKind::Login(LoginMeta { url: stored, .. })) =
        (&mut secret, &existing.kind)
    {
        url.clone_from(stored);
    }
    let update = VaultUpdate { secret };
    update.validate(&existing.kind)?;
    Ok(update)
}

fn form_secret(form: &VaultForm, editing: bool) -> VaultSecretInput {
    let given = |text: &str| (!text.is_empty()).then(|| text.to_owned());
    match form.kind {
        FormKind::Value => VaultSecretInput::Value {
            value: given(&form.value),
        },
        FormKind::OnePassword => VaultSecretInput::OnePassword {
            reference: form.reference.trim().into(),
        },
        FormKind::Login => VaultSecretInput::Login {
            url: form.url.trim().into(),
            fields: match form.source {
                FormSource::Saved => LoginInput::Saved {
                    username: form.username.trim().into(),
                    password: given(&form.password),
                    totp: if editing && form.remove_totp {
                        TotpInput::Remove
                    } else if form.totp.trim().is_empty() {
                        TotpInput::Keep
                    } else {
                        TotpInput::Set {
                            setup: form.totp.trim().into(),
                        }
                    },
                },
                FormSource::OnePassword => LoginInput::OnePassword {
                    username: form.username_ref.trim().into(),
                    password: form.password_ref.trim().into(),
                    totp: given(form.totp_ref.trim()),
                },
            },
        },
    }
}

/// The form for editing `entry`: its metadata filled in, every value empty.
/// An app token or a newer Bruhs' kind isn't editable
/// ([`VaultKind::editable`]): its form keeps only the name and scope, and
/// [`form_update`] refuses any change built from it.
#[must_use]
pub fn form_for(entry: &VaultEntry) -> VaultForm {
    let mut form = VaultForm {
        name: entry.name.clone(),
        ..VaultForm::default()
    };
    match &entry.scope {
        CredentialScope::Global => form.scope = FormScope::Global,
        CredentialScope::Project { path } => {
            form.scope = FormScope::Project;
            form.project.clone_from(path);
        }
        CredentialScope::Bruh { agent_id } => {
            form.scope = FormScope::Bruh;
            form.bruh.clone_from(agent_id);
        }
    }
    match &entry.kind {
        VaultKind::Value => form.kind = FormKind::Value,
        VaultKind::OnePassword { reference } => {
            form.kind = FormKind::OnePassword;
            form.reference.clone_from(reference);
        }
        VaultKind::Login(LoginMeta { url, fields }) => {
            form.kind = FormKind::Login;
            form.url.clone_from(url);
            match fields {
                LoginFields::Saved { username, .. } => {
                    form.source = FormSource::Saved;
                    form.username.clone_from(username);
                }
                LoginFields::OnePassword {
                    username,
                    password,
                    totp,
                } => {
                    form.source = FormSource::OnePassword;
                    form.username_ref.clone_from(username);
                    form.password_ref.clone_from(password);
                    form.totp_ref = totp.clone().unwrap_or_default();
                }
            }
        }
        VaultKind::AppToken(_) | VaultKind::Other => {}
    }
    form
}

/// One row of the Vault page. Metadata only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultRow {
    pub id: String,
    pub name: String,
    pub kind: &'static str,
    pub scope: String,
    pub detail: Option<String>,
    pub state: &'static str,
    pub attention: Option<&'static str>,
    pub checkable: bool,
    /// Whether the page offers Edit ([`VaultKind::editable`]).
    pub editable: bool,
    /// Where the entry is managed instead, shown in place of Edit:
    /// [`MANAGE_IN_APPS`] for an app token. A newer Bruhs' kind says
    /// [`FROM_NEWER`] in its detail.
    pub hint: Option<&'static str>,
}

fn bruh_name<'a>(bruhs: &'a [VaultBruh], agent_id: &str) -> Option<&'a str> {
    bruhs
        .iter()
        .find(|bruh| bruh.agent_id == agent_id)
        .map(|bruh| bruh.name.as_str())
}

fn scope_line(scope: &CredentialScope, bruhs: &[VaultBruh]) -> String {
    match scope {
        CredentialScope::Global => SCOPE_GLOBAL.into(),
        CredentialScope::Project { path } => format!("Project: {}", review_text(path)),
        CredentialScope::Bruh { agent_id } => {
            let name = bruh_name(bruhs, agent_id).unwrap_or(agent_id);
            format!("{}'s own", review_text(name))
        }
    }
}

fn detail_line(kind: &VaultKind) -> Option<String> {
    match kind {
        VaultKind::Value => None,
        VaultKind::AppToken(meta) => {
            let app = crate::apps::connector(&meta.app)
                .map_or_else(|| review_text(&meta.app), |connector| connector.name.into());
            Some(format!("{app} as {}", review_text(&meta.account)))
        }
        VaultKind::Other => Some(FROM_NEWER.into()),
        VaultKind::OnePassword { reference } => Some(review_text(reference)),
        VaultKind::Login(meta) => {
            let origin = meta.origin().unwrap_or_else(|| review_text(&meta.url));
            Some(match &meta.fields {
                LoginFields::OnePassword { .. } => format!("{origin} from 1Password"),
                LoginFields::Saved { username, totp } => {
                    let mut line = if username.is_empty() {
                        origin
                    } else {
                        format!("{origin} as {}", review_text(username))
                    };
                    if *totp {
                        line.push_str(", with a one-time code");
                    }
                    line
                }
            })
        }
    }
}

/// How an entry reads on the Vault page.
#[must_use]
pub fn vault_row(entry: &VaultEntry, bruhs: &[VaultBruh]) -> VaultRow {
    let kind = match &entry.kind {
        VaultKind::Value => KIND_KEY,
        VaultKind::Login(_) => KIND_LOGIN,
        VaultKind::OnePassword { .. } => KIND_ONE_PASSWORD,
        VaultKind::AppToken(_) => KIND_APP,
        VaultKind::Other => KIND_OTHER,
    };
    // Ruling F2: only values read from 1Password say so; a Saved login is
    // in the keychain, like a key.
    let from_one_password = matches!(
        &entry.kind,
        VaultKind::OnePassword { .. }
            | VaultKind::Login(LoginMeta {
                fields: LoginFields::OnePassword { .. },
                ..
            })
    );
    let state = if from_one_password {
        STATE_IN_ONE_PASSWORD
    } else {
        match entry.state {
            SavedCredentialState::Saved => STATE_SAVED,
            SavedCredentialState::NeedsAttention => STATE_NEEDS_ATTENTION,
        }
    };
    VaultRow {
        id: entry.id.clone(),
        name: review_text(&entry.name),
        kind,
        scope: scope_line(&entry.scope, bruhs),
        detail: detail_line(&entry.kind),
        state,
        attention: entry.attention.map(super::VaultAttention::message),
        // An app token's Check runs its test call; a newer kind can't be read.
        checkable: !matches!(entry.kind, VaultKind::Other),
        editable: entry.kind.editable(),
        hint: matches!(entry.kind, VaultKind::AppToken(_)).then_some(MANAGE_IN_APPS),
    }
}

/// Whether an entry matches a search: its name, sign-in page, project
/// folder or bruh name, ignoring case. An empty search matches everything.
#[must_use]
pub fn matches_search(entry: &VaultEntry, bruhs: &[VaultBruh], query: &str) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    let found = |text: &str| text.to_lowercase().contains(&query);
    found(&entry.name)
        || matches!(&entry.kind, VaultKind::Login(meta) if found(&meta.url))
        || match &entry.scope {
            CredentialScope::Global => false,
            CredentialScope::Project { path } => found(path),
            CredentialScope::Bruh { agent_id } => bruh_name(bruhs, agent_id).is_some_and(found),
        }
}

/// A row's accessible name.
#[must_use]
pub fn row_label(name: &str) -> String {
    format!("Vault entry: {}", review_text(name))
}

/// A bruh as the One bruh picker lists it.
#[must_use]
pub fn bruh_choice_label(bruh: &VaultBruh) -> String {
    review_text(&bruh.name)
}

#[must_use]
pub fn delete_entry_confirm(name: &str) -> String {
    format!(
        "Delete {} from the vault? Bruhs and threads that use it lose it.",
        review_text(name)
    )
}

#[cfg(test)]
mod tests {
    use super::super::tests::{assert_no_debug_or_clone, canary};
    use super::super::*;

    fn ada() -> Vec<VaultBruh> {
        vec![VaultBruh {
            agent_id: "a".into(),
            name: "Ada".into(),
        }]
    }

    #[test]
    fn forms_build_requests_and_rows_read_plainly() {
        let form = VaultForm {
            name: "Shop".into(),
            kind: FormKind::Login,
            url: "https://shop.test/in".into(),
            username: "me@shop.test".into(),
            password: "pw".into(),
            ..Default::default()
        };
        assert!(matches!(
            form_create(&form).unwrap().secret,
            VaultSecretInput::Login { .. }
        ));
        let one_password = VaultForm {
            kind: FormKind::Login,
            source: FormSource::OnePassword,
            url: "https://shop.test/in".into(),
            name: "Shop".into(),
            username_ref: "op://P/Shop/username".into(),
            password_ref: "op://P/Shop/password".into(),
            ..Default::default()
        };
        assert!(form_create(&one_password).is_ok());
        let entry = VaultEntry {
            id: "v1-a".into(),
            name: "Shop".into(),
            scope: CredentialScope::Bruh {
                agent_id: "a".into(),
            },
            kind: VaultKind::Login(LoginMeta {
                url: "https://shop.test/in".into(),
                fields: LoginFields::Saved {
                    username: "me".into(),
                    totp: true,
                },
            }),
            state: SavedCredentialState::Saved,
            attention: Some(VaultAttention::OnePasswordNeedsYou),
        };
        let row = vault_row(&entry, &ada());
        assert_eq!(
            (row.kind, row.scope.as_str(), row.detail.as_deref()),
            (
                "Login",
                "Ada's own",
                Some("https://shop.test as me, with a one-time code")
            )
        );
        assert_eq!(
            row.attention,
            Some("1Password needs you. Sign in to the 1Password app, then check again.")
        );
        // Ruling F2: a Saved login is in the keychain, not 1Password.
        assert_eq!(row.state, STATE_SAVED);
        assert!(form_for(&entry).password.is_empty());
        assert!(matches_search(&entry, &ada(), "ada"));
        assert_eq!(
            delete_entry_confirm("Shop"),
            "Delete Shop from the vault? Bruhs and threads that use it lose it."
        );
    }

    fn entry(scope: CredentialScope, kind: VaultKind) -> VaultEntry {
        VaultEntry {
            id: "v1-b".into(),
            name: "Deploy".into(),
            scope,
            kind,
            state: SavedCredentialState::Saved,
            attention: None,
        }
    }

    #[test]
    fn rows_read_each_scope_kind_and_state() {
        let key = entry(
            CredentialScope::Project {
                path: "/w/a\u{202e}b".into(),
            },
            VaultKind::Value,
        );
        let row = vault_row(&key, &[]);
        assert_eq!(row.kind, "Key");
        assert_eq!(row.scope, "Project: /w/a\\u{202e}b");
        assert_eq!((row.detail, row.state), (None, STATE_SAVED));
        assert!(row.checkable && row.attention.is_none());
        let mut stuck = key;
        stuck.state = SavedCredentialState::NeedsAttention;
        assert_eq!(vault_row(&stuck, &[]).state, STATE_NEEDS_ATTENTION);

        let reference = entry(
            CredentialScope::Global,
            VaultKind::OnePassword {
                reference: "op://P/Deploy/token".into(),
            },
        );
        let row = vault_row(&reference, &[]);
        assert_eq!(
            (
                row.kind,
                row.scope.as_str(),
                row.detail.as_deref(),
                row.state
            ),
            (
                "1Password",
                "All projects",
                Some("op://P/Deploy/token"),
                STATE_IN_ONE_PASSWORD
            )
        );

        let login = entry(
            CredentialScope::Bruh {
                agent_id: "gone".into(),
            },
            VaultKind::Login(LoginMeta {
                url: "https://Shop.test:443/in".into(),
                fields: LoginFields::OnePassword {
                    username: "op://P/S/username".into(),
                    password: "op://P/S/password".into(),
                    totp: None,
                },
            }),
        );
        let row = vault_row(&login, &ada());
        assert_eq!(
            (row.scope.as_str(), row.detail.as_deref(), row.state),
            (
                "gone's own",
                Some("https://shop.test from 1Password"),
                STATE_IN_ONE_PASSWORD
            )
        );
    }

    /// App tokens and entries from a newer Bruhs are read only here: the
    /// form can't hold them, and any update it builds is refused.
    #[test]
    fn app_tokens_and_other_kinds_are_not_editable() {
        let app = entry(
            CredentialScope::Global,
            VaultKind::AppToken(AppTokenMeta {
                app: "jira".into(),
                account: "me\u{202e}".into(),
                params: std::collections::BTreeMap::new(),
            }),
        );
        let row = vault_row(&app, &[]);
        assert_eq!(
            (row.kind, row.detail.as_deref(), row.hint, row.editable),
            (
                KIND_APP,
                Some("Jira as me\\u{202e}"),
                Some(MANAGE_IN_APPS),
                false
            )
        );
        // An app the catalog no longer has still reads plainly.
        let gone = entry(
            CredentialScope::Global,
            VaultKind::AppToken(AppTokenMeta {
                app: "gone".into(),
                account: "me".into(),
                params: std::collections::BTreeMap::new(),
            }),
        );
        assert_eq!(vault_row(&gone, &[]).detail.as_deref(), Some("gone as me"));
        let other = entry(CredentialScope::Global, VaultKind::Other);
        let row = vault_row(&other, &[]);
        assert_eq!(
            (row.kind, row.editable, row.checkable),
            (KIND_OTHER, false, false)
        );
        for stored in [&app, &other] {
            assert!(!stored.kind.editable());
            let mut form = form_for(stored);
            assert_eq!(form.name, "Deploy");
            form.value = "v".into();
            for kind in [FormKind::Value, FormKind::Login, FormKind::OnePassword] {
                form.kind = kind;
                assert_eq!(form_update(&form, stored).err(), Some(KIND_FIXED));
            }
        }
        for kind in [
            VaultKind::Value,
            VaultKind::OnePassword {
                reference: "op://P/I/f".into(),
            },
        ] {
            assert!(
                kind.editable() && vault_row(&entry(CredentialScope::Global, kind), &[]).editable
            );
        }
        let key = vault_row(&entry(CredentialScope::Global, VaultKind::Value), &[]);
        assert_eq!(key.hint, None);
    }

    #[test]
    fn searches_look_at_names_sites_folders_and_bruhs() {
        let login = entry(
            CredentialScope::Global,
            VaultKind::Login(LoginMeta {
                url: "https://Shop.test/in".into(),
                fields: LoginFields::Saved {
                    username: "me".into(),
                    totp: false,
                },
            }),
        );
        assert!(matches_search(&login, &[], "SHOP.test"));
        assert!(matches_search(&login, &[], "deploy"));
        assert!(matches_search(&login, &[], "  "));
        assert!(!matches_search(&login, &[], "me"));
        let key = entry(
            CredentialScope::Project {
                path: "/Work/App".into(),
            },
            VaultKind::Value,
        );
        assert!(matches_search(&key, &[], "work/app"));
        assert!(!matches_search(&key, &ada(), "ada"));
        let own = entry(
            CredentialScope::Bruh {
                agent_id: "a".into(),
            },
            VaultKind::Value,
        );
        assert!(matches_search(&own, &ada(), "AD"));
        assert!(!matches_search(&own, &[], "ada"));
    }

    #[test]
    fn forms_map_scopes_and_keep_empty_values() {
        let key = VaultForm {
            name: " Deploy ".into(),
            kind: FormKind::Value,
            scope: FormScope::Project,
            project: " /w ".into(),
            value: "v".into(),
            ..Default::default()
        };
        let create = form_create(&key).unwrap();
        assert_eq!(create.name, "Deploy");
        assert_eq!(create.scope, CredentialScope::Project { path: "/w".into() });
        let no_bruh = VaultForm {
            scope: FormScope::Bruh,
            ..VaultForm {
                name: "N".into(),
                kind: FormKind::Value,
                value: "v".into(),
                ..Default::default()
            }
        };
        assert_eq!(form_create(&no_bruh).err(), Some(CHOOSE_BRUH));
        let empty = VaultForm {
            name: "N".into(),
            kind: FormKind::Value,
            ..Default::default()
        };
        assert_eq!(form_create(&empty).err(), Some(ENTER_VALUE));

        let stored = entry(CredentialScope::Global, VaultKind::Value);
        let mut edit = form_for(&stored);
        assert_eq!(
            (edit.kind, edit.scope, edit.name.as_str()),
            (FormKind::Value, FormScope::Global, "Deploy")
        );
        let update = form_update(&edit, &stored).unwrap();
        assert!(matches!(
            update.secret,
            VaultSecretInput::Value { value: None }
        ));
        edit.kind = FormKind::OnePassword;
        edit.reference = "op://P/A/b".into();
        assert_eq!(form_update(&edit, &stored).err(), Some(KIND_FIXED));
    }

    #[test]
    fn login_edits_keep_values_and_the_origin() {
        let stored = entry(
            CredentialScope::Global,
            VaultKind::Login(LoginMeta {
                url: "https://shop.test/in".into(),
                fields: LoginFields::Saved {
                    username: "me".into(),
                    totp: true,
                },
            }),
        );
        let mut edit = form_for(&stored);
        assert_eq!(
            (
                edit.kind,
                edit.source,
                edit.url.as_str(),
                edit.username.as_str()
            ),
            (
                FormKind::Login,
                FormSource::Saved,
                "https://shop.test/in",
                "me"
            )
        );
        assert!(edit.password.is_empty() && edit.totp.is_empty() && !edit.remove_totp);
        let update = form_update(&edit, &stored).unwrap();
        assert!(matches!(
            update.secret,
            VaultSecretInput::Login {
                fields: LoginInput::Saved {
                    password: None,
                    totp: TotpInput::Keep,
                    ..
                },
                ..
            }
        ));
        edit.remove_totp = true;
        let update = form_update(&edit, &stored).unwrap();
        assert!(matches!(
            update.secret,
            VaultSecretInput::Login {
                fields: LoginInput::Saved {
                    totp: TotpInput::Remove,
                    ..
                },
                ..
            }
        ));
        edit.remove_totp = false;
        edit.totp = " GEZDGNBVGY3TQOJQ ".into();
        let update = form_update(&edit, &stored).unwrap();
        assert!(matches!(
            update.secret,
            VaultSecretInput::Login {
                fields: LoginInput::Saved {
                    totp: TotpInput::Set { .. },
                    ..
                },
                ..
            }
        ));
        // Ruling F3: the sign-in page is fixed on an edit. Whatever the form
        // holds, the body carries the stored page, so no update moves where
        // the password is filled; the store still refuses one that does.
        edit.url = "https://other.test/in".into();
        let update = form_update(&edit, &stored).unwrap();
        assert!(matches!(
            &update.secret,
            VaultSecretInput::Login { url, .. } if url == "https://shop.test/in"
        ));
        let moved = VaultUpdate {
            secret: VaultSecretInput::Login {
                url: "https://other.test/in".into(),
                fields: LoginInput::Saved {
                    username: "me".into(),
                    password: None,
                    totp: TotpInput::Keep,
                },
            },
        };
        assert_eq!(moved.validate(&stored.kind).err(), Some(ORIGIN_FIXED));

        let referenced = entry(
            CredentialScope::Global,
            VaultKind::Login(LoginMeta {
                url: "https://shop.test/in".into(),
                fields: LoginFields::OnePassword {
                    username: "op://P/S/username".into(),
                    password: "op://P/S/password".into(),
                    totp: Some("op://P/S/otp?attribute=otp".into()),
                },
            }),
        );
        let edit = form_for(&referenced);
        assert_eq!(
            (
                edit.source,
                edit.username_ref.as_str(),
                edit.password_ref.as_str(),
                edit.totp_ref.as_str()
            ),
            (
                FormSource::OnePassword,
                "op://P/S/username",
                "op://P/S/password",
                "op://P/S/otp?attribute=otp"
            )
        );
        assert!(form_update(&edit, &referenced).is_ok());
        let reference = entry(
            CredentialScope::Bruh {
                agent_id: "a".into(),
            },
            VaultKind::OnePassword {
                reference: "op://P/A/b".into(),
            },
        );
        let edit = form_for(&reference);
        assert_eq!(
            (
                edit.kind,
                edit.scope,
                edit.bruh.as_str(),
                edit.reference.as_str()
            ),
            (FormKind::OnePassword, FormScope::Bruh, "a", "op://P/A/b")
        );
    }

    #[test]
    fn the_form_shows_the_fields_its_kind_needs() {
        use FormField::*;
        assert_eq!(
            form_fields(FormKind::Login, FormSource::Saved),
            [Kind, Name, UsedBy, SignIn, StoredIn, Username, Password, Totp]
        );
        assert_eq!(
            form_fields(FormKind::Login, FormSource::OnePassword),
            [
                Kind,
                Name,
                UsedBy,
                SignIn,
                StoredIn,
                UsernameRef,
                PasswordRef,
                TotpRef
            ]
        );
        for source in [FormSource::Saved, FormSource::OnePassword] {
            assert_eq!(
                form_fields(FormKind::Value, source),
                [Kind, Name, UsedBy, Value]
            );
            assert_eq!(
                form_fields(FormKind::OnePassword, source),
                [Kind, Name, UsedBy, Reference]
            );
        }
        // Every field the form can draw is in page order in ALL.
        for (kind, source) in [
            (FormKind::Login, FormSource::Saved),
            (FormKind::Login, FormSource::OnePassword),
            (FormKind::Value, FormSource::Saved),
            (FormKind::OnePassword, FormSource::Saved),
        ] {
            let order: Vec<_> = FormField::ALL
                .into_iter()
                .filter(|field| form_fields(kind, source).contains(field))
                .collect();
            assert_eq!(order, form_fields(kind, source));
        }
    }

    #[test]
    fn edits_fix_the_name_kind_scope_and_sign_in_page() {
        let fixed: Vec<_> = form_fields(FormKind::Login, FormSource::Saved)
            .into_iter()
            .filter(|field| field.fixed_on_edit())
            .collect();
        assert_eq!(
            fixed,
            [
                FormField::Kind,
                FormField::Name,
                FormField::UsedBy,
                FormField::SignIn
            ]
        );
        let values: Vec<_> = FormField::ALL
            .into_iter()
            .filter(|field| field.holds_value())
            .collect();
        assert_eq!(
            values,
            [FormField::Password, FormField::Totp, FormField::Value]
        );
        assert!(!values.iter().any(|field| field.fixed_on_edit()));
        assert_eq!(
            fixed_hint_after(&form_fields(FormKind::Login, FormSource::OnePassword)),
            Some(FormField::SignIn)
        );
        assert_eq!(
            fixed_hint_after(&form_fields(FormKind::Value, FormSource::Saved)),
            Some(FormField::UsedBy)
        );
    }

    #[test]
    fn fields_rows_and_bruhs_read_plainly() {
        assert_eq!(
            FormField::ALL.map(FormField::label),
            [
                FIELD_KIND,
                FIELD_NAME,
                FIELD_USED_BY,
                FIELD_SIGN_IN,
                FIELD_STORED_IN,
                FIELD_USERNAME,
                FIELD_PASSWORD,
                FIELD_TOTP,
                FIELD_USERNAME_REF,
                FIELD_PASSWORD_REF,
                FIELD_TOTP_REF,
                FIELD_VALUE,
                FIELD_REFERENCE
            ]
        );
        assert_eq!(row_label("Shop"), "Vault entry: Shop");
        assert_eq!(row_label("a\u{202e}b"), "Vault entry: a\\u{202e}b");
        let bruh = VaultBruh {
            agent_id: "a".into(),
            name: "Ash\u{202e}".into(),
        };
        assert_eq!(bruh_choice_label(&bruh), "Ash\\u{202e}");
    }

    #[test]
    fn select_options_round_trip_and_unknown_keys_fall_back() {
        for (choice, key, label) in KIND_OPTIONS {
            assert_eq!(
                option_for(&KIND_OPTIONS, option_key(&KIND_OPTIONS, choice)),
                choice
            );
            assert!(!key.is_empty());
            assert_eq!(label, kind_label(choice));
        }
        for (choice, ..) in SCOPE_OPTIONS {
            assert_eq!(
                option_for(&SCOPE_OPTIONS, option_key(&SCOPE_OPTIONS, choice)),
                choice
            );
        }
        assert_eq!(option_for(&SCOPE_OPTIONS, "bruh"), FormScope::Bruh);
        assert_eq!(option_for(&SOURCE_OPTIONS, "nope"), FormSource::Saved);
        assert_eq!(
            SOURCE_OPTIONS.map(|(_, _, label)| label),
            [SOURCE_SAVED, SOURCE_ONE_PASSWORD]
        );
    }

    #[test]
    fn buttons_say_why_they_wait() {
        assert_eq!(wait_reason(VaultBusy::Changing, true), Some(LOADING));
        assert_eq!(
            wait_reason(VaultBusy::Changing, false),
            Some(WAIT_FOR_CHANGE)
        );
        assert_eq!(
            wait_reason(VaultBusy::Checking, false),
            Some(WAIT_FOR_CHECK)
        );
        assert_eq!(wait_reason(VaultBusy::Idle, true), None);
        assert_eq!(wait_reason(VaultBusy::Idle, false), None);
    }

    #[test]
    fn kinds_have_their_labels() {
        assert_eq!(
            [FormKind::Login, FormKind::Value, FormKind::OnePassword].map(kind_label),
            ["Login", "Key", "1Password"]
        );
    }

    #[test]
    fn forms_and_rows_never_carry_values() {
        assert_no_debug_or_clone!(VaultForm);
        let (password, setup_word) = (canary(), canary());
        let form = VaultForm {
            name: "Shop".into(),
            kind: FormKind::Login,
            url: "https://shop.test/in".into(),
            username: "me".into(),
            password: password.clone(),
            totp: format!("GEZDGNBVGY3TQOJQ{setup_word}"),
            ..Default::default()
        };
        let error = form_create(&form).err().unwrap();
        assert!(!error.contains(&setup_word) && !error.contains(&password));
        let mut form = form;
        form.totp = "GEZDGNBVGY3TQOJQ".into();
        let create = form_create(&form).unwrap();
        let stored = VaultEntry {
            id: "v1-c".into(),
            name: create.name.clone(),
            scope: create.scope.clone(),
            kind: create.secret.kind_meta().unwrap(),
            state: SavedCredentialState::Saved,
            attention: None,
        };
        let row = vault_row(&stored, &[]);
        let text = format!("{row:?}{stored:?}");
        assert!(!text.contains(&password), "{text}");
        let edit = form_for(&stored);
        assert!(edit.password.is_empty() && edit.totp.is_empty() && edit.value.is_empty());
    }
}
