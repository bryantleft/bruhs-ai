//! A bruh's apps in the runtime: which runs get the app and mail tools,
//! how a call picks its account or mailbox from the bruh's live grants,
//! and what `list_apps` tells it. The keychain is the in-memory store, the
//! mail a synthetic Gmail connection, and no call leaves the test.

use super::approval::decide;
use super::apps::pick_mailbox;
use super::computer_tests::full_claude;
use super::crew_tests::{
    approvals, claim_one, decide_approval, decided, deny, finish, queued_run, runtime_with,
    waiting_approval, Decision, LONG_WAIT, NOT_ANSWERED, SHORT_WAIT,
};
use super::{worker, ReactiveRuntime, StepAllowance};
use crate::apps::{AppHttp, AppsHost};
use crate::mail::compose::test_support::ConnectedMail;
use crate::mail::gmail::GmailClient;
use crate::mail::outlook::OutlookClient;
use crate::mail::{SyncBatch, ThreadWrite};
use crate::secrets::{SecretManager, SecretStore};
use crate::workbench_tools::BruhTools;
use crate::PermissionDecision;
use bruhs_models::agent::AgentProvider;
use bruhs_models::apps::{AppMailbox, MAX_ACCOUNT_CHARS};
use bruhs_models::connections::ServiceProvider;
use bruhs_models::crew::*;
use bruhs_models::mail::{
    ConnectedMailMetadata, ConnectedMailThread, MailAddress, MailEnvelope, MailMessageDetail,
    MailReplyTarget,
};
use bruhs_models::reactive::{ReactiveAgent, ReactiveCommand, ReactiveRun};
use bruhs_models::vault::{CredentialScope, VaultCreate, VaultSecretInput, OWNER_ONLY_CARD};
use bruhs_models::{InboxConversation, InboxMessage, InboxSource};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use wiremock::MockServer;

/// A random value no listing may hold.
fn canary() -> String {
    format!("tok_{}", uuid::Uuid::new_v4().simple())
}

fn bruh(agent_id: &str) -> CredentialScope {
    CredentialScope::Bruh {
        agent_id: agent_id.into(),
    }
}

/// Claude bruhs `a` (Ask first) and `b`, a vault and an apps host on a
/// wiremock upstream whose mail is a synthetic Gmail mailbox.
struct Fixture {
    _dir: tempfile::TempDir,
    _server: MockServer,
    runtime: ReactiveRuntime,
    host: AppsHost,
    mail: ConnectedMail,
    tokens: Vec<String>,
}

impl Fixture {
    async fn new() -> Self {
        let (dir, runtime) = runtime_with(&[full_claude("a"), full_claude("b")]).await;
        runtime
            .command(ReactiveCommand::SetBruh {
                agent_id: "a".into(),
                look: BruhLook::default(),
                access: BruhAccess {
                    level: AccessLevel::Ask,
                    ..BruhAccess::default()
                },
                effort: None,
            })
            .await
            .unwrap();
        let mail = ConnectedMail::gmail().await;
        let vault =
            SecretManager::new(SecretStore::in_memory()).with_connections(mail.connections.clone());
        let server = MockServer::start().await;
        let origin = format!("http://{}", server.address());
        let host = AppsHost::new(
            vault.clone(),
            AppHttp::with_upstream(*server.address()).unwrap(),
            Some(mail.runtime.clone()),
            Arc::new(GmailClient::for_test_origin(&origin).unwrap()),
            Arc::new(OutlookClient::for_test_origin(&origin).unwrap()),
        );
        runtime.attach_vault(vault).unwrap();
        runtime.attach_apps(host.clone()).unwrap();
        Self {
            _dir: dir,
            _server: server,
            runtime,
            host,
            mail,
            tokens: Vec::new(),
        }
    }

    /// Saves an app token and returns its entry id.
    async fn token(
        &mut self,
        name: &str,
        scope: CredentialScope,
        app: &str,
        account: &str,
        params: &[(&str, &str)],
    ) -> String {
        let token = canary();
        self.tokens.push(token.clone());
        self.host
            .vault()
            .vault_create(VaultCreate {
                name: name.into(),
                scope,
                secret: VaultSecretInput::AppToken {
                    app: app.into(),
                    account: account.into(),
                    params: params
                        .iter()
                        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                        .collect::<BTreeMap<_, _>>(),
                    token: Some(token),
                    key: None,
                },
            })
            .await
            .unwrap()
            .id
    }

    async fn grant(&self, agent_id: &str, grant: AppGrant) {
        self.runtime
            .command(ReactiveCommand::SetAppGrant {
                agent_id: agent_id.into(),
                grant,
                granted: true,
            })
            .await
            .unwrap();
    }

    async fn grant_token(&self, agent_id: &str, entry_id: &str) {
        self.grant(
            agent_id,
            AppGrant::Token {
                entry_id: entry_id.into(),
            },
        )
        .await;
    }

    async fn grant_mailbox(&self, agent_id: &str) {
        self.grant(
            agent_id,
            AppGrant::Mailbox {
                mailbox_id: self.mail.mailbox.as_str().into(),
            },
        )
        .await;
    }

    /// The synthetic mailbox as the apps host lists it.
    async fn mailbox(&self) -> AppMailbox {
        let mailboxes = self.host.mailboxes().await.unwrap();
        assert_eq!(mailboxes.len(), 1, "{mailboxes:?}");
        mailboxes.into_iter().next().unwrap()
    }
}

fn run() -> ReactiveRun {
    ReactiveRun {
        session_id: None,
        ..queued_run("Look it up.")
    }
}

fn profile(level: AccessLevel) -> BruhProfile {
    BruhProfile {
        access: BruhAccess {
            level,
            ..BruhAccess::default()
        },
        ..BruhProfile::default()
    }
}

