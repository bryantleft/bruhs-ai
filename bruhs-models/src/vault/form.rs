//! The Vault page's form and rows, shared by web and desktop so neither
//! invents a rule or a line of copy.

use super::{
    CredentialScope, LoginFields, LoginInput, LoginMeta, TotpInput, VaultBruh, VaultCreate,
    VaultEntry, VaultKind, VaultSecretInput, VaultUpdate, KIND_KEY, KIND_LOGIN, KIND_ONE_PASSWORD,
    SCOPE_GLOBAL, STATE_IN_ONE_PASSWORD, STATE_NEEDS_ATTENTION, STATE_SAVED,
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
/// one, as does an empty setup key unless Remove one-time code is on.
pub fn form_update(form: &VaultForm, existing: &VaultEntry) -> Result<VaultUpdate, &'static str> {
    let update = VaultUpdate {
        secret: form_secret(form, true),
    };
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
        checkable: true,
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
        edit.url = "https://other.test/in".into();
        assert_eq!(form_update(&edit, &stored).err(), Some(ORIGIN_FIXED));

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
