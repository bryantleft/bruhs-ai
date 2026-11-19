//! A bruh's vault in the runtime: the gate's login, the route's value and
//! its re-checks, the slow read that ends with owner copy, what `logins`
//! lists, deleting a bruh's own entries and the preamble's flag. The
//! keychain is the in-memory store and 1Password a stand-in `op`.

use super::approval::decide;
use super::computer_tests::{eventually, full_claude, set_computer};
use super::crew_tests::{approvals, claim_one, deny, runtime_with, LONG_WAIT};
use super::vault::{within, VaultRefusal};
use super::{worker, ReactiveError, ReactiveRuntime};
use crate::local_tools::{FakePages, FillValue, LocalToolsHost};
use crate::secrets::{OnePassword, SecretManager, SecretStore};
use crate::PermissionDecision;
use bruhs_models::agent::AgentProvider;
use bruhs_models::crew::*;
use bruhs_models::local_tools::SecretRef;
use bruhs_models::reactive::*;
use bruhs_models::vault::{
    CredentialScope, LoginInput, SecretField, TotpInput, VaultAttention, VaultCreate,
    VaultSecretInput,
};
use serde_json::{json, Value};
use std::{
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

const SHOP: &str = "https://shop.test/in";
const USERNAME: &str = "owner@shop.test";

/// A random value no listing may hold.
fn canary(label: &str) -> String {
    format!("{label}-{}", uuid::Uuid::new_v4().simple())
}

fn ask() -> BruhAccess {
    BruhAccess {
        level: AccessLevel::Ask,
        ..BruhAccess::default()
    }
}

/// A stand-in `op` that sleeps `delay` seconds, then answers
/// `op://P/Login/<field>` with `<field>-value`.
fn stand_in(folder: &Path, delay: u32) -> PathBuf {
    let path = folder.join("op");
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\n/bin/sleep {delay}\ncase \"$4\" in\n  op://P/Login/*) printf '%s-value' \"${{4##*/}}\" ;;\n  *) echo \"[ERROR] isn't an item\" >&2; exit 1 ;;\nesac\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn manager(folder: &Path, delay: u32) -> SecretManager {
    SecretManager::new(SecretStore::in_memory()).with_one_password(OnePassword::at(
        stand_in(folder, delay),
        Duration::from_secs(10),
    ))
}

fn saved_login(name: &str, scope: CredentialScope, password: &str) -> VaultCreate {
    VaultCreate {
        name: name.into(),
        scope,
        secret: VaultSecretInput::Login {
            url: SHOP.into(),
            fields: LoginInput::Saved {
                username: USERNAME.into(),
                password: Some(password.into()),
                totp: TotpInput::Keep,
            },
        },
    }
}

fn op_login(name: &str) -> VaultCreate {
    VaultCreate {
        name: name.into(),
        scope: CredentialScope::Global,
        secret: VaultSecretInput::Login {
            url: SHOP.into(),
            fields: LoginInput::OnePassword {
                username: "op://P/Login/username".into(),
                password: "op://P/Login/password".into(),
                totp: None,
            },
        },
    }
}

fn bruh(agent_id: &str) -> CredentialScope {
    CredentialScope::Bruh {
        agent_id: agent_id.into(),
    }
}

fn set_vault(agent_id: &str, entries: &[&str]) -> ReactiveCommand {
    ReactiveCommand::SetVault {
        agent_id: agent_id.into(),
        entries: entries.iter().map(|id| (*id).to_owned()).collect(),
    }
}

/// Ask Claude bruhs `a` and `b`; `a` has Computer on, its browser open on
/// an attached host, and the runtime has `vault`.
async fn vault_runtime(
    vault: SecretManager,
    wait: Option<Duration>,
) -> (
    tempfile::TempDir,
    ReactiveRuntime,
    Arc<LocalToolsHost>,
    FakePages,
) {
    let (dir, runtime) = runtime_with(&[full_claude("a"), full_claude("b")]).await;
    let runtime = match wait {
        Some(wait) => runtime.with_vault_wait(wait),
        None => runtime,
    };
    for id in ["a", "b"] {
        runtime
            .command(ReactiveCommand::SetBruh {
                agent_id: id.into(),
                look: BruhLook::default(),
                access: ask(),
                effort: None,
            })
            .await
            .unwrap();
    }
    let (host, fake) = LocalToolsHost::with_fake_pages();
    runtime.attach_computers(host.clone()).unwrap();
    runtime.attach_vault(vault).unwrap();
    runtime.command(set_computer("a", true)).await.unwrap();
    eventually(|| async { host.computer_state("a").await.is_ok() }).await;
    host.open_computer("a").await.unwrap();
    (dir, runtime, host, fake)
}

/// A chat claimed for `a`, now Running.
async fn running(runtime: &ReactiveRuntime) -> ReactiveRun {
    runtime
        .command(ReactiveCommand::Chat {
            agent_id: "a".into(),
            message: "Sign in.".into(),
            run_id: None,
        })
        .await
        .unwrap();
    claim_one(runtime).await.unwrap().run
}

fn fill(login_id: &str, field: &str) -> Value {
    json!({"command": {
        "action": "fill",
        "selector": "#password",
        "secret": {"login_id": login_id, "field": field},
    }})
}

/// The gate's answer to one call of `a`'s browser tool at Ask.
async fn gate(
    runtime: &ReactiveRuntime,
    run: &ReactiveRun,
    input: &Value,
) -> Option<PermissionDecision> {
    let project = std::env::temp_dir().to_string_lossy().into_owned();
    decide(
        runtime,
        &ask(),
        AgentProvider::Claude,
        true,
        run,
        runtime
            .allowances
            .current_claim(&run.id)
            .unwrap_or_default(),
        &project,
        BRUH_BROWSER_TOOL,
        input,
        LONG_WAIT,
    )
    .await
    .unwrap()
}

fn secret(login_id: &str, field: SecretField) -> SecretRef {
    SecretRef {
        login_id: login_id.into(),
        field,
    }
}

#[tokio::test]
async fn a_granted_shared_login_is_allowed_on_its_origin_without_a_card() {
    let folder = tempfile::tempdir().unwrap();
    let vault = manager(folder.path(), 0);
    let login = vault
        .vault_create(saved_login("Shop", CredentialScope::Global, &canary("pw")))
        .await
        .unwrap();
    let (_dir, runtime, _host, fake) = vault_runtime(vault, None).await;
    runtime.command(set_vault("a", &[&login.id])).await.unwrap();
    fake.set_url("a", SHOP);
    let run = running(&runtime).await;
    let input = fill(&login.id, "password");
    assert_eq!(
        gate(&runtime, &run, &input).await,
        Some(PermissionDecision::Allow)
    );
    assert!(approvals(&runtime, "a").await.is_empty(), "no card opened");
    let allowance = runtime
        .consume_browser_allowance("a", &input["command"])
        .await
        .unwrap()
        .expect("the allowed fill may reach the route once");
    assert_eq!(allowance.page_url.as_deref(), Some(SHOP));
    assert_eq!(
        runtime
            .consume_browser_allowance("a", &input["command"])
            .await
            .unwrap(),
        None
    );
    // A field the login lacks, and another site, are refused.
    assert_eq!(
        gate(&runtime, &run, &fill(&login.id, "totp")).await,
        deny(LOGIN_NO_FIELD)
    );
    fake.set_url("a", "https://other.test/");
    assert_eq!(gate(&runtime, &run, &input).await, deny(LOGIN_WRONG_ORIGIN));
    assert!(approvals(&runtime, "a").await.is_empty());
}

#[tokio::test]
async fn an_ungranted_or_other_bruhs_login_is_refused() {
    let folder = tempfile::tempdir().unwrap();
    let vault = manager(folder.path(), 0);
    let shared = vault
        .vault_create(saved_login("Shop", CredentialScope::Global, &canary("pw")))
        .await
        .unwrap();
    let others = vault
        .vault_create(saved_login("B shop", bruh("b"), &canary("pw")))
        .await
        .unwrap();
    let (_dir, runtime, _host, fake) = vault_runtime(vault, None).await;
    fake.set_url("a", SHOP);
    let run = running(&runtime).await;
    // An ungranted Global login, its id named exactly.
    assert_eq!(
        gate(&runtime, &run, &fill(&shared.id, "password")).await,
        deny(LOGIN_NOT_GRANTED)
    );
    // Another bruh's own login, even when "granted".
    runtime
        .command(set_vault("a", &[&others.id]))
        .await
        .unwrap();
    assert_eq!(
        gate(&runtime, &run, &fill(&others.id, "password")).await,
        deny(LOGIN_NOT_GRANTED)
    );
    assert_eq!(
        gate(&runtime, &run, &fill("v1-nothing", "password")).await,
        deny(LOGIN_NOT_GRANTED)
    );
    assert!(approvals(&runtime, "a").await.is_empty());
    for id in [&shared.id, &others.id] {
        assert_eq!(
            runtime
                .consume_browser_allowance("a", &fill(id, "password")["command"])
                .await
                .unwrap(),
            None,
            "a refused fill leaves no allowance"
        );
    }
}

#[tokio::test]
async fn a_runtime_without_a_vault_refuses_every_fill() {
    let (_dir, runtime) = runtime_with(&[full_claude("a")]).await;
    let (host, fake) = LocalToolsHost::with_fake_pages();
    runtime.attach_computers(host.clone()).unwrap();
    runtime
        .command(ReactiveCommand::SetBruh {
            agent_id: "a".into(),
            look: BruhLook::default(),
            access: ask(),
            effort: None,
        })
        .await
        .unwrap();
    runtime.command(set_computer("a", true)).await.unwrap();
    eventually(|| async { host.computer_state("a").await.is_ok() }).await;
    host.open_computer("a").await.unwrap();
    fake.set_url("a", SHOP);
    let run = running(&runtime).await;
    assert!(runtime.vault().is_none());
    assert_eq!(
        gate(&runtime, &run, &fill("v1-login", "password")).await,
        deny(LOGIN_NOT_GRANTED)
    );
    assert!(runtime.logins("a").await.unwrap().is_empty());
    assert!(matches!(
        runtime
            .fill_value("a", &secret("v1-login", SecretField::Password), Some(SHOP))
            .await,
        Err(VaultRefusal::Refused(LOGIN_NOT_GRANTED))
    ));
}

/// The gate's vault lookup ends within its wait, and a read that doesn't
/// answer counts as no login.
#[tokio::test(start_paused = true)]
async fn a_vault_lookup_that_never_answers_fails_closed() {
    let started = tokio::time::Instant::now();
    let read = within(
        Duration::from_secs(30),
        std::future::pending::<Result<(), crate::secrets::SecretError>>(),
    )
    .await;
    assert!(matches!(read, Err(ReactiveError::Storage(_))));
    assert_eq!(started.elapsed(), Duration::from_secs(30));
}

#[tokio::test]
async fn fill_value_rechecks_and_resolves() {
    let folder = tempfile::tempdir().unwrap();
    let vault = manager(folder.path(), 0);
    let pw = canary("pw");
    let login = vault
        .vault_create(saved_login("Shop", CredentialScope::Global, &pw))
        .await
        .unwrap();
    let (_dir, runtime, _host, _fake) = vault_runtime(vault, None).await;
    runtime.command(set_vault("a", &[&login.id])).await.unwrap();
    let wanted = secret(&login.id, SecretField::Password);
    match runtime.fill_value("a", &wanted, Some(SHOP)).await {
        Ok(FillValue {
            login_id,
            origin,
            field,
            value,
        }) => {
            assert_eq!(login_id, login.id);
            assert_eq!(origin, "https://shop.test");
            assert_eq!(field, SecretField::Password);
            assert_eq!(value.as_str(), pw);
        }
        Err(refusal) => panic!("refused: {refusal:?}"),
    }
    let username = runtime
        .fill_value("a", &secret(&login.id, SecretField::Username), Some(SHOP))
        .await;
    assert!(matches!(username, Ok(FillValue { value, .. }) if value.as_str() == USERNAME));
    let refused = |result: Result<FillValue, VaultRefusal>| result.err();
    assert_eq!(
        refused(
            runtime
                .fill_value("a", &wanted, Some("https://other.test/"))
                .await
        ),
        Some(VaultRefusal::Refused(LOGIN_WRONG_ORIGIN))
    );
    assert_eq!(
        refused(runtime.fill_value("a", &wanted, None).await),
        Some(VaultRefusal::Refused(PAGE_UNKNOWN))
    );
    assert_eq!(
        refused(
            runtime
                .fill_value("a", &secret(&login.id, SecretField::Totp), Some(SHOP))
                .await
        ),
        Some(VaultRefusal::Refused(LOGIN_NO_FIELD))
    );
    // Bruh b was never granted it.
    assert_eq!(
        refused(runtime.fill_value("b", &wanted, Some(SHOP)).await),
        Some(VaultRefusal::Refused(LOGIN_NOT_GRANTED))
    );
    // The grant is read again on every fill.
    runtime.command(set_vault("a", &[])).await.unwrap();
    assert_eq!(
        refused(runtime.fill_value("a", &wanted, Some(SHOP)).await),
        Some(VaultRefusal::Refused(LOGIN_NOT_GRANTED))
    );
}

#[tokio::test]
async fn a_slow_vault_read_ends_the_fill_with_owner_copy() {
    let folder = tempfile::tempdir().unwrap();
    let wait = Duration::from_millis(200);
    let vault = manager(folder.path(), 5).with_read_wait(wait);
    let login = vault.vault_create(op_login("Op shop")).await.unwrap();
    let (_dir, runtime, _host, _fake) = vault_runtime(vault.clone(), Some(wait)).await;
    runtime.command(set_vault("a", &[&login.id])).await.unwrap();
    let started = std::time::Instant::now();
    let filled = runtime
        .fill_value("a", &secret(&login.id, SecretField::Password), Some(SHOP))
        .await;
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(
        filled.err(),
        Some(VaultRefusal::Unavailable(ONE_PASSWORD_NEEDS_OWNER))
    );
    let listing = vault.vault_listing().await.unwrap();
    let entry = listing
        .entries
        .iter()
        .find(|entry| entry.id == login.id)
        .unwrap();
    assert_eq!(entry.attention, Some(VaultAttention::OnePasswordNeedsYou));
}

#[tokio::test]
async fn logins_list_names_origins_and_fields_only() {
    let folder = tempfile::tempdir().unwrap();
    let vault = manager(folder.path(), 0);
    let pw = canary("pw");
    let shared = vault
        .vault_create(saved_login("Shop", CredentialScope::Global, &pw))
        .await
        .unwrap();
    let own = vault
        .vault_create(saved_login("Its own", bruh("a"), &pw))
        .await
        .unwrap();
    let op = vault.vault_create(op_login("Op shop")).await.unwrap();
    // Neither listed: ungranted, and another bruh's.
    vault
        .vault_create(saved_login("Ungranted", CredentialScope::Global, &pw))
        .await
        .unwrap();
    vault
        .vault_create(saved_login("B shop", bruh("b"), &pw))
        .await
        .unwrap();
    let (_dir, runtime, _host, _fake) = vault_runtime(vault, None).await;
    runtime
        .command(set_vault("a", &[&shared.id, &op.id]))
        .await
        .unwrap();
    let listed = serde_json::to_value(runtime.logins("a").await.unwrap()).unwrap();
    let target = |id: &str, name: &str| {
        json!({
            "login_id": id,
            "name": name,
            "url": SHOP,
            "origin": "https://shop.test",
            "fields": ["username", "password"],
        })
    };
    assert_eq!(
        listed,
        json!([
            target(&own.id, "Its own"),
            target(&op.id, "Op shop"),
            target(&shared.id, "Shop"),
        ])
    );
    let text = listed.to_string();
    for absent in [USERNAME, pw.as_str(), "op://"] {
        assert!(!text.contains(absent), "{absent} listed");
    }
    assert!(runtime.logins("nobody").await.unwrap().is_empty());
}

#[tokio::test]
async fn deleting_a_bruh_deletes_its_own_vault_entries() {
    let folder = tempfile::tempdir().unwrap();
    let vault = manager(folder.path(), 0);
    let pw = canary("pw");
    vault
        .vault_create(saved_login("Its own", bruh("a"), &pw))
        .await
        .unwrap();
    let shared = vault
        .vault_create(saved_login("Shop", CredentialScope::Global, &pw))
        .await
        .unwrap();
    // A bruh this runtime never had: another reactive database's, say.
    let elsewhere = vault
        .vault_create(saved_login("Elsewhere", bruh("z"), &pw))
        .await
        .unwrap();
    let (_dir, runtime, _host, _fake) = vault_runtime(vault.clone(), None).await;
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::DeleteAgent { id: "a".into() },
        })
        .await
        .unwrap();
    eventually(|| async { vault.vault_entries().await.unwrap().len() == 2 }).await;
    let mut kept: Vec<String> = vault
        .vault_entries()
        .await
        .unwrap()
        .into_iter()
        .map(|entry| entry.id)
        .collect();
    kept.sort();
    let mut expected = vec![shared.id, elsewhere.id];
    expected.sort();
    assert_eq!(kept, expected);
    // The deletion's record is used up: nothing is forgotten again.
    eventually(|| async { runtime.vault_forgets_for_tests().await.is_empty() }).await;
}