async fn turn_for(
    fixture: &Fixture,
    agent: &ReactiveAgent,
    level: AccessLevel,
) -> crate::AgentTurn {
    let provider = bruhs_models::agent::AgentProvider::for_model(&agent.model);
    let vault = fixture
        .runtime
        .turn_vault(
            &agent.id,
            false,
            false,
            apps_unavailable(level, provider).is_none(),
        )
        .await;
    worker::turn(
        agent,
        &profile(level),
        &run(),
        "",
        Path::new("/home"),
        &vault,
    )
}

#[tokio::test]
async fn a_run_gets_app_tools_only_with_a_usable_account() {
    let mut fixture = Fixture::new().await;
    let github = fixture
        .token(
            "GitHub octo",
            CredentialScope::Global,
            "github",
            "octo",
            &[],
        )
        .await;
    let agent = full_claude("a");

    // No grants: no app or mail tools, and the bruh never sends as the owner.
    let plain = turn_for(&fixture, &agent, AccessLevel::Ask).await;
    assert_eq!(plain.bruh_tools, None);
    assert!(plain.prompt.contains(NEVER_SEND_PREAMBLE));
    assert!(!plain.prompt.contains("Apps you may use"));

    fixture.grant_token("a", &github).await;
    let apps = turn_for(&fixture, &agent, AccessLevel::Ask).await;
    assert_eq!(
        apps.bruh_tools,
        Some(BruhTools {
            agent_id: "a".into(),
            browser: false,
            apps: true,
            mail: false,
        })
    );
    assert!(
        apps.prompt.contains("Apps you may use: GitHub."),
        "{}",
        apps.prompt
    );
    assert!(apps.prompt.contains(SEND_ASK_PREAMBLE));
    assert!(!apps.prompt.contains(NEVER_SEND_PREAMBLE));

    fixture.grant_mailbox("a").await;
    let label = fixture.mailbox().await.account;
    let both = turn_for(&fixture, &agent, AccessLevel::Ask).await;
    assert_eq!(
        both.bruh_tools,
        Some(BruhTools {
            agent_id: "a".into(),
            browser: false,
            apps: true,
            mail: true,
        })
    );
    assert!(
        both.prompt
            .contains(&format!("send_mail sends email as the owner from {label}.")),
        "{}",
        both.prompt
    );

    // Read only, or Codex at any level: no app or mail tools, whatever is
    // granted, and the preamble still forbids sending.
    let codex = ReactiveAgent {
        model: "gpt-5.3-codex".into(),
        ..full_claude("a")
    };
    for (agent, level) in [
        (full_claude("a"), AccessLevel::ReadOnly),
        (codex.clone(), AccessLevel::Full),
        (codex, AccessLevel::Ask),
    ] {
        let turn = turn_for(&fixture, &agent, level).await;
        assert_eq!(turn.bruh_tools, None, "{} {level:?}", agent.model);
        assert!(turn.prompt.contains(NEVER_SEND_PREAMBLE), "{}", turn.prompt);
        assert!(!turn.prompt.contains("Apps you may use"));
        assert!(!turn.prompt.contains("send_mail"));
    }

    // A turn built from a vault read without apps lists none either.
    let worker_vault = worker::TurnVault {
        apps: vec!["GitHub".into()],
        mail: vec![label],
        ..worker::TurnVault::default()
    };
    let read_only = worker::turn(
        &full_claude("a"),
        &profile(AccessLevel::ReadOnly),
        &run(),
        "",
        Path::new("/home"),
        &worker_vault,
    );
    assert_eq!(read_only.bruh_tools, None);
    assert!(read_only
        .prompt
        .ends_with(&format!("{NEVER_SEND_PREAMBLE}\n\nLook it up.")));
}

#[tokio::test]
async fn usable_apps_are_the_granted_names_and_live_mailboxes() {
    let mut fixture = Fixture::new().await;
    let shared = fixture
        .token(
            "GitHub octo",
            CredentialScope::Global,
            "github",
            "octo",
            &[],
        )
        .await;
    fixture
        .token("GitHub work", bruh("a"), "github", "work", &[])
        .await;
    fixture
        .token("Notion", CredentialScope::Global, "notion", "me", &[])
        .await;
    fixture
        .token("Linear", bruh("b"), "linear", "me in x", &[])
        .await;
    fixture.grant_token("a", &shared).await;
    // A stale mailbox grant grants nothing.
    fixture
        .grant(
            "a",
            AppGrant::Mailbox {
                mailbox_id: "gone".into(),
            },
        )
        .await;
    assert_eq!(
        fixture.runtime.usable_apps("a").await,
        (vec!["GitHub".to_owned()], Vec::<String>::new())
    );
    fixture.grant_mailbox("a").await;
    let label = fixture.mailbox().await.account;
    assert_eq!(
        fixture.runtime.usable_apps("a").await,
        (vec!["GitHub".to_owned()], vec![label])
    );
    assert_eq!(
        fixture.runtime.usable_apps("b").await,
        (vec!["Linear".to_owned()], Vec::<String>::new())
    );
    assert_eq!(
        fixture.runtime.usable_apps("nobody").await,
        (Vec::new(), Vec::new())
    );
}

