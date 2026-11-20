use super::{AppsSettings, CredentialsChanged, Load, Notice, Open};
use crate::server_client::{ServerClient, ServerError};
use crate::{app_state::DesktopState, theme};
use bruhs_models::apps::{
    connected_as, disconnect_confirm, AppAccount, AppMailbox, AppTestResult, AppsBruh, AppsListing,
    CONNECTORS,
};
use bruhs_models::connections::ServiceProvider;
use bruhs_models::crew::{
    apps_reason, AppGrant, ServiceNotice, APPS_NEED_ASK, APPS_PROTOCOL, VAULT_PROTOCOL,
};
use bruhs_models::secrets::SavedCredentialState;
use bruhs_models::vault::{CredentialScope, VaultAttention, VaultBusy};
use gpui::{
    AppContext as _, Entity, FocusHandle, Focusable as _, Modifiers, TestAppContext,
    VisualTestContext,
};
use std::{cell::RefCell, rc::Rc};

fn github() -> AppAccount {
    AppAccount {
        id: "v1-gh".into(),
        app: "github".into(),
        name: "GitHub octo".into(),
        account: "octo".into(),
        scope: CredentialScope::Global,
        state: SavedCredentialState::Saved,
        attention: Some(VaultAttention::KeychainNeedsYou),
    }
}

fn listing() -> AppsListing {
    AppsListing {
        accounts: vec![github()],
        mailboxes: vec![AppMailbox {
            mailbox_id: "m1".into(),
            provider: ServiceProvider::Gmail,
            account: "me@gmail.test".into(),
        }],
        bruhs: vec![
            AppsBruh {
                agent_id: "ash".into(),
                name: "Ash".into(),
                apps: Vec::new(),
                mail: Vec::new(),
                unavailable: None,
            },
            AppsBruh {
                agent_id: "cody".into(),
                name: "Cody".into(),
                apps: Vec::new(),
                mail: Vec::new(),
                unavailable: Some(APPS_NEED_ASK.into()),
            },
        ],
        protocol: APPS_PROTOCOL,
    }
}

/// The page alone in a window, its listing as `listing` (or none yet).
fn page(
    cx: &mut TestAppContext,
    listing: Option<AppsListing>,
) -> (Entity<AppsSettings>, &mut VisualTestContext) {
    cx.update(|cx| {
        bruhs_ui::init(cx);
        theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().unwrap());
    });
    let capture = Rc::new(RefCell::new(None));
    let output = capture.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let client = ServerClient::new("http://127.0.0.1:9", "http://127.0.0.1:9").unwrap();
        let view = cx.new(|cx| AppsSettings::new(client, window, cx));
        *output.borrow_mut() = Some(view.clone());
        bruhs_ui::Root::new(view, window, cx)
    });
    let view = capture.borrow().clone().unwrap();
    // Fourteen apps and the Mail group: room for every one, so clicks
    // land on screen.
    cx.simulate_resize(gpui::size(gpui::px(900.0), gpui::px(3200.0)));
    cx.run_until_parked();
    if let Some(listing) = listing {
        view.update(cx, |view, cx| view.show_listing_for_test(listing, cx));
        cx.run_until_parked();
    }
    (view, cx)
}

fn requests(view: &Entity<AppsSettings>, cx: &mut VisualTestContext) -> Vec<String> {
    view.read_with(cx, |view, _| view.apps_requests_for_test())
}

fn leak(selector: &str) -> &'static str {
    Box::leak(selector.to_owned().into_boxed_str())
}

fn rendered(cx: &mut VisualTestContext, selector: &str) -> bool {
    cx.debug_bounds(leak(selector)).is_some()
}

fn top(cx: &mut VisualTestContext, selector: &str) -> gpui::Pixels {
    cx.debug_bounds(leak(selector))
        .unwrap_or_else(|| panic!("{selector} is not drawn"))
        .top()
}

