//! Settings, Apps' forms (D21, ruling F17): the values each app's connect
//! and replace forms ask for, in page order, and the checked request each
//! sends. Web and desktop draw both forms from these.

use super::super::{connector, validate_param, AppConnector, AppParam, ENTER_TOKEN};
use super::{AppConnect, AppReplace, SCOPE_SHARED};
use crate::vault::{
    validate_entry_name, CredentialScope, FormScope, APPS_NOT_PROJECTS, CHOOSE_APP, CHOOSE_BRUH,
    SCOPE_BRUH,
};
use std::collections::BTreeMap;

/// The Used by choices, in page order: an app account is shared with the
/// bruhs it is granted to, or one bruh's own. Never a project (D8).
pub const APP_SCOPE_OPTIONS: [(FormScope, &str, &str); 2] = [
    (FormScope::Global, "global", SCOPE_SHARED),
    (FormScope::Bruh, "bruh", SCOPE_BRUH),
];

/// What the owner typed into one app's connect form, by param id. Holds
/// the token: deliberately no `Debug` or `Clone`. `bruh` is the agent id
/// for a One bruh account.
pub struct ConnectForm {
    pub app: &'static str,
    pub name: String,
    pub scope: FormScope,
    pub bruh: String,
    pub values: BTreeMap<&'static str, String>,
}

/// Where a value sits in the form: where the account lives, then who it
/// is, then its secrets, the token last.
fn field_rank(param: &AppParam) -> u8 {
    match param.id {
        "site" | "shop" => 0,
        "email" => 1,
        "key" => 2,
        _ => 3,
    }
}

/// The values `app`'s connect form asks for, in page order: Site or Shop,
/// Email, API key, then Token.
#[must_use]
pub fn form_fields(app: &AppConnector) -> Vec<&'static AppParam> {
    let params: &'static [AppParam] = app.params;
    let mut fields: Vec<&'static AppParam> = params.iter().collect();
    fields.sort_by_key(|param| field_rank(param));
    fields
}

/// The values a Replace token form asks for: the secret ones, in page
/// order. The account's other values stay as connected.
#[must_use]
pub fn replace_fields(app: &AppConnector) -> Vec<&'static AppParam> {
    form_fields(app)
        .into_iter()
        .filter(|param| param.secret)
        .collect()
}

fn value<'a>(values: &'a BTreeMap<&'static str, String>, id: &str) -> &'a str {
    values.get(id).map_or("", String::as_str)
}

/// The connect request the form sends, checked as the server checks it:
/// the name (optional), the scope, and each value in page order, so the
/// first wrong field is the one named. Values are trimmed, and a site or
/// shop lower-cased.
///
/// # Errors
///
/// The owner copy for the first thing wrong.
pub fn connect_request(form: &ConnectForm) -> Result<AppConnect, &'static str> {
    let app = connector(form.app).ok_or(CHOOSE_APP)?;
    let scope = match form.scope {
        FormScope::Global => CredentialScope::Global,
        FormScope::Project => return Err(APPS_NOT_PROJECTS),
        FormScope::Bruh if form.bruh.trim().is_empty() => return Err(CHOOSE_BRUH),
        FormScope::Bruh => CredentialScope::Bruh {
            agent_id: form.bruh.trim().to_owned(),
        },
    };
    let name = form.name.trim();
    let name = if name.is_empty() {
        None
    } else {
        validate_entry_name(name)?;
        Some(name.to_owned())
    };
    let mut params = BTreeMap::new();
    let (mut token, mut key) = (None, None);
    for param in form_fields(app) {
        let checked = validate_param(param, value(&form.values, param.id))?;
        match param.id {
            "key" => key = Some(checked),
            _ if param.secret => token = Some(checked),
            id => {
                params.insert(id.to_owned(), checked);
            }
        }
    }
    Ok(AppConnect {
        app: app.id.to_owned(),
        name,
        scope,
        params,
        token: token.ok_or(ENTER_TOKEN)?,
        key,
    })
}

/// The replace request for one of `app`'s accounts: a new token, checked,
/// and a new key only when one was typed, since an empty key keeps the
/// stored one.
///
/// # Errors
///
/// The owner copy for the first thing wrong.
pub fn replace_request(
    app: &AppConnector,
    values: &BTreeMap<&'static str, String>,
) -> Result<AppReplace, &'static str> {
    let (mut token, mut key) = (None, None);
    for param in replace_fields(app) {
        let typed = value(values, param.id);
        if param.id == "key" {
            if !typed.trim().is_empty() {
                key = Some(validate_param(param, typed)?);
            }
        } else {
            token = Some(validate_param(param, typed)?);
        }
    }
    Ok(AppReplace {
        token: token.ok_or(ENTER_TOKEN)?,
        key,
    })
}

#[cfg(test)]
mod tests {
    use super::super::super::{connector, ENTER_TOKEN, SITE_SHAPE, TOKEN_SHAPE};
    use super::*;
    use crate::vault::{CredentialScope, FormScope, APPS_NOT_PROJECTS, CHOOSE_BRUH, NAME_TOO_LONG};
    use std::collections::BTreeMap;