/// Caches a thread whose one message the mailbox received, and returns a
/// reply target naming that message.
async fn seed_thread(mail: &ConnectedMail) -> MailReplyTarget {
    let session = mail.session().await;
    let mailbox = session.authorization().mailbox().clone();
    let remote = mailbox.thread("source-thread").unwrap();
    let id = mailbox.message("source-message").unwrap().canonical_id;
    let mut message = InboxMessage::incoming("Source", "Source body", 1);
    message.id = id.clone();
    message.external_id = Some("source-message".into());
    let mut conversation = InboxConversation::new(InboxSource::Email, "Plans", vec![], 1);
    conversation.id = remote.canonical_id.clone();
    conversation.external_id = Some(remote.external_id.clone());
    conversation.receive(message);
    let record = ConnectedMailThread {
        conversation,
        connected_mail: ConnectedMailMetadata {
            provider: ServiceProvider::Gmail,
            mailbox_id: mailbox.as_str().into(),
            external_thread_id: remote.external_id.clone(),
            messages: vec![MailMessageDetail {
                id: id.clone(),
                external_id: "source-message".into(),
                label_ids: vec![],
                attachments: vec![],
                body_from_html: false,
                envelope: MailEnvelope {
                    from: vec![MailAddress {
                        name: String::new(),
                        address: "bo@x.test".into(),
                    }],
                    subject: "Plans for Friday".into(),
                    message_id: Some("source@x.test".into()),
                    ..Default::default()
                },
            }],
        },
    };
    session
        .apply_sync(
            session.revision().await.unwrap(),
            SyncBatch {
                upserts: vec![ThreadWrite {
                    reference: remote,
                    payload: serde_json::to_string(&record).unwrap(),
                }],
                confirmed_deleted: vec![],
                cursor: None,
            },
        )
        .await
        .unwrap();
    drop(session);
    MailReplyTarget {
        external_thread_id: "source-thread".into(),
        message_id: id,
    }
}

#[tokio::test]
async fn lookups_pick_one_account_and_refuse_the_rest() {
    let mut fixture = Fixture::new().await;
    let octo = fixture
        .token(
            "GitHub octo",
            CredentialScope::Global,
            "github",
            "octo",
            &[],
        )
        .await;
    let work = fixture
        .token("GitHub work", bruh("a"), "github", "work", &[])
        .await;
    let notion = fixture
        .token("Notion", CredentialScope::Global, "notion", "me", &[])
        .await;
    fixture
        .token("Linear", bruh("b"), "linear", "me in x", &[])
        .await;
    fixture.grant_token("a", &octo).await;
    let runtime = &fixture.runtime;
    let lookup = |input: Value| async move { runtime.app_lookup("a", &input).await };
    let found = |entry_id: &str, account: &str| {
        AppLookup::Found(AppTarget {
            entry_id: entry_id.into(),
            app: "github".into(),
            account: account.into(),
        })
    };

    // Two usable GitHub accounts: the call names one, by label or name.
    assert_eq!(
        lookup(json!({"app": "github"})).await,
        AppLookup::Missing(APP_ACCOUNT_NEEDED)
    );
    assert_eq!(
        lookup(json!({"app": "github", "account": "work"})).await,
        found(&work, "work")
    );
    assert_eq!(
        lookup(json!({"app": "github", "account": "GitHub octo"})).await,
        found(&octo, "octo")
    );
    assert_eq!(
        lookup(json!({"app": "github", "account": "nope"})).await,
        AppLookup::Missing(APP_ACCOUNT_UNKNOWN)
    );
    // An ungranted Global account, another bruh's own, or no such app.
    for app in ["notion", "linear", "slack", "nope"] {
        assert_eq!(
            lookup(json!({"app": app})).await,
            AppLookup::Missing(APP_NOT_GRANTED),
            "{app}"
        );
    }
    assert_eq!(
        lookup(json!({"method": "GET"})).await,
        AppLookup::Missing(APP_NOT_GRANTED)
    );
    // Bruh b holds none of them.
    assert_eq!(
        fixture
            .runtime
            .app_lookup("b", &json!({"app": "github"}))
            .await,
        AppLookup::Missing(APP_NOT_GRANTED)
    );

    // Granted, the Notion account is the only one, so it needs no name.
    fixture.grant_token("a", &notion).await;
    assert_eq!(
        fixture
            .runtime
            .app_lookup("a", &json!({"app": "notion"}))
            .await,
        AppLookup::Found(AppTarget {
            entry_id: notion.clone(),
            app: "notion".into(),
            account: "me".into(),
        })
    );

    // Case doesn't matter, as for mailboxes.
    assert_eq!(
        lookup(json!({"app": "github", "account": "WORK"})).await,
        found(&work, "work")
    );

    // The same account connected twice, shared (granted) and as the
    // bruh's own, with the default name each time: name and label pick
    // neither, and each is still pickable by the name it is shown under.
    let twin = fixture
        .token("GitHub octo", bruh("a"), "github", "octo", &[])
        .await;
    let runtime = &fixture.runtime;
    let lookup = |input: Value| async move { runtime.app_lookup("a", &input).await };
    for account in ["octo", "GitHub octo", "github OCTO"] {
        assert_eq!(
            lookup(json!({"app": "github", "account": account})).await,
            AppLookup::Missing(APP_ACCOUNT_NEEDED),
            "{account}"
        );
    }
    assert_eq!(
        lookup(json!({"app": "github", "account": "octo (own)"})).await,
        found(&twin, "octo (own)")
    );
    assert_eq!(
        lookup(json!({"app": "github", "account": "Octo (Shared)"})).await,
        found(&octo, "octo (shared)")
    );

    // One entry's name is another's label: that value picks neither, and
    // the name is never preferred (ruling A Task 11 I2).
    let team = fixture.token("me", bruh("a"), "notion", "team", &[]).await;
    let runtime = &fixture.runtime;
    let notion_lookup = |account: &str| {
        let input = json!({"app": "notion", "account": account});
        async move { runtime.app_lookup("a", &input).await }
    };
    assert_eq!(
        notion_lookup("me").await,
        AppLookup::Missing(APP_ACCOUNT_NEEDED)
    );
    let notion_found = |entry_id: &str, account: &str| {
        AppLookup::Found(AppTarget {
            entry_id: entry_id.into(),
            app: "notion".into(),
            account: account.into(),
        })
    };
    // The shared account whose label is taken shows its scope.
    assert_eq!(
        notion_lookup("me (shared)").await,
        notion_found(&notion, "me (shared)")
    );
    assert_eq!(notion_lookup("team").await, notion_found(&team, "team"));

    // Revoked, the shared account is gone at once, and the bruh's own
    // takes the plain label back.
    fixture
        .runtime
        .command(ReactiveCommand::SetAppGrant {
            agent_id: "a".into(),
            grant: AppGrant::Token {
                entry_id: octo.clone(),
            },
            granted: false,
        })
        .await
        .unwrap();
    assert_eq!(
        fixture
            .runtime
            .app_lookup("a", &json!({"app": "github", "account": "octo (shared)"}))
            .await,
        AppLookup::Missing(APP_ACCOUNT_UNKNOWN)
    );
    assert_eq!(
        fixture
            .runtime
            .app_lookup("a", &json!({"app": "github", "account": "GitHub octo"}))
            .await,
        found(&twin, "octo")
    );

    // Mail: only a granted, connected mailbox, picked by `from`.
    let mail = |from: Option<&str>| {
        let mut input = json!({"to": "ann@x.test", "subject": "Hi", "body": "Hello."});
        if let Some(from) = from {
            input["from"] = json!(from);
        }
        input
    };
    assert_eq!(
        fixture.runtime.mail_lookup("a", &mail(None)).await,
        MailLookup::Missing(MAIL_NOT_GRANTED)
    );
    fixture.grant_mailbox("a").await;
    let mailbox = fixture.mailbox().await;
    let target = MailTarget {
        mailbox_id: mailbox.mailbox_id.clone(),
        account: mailbox.account.clone(),
        reply_subject: None,
    };
    assert_eq!(
        fixture.runtime.mail_lookup("a", &mail(None)).await,
        MailLookup::Found(target.clone())
    );
    assert_eq!(
        fixture
            .runtime
            .mail_lookup("a", &mail(Some(&mailbox.account.to_uppercase())))
            .await,
        MailLookup::Found(target.clone())
    );
    assert_eq!(
        fixture
            .runtime
            .mail_lookup("a", &mail(Some("other@x.test")))
            .await,
        MailLookup::Missing(MAIL_NOT_GRANTED)
    );
    assert_eq!(
        fixture.runtime.mail_lookup("b", &mail(None)).await,
        MailLookup::Missing(MAIL_NOT_GRANTED)
    );
    // A reply's target carries the subject the reply goes out with.
    let reply = seed_thread(&fixture.mail).await;
    let input = json!({"to": "bo@x.test", "body": "Friday works.", "reply": reply});
    assert_eq!(
        fixture.runtime.mail_lookup("a", &input).await,
        MailLookup::Found(MailTarget {
            reply_subject: Some("Plans for Friday".into()),
            ..target
        })
    );

    // Two granted mailboxes: `from` must pick one.
    let boxes = [
        AppMailbox {
            mailbox_id: "m1".into(),
            provider: ServiceProvider::Gmail,
            account: "me@x.test".into(),
        },
        AppMailbox {
            mailbox_id: "m2".into(),
            provider: ServiceProvider::Outlook,
            account: "work@y.test".into(),
        },
    ];
    assert_eq!(
        pick_mailbox(&boxes, None).map(|mailbox| &mailbox.mailbox_id),
        Err(MAIL_FROM_NEEDED)
    );
    assert_eq!(
        pick_mailbox(&boxes, Some("WORK@y.test")).map(|mailbox| &mailbox.mailbox_id),
        Ok(&"m2".to_owned())
    );
    assert_eq!(
        pick_mailbox(&boxes, Some("x@z.test")).map(|mailbox| &mailbox.mailbox_id),
        Err(MAIL_NOT_GRANTED)
    );
    assert_eq!(
        pick_mailbox(&boxes[..1], None).map(|mailbox| &mailbox.mailbox_id),
        Ok(&"m1".to_owned())
    );
    assert_eq!(pick_mailbox(&[], None).err(), Some(MAIL_NOT_GRANTED));
}