fn click(cx: &mut VisualTestContext, selector: &str) {
    let bounds = cx
        .debug_bounds(leak(selector))
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

fn param(
    view: &Entity<AppsSettings>,
    cx: &mut VisualTestContext,
    id: &str,
) -> Entity<bruhs_ui::input::InputState> {
    view.read_with(cx, |view, _| {
        view.form
            .as_ref()
            .and_then(|form| form.param(id))
            .cloned()
            .unwrap_or_else(|| panic!("the form has no {id} field"))
    })
}

fn value(cx: &mut VisualTestContext, input: &Entity<bruhs_ui::input::InputState>) -> String {
    cx.update(|_, cx| input.read(cx).value().to_string())
}

/// Tab until `target` has focus, as the owner would.
fn tab_to(cx: &mut VisualTestContext, target: &Entity<bruhs_ui::input::InputState>) {
    let handle = cx.update(|_, cx| target.focus_handle(cx));
    for _ in 0..6 {
        press(cx, "tab");
        if focused(cx).as_ref() == Some(&handle) {
            return;
        }
    }
    panic!("Tab never reached the field");
}

#[gpui::test]
fn the_apps_page_lists_apps_accounts_and_mailboxes(cx: &mut TestAppContext) {
    let (view, cx) = page(cx, Some(listing()));
    assert_eq!(requests(&view, cx), ["GET /api/apps"]);
    // The fourteen apps, in catalog order, each with Connect.
    let mut last = None;
    for app in &CONNECTORS {
        let section = format!("desktop-apps-app-{}", app.id);
        let at = top(cx, &section);
        assert!(
            last.is_none_or(|last| at > last),
            "{} is out of order",
            app.id
        );
        last = Some(at);
        assert!(rendered(cx, &format!("desktop-apps-connect-{}", app.id)));
    }
    assert_eq!(CONNECTORS.len(), 14);
    // The Mail group sits above the apps, with its one mailbox.
    assert!(top(cx, "desktop-apps-mail") < top(cx, "desktop-apps-app-github"));
    assert!(rendered(cx, "desktop-apps-mailbox-m1"));
    // GitHub's account, with its buttons and stored attention.
    assert!(rendered(cx, "desktop-apps-account-v1-gh"));
    for button in ["test", "replace", "disconnect"] {
        assert!(
            rendered(cx, &format!("desktop-apps-{button}-v1-gh")),
            "{button}"
        );
    }
    assert!(rendered(cx, "desktop-apps-attention-v1-gh"));
    // A switch per bruh under the shared account and the mailbox; Cody
    // can't use apps at its level and says so.
    for target in ["v1-gh", "m1"] {
        for agent in ["ash", "cody"] {
            assert!(
                rendered(cx, &format!("desktop-apps-grant-{target}-{agent}")),
                "{target} {agent}"
            );
        }
        assert!(rendered(
            cx,
            &format!("desktop-apps-need-ask-{target}:cody")
        ));
        assert!(!rendered(
            cx,
            &format!("desktop-apps-need-ask-{target}:ash")
        ));
    }
    assert!(!rendered(cx, "desktop-apps-form"));
    assert!(!rendered(cx, "desktop-apps-grants-off"));
}

#[gpui::test]
fn connect_by_keyboard_sends_one_request_and_clears_the_token(cx: &mut TestAppContext) {
    let (view, cx) = page(cx, Some(listing()));
    let connect = view.read_with(cx, |view, _| view.connect_focus["jira"].clone());
    cx.update(|window, cx| connect.focus(window, cx));
    press(cx, "enter");
    assert!(rendered(cx, "desktop-apps-form"));
    let name = view.read_with(cx, |view, _| {
        view.form
            .as_ref()
            .and_then(|form| form.name.clone())
            .unwrap()
    });
    let name_focus = cx.update(|_, cx| name.focus_handle(cx));
    assert_eq!(focused(cx), Some(name_focus), "the form starts on Name");
    // Jira asks for Site, Email and Token, in that order.
    let ids = view.read_with(cx, |view, _| {
        let form = view.form.as_ref().unwrap();
        form.params
            .iter()
            .map(|(param, _)| param.id)
            .collect::<Vec<_>>()
    });
    assert_eq!(ids, ["site", "email", "token"]);
    let canary = format!("jira-canary-{}", uuid::Uuid::new_v4());
    let site = param(&view, cx, "site");
    tab_to(cx, &site);
    cx.simulate_input("acme");
    let email = param(&view, cx, "email");
    tab_to(cx, &email);
    cx.simulate_input("me@acme.test");
    let token = param(&view, cx, "token");
    tab_to(cx, &token);
    cx.simulate_input(&canary);
    assert_eq!(value(cx, &token), canary);
    press(cx, "enter");
    let sent = requests(&view, cx);
    assert_eq!(sent, ["GET /api/apps", "POST /api/apps/accounts"]);
    assert!(!sent.concat().contains(&canary), "{sent:?}");
    assert!(
        value(cx, &token).is_empty(),
        "the token is cleared on submit"
    );
    // The site and email stay for a failed test; the server answers.
    assert_eq!(value(cx, &site), "acme");
    let serial = view.read_with(cx, |view, _| view.form.as_ref().unwrap().serial);
    let account = AppAccount {
        id: "v1-jira".into(),
        app: "jira".into(),
        name: "Jira me".into(),
        account: "me on acme".into(),
        scope: CredentialScope::Global,
        state: SavedCredentialState::Saved,
        attention: None,
    };
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.answered(serial, Ok(account), window, cx);
        });
    });
    cx.run_until_parked();
    assert!(!rendered(cx, "desktop-apps-form"));
    assert!(rendered(cx, "desktop-apps-account-v1-jira"));
    assert_eq!(focused(cx), Some(connect), "focus returns to Connect");
    view.read_with(cx, |view, _| {
        assert_eq!(view.notice, Some(Notice::Done(connected_as("me on acme"))));
        assert_eq!(view.busy, VaultBusy::Idle);
    });
}

