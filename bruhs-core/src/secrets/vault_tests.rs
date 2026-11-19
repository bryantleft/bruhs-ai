//! The manager's vault reads: values, login fields, one-time codes, checks
//! and the attention each read leaves on its entry. 1Password is always a
//! stand-in `op`; the keychain is the in-memory store.

use super::*;
use bruhs_models::vault::{CredentialScope, LoginInput, TotpInput, VaultCreate, VaultSecretInput};
use std::{
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
};

const SETUP_KEY: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
const WAIT: Duration = Duration::from_secs(10);

fn canary(label: &str) -> String {
    format!("{label}-{}", uuid::Uuid::new_v4().simple())
}

fn saved_login(name: &str, password: &str, totp: TotpInput) -> VaultCreate {
    VaultCreate {
        name: name.into(),
        scope: CredentialScope::Global,
        secret: VaultSecretInput::Login {
            url: "https://shop.test/in".into(),
            fields: LoginInput::Saved {
                username: "owner@shop.test".into(),
                password: Some(password.into()),
                totp,
            },
        },
    }
}

fn op_login(name: &str) -> VaultCreate {
    VaultCreate {
        name: name.into(),
        scope: CredentialScope::Global,
        secret: VaultSecretInput::Login {
            url: "https://shop.test/in".into(),
            fields: LoginInput::OnePassword {
                username: "op://P/Login/username".into(),
                password: "op://P/Login/password".into(),
                totp: Some("op://P/Login/otp?attribute=otp".into()),
            },
        },
    }
}

fn key(name: &str, secret: VaultSecretInput) -> VaultCreate {
    VaultCreate {
        name: name.into(),
        scope: CredentialScope::Global,
        secret,
    }
}