/// Names shown to the bruh stay unique and fit an account, even for a
/// long label held twice at one scope, where only a number tells them
/// apart.
#[tokio::test]
async fn shown_account_names_are_unique_and_fit() {
    let mut fixture = Fixture::new().await;
    let long = "l".repeat(MAX_ACCOUNT_CHARS);
    let shared = fixture
        .token("Long shared", CredentialScope::Global, "github", &long, &[])
        .await;
    fixture.grant_token("a", &shared).await;
    let first = fixture
        .token("Long one", bruh("a"), "github", &long, &[])
        .await;
    let second = fixture
        .token("Long two", bruh("a"), "github", &long, &[])
        .await;
    let listed = fixture
        .runtime
        .list_apps("a", Some("github"))
        .await
        .unwrap();
    let shown: Vec<String> = listed["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|account| account["account"].as_str().unwrap().to_owned())
        .collect();
    let cut = |suffix: &str| {
        format!(
            "{}{suffix}",
            "l".repeat(MAX_ACCOUNT_CHARS - suffix.chars().count())
        )
    };
    // Catalog order is by name: Long one, Long shared, Long two.
    assert_eq!(shown, [cut(" (1)"), cut(" (shared)"), cut(" (2)")]);
    for (account, entry_id) in [
        (&shown[0], &first),
        (&shown[1], &shared),
        (&shown[2], &second),
    ] {
        assert!(account.chars().count() <= MAX_ACCOUNT_CHARS);
        assert_eq!(
            fixture
                .runtime
                .app_lookup("a", &json!({"app": "github", "account": account}))
                .await,
            AppLookup::Found(AppTarget {
                entry_id: entry_id.clone(),
                app: "github".into(),
                account: account.clone(),
            })
        );
    }
}

#[tokio::test]
async fn lookups_fail_closed_without_apps() {
    let (_dir, runtime) = runtime_with(&[full_claude("a")]).await;
    assert_eq!(
        runtime.app_lookup("a", &json!({"app": "github"})).await,
        AppLookup::Missing(APP_NOT_GRANTED)
    );
    assert_eq!(
        runtime
            .mail_lookup("a", &json!({"to": "ann@x.test", "body": "Hi"}))
            .await,
        MailLookup::Missing(MAIL_NOT_GRANTED)
    );
    assert_eq!(runtime.usable_apps("a").await, (Vec::new(), Vec::new()));
    assert!(runtime.list_apps("a", None).await.is_err());
}

#[tokio::test]
async fn list_apps_names_accounts_and_guides_without_tokens() {
    let mut fixture = Fixture::new().await;
    let octo = fixture
        .token(
            "GitHub octo",
            CredentialScope::Global,
            "github",
            "octo",
            &[],
        )
        .await;
    fixture
        .token(
            "Jira acme",
            bruh("a"),
            "jira",
            "me@x.test on acme",
            &[("site", "acme"), ("email", "me@x.test")],
        )
        .await;
    fixture
        .token("Notion", CredentialScope::Global, "notion", "me", &[])
        .await;
    fixture.grant_token("a", &octo).await;
    fixture.grant_mailbox("a").await;
    let label = fixture.mailbox().await.account;

    let all = fixture.runtime.list_apps("a", None).await.unwrap();
    assert_eq!(
        all,
        json!({
            "apps": [
                {"app": "github", "name": "GitHub", "accounts": [
                    {"name": "GitHub octo", "account": "octo", "base_url": "https://api.github.com"}
                ]},
                {"app": "jira", "name": "Jira", "accounts": [
                    {"name": "Jira acme", "account": "me@x.test on acme", "base_url": "https://acme.atlassian.net"}
                ]}
            ],
            "mail": [label]
        })
    );

    let github = fixture
        .runtime
        .list_apps("a", Some("github"))
        .await
        .unwrap();
    let connector = bruhs_models::apps::connector("github").unwrap();
    assert_eq!(github["app"], "github");
    assert_eq!(github["name"], "GitHub");
    assert_eq!(github["guide"], connector.guide);
    assert_eq!(github["accounts"], all["apps"][0]["accounts"]);
    // An app it may not use, or no such app, says so.
    for app in ["notion", "nope"] {
        let refused = fixture.runtime.list_apps("a", Some(app)).await.unwrap_err();
        assert_eq!(refused.to_string(), APP_NOT_GRANTED, "{app}");
    }

    let text = format!(
        "{all}{github}{:?}",
        fixture.runtime.list_apps("b", None).await
    );
    for token in &fixture.tokens {
        assert!(!text.contains(token.as_str()));
    }
    assert!(!text.to_lowercase().contains("authorization"));
}

// ── The gate on app and mail calls ─────────────────────────────────────────

const DENIED: &str = "The owner denied this. Continue without it.";

fn access(level: AccessLevel, send_as_me: bool, can_deliver: bool) -> BruhAccess {
    BruhAccess {
        level,
        send_as_me,
        can_deliver,
    }
}

/// Saves `access` for bruh `a`, then claims a chat for it: a Running run.
/// Saving access stops `a`'s earlier runs.
async fn claimed_at(fixture: &Fixture, access: &BruhAccess) -> ReactiveRun {
    let runtime = &fixture.runtime;
    runtime
        .command(ReactiveCommand::SetBruh {
            agent_id: "a".into(),
            look: BruhLook::default(),
            access: access.clone(),
            effort: None,
        })
        .await
        .unwrap();
    runtime
        .command(ReactiveCommand::Chat {
            agent_id: "a".into(),
            message: "Do the work.".into(),
            run_id: None,
        })
        .await
        .unwrap();
    let run = claim_one(runtime).await.unwrap().run;
    assert_eq!(run.agent_id, "a");
    run
}

/// `approval::decide` for a Claude bruh at `access`, in the background.
fn spawn_gate(
    fixture: &Fixture,
    access: &BruhAccess,
    run: &ReactiveRun,
    tool: &str,
    input: Value,
    timeout: Duration,
) -> tokio::task::JoinHandle<Decision> {
    let (runtime, access, run, tool) = (
        fixture.runtime.clone(),
        access.clone(),
        run.clone(),
        tool.to_owned(),
    );
    tokio::spawn(async move {
        decide(
            &runtime,
            &access,
            AgentProvider::Claude,
            false,
            &run,
            runtime
                .allowances
                .current_claim(&run.id)
                .unwrap_or_default(),
            "",
            &tool,
            &input,
            timeout,
        )
        .await
    })
}

fn allow() -> Option<PermissionDecision> {
    Some(PermissionDecision::Allow)
}

async fn consume(fixture: &Fixture, tool: &str, input: &Value) -> Option<StepAllowance> {
    fixture
        .runtime
        .consume_app_allowance("a", tool, input)
        .await
        .unwrap()
}

#[tokio::test]
async fn reads_run_at_ask_and_leave_one_allowance() {
    let mut fixture = Fixture::new().await;
    let octo = fixture
        .token(
            "GitHub octo",
            CredentialScope::Global,
            "github",
            "octo",
            &[],
        )
        .await;
    fixture.grant_token("a", &octo).await;
    let run = claimed_at(&fixture, &access(AccessLevel::Ask, false, false)).await;
    let input =
        json!({"app": "github", "method": "GET", "path": "/repos/o/r", "query": {"per_page": 1}});
    assert_eq!(
        fixture
            .runtime
            .gate_app_call_for_tests(&run, APP_REQUEST_TOOL, &input)
            .await
            .unwrap(),
        allow()
    );
    assert!(approvals(&fixture.runtime, "a").await.is_empty());

    // Matched on the tool and the input exactly as sent.
    let reencoded =
        json!({"app": "github", "method": "GET", "path": "/repos/o/r", "query": {"per_page": 1.0}});
    assert_eq!(consume(&fixture, APP_REQUEST_TOOL, &reencoded).await, None);
    assert_eq!(consume(&fixture, SEND_MAIL_TOOL, &input).await, None);
    assert_eq!(consume(&fixture, BRUH_BROWSER_TOOL, &input).await, None);
    assert_eq!(
        fixture
            .runtime
            .consume_browser_allowance("a", &input)
            .await
            .unwrap(),
        None
    );
    let allowance = consume(&fixture, APP_REQUEST_TOOL, &input).await.unwrap();
    assert_eq!(allowance.tool, APP_REQUEST_TOOL);
    assert_eq!(allowance.command, input);
    assert_eq!(allowance.run_id, run.id);
    assert_eq!(allowance.target.as_deref(), Some(octo.as_str()));
    assert_eq!(allowance.page_url, None);
    assert_eq!(consume(&fixture, APP_REQUEST_TOOL, &input).await, None);

    // list_apps is a read too.
    let list = json!({});
    assert_eq!(
        fixture
            .runtime
            .gate_app_call_for_tests(&run, LIST_APPS_TOOL, &list)
            .await
            .unwrap(),
        allow()
    );
    let listed = consume(&fixture, LIST_APPS_TOOL, &list).await.unwrap();
    assert_eq!(listed.target, None);
    assert!(approvals(&fixture.runtime, "a").await.is_empty());
}

#[tokio::test]
async fn writes_and_sends_ask_with_whole_cards() {
    let mut fixture = Fixture::new().await;
    let notion = fixture
        .token("Notion", CredentialScope::Global, "notion", "ws", &[])
        .await;
    let slack = fixture
        .token("Slack", CredentialScope::Global, "slack", "me", &[])
        .await;
    fixture.grant_token("a", &notion).await;
    fixture.grant_token("a", &slack).await;
    let ask = access(AccessLevel::Ask, false, false);
    let run = claimed_at(&fixture, &ask).await;
    let patch = json!({"app": "notion", "method": "PATCH", "path": "/v1/pages/p", "body": {"archived": true}});

    // Approved: the call goes once.
    let task = spawn_gate(
        &fixture,
        &ask,
        &run,
        APP_REQUEST_TOOL,
        patch.clone(),
        LONG_WAIT,
    );
    let item = waiting_approval(&fixture.runtime, "a").await;
    let request = item.approval.clone().unwrap();
    assert_eq!(request.tool, APP_REQUEST_TOOL);
    assert_eq!(request.summary, "Notion: PATCH /v1/pages/p");
    let detail = request.detail.unwrap();
    assert!(detail.starts_with("Account: "), "{detail}");
    assert!(detail.contains("\"archived\":true"), "{detail}");
    assert_eq!(consume(&fixture, APP_REQUEST_TOOL, &patch).await, None);
    decide_approval(&fixture.runtime, &item.id, true).await;
    assert_eq!(decided(task).await, allow());
    // The card's input, with its "as", is not the call.
    let card = app_card_input(
        &patch,
        &AppTarget {
            entry_id: notion.clone(),
            app: "notion".into(),
            account: "ws".into(),
        },
    );
    assert_eq!(consume(&fixture, APP_REQUEST_TOOL, &card).await, None);
    let allowance = consume(&fixture, APP_REQUEST_TOOL, &patch).await.unwrap();
    assert_eq!(allowance.target.as_deref(), Some(notion.as_str()));
    assert_eq!(consume(&fixture, APP_REQUEST_TOOL, &patch).await, None);

    // Denied: p3's copy, and nothing to send.
    let task = spawn_gate(
        &fixture,
        &ask,
        &run,
        APP_REQUEST_TOOL,
        patch.clone(),
        LONG_WAIT,
    );
    let item = waiting_approval(&fixture.runtime, "a").await;
    decide_approval(&fixture.runtime, &item.id, false).await;
    assert_eq!(decided(task).await, deny(DENIED));
    assert_eq!(consume(&fixture, APP_REQUEST_TOOL, &patch).await, None);

    // Full without Send as me: a post as the owner still asks.
    let full = access(AccessLevel::Full, false, false);
    let run = claimed_at(&fixture, &full).await;
    let post = json!({"app": "slack", "method": "POST", "path": "/api/chat.postMessage", "body": {"channel": "C1", "text": "hi"}});
    let task = spawn_gate(
        &fixture,
        &full,
        &run,
        APP_REQUEST_TOOL,
        post.clone(),
        LONG_WAIT,
    );
    let item = waiting_approval(&fixture.runtime, "a").await;
    assert_eq!(
        item.approval.as_ref().unwrap().summary,
        "As you on Slack: POST /api/chat.postMessage"
    );
    decide_approval(&fixture.runtime, &item.id, false).await;
    assert_eq!(decided(task).await, deny(DENIED));
    // A Full write runs at once.
    assert_eq!(
        decided(spawn_gate(
            &fixture,
            &full,
            &run,
            APP_REQUEST_TOOL,
            patch.clone(),
            SHORT_WAIT
        ))
        .await,
        allow()
    );
    assert!(consume(&fixture, APP_REQUEST_TOOL, &patch).await.is_some());

    // Full with Send as me: the post runs at once.
    let sends = access(AccessLevel::Full, true, false);
    let run = claimed_at(&fixture, &sends).await;
    let before = approvals(&fixture.runtime, "a").await.len();
    assert_eq!(
        fixture
            .runtime
            .gate_app_call_for_tests(&run, APP_REQUEST_TOOL, &post)
            .await
            .unwrap(),
        allow()
    );
    assert_eq!(approvals(&fixture.runtime, "a").await.len(), before);
    assert!(consume(&fixture, APP_REQUEST_TOOL, &post).await.is_some());
}

#[tokio::test]
async fn send_mail_is_a_send() {
    let fixture = Fixture::new().await;
    fixture.grant_mailbox("a").await;
    let mailbox = fixture.mailbox().await;
    let mail =
        json!({"to": "ann@x.test", "subject": "Lunch", "body": "Noon works.\nSee you there."});

    let sends = access(AccessLevel::Full, true, false);
    let run = claimed_at(&fixture, &sends).await;
    assert_eq!(
        fixture
            .runtime
            .gate_app_call_for_tests(&run, SEND_MAIL_TOOL, &mail)
            .await
            .unwrap(),
        allow()
    );
    let allowance = consume(&fixture, SEND_MAIL_TOOL, &mail).await.unwrap();
    assert_eq!(allowance.command, mail);
    assert_eq!(
        allowance.target.as_deref(),
        Some(mailbox.mailbox_id.as_str())
    );
    assert!(approvals(&fixture.runtime, "a").await.is_empty());

    let full = access(AccessLevel::Full, false, false);
    let run = claimed_at(&fixture, &full).await;
    let task = spawn_gate(
        &fixture,
        &full,
        &run,
        SEND_MAIL_TOOL,
        mail.clone(),
        LONG_WAIT,
    );
    let item = waiting_approval(&fixture.runtime, "a").await;
    let request = item.approval.clone().unwrap();
    assert_eq!(request.tool, SEND_MAIL_TOOL);
    assert_eq!(request.summary, "Email ann@x.test: Lunch");
    let detail = request.detail.unwrap();
    assert!(detail.contains("Noon works.\nSee you there."), "{detail}");
    assert!(detail.contains(&mailbox.account), "{detail}");
    decide_approval(&fixture.runtime, &item.id, true).await;
    assert_eq!(decided(task).await, allow());
    assert!(consume(&fixture, SEND_MAIL_TOOL, &mail).await.is_some());
    assert_eq!(consume(&fixture, SEND_MAIL_TOOL, &mail).await, None);
}

#[tokio::test]
async fn forge_delivery_needs_can_deliver() {
    let mut fixture = Fixture::new().await;
    let octo = fixture
        .token(
            "GitHub octo",
            CredentialScope::Global,
            "github",
            "octo",
            &[],
        )
        .await;
    fixture.grant_token("a", &octo).await;
    let merge = json!({"app": "github", "method": "PUT", "path": "/repos/o/r/pulls/1/merge"});
    let run = claimed_at(&fixture, &access(AccessLevel::Full, true, false)).await;
    assert_eq!(
        fixture
            .runtime
            .gate_app_call_for_tests(&run, APP_REQUEST_TOOL, &merge)
            .await
            .unwrap(),
        deny(DELIVERY_DENIAL)
    );
    assert_eq!(consume(&fixture, APP_REQUEST_TOOL, &merge).await, None);
    let run = claimed_at(&fixture, &access(AccessLevel::Full, true, true)).await;
    assert_eq!(
        fixture
            .runtime
            .gate_app_call_for_tests(&run, APP_REQUEST_TOOL, &merge)
            .await
            .unwrap(),
        allow()
    );
    assert!(consume(&fixture, APP_REQUEST_TOOL, &merge).await.is_some());
    assert!(approvals(&fixture.runtime, "a").await.is_empty());
}

#[tokio::test]
async fn a_revoked_grant_stops_the_next_call() {
    let mut fixture = Fixture::new().await;
    let octo = fixture
        .token(
            "GitHub octo",
            CredentialScope::Global,
            "github",
            "octo",
            &[],
        )
        .await;
    fixture.grant_token("a", &octo).await;
    fixture.grant_mailbox("a").await;
    let run = claimed_at(&fixture, &access(AccessLevel::Full, true, false)).await;
    let read = json!({"app": "github", "method": "GET", "path": "/repos/o/r"});
    let mail = json!({"to": "ann@x.test", "subject": "Lunch", "body": "Noon?"});
    let gate = |tool: &'static str, input: Value| {
        let (runtime, run) = (fixture.runtime.clone(), run.clone());
        async move {
            runtime
                .gate_app_call_for_tests(&run, tool, &input)
                .await
                .unwrap()
        }
    };
    assert_eq!(gate(APP_REQUEST_TOOL, read.clone()).await, allow());
    assert_eq!(gate(SEND_MAIL_TOOL, mail.clone()).await, allow());
    for grant in [
        AppGrant::Token {
            entry_id: octo.clone(),
        },
        AppGrant::Mailbox {
            mailbox_id: fixture.mail.mailbox.as_str().into(),
        },
    ] {
        fixture
            .runtime
            .command(ReactiveCommand::SetAppGrant {
                agent_id: "a".into(),
                grant,
                granted: false,
            })
            .await
            .unwrap();
    }
    // The revoke cancelled nothing: the same run's next calls are refused.
    assert_eq!(gate(APP_REQUEST_TOOL, read).await, deny(APP_NOT_GRANTED));
    assert_eq!(gate(SEND_MAIL_TOOL, mail).await, deny(MAIL_NOT_GRANTED));
}