/// A connect form's refusal stays on the form and sends nothing; Cancel
/// and Settings closing empty every token field.
#[gpui::test]
fn tokens_are_emptied_on_cancel_and_close(cx: &mut TestAppContext) {
    let (view, cx) = page(cx, Some(listing()));
    click(cx, "desktop-apps-connect-github");
    let token = param(&view, cx, "token");
    let handle = cx.update(|_, cx| token.focus_handle(cx));
    cx.update(|window, cx| window.focus(&handle, cx));
    cx.simulate_input("short");
    click(cx, "desktop-apps-submit");
    assert!(rendered(cx, "desktop-apps-form-error"));
    assert_eq!(requests(&view, cx), ["GET /api/apps"]);
    assert!(value(cx, &token).is_empty(), "read and emptied on submit");
    cx.update(|window, cx| window.focus(&handle, cx));
    cx.simulate_input("cancel-canary-123");
    click(cx, "desktop-apps-cancel");
    assert!(value(cx, &token).is_empty());
    assert!(!rendered(cx, "desktop-apps-form"));
    let connect = view.read_with(cx, |view, _| view.connect_focus["github"].clone());
    assert_eq!(focused(cx), Some(connect), "Cancel returns to Connect");
    // Replace token: Settings closing drops the form and its token.
    click(cx, "desktop-apps-replace-v1-gh");
    assert!(view.read_with(cx, |view, _| matches!(
        view.form.as_ref().map(|form| &form.open),
        Some(Open::Replace(id)) if id == "v1-gh"
    )));
    let token = param(&view, cx, "token");
    let handle = cx.update(|_, cx| token.focus_handle(cx));
    assert_eq!(focused(cx), Some(handle), "Replace starts on the token");
    cx.simulate_input("close-canary-123");
    assert!(view.read_with(cx, |view, _| view.has_editor()));
    cx.update(|window, cx| view.update(cx, |view, cx| view.clear_editor(window, cx)));
    cx.run_until_parked();
    assert!(value(cx, &token).is_empty());
    assert!(!view.read_with(cx, |view, _| view.has_editor()));
    assert_eq!(requests(&view, cx), ["GET /api/apps"]);
}

#[gpui::test]
fn a_grant_switch_sends_one_set_app_grant(cx: &mut TestAppContext) {
    let (view, cx) = page(cx, Some(listing()));
    click(cx, "desktop-apps-grant-v1-gh-ash");
    assert_eq!(
        requests(&view, cx),
        ["GET /api/apps", "POST /api/reactive/commands"]
    );
    let token = AppGrant::Token {
        entry_id: "v1-gh".into(),
    };
    view.read_with(cx, |view, _| {
        assert_eq!(
            view.last_grant,
            Some(("ash".to_string(), token.clone(), true))
        );
    });
    // A second flip waits for the first.
    click(cx, "desktop-apps-grant-m1-ash");
    assert_eq!(requests(&view, cx).len(), 2);
    // The server answers and the listing shows the grant.
    let mut next = listing();
    next.bruhs[0].apps = vec!["v1-gh".into()];
    let key = "v1-gh:ash".to_string();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.flipped(&key, "ash", &token, true, Ok(()), Some(next), window, cx);
        });
    });
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert_eq!(view.busy, VaultBusy::Idle);
        assert_eq!(view.listing.bruhs[0].apps, ["v1-gh"]);
    });
    // An older service: every switch is off, says why, and sends nothing.
    let mut older = listing();
    older.protocol = VAULT_PROTOCOL;
    view.update(cx, |view, cx| view.show_listing_for_test(older, cx));
    cx.run_until_parked();
    assert!(rendered(cx, "desktop-apps-grants-off"));
    assert_eq!(
        view.read_with(cx, |view, _| view.grants_off()),
        apps_reason(Some(VAULT_PROTOCOL))
    );
    assert!(apps_reason(Some(VAULT_PROTOCOL)).is_some());
    let before = requests(&view, cx).len();
    click(cx, "desktop-apps-grant-v1-gh-ash");
    click(cx, "desktop-apps-grant-m1-cody");
    assert_eq!(requests(&view, cx).len(), before);
}