/// A stand-in `op` that answers only while `folder/signed-in` exists, with
/// `folder/<field>` for `op://P/Login/<field>` and `op://P/Key/value`.
fn stand_in(folder: &Path) -> PathBuf {
    let path = folder.join("op");
    let dir = folder.display();
    std::fs::write(
        &path,
        format!(
            r#"#!/bin/sh
[ -e '{dir}/signed-in' ] || {{ echo '[ERROR] account is not signed in' >&2; exit 1; }}
case "$4" in
  op://P/Login/username) /bin/cat '{dir}/username' ;;
  op://P/Login/password) /bin/cat '{dir}/password' ;;
  op://P/Login/otp?attribute=otp) printf '123456' ;;
  op://P/Key/value) /bin/cat '{dir}/value' ;;
  *) echo "[ERROR] isn't an item" >&2; exit 1 ;;
esac
"#
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn manager(folder: &Path) -> SecretManager {
    SecretManager::new(SecretStore::in_memory())
        .with_one_password(OnePassword::at(stand_in(folder), WAIT))
}

async fn attention_of(manager: &SecretManager, id: &str) -> Option<VaultAttention> {
    manager
        .vault_listing()
        .await
        .unwrap()
        .entries
        .into_iter()
        .find(|entry| entry.id == id)
        .and_then(|entry| entry.attention)
}

#[tokio::test]
async fn logins_resolve_each_field_and_record_attention() {
    let folder = tempfile::tempdir().unwrap();
    let manager = manager(folder.path());
    let password = canary("password");
    let saved = manager
        .vault_create(saved_login(
            "Saved",
            &password,
            TotpInput::Set {
                setup: SETUP_KEY.into(),
            },
        ))
        .await
        .unwrap();
    let username = manager
        .resolve_login(&saved.id, SecretField::Username)
        .await
        .unwrap();
    assert_eq!(username.as_str(), "owner@shop.test");
    let got = manager
        .resolve_login(&saved.id, SecretField::Password)
        .await
        .unwrap();
    assert_eq!(got.as_str(), password);
    let code = manager
        .resolve_login(&saved.id, SecretField::Totp)
        .await
        .unwrap();
    assert_eq!(code.len(), 6);
    assert!(code.bytes().all(|b| b.is_ascii_digit()));

    let op = manager.vault_create(op_login("Op")).await.unwrap();
    let signed_out = manager.resolve_login(&op.id, SecretField::Password).await;
    assert!(matches!(
        signed_out,
        Err(SecretError::OnePassword(OnePasswordError::NeedsOwner))
    ));
    assert_eq!(
        attention_of(&manager, &op.id).await,
        Some(VaultAttention::OnePasswordNeedsYou)
    );
    assert_eq!(attention_of(&manager, &saved.id).await, None);

    let op_password = canary("op-password");
    std::fs::write(folder.path().join("password"), &op_password).unwrap();
    std::fs::write(folder.path().join("username"), "op-user").unwrap();
    std::fs::write(folder.path().join("signed-in"), "").unwrap();
    let got = manager
        .resolve_login(&op.id, SecretField::Password)
        .await
        .unwrap();
    assert_eq!(got.as_str(), op_password);
    assert_eq!(attention_of(&manager, &op.id).await, None);
    let user = manager
        .resolve_login(&op.id, SecretField::Username)
        .await
        .unwrap();
    assert_eq!(user.as_str(), "op-user");
    let code = manager
        .resolve_login(&op.id, SecretField::Totp)
        .await
        .unwrap();
    assert_eq!(code.as_str(), "123456");

    // A login without a code has no such field; that isn't the entry's fault.
    let plain = manager
        .vault_create(saved_login("Plain", &canary("password"), TotpInput::Keep))
        .await
        .unwrap();
    assert!(matches!(
        manager.resolve_login(&plain.id, SecretField::Totp).await,
        Err(SecretError::NotSaved)
    ));
    assert_eq!(attention_of(&manager, &plain.id).await, None);
    let listing = serde_json::to_string(&manager.vault_listing().await.unwrap()).unwrap();
    assert!(!listing.contains(&password) && !listing.contains(&op_password));
}

#[tokio::test]
async fn values_resolve_from_the_keychain_or_1password() {
    let folder = tempfile::tempdir().unwrap();
    let manager = manager(folder.path());
    let value = canary("value");
    let stored = manager
        .vault_create(key(
            "KEY",
            VaultSecretInput::Value {
                value: Some(value.clone()),
            },
        ))
        .await
        .unwrap();
    assert_eq!(
        manager.resolve_value(&stored.id).await.unwrap().as_str(),
        value
    );
    let reference = manager
        .vault_create(key(
            "OP_KEY",
            VaultSecretInput::OnePassword {
                reference: "op://P/Key/value".into(),
            },
        ))
        .await
        .unwrap();
    let op_value = canary("op-value");
    std::fs::write(folder.path().join("value"), format!("{op_value}\n")).unwrap();
    std::fs::write(folder.path().join("signed-in"), "").unwrap();
    assert_eq!(
        manager.resolve_value(&reference.id).await.unwrap().as_str(),
        op_value
    );
    let login = manager
        .vault_create(saved_login("Login", &canary("password"), TotpInput::Keep))
        .await
        .unwrap();
    assert!(matches!(
        manager.resolve_value(&login.id).await,
        Err(SecretError::NotSaved)
    ));
    assert!(matches!(
        manager.resolve_value("v1-unknown").await,
        Err(SecretError::NotSaved)
    ));
    assert!(matches!(
        manager.vault_check("v1-unknown").await,
        Err(SecretError::NotSaved)
    ));

    // A key whose keychain item went missing is NotSaved and needs a look.
    let gone = manager
        .vault_create(key(
            "GONE",
            VaultSecretInput::Value {
                value: Some(canary("value")),
            },
        ))
        .await
        .unwrap();
    manager
        .store
        .remove_item(&format!("vault:global:{}", gone.id));
    assert!(matches!(
        manager.resolve_value(&gone.id).await,
        Err(SecretError::NotSaved)
    ));
    assert_eq!(
        attention_of(&manager, &gone.id).await,
        Some(VaultAttention::Failed)
    );
}

#[tokio::test]
async fn checks_report_without_values() {
    let folder = tempfile::tempdir().unwrap();
    let manager = manager(folder.path());
    let password = canary("password");
    let saved = manager
        .vault_create(saved_login(
            "Saved",
            &password,
            TotpInput::Set {
                setup: SETUP_KEY.into(),
            },
        ))
        .await
        .unwrap();
    let check = manager.vault_check(&saved.id).await.unwrap();
    assert_eq!(
        check,
        VaultCheck {
            ok: true,
            message: "Ready.".into()
        }
    );
    assert!(!serde_json::to_string(&check).unwrap().contains(&password));

    let op = manager.vault_create(op_login("Op")).await.unwrap();
    let check = manager.vault_check(&op.id).await.unwrap();
    assert!(!check.ok);
    assert_eq!(check.message, VaultAttention::OnePasswordNeedsYou.message());
    assert_eq!(
        attention_of(&manager, &op.id).await,
        Some(VaultAttention::OnePasswordNeedsYou)
    );
    let op_password = canary("op-password");
    std::fs::write(folder.path().join("password"), &op_password).unwrap();
    std::fs::write(folder.path().join("username"), "op-user").unwrap();
    std::fs::write(folder.path().join("signed-in"), "").unwrap();
    let check = manager.vault_check(&op.id).await.unwrap();
    assert!(check.ok);
    assert!(!serde_json::to_string(&check)
        .unwrap()
        .contains(&op_password));
    assert_eq!(attention_of(&manager, &op.id).await, None);

    let missing = SecretManager::new(SecretStore::in_memory())
        .with_one_password(OnePassword::at("/nonexistent/op".into(), WAIT));
    let op = missing.vault_create(op_login("Op")).await.unwrap();
    let check = missing.vault_check(&op.id).await.unwrap();
    assert_eq!(check.message, "1Password CLI isn't installed.");
    let listing = missing.vault_listing().await.unwrap();
    assert!(!listing.one_password);
    assert!(listing.bruhs.is_empty());
    assert!(manager.vault_listing().await.unwrap().one_password);
}

#[tokio::test]
async fn a_damaged_code_setup_is_refused_not_used() {
    let folder = tempfile::tempdir().unwrap();
    let manager = manager(folder.path());
    let login = manager
        .vault_create(saved_login(
            "Saved",
            &canary("password"),
            TotpInput::Set {
                setup: SETUP_KEY.into(),
            },
        ))
        .await
        .unwrap();
    let account = format!("vault:global:{}", login.id);
    for (digits, period) in [(6, 0), (12, 30), (0, 30), (6, 7)] {
        manager.store.put_item(
            &account,
            &format!(
                r#"{{"password":"p","totp":{{"secret":"{SETUP_KEY}","digits":{digits},"period":{period},"algorithm":"sha1"}}}}"#
            ),
        );
        assert!(matches!(
            manager.resolve_login(&login.id, SecretField::Totp).await,
            Err(SecretError::NotSaved)
        ));
        assert_eq!(
            attention_of(&manager, &login.id).await,
            Some(VaultAttention::Failed)
        );
        assert!(!manager.vault_check(&login.id).await.unwrap().ok);
    }
}

#[tokio::test]
async fn a_stuck_keychain_read_is_not_stacked() {
    let folder = tempfile::tempdir().unwrap();
    let manager = manager(folder.path()).with_read_wait(Duration::from_millis(200));
    let password = canary("password");
    let login = manager
        .vault_create(saved_login("Saved", &password, TotpInput::Keep))
        .await
        .unwrap();
    let release = Release::hold(&manager.store);
    let started = std::time::Instant::now();
    assert!(matches!(
        manager
            .resolve_login(&login.id, SecretField::Password)
            .await,
        Err(SecretError::Storage)
    ));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(
        attention_of(&manager, &login.id).await,
        Some(VaultAttention::KeychainNeedsYou)
    );
    assert_eq!(manager.store.held_reads(), 1);
    // The first read is still parked and its waiter gave up: later ones
    // answer at once, without waiting on it or parking another thread,
    // even with a long read wait of their own.
    let patient = manager.clone().with_read_wait(Duration::from_secs(10));
    for _ in 0..3 {
        let started = std::time::Instant::now();
        assert!(matches!(
            patient
                .resolve_login(&login.id, SecretField::Password)
                .await,
            Err(SecretError::Storage)
        ));
        assert!(!patient.vault_check(&login.id).await.unwrap().ok);
        assert!(started.elapsed() < Duration::from_secs(1));
    }
    assert_eq!(manager.store.held_reads(), 1);
    // Saves never wait on it either.
    tokio::time::timeout(
        Duration::from_secs(5),
        manager.vault_create(saved_login("Other", &canary("password"), TotpInput::Keep)),
    )
    .await
    .expect("a stuck read doesn't hold the lock")
    .unwrap();
    drop(release);
    tokio::time::timeout(Duration::from_secs(5), async {
        while !manager
            .reading
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the parked read ends once the keychain answers");
    let got = manager
        .resolve_login(&login.id, SecretField::Password)
        .await
        .unwrap();
    assert_eq!(got.as_str(), password);
    assert_eq!(attention_of(&manager, &login.id).await, None);
}

/// Ruling A1 M1: an update that keeps the stored password reads it as a
/// fill does, so a read stuck on a keychain prompt fails it within the
/// read wait, and parks no second read.
#[tokio::test]
async fn an_update_keeping_the_password_waits_no_longer_than_a_read() {
    let folder = tempfile::tempdir().unwrap();
    let manager = manager(folder.path()).with_read_wait(Duration::from_millis(200));
    let password = canary("password");
    let login = manager
        .vault_create(saved_login("Saved", &password, TotpInput::Keep))
        .await
        .unwrap();
    let release = Release::hold(&manager.store);
    let stuck = {
        let manager = manager.clone();
        let id = login.id.clone();
        tokio::spawn(async move { manager.resolve_login(&id, SecretField::Password).await })
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        while manager.store.held_reads() < 1 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the fill's read reaches the keychain");
    let rename = || VaultUpdate {
        secret: VaultSecretInput::Login {
            url: "https://shop.test/in".into(),
            fields: LoginInput::Saved {
                username: "new@shop.test".into(),
                password: None,
                // Adding a code keeps the stored password, so it is read.
                totp: TotpInput::Set {
                    setup: SETUP_KEY.into(),
                },
            },
        },
    };
    for _ in 0..3 {
        let started = std::time::Instant::now();
        let update = tokio::time::timeout(
            Duration::from_secs(5),
            manager.vault_update(&login.id, rename()),
        )
        .await
        .expect("the update ends within the read wait");
        assert!(matches!(update, Err(SecretError::Storage)), "{update:?}");
        assert!(started.elapsed() < Duration::from_secs(2));
    }
    assert_eq!(manager.store.held_reads(), 1, "no read was parked again");
    assert_eq!(
        attention_of(&manager, &login.id).await,
        Some(VaultAttention::KeychainNeedsYou)
    );
    drop(release);
    let _ = stuck.await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !manager
            .reading
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the parked read ends once the keychain answers");
    // Answered, the same update keeps the password.
    let entry = manager.vault_update(&login.id, rename()).await.unwrap();
    assert_eq!(
        entry.state,
        bruhs_models::secrets::SavedCredentialState::Saved
    );
    let got = manager
        .resolve_login(&login.id, SecretField::Password)
        .await
        .unwrap();
    assert_eq!(got.as_str(), password);
    assert_eq!(
        manager
            .resolve_login(&login.id, SecretField::Username)
            .await
            .unwrap()
            .as_str(),
        "new@shop.test"
    );
}

#[tokio::test]
async fn a_second_read_waits_for_the_first_instead_of_failing() {
    let folder = tempfile::tempdir().unwrap();
    let manager = manager(folder.path()).with_read_wait(Duration::from_secs(10));
    let password = canary("password");
    let login = manager
        .vault_create(saved_login("Saved", &password, TotpInput::Keep))
        .await
        .unwrap();
    let release = Release::hold(&manager.store);
    let read = |manager: SecretManager, id: String| {
        tokio::spawn(async move {
            manager
                .resolve_login(&id, SecretField::Password)
                .await
                .map(|value| value.as_str().to_owned())
        })
    };
    let first = read(manager.clone(), login.id.clone());
    tokio::time::timeout(Duration::from_secs(5), async {
        while manager.store.held_reads() < 1 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the first read reaches the keychain");
    let second = read(manager.clone(), login.id.clone());
    let check = {
        let manager = manager.clone();
        let id = login.id.clone();
        tokio::spawn(async move { manager.vault_check(&id).await })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    // Both wait on the first read without starting their own.
    assert!(!second.is_finished() && !check.is_finished());
    assert_eq!(manager.store.held_reads(), 1);
    drop(release);
    assert_eq!(first.await.unwrap().unwrap(), password);
    assert_eq!(second.await.unwrap().unwrap(), password);
    assert!(check.await.unwrap().unwrap().ok);
    assert_eq!(attention_of(&manager, &login.id).await, None);
    assert!(manager
        .reading
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_empty());
}

#[tokio::test]
async fn a_slow_1password_read_ends_at_the_read_wait() {
    let folder = tempfile::tempdir().unwrap();
    let slow = folder.path().join("slow-op");
    std::fs::write(&slow, "#!/bin/sh\n/bin/sleep 5\nprintf late\n").unwrap();
    std::fs::set_permissions(&slow, std::fs::Permissions::from_mode(0o755)).unwrap();
    let manager = SecretManager::new(SecretStore::in_memory())
        .with_one_password(OnePassword::at(slow, WAIT))
        .with_read_wait(Duration::from_millis(200));
    let op = manager.vault_create(op_login("Op")).await.unwrap();
    let started = std::time::Instant::now();
    assert!(matches!(
        manager.resolve_login(&op.id, SecretField::Password).await,
        Err(SecretError::OnePassword(OnePasswordError::NeedsOwner))
    ));
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(
        attention_of(&manager, &op.id).await,
        Some(VaultAttention::OnePasswordNeedsYou)
    );
}

#[tokio::test]
async fn the_app_state_builder_hands_op_to_the_manager() {
    let folder = tempfile::tempdir().unwrap();
    let state = crate::store::AppState::builder()
        .secret_store(SecretStore::in_memory())
        .one_password(OnePassword::at(stand_in(folder.path()), WAIT))
        .build();
    assert!(state.secrets().vault_listing().await.unwrap().one_password);
    let state = crate::store::AppState::builder()
        .secret_store(SecretStore::in_memory())
        .one_password(OnePassword::at("/nonexistent/op".into(), WAIT))
        .build();
    assert!(!state.secrets().vault_listing().await.unwrap().one_password);
}

/// Holds keychain reads until dropped, so a failed assertion never leaves
/// the runtime's blocking pool waiting.
struct Release(SecretStore);

impl Release {
    fn hold(store: &SecretStore) -> Self {
        store.hold_reads(true);
        Self(store.clone())
    }
}

impl Drop for Release {
    fn drop(&mut self) {
        self.0.hold_reads(false);
    }
}