/// Ruling F25: a late approval leaves a grant for the card, and the resumed
/// run's same call uses it once, leaving its allowance on the raw input
/// and the account the gate picked now.
#[tokio::test]
async fn a_late_approval_runs_once_in_the_resumed_run() {
    let mut fixture = Fixture::new().await;
    let notion = fixture
        .token("Notion", CredentialScope::Global, "notion", "ws", &[])
        .await;
    fixture.grant_token("a", &notion).await;
    let ask = access(AccessLevel::Ask, false, false);
    let run = claimed_at(&fixture, &ask).await;
    let patch = json!({"app": "notion", "method": "PATCH", "path": "/v1/pages/p", "body": {"archived": true}});
    assert_eq!(
        decided(spawn_gate(
            &fixture,
            &ask,
            &run,
            APP_REQUEST_TOOL,
            patch.clone(),
            SHORT_WAIT
        ))
        .await,
        deny(NOT_ANSWERED)
    );
    let [item] = approvals(&fixture.runtime, "a").await.try_into().unwrap();
    finish(
        &fixture.runtime,
        &run.id,
        "Waiting on the owner's approval.",
    )
    .await;
    decide_approval(&fixture.runtime, &item.id, true).await;
    let resumed = claim_one(&fixture.runtime).await.unwrap().run;
    assert_ne!(resumed.id, run.id);

    assert_eq!(
        decided(spawn_gate(
            &fixture,
            &ask,
            &resumed,
            APP_REQUEST_TOOL,
            patch.clone(),
            SHORT_WAIT
        ))
        .await,
        allow()
    );
    assert_eq!(approvals(&fixture.runtime, "a").await.len(), 1);
    let allowance = consume(&fixture, APP_REQUEST_TOOL, &patch).await.unwrap();
    assert_eq!(allowance.run_id, resumed.id);
    assert_eq!(allowance.command, patch);
    assert_eq!(allowance.target.as_deref(), Some(notion.as_str()));
    assert_eq!(consume(&fixture, APP_REQUEST_TOOL, &patch).await, None);
    // The grant is used up: the same call asks again.
    assert_eq!(
        decided(spawn_gate(
            &fixture,
            &ask,
            &resumed,
            APP_REQUEST_TOOL,
            patch.clone(),
            SHORT_WAIT
        ))
        .await,
        deny(NOT_ANSWERED)
    );
    assert_eq!(approvals(&fixture.runtime, "a").await.len(), 2);
}