#[gpui::test]
fn disconnect_asks_first(cx: &mut TestAppContext) {
    let (view, cx) = page(cx, Some(listing()));
    click(cx, "desktop-apps-disconnect-v1-gh");
    assert!(rendered(cx, "desktop-apps-confirm"));
    assert_eq!(
        view.read_with(cx, |view, _| view.confirm_line()),
        Some(disconnect_confirm("GitHub octo"))
    );
    let keep = view.read_with(cx, |view, _| view.rows_focus["v1-gh"].keep.clone());
    assert_eq!(focused(cx), Some(keep), "Keep, the safe choice, has focus");
    click(cx, "desktop-apps-keep");
    assert!(!rendered(cx, "desktop-apps-confirm"));
    let disconnect = view.read_with(cx, |view, _| view.rows_focus["v1-gh"].disconnect.clone());
    assert_eq!(focused(cx), Some(disconnect), "Keep returns to Disconnect");
    assert_eq!(requests(&view, cx), ["GET /api/apps"], "Keep sends nothing");
    click(cx, "desktop-apps-disconnect-v1-gh");
    click(cx, "desktop-apps-confirm");
    assert_eq!(
        requests(&view, cx).last().map(String::as_str),
        Some("DELETE /api/vault/v1-gh")
    );
    // Gone: its row and every bruh's grant of it.
    let mut granted = listing();
    granted.bruhs[0].apps = vec!["v1-gh".into()];
    view.update(cx, |view, _| view.listing = granted);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.disconnected("v1-gh".into(), Ok(()), window, cx);
        });
    });
    cx.run_until_parked();
    assert!(!rendered(cx, "desktop-apps-account-v1-gh"));
    view.read_with(cx, |view, _| {
        assert!(view.listing.accounts.is_empty());
        assert!(view.listing.bruhs[0].apps.is_empty());
    });
}

/// Final review BC M4: a connect, a replace and a disconnect each tell
/// the Vault section, and a Vault change has a loaded page read again
/// quietly, keeping what it has open.
#[gpui::test]
fn account_changes_tell_the_vault_section(cx: &mut TestAppContext) {
    let (view, cx) = page(cx, Some(listing()));
    let told = Rc::new(std::cell::Cell::new(0));
    let count = told.clone();
    cx.update(|_, cx| {
        cx.subscribe(&view, move |_, _: &CredentialsChanged, _| {
            count.set(count.get() + 1);
        })
        .detach();
    });
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.answered(0, Ok(github()), window, cx);
        });
    });
    cx.run_until_parked();
    assert_eq!(told.get(), 1, "a connect or a replace");
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.disconnected("v1-gh".into(), Ok(()), window, cx);
        });
    });
    cx.run_until_parked();
    assert_eq!(told.get(), 2, "a disconnect");

    click(cx, "desktop-apps-connect-github");
    assert!(rendered(cx, "desktop-apps-form"));
    let before = requests(&view, cx).len();
    cx.update(|window, cx| view.update(cx, |view, cx| view.reread(window, cx)));
    cx.run_until_parked();
    assert_eq!(requests(&view, cx)[before..], ["GET /api/apps"]);
    assert!(rendered(cx, "desktop-apps-form"), "the open form stays");
    assert!(
        view.read_with(cx, |view, _| view.load == Load::Ready),
        "a quiet read keeps the page loaded"
    );
}