#[tokio::test]
async fn the_preamble_mentions_logins_only_when_usable() {
    let folder = tempfile::tempdir().unwrap();
    let vault = manager(folder.path(), 0);
    let login = vault
        .vault_create(saved_login("Shop", CredentialScope::Global, &canary("pw")))
        .await
        .unwrap();
    let (_dir, runtime, _host, _fake) = vault_runtime(vault, None).await;
    let run = running(&runtime).await;
    let profile = BruhProfile {
        access: ask(),
        computer: true,
        ..BruhProfile::default()
    };
    let prompt = |vault: &worker::TurnVault| {
        worker::turn(
            &full_claude("a"),
            &profile,
            &run,
            "",
            Path::new("/home"),
            vault,
        )
        .prompt
    };
    let without = runtime.turn_vault("a", true).await;
    assert!(!prompt(&without).contains(VAULT_PREAMBLE));
    runtime.command(set_vault("a", &[&login.id])).await.unwrap();
    let with = runtime.turn_vault("a", true).await;
    assert!(prompt(&with).contains(VAULT_PREAMBLE));
    // Computer off: no vault read, no paragraph.
    assert!(!runtime.turn_vault("a", false).await.logins);
}

fn key(name: &str, scope: CredentialScope) -> VaultCreate {
    VaultCreate {
        name: name.into(),
        scope,
        secret: VaultSecretInput::Value {
            value: Some(canary("key")),
        },
    }
}