/// Ruling F10: a call without `account` resolves again on each use, and
/// its allowance names the account the gate picked for it.
#[tokio::test]
async fn an_allowance_names_the_account_the_gate_picked() {
    let mut fixture = Fixture::new().await;
    let octo = fixture
        .token(
            "GitHub octo",
            CredentialScope::Global,
            "github",
            "octo",
            &[],
        )
        .await;
    let work = fixture
        .token(
            "GitHub work",
            CredentialScope::Global,
            "github",
            "work",
            &[],
        )
        .await;
    fixture.grant_token("a", &octo).await;
    fixture.grant_token("a", &work).await;
    let full = access(AccessLevel::Full, true, false);
    let run = claimed_at(&fixture, &full).await;
    let named = json!({"app": "github", "account": "work", "method": "POST", "path": "/repos/o/r/issues", "body": {"title": "Hi"}});
    assert_eq!(
        decided(spawn_gate(
            &fixture,
            &full,
            &run,
            APP_REQUEST_TOOL,
            named.clone(),
            SHORT_WAIT
        ))
        .await,
        allow()
    );
    let allowance = consume(&fixture, APP_REQUEST_TOOL, &named).await.unwrap();
    assert_eq!(allowance.target.as_deref(), Some(work.as_str()));

    // Two accounts and none named: refused until one goes.
    let unnamed = json!({"app": "github", "method": "POST", "path": "/repos/o/r/issues", "body": {"title": "Hi"}});
    assert_eq!(
        decided(spawn_gate(
            &fixture,
            &full,
            &run,
            APP_REQUEST_TOOL,
            unnamed.clone(),
            SHORT_WAIT
        ))
        .await,
        deny(APP_ACCOUNT_NEEDED)
    );
    fixture
        .runtime
        .command(ReactiveCommand::SetAppGrant {
            agent_id: "a".into(),
            grant: AppGrant::Token {
                entry_id: work.clone(),
            },
            granted: false,
        })
        .await
        .unwrap();
    assert_eq!(
        decided(spawn_gate(
            &fixture,
            &full,
            &run,
            APP_REQUEST_TOOL,
            unnamed.clone(),
            SHORT_WAIT
        ))
        .await,
        allow()
    );
    let allowance = consume(&fixture, APP_REQUEST_TOOL, &unnamed).await.unwrap();
    assert_eq!(allowance.target.as_deref(), Some(octo.as_str()));
}

