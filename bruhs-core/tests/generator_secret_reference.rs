//! Exercise the desktop/host store and the 1Password CLI protocol without a user vault.
#![cfg(unix)]
#![expect(
    clippy::unwrap_used,
    reason = "integration test assertions and fixtures"
)]
use bruhs_core::{
    secret_sources::{self, Change, SecretReference, Source},
    text_generators::{self as api, CredentialEdit, Draft, Preset},
};
use serde_json::json;
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};
use wiremock::{
    matchers::{header, method, path},
    Mock, MockServer, ResponseTemplate,
};

const SOURCE: &str = "work-account";
const FIELD: &str = "op://Team Vault/Model API/credentials/key";
fn reference() -> SecretReference {
    SecretReference {
        source: SOURCE.into(),
        reference: FIELD.into(),
    }
}

#[test]
fn generator_resolves_rotated_keys_and_unsets_removed_sources() {
    let dir = tempfile::tempdir().unwrap();
    if let Some(root) = std::env::var_os("BRUHS_REFERENCE_FIXTURE") {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(run(Path::new(&root)));
        return;
    }
    let root = dir.path();
    let path = std::env::join_paths(std::iter::once(root.to_path_buf()).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))
    .unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "generator_resolves_rotated_keys_and_unsets_removed_sources",
            "--nocapture",
        ])
        .env("BRUHS_REFERENCE_FIXTURE", root)
        .env("HOME", root)
        .env("XDG_CONFIG_HOME", root)
        .env("BRUHS_GENERATORS_DB", root.join("generator.db"))
        .env("PATH", path)
        .status()
        .unwrap();
    assert!(status.success());
}

async fn run(root: &Path) {
    keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
    let cli = root.join("op");
    fs::write(&cli,r#"#!/bin/sh
[ "$#" -eq 5 ] && [ "$1" = read ] && [ "$2" = --no-newline ] && [ "$3" = --account ] && [ "$4" = team.example ] && [ "$5" = 'op://Team Vault/Model API/credentials/key' ] || exit 1
if test -f "$BRUHS_REFERENCE_FIXTURE/locked"; then cat "$BRUHS_REFERENCE_FIXTURE/value" >&2; exit 1; fi
cat "$BRUHS_REFERENCE_FIXTURE/value"
"#).unwrap();
    fs::set_permissions(cli, fs::Permissions::from_mode(0o700)).unwrap();
    secret_sources::change(Change::SaveSource(
        SOURCE.into(),
        Source {
            name: "Team vault".into(),
            account: "team.example".into(),
        },
    ))
    .unwrap();
    let server = MockServer::start().await;
    let new_draft = |credential| Draft {
        preset: Preset::Custom,
        label: "Team generator".into(),
        url: server.uri(),
        credential,
    };
    let value = root.join("value");
    fs::write(&value, " ").unwrap();
    assert_eq!(
        reference().check().await.unwrap_err().to_string(),
        "The referenced field is empty."
    );
    fs::write(&value, "fixture-first-key").unwrap();
    reference().check().await.unwrap();
    let models = ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":"writer"}]}));
    Mock::given(method("GET"))
        .and(path("/models"))
        .and(header("authorization", "Bearer fixture-first-key"))
        .respond_with(models.clone())
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .and(header("authorization", "Bearer fixture-manual-key"))
        .respond_with(models.clone())
        .expect(1)
        .mount(&server)
        .await;
    let manual = api::connect(
        None,
        new_draft(CredentialEdit::Value(
            "fixture-manual-key".to_owned().into(),
        )),
    )
    .await
    .unwrap();
    let old_record = serde_json::to_value(&manual).unwrap();
    assert!(old_record.get("reference").is_none() && old_record["key"].is_string());
    let saved = api::connect(
        Some(manual),
        new_draft(CredentialEdit::Reference(reference())),
    )
    .await
    .unwrap();
    assert!(serde_json::to_value(&saved).unwrap()["key"].is_null());
    let row = api::list().await.unwrap().pop().unwrap();
    assert_eq!(saved.id, row.id);
    assert_eq!(row.reference.as_ref(), Some(&reference()));
    assert!(row.has_key());
    fs::write(&value, "fixture-rotated-key").unwrap();
    Mock::given(method("GET"))
        .and(path("/models"))
        .and(header("authorization", "Bearer fixture-rotated-key"))
        .respond_with(models.clone())
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("authorization", "Bearer fixture-rotated-key"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"choices":[{"message":{"content":"OK"}}]})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let refreshed = api::refresh(row).await.unwrap();
    api::test(refreshed.clone(), "writer".into()).await.unwrap();
    fs::write(root.join("locked"), "").unwrap();
    let check_error = reference().check().await.unwrap_err().to_string();
    assert!(check_error.contains("1Password") && !check_error.contains("fixture-rotated-key"));
    let error = api::connect(Some(refreshed.clone()), new_draft(CredentialEdit::Keep))
        .await
        .unwrap_err();
    assert!(error.contains("1Password") && !error.contains("fixture-rotated-key"));
    assert_eq!(api::list().await.unwrap()[0].revision, refreshed.revision);
    assert_eq!(
        api::secret_source_consumers(SOURCE).unwrap(),
        ["Team generator"]
    );
    secret_sources::change(Change::RemoveSource(SOURCE.into())).unwrap();
    assert!(api::secret_source_consumers(SOURCE).unwrap().is_empty());
    assert!(reference()
        .check()
        .await
        .unwrap_err()
        .to_string()
        .contains("unavailable"));
    let cleared = api::list().await.unwrap().pop().unwrap();
    assert!(
        cleared.reference.is_none() && !cleared.has_key() && cleared.revision > refreshed.revision
    );
    assert!(api::enable(refreshed.clone(), false).await.is_err());
    let metadata = fs::read(root.join("generator.db")).unwrap();
    for key in [
        "fixture-first-key",
        "fixture-rotated-key",
        "fixture-manual-key",
    ] {
        assert!(!metadata.windows(key.len()).any(|w| w == key.as_bytes()));
    }
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(models)
        .expect(1)
        .mount(&server)
        .await;
    let cleared = api::connect(
        Some(cleared),
        new_draft(CredentialEdit::Value(String::new().into())),
    )
    .await
    .unwrap();
    assert!(!cleared.has_key() && cleared.reference.is_none());
    api::remove(cleared).await.unwrap();
}