#[tokio::test]
async fn keys_are_listed_by_name_and_matched_own_first() {
    let folder = tempfile::tempdir().unwrap();
    let vault = manager(folder.path(), 0);
    let shared = vault
        .vault_create(key("DEPLOY_TOKEN", CredentialScope::Global))
        .await
        .unwrap();
    let own = vault
        .vault_create(key("DEPLOY_TOKEN", bruh("a")))
        .await
        .unwrap();
    let other = vault
        .vault_create(key("API_KEY", CredentialScope::Global))
        .await
        .unwrap();
    vault
        .vault_create(key("UNGRANTED", CredentialScope::Global))
        .await
        .unwrap();
    vault.vault_create(key("B_ONLY", bruh("b"))).await.unwrap();
    let login = vault
        .vault_create(saved_login("Shop", CredentialScope::Global, &canary("pw")))
        .await
        .unwrap();
    let (_dir, runtime, _host, _fake) = vault_runtime(vault, None).await;
    runtime
        .command(set_vault("a", &[&shared.id, &other.id, &login.id]))
        .await
        .unwrap();
    assert_eq!(runtime.usable_keys("a").await, ["API_KEY", "DEPLOY_TOKEN"]);
    assert_eq!(
        runtime.usable_value("a", "DEPLOY_TOKEN").await,
        Some(own.id)
    );
    assert_eq!(runtime.usable_value("a", "API_KEY").await, Some(other.id));
    for absent in ["Shop", "UNGRANTED", "B_ONLY", "api_key"] {
        assert_eq!(runtime.usable_value("a", absent).await, None, "{absent}");
    }
    assert_eq!(runtime.usable_value("b", "DEPLOY_TOKEN").await, None);
    assert_eq!(runtime.usable_keys("b").await, ["B_ONLY"]);
}