#[tokio::test]
async fn pages_can_not_grant_apps_approve_sends_or_save_app_polls() {
    let fixture = Fixture::new().await;
    let runtime = &fixture.runtime;
    let run = claimed_at(&fixture, &access(AccessLevel::Ask, false, false)).await;
    let target = AppTarget {
        entry_id: "e".into(),
        app: "notion".into(),
        account: "ws".into(),
    };
    let app = runtime
        .open_approval_for_tests(
            &run,
            APP_REQUEST_TOOL,
            &app_card_input(
                &json!({"app": "notion", "method": "PATCH", "path": "/v1/pages/p"}),
                &target,
            ),
        )
        .await
        .unwrap();
    let mail = runtime
        .open_approval_for_tests(
            &run,
            SEND_MAIL_TOOL,
            &mail_card_input(
                &json!({"to": "ann@x.test", "subject": "Lunch", "body": "Noon?"}),
                &MailTarget {
                    mailbox_id: "m".into(),
                    account: "me@x.test".into(),
                    reply_subject: None,
                },
            ),
        )
        .await
        .unwrap();
    let bash = runtime
        .open_approval_for_tests(&run, "Bash", &json!({"command": "make"}))
        .await
        .unwrap();
    for (id, expected) in [
        (app, Some(OWNER_ONLY_CARD)),
        (mail, Some(OWNER_ONLY_CARD)),
        (bash, None),
        ("unknown".to_owned(), None),
    ] {
        for approve in [true, false] {
            let decide = ReactiveCommand::DecideApproval {
                id: id.clone(),
                approve,
            };
            assert_eq!(
                runtime.owner_only_refusal(&decide).await.unwrap(),
                expected,
                "{decide:?}"
            );
        }
    }
    let grant = ReactiveCommand::SetAppGrant {
        agent_id: "a".into(),
        grant: AppGrant::Mailbox {
            mailbox_id: "m".into(),
        },
        granted: true,
    };
    assert_eq!(
        runtime.owner_only_refusal(&grant).await.unwrap(),
        Some(OWNER_ONLY_APPS)
    );
}