/// A Disconnect the service refused keeps its question open over the
/// row's error, so trying again is one press, as on the web, whose
/// `asking` stays set on a failure (Task 16 minor).
#[gpui::test]
fn a_refused_disconnect_keeps_the_question(cx: &mut TestAppContext) {
    let (view, cx) = page(cx, Some(listing()));
    click(cx, "desktop-apps-disconnect-v1-gh");
    click(cx, "desktop-apps-confirm");
    let refused = ServerError::Status {
        request: "DELETE /api/vault/v1-gh".into(),
        code: reqwest::StatusCode::CONFLICT,
        body: r#"{"error":"Refused."}"#.into(),
    };
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.disconnected("v1-gh".into(), Err(refused), window, cx);
        });
    });
    cx.run_until_parked();
    assert!(rendered(cx, "desktop-apps-error-v1-gh"));
    assert!(rendered(cx, "desktop-apps-confirm"), "the question stays");
    assert!(rendered(cx, "desktop-apps-account-v1-gh"));
    let before = requests(&view, cx).len();
    click(cx, "desktop-apps-confirm");
    assert_eq!(
        requests(&view, cx).len(),
        before + 1,
        "one press tries again"
    );
}

#[gpui::test]
fn a_refused_token_shows_why_on_its_row(cx: &mut TestAppContext) {
    let (view, cx) = page(cx, Some(listing()));
    click(cx, "desktop-apps-test-v1-gh");
    assert_eq!(
        requests(&view, cx).last().map(String::as_str),
        Some("POST /api/apps/accounts/v1-gh/test")
    );
    // The test route answers 200 with `ok: false`.
    let refused = AppTestResult {
        ok: false,
        account: None,
        message: "GitHub refused the token. Check it and what it may do.".into(),
    };
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.test_answered("v1-gh".into(), Ok(refused), window, cx);
        });
    });
    cx.run_until_parked();
    assert!(rendered(cx, "desktop-apps-tested-v1-gh"));
    // The test's line stands in for the stored attention, which stays.
    assert!(!rendered(cx, "desktop-apps-attention-v1-gh"));
    view.read_with(cx, |view, _| {
        assert!(view.listing.accounts[0].attention.is_some());
        assert!(!view.tested["v1-gh"].0);
    });
}

#[gpui::test]
fn an_older_server_shows_the_service_notice(cx: &mut TestAppContext) {
    let (view, cx) = page(cx, None);
    let not_found = ServerError::Status {
        request: "GET /api/apps".into(),
        code: reqwest::StatusCode::NOT_FOUND,
        body: String::new(),
    };
    view.update(cx, |view, cx| {
        view.answer_listing_for_test(Err(not_found), cx)
    });
    cx.run_until_parked();
    assert!(rendered(cx, "desktop-apps-notice"));
    assert_eq!(
        view.read_with(cx, |view, _| view.status_line()),
        Some(ServiceNotice::ServingOld.message().to_string())
    );
    // Connect waits for a listing it can add to.
    click(cx, "desktop-apps-connect-github");
    assert!(!rendered(cx, "desktop-apps-form"));
}

#[gpui::test]
fn a_client_on_another_address_is_told_the_exact_one(cx: &mut TestAppContext) {
    let (view, cx) = page(cx, None);
    let forbidden = ServerError::Status {
        request: "GET /api/apps".into(),
        code: reqwest::StatusCode::FORBIDDEN,
        body: String::new(),
    };
    view.update(cx, |view, cx| {
        view.answer_listing_for_test(Err(forbidden), cx)
    });
    cx.run_until_parked();
    assert!(rendered(cx, "desktop-apps-notice"));
    assert_eq!(
        view.read_with(cx, |view, _| view.status_line()),
        Some("Open Bruhs at http://127.0.0.1:9 to see and change apps.".to_string())
    );
}

/// `apps-connect:<app>` (same-size captures) opens that app's Connect form
/// as its Connect button does, and nothing for an app that isn't listed.
#[gpui::test]
fn the_connect_scene_opens_that_apps_form(cx: &mut TestAppContext) {
    let (view, cx) = page(cx, Some(listing()));
    let unknown = view.update_in(cx, |view, window, cx| {
        view.smoke_open_connect("nope", window, cx)
    });
    assert!(!unknown);
    assert!(!rendered(cx, "desktop-apps-form"));
    let opened = view.update_in(cx, |view, window, cx| {
        view.smoke_open_connect("jira", window, cx)
    });
    cx.run_until_parked();
    assert!(opened);
    assert!(rendered(cx, "desktop-apps-form"));
    view.read_with(cx, |view, _| {
        assert!(matches!(
            view.form.as_ref().map(|form| &form.open),
            Some(Open::Connect("jira"))
        ));
    });
}