#[tokio::test]
async fn a_hanging_vault_refuses_the_fill_within_the_wait() {
    let folder = tempfile::tempdir().unwrap();
    let vault = manager(folder.path(), 0);
    let login = vault
        .vault_create(saved_login("Shop", CredentialScope::Global, &canary("pw")))
        .await
        .unwrap();
    let wait = Duration::from_millis(200);
    let (_dir, runtime, _host, fake) = vault_runtime(vault.clone(), Some(wait)).await;
    runtime.command(set_vault("a", &[&login.id])).await.unwrap();
    fake.set_url("a", SHOP);
    let run = running(&runtime).await;
    let input = fill(&login.id, "password");
    // Reached through the vault while it answers.
    assert_eq!(
        gate(&runtime, &run, &input).await,
        Some(PermissionDecision::Allow)
    );
    runtime
        .consume_browser_allowance("a", &input["command"])
        .await
        .unwrap()
        .unwrap();
    let held = vault.hold_store_for_tests().unwrap();
    let started = std::time::Instant::now();
    assert_eq!(gate(&runtime, &run, &input).await, deny(LOGIN_NOT_GRANTED));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(
        runtime
            .consume_browser_allowance("a", &input["command"])
            .await
            .unwrap(),
        None
    );
    let filled = runtime
        .fill_value("a", &secret(&login.id, SecretField::Password), Some(SHOP))
        .await;
    assert_eq!(
        filled.err(),
        Some(VaultRefusal::Unavailable(VAULT_READ_FAILED))
    );
    assert!(runtime.logins("a").await.is_err());
    drop(held);
    // Once the catalog answers again, so does the gate.
    assert_eq!(
        gate(&runtime, &run, &input).await,
        Some(PermissionDecision::Allow)
    );
}