    fn ids(app: &str) -> Vec<&'static str> {
        form_fields(connector(app).unwrap())
            .iter()
            .map(|param| param.id)
            .collect()
    }

    fn jira(token: &str) -> ConnectForm {
        ConnectForm {
            app: "jira",
            name: String::new(),
            scope: FormScope::Global,
            bruh: String::new(),
            values: BTreeMap::from([
                ("site", "Acme".into()),
                ("email", "me@acme.test".into()),
                ("token", token.into()),
            ]),
        }
    }

    #[test]
    fn each_app_shows_the_fields_it_needs() {
        assert_eq!(ids("github"), ["token"]);
        assert_eq!(ids("trello"), ["key", "token"]);
        assert_eq!(ids("jira"), ["site", "email", "token"]);
        assert_eq!(ids("shopify"), ["shop", "token"]);
        for app in crate::apps::CONNECTORS {
            let fields = form_fields(&app);
            assert_eq!(fields.len(), app.params.len(), "{}", app.id);
            assert_eq!(fields.last().map(|param| param.id), Some("token"));
            let secret: Vec<_> = replace_fields(&app).iter().map(|p| p.id).collect();
            assert!(secret.ends_with(&["token"]), "{}", app.id);
            assert!(replace_fields(&app).iter().all(|param| param.secret));
        }
        assert_eq!(
            replace_fields(connector("trello").unwrap())
                .iter()
                .map(|param| param.id)
                .collect::<Vec<_>>(),
            ["key", "token"]
        );
    }

    #[test]
    fn a_connect_request_is_checked_and_normalized() {
        let mut form = jira("t0k3n-canary");
        let request = connect_request(&form).unwrap();
        assert_eq!(
            (
                request.app.as_str(),
                request.params.get("site").map(String::as_str)
            ),
            ("jira", Some("acme"))
        );
        assert_eq!(request.scope, CredentialScope::Global);
        assert_eq!(request.name, None);
        assert_eq!(request.token, "t0k3n-canary");
        assert_eq!(request.key, None);
        // Only the non-secret values are params (ruling F24).
        assert_eq!(
            request
                .params
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["email", "site"]
        );
        form.values.insert("token", String::new());
        assert_eq!(connect_request(&form).err(), Some(ENTER_TOKEN));
        form.values.insert("token", "short".into());
        assert_eq!(connect_request(&form).err(), Some(TOKEN_SHAPE));
        // The first field in page order that's wrong is the one named.
        form.values.insert("site", "acme.atlassian.net".into());
        assert_eq!(connect_request(&form).err(), Some(SITE_SHAPE));

        let mut form = jira(" t0k3n-canary ");
        form.name = "  Work Jira ".into();
        form.scope = FormScope::Bruh;
        assert_eq!(connect_request(&form).err(), Some(CHOOSE_BRUH));
        form.bruh = " a ".into();
        let request = connect_request(&form).unwrap();
        assert_eq!(request.name.as_deref(), Some("Work Jira"));
        assert_eq!(request.token, "t0k3n-canary");
        assert_eq!(
            request.scope,
            CredentialScope::Bruh {
                agent_id: "a".into()
            }
        );
        form.scope = FormScope::Project;
        assert_eq!(connect_request(&form).err(), Some(APPS_NOT_PROJECTS));
        form.scope = FormScope::Global;
        form.name = "n".repeat(200);
        assert_eq!(connect_request(&form).err(), Some(NAME_TOO_LONG));
    }

    #[test]
    fn trello_sends_its_key_beside_the_token() {
        let form = ConnectForm {
            app: "trello",
            name: String::new(),
            scope: FormScope::Global,
            bruh: String::new(),
            values: BTreeMap::from([
                ("key", "k3y-canary".into()),
                ("token", "t0k3n-canary".into()),
            ]),
        };
        let request = connect_request(&form).unwrap();
        assert_eq!(request.key.as_deref(), Some("k3y-canary"));
        assert!(request.params.is_empty());
    }

    #[test]
    fn a_replace_needs_the_token_and_keeps_an_empty_key() {
        let trello = connector("trello").unwrap();
        let values = BTreeMap::from([("key", String::new()), ("token", "t0k3n-canary".into())]);
        let replace = replace_request(trello, &values).unwrap();
        assert_eq!(
            (replace.token.as_str(), replace.key),
            ("t0k3n-canary", None)
        );
        let values = BTreeMap::from([
            ("key", "k3y-canary".into()),
            ("token", "t0k3n-canary".into()),
        ]);
        assert_eq!(
            replace_request(trello, &values).unwrap().key.as_deref(),
            Some("k3y-canary")
        );
        let github = connector("github").unwrap();
        assert_eq!(
            replace_request(github, &BTreeMap::new()).err(),
            Some(ENTER_TOKEN)
        );
        // Values that aren't secrets never ride a replace.
        let values = BTreeMap::from([
            ("site", "elsewhere".into()),
            ("token", "t0k3n-canary".into()),
        ]);
        assert!(replace_request(connector("jira").unwrap(), &values).is_ok());
    }

    #[test]
    fn the_used_by_choices_are_shared_or_one_bruh() {
        assert_eq!(
            APP_SCOPE_OPTIONS.map(|(scope, key, _)| (scope, key)),
            [(FormScope::Global, "global"), (FormScope::Bruh, "bruh")]
        );
        assert_eq!(APP_SCOPE_OPTIONS[0].2, super::super::SCOPE_SHARED);
    }
}