#[tokio::test]
async fn a_forget_that_fails_keeps_its_record_and_tries_again() {
    let folder = tempfile::tempdir().unwrap();
    let vault = manager(folder.path(), 0);
    vault
        .vault_create(saved_login("Its own", bruh("a"), &canary("pw")))
        .await
        .unwrap();
    let wait = Duration::from_millis(200);
    let (_dir, runtime, _host, _fake) = vault_runtime(vault.clone(), Some(wait)).await;
    let held = vault.hold_store_for_tests().unwrap();
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::DeleteAgent { id: "a".into() },
        })
        .await
        .unwrap();
    // Past a pass's wait, the record is kept.
    tokio::time::sleep(wait * 3).await;
    assert_eq!(
        runtime.vault_forgets_for_tests().await,
        std::collections::HashSet::from(["a".to_owned()])
    );
    drop(held);
    // The next pass (any change) deletes the entries and then the record.
    runtime.command(set_vault("b", &[])).await.unwrap();
    eventually(|| async { runtime.vault_forgets_for_tests().await.is_empty() }).await;
    assert!(vault.vault_entries().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_new_agent_cannot_take_a_deleted_bruhs_id_until_its_entries_go() {
    let (_dir, runtime) = runtime_with(&[full_claude("a"), full_claude("b")]).await;
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::DeleteAgent { id: "a".into() },
        })
        .await
        .unwrap();
    let refused = |result: Result<ReactiveSnapshot, ReactiveError>| matches!(result, Err(ReactiveError::Conflict(message)) if message == BRUH_ID_PENDING_DELETE);
    let create = || ReactiveCommand::CreateBruh {
        agent: full_claude("a"),
        look: BruhLook::default(),
        access: ask(),
        effort: None,
    };
    let save = || ReactiveCommand::Apply {
        change: ReactiveChange::SaveAgent {
            agent: full_claude("a"),
        },
    };
    assert!(refused(runtime.command(create()).await));
    assert!(refused(runtime.command(save()).await));
    // An existing agent saves as before.
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveAgent {
                agent: ReactiveAgent {
                    name: "Renamed".into(),
                    ..full_claude("b")
                },
            },
        })
        .await
        .unwrap();
    // Once the vault has deleted its entries, the id is free.
    let folder = tempfile::tempdir().unwrap();
    runtime.attach_vault(manager(folder.path(), 0)).unwrap();
    eventually(|| async { runtime.vault_forgets_for_tests().await.is_empty() }).await;
    runtime.command(create()).await.unwrap();
}

#[tokio::test]
async fn a_grant_revoked_or_login_deleted_during_the_read_types_nothing() {
    let folder = tempfile::tempdir().unwrap();
    let vault = manager(folder.path(), 1);
    let login = vault.vault_create(op_login("Op shop")).await.unwrap();
    let (_dir, runtime, _host, _fake) = vault_runtime(vault.clone(), None).await;
    runtime.command(set_vault("a", &[&login.id])).await.unwrap();
    let wanted = secret(&login.id, SecretField::Password);
    let fill_soon = || {
        let (runtime, wanted) = (runtime.clone(), wanted.clone());
        tokio::spawn(async move { runtime.fill_value("a", &wanted, Some(SHOP)).await.err() })
    };
    // The same slow read, left alone, types its value.
    let filled = runtime.fill_value("a", &wanted, Some(SHOP)).await;
    assert!(matches!(filled, Ok(FillValue { value, .. }) if value.as_str() == "password-value"));

    let task = fill_soon();
    tokio::time::sleep(Duration::from_millis(300)).await;
    runtime.command(set_vault("a", &[])).await.unwrap();
    assert_eq!(
        task.await.unwrap(),
        Some(VaultRefusal::Refused(LOGIN_NOT_GRANTED))
    );

    runtime.command(set_vault("a", &[&login.id])).await.unwrap();
    let task = fill_soon();
    tokio::time::sleep(Duration::from_millis(300)).await;
    vault.vault_delete(&login.id).await.unwrap();
    assert_eq!(
        task.await.unwrap(),
        Some(VaultRefusal::Refused(LOGIN_NOT_GRANTED))
    );
}

/// The grants of `agent_id` as the snapshot shows them.
async fn grants(runtime: &ReactiveRuntime, agent_id: &str) -> Vec<String> {
    runtime
        .snapshot()
        .await
        .unwrap()
        .bruhs
        .into_iter()
        .find(|bruh| bruh.agent_id == agent_id)
        .map(|bruh| bruh.vault)
        .unwrap_or_default()
}

/// Ruling V Task 10 I2: a deleted entry leaves every bruh's grants in one
/// transaction, so a revoke at the same moment is never undone.
#[tokio::test]
async fn forgetting_an_entry_drops_it_from_every_grant_and_keeps_a_revoke() {
    let (_dir, runtime) = runtime_with(&[full_claude("a"), full_claude("b")]).await;
    runtime.command(set_vault("a", &["d", "x"])).await.unwrap();
    runtime.command(set_vault("b", &["d", "y"])).await.unwrap();
    let mut changes = runtime.watch_changes();
    changes.borrow_and_update();
    assert_eq!(runtime.forget_vault_entry("d").await.unwrap(), 2);
    assert!(changes.has_changed().unwrap(), "the change is published");
    assert_eq!(grants(&runtime, "a").await, ["x"]);
    assert_eq!(grants(&runtime, "b").await, ["y"]);
    changes.borrow_and_update();
    assert_eq!(runtime.forget_vault_entry("d").await.unwrap(), 0);
    assert!(
        !changes.has_changed().unwrap(),
        "nothing changed, nothing sent"
    );

    // The owner revokes `x` while `d` is deleted, in either order: `x`
    // never comes back.
    for _ in 0..50 {
        runtime.command(set_vault("a", &["d", "x"])).await.unwrap();
        let (revoked, forgotten) = tokio::join!(
            runtime.command(set_vault("a", &["d"])),
            runtime.forget_vault_entry("d"),
        );
        revoked.unwrap();
        forgotten.unwrap();
        let now = grants(&runtime, "a").await;
        assert!(
            now.is_empty() || now == ["d"],
            "a revoked grant came back: {now:?}"
        );
        assert!(!now.contains(&"x".to_owned()));
    }
}
