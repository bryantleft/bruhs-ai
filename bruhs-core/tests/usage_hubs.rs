//! Exercise the persistent hub store and named-source cleanup in an isolated user profile.
use bruhs_core::{
    secret_sources::{self, Change, SecretReference, Source},
    usage_hubs::{self as hubs, ManagementKey},
};
use std::process::Command;

#[test]
fn hub_replacement_and_source_removal_survive_reopen() {
    if std::env::var_os("BRUHS_TEST_HUB_PROFILE").is_none() {
        let temp = tempfile::tempdir().unwrap();
        let profile = temp.path();
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "hub_replacement_and_source_removal_survive_reopen",
            ])
            .env("BRUHS_TEST_HUB_PROFILE", "1")
            .env("HOME", profile)
            .env("CFFIXED_USER_HOME", profile)
            .env("XDG_CONFIG_HOME", profile.join("config"))
            .env("XDG_DATA_HOME", profile.join("data"))
            .env("APPDATA", profile.join("config"))
            .env("LOCALAPPDATA", profile.join("data"))
            .env("BRUHS_GENERATORS_DB", profile.join("generators.sqlite"))
            .env("BRUHS_WORKBENCH_DIR", profile.join("workbench"))
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    let source = Source {
        name: "Fixture vault".into(),
        account: "fixture".into(),
    };
    secret_sources::change(Change::SaveSource("test".into(), source.clone())).unwrap();
    let key = || {
        ManagementKey::Reference(SecretReference {
            source: "test".into(),
            reference: "op://Fixtures/Hub/password".into(),
        })
    };
    let url = "https://fixture.example/team";
    let original = hubs::save(url, "", key(), None).unwrap().remove(0);
    assert_eq!(original.label, "fixture.example");
    assert!(hubs::save(url, "stale", key(), None).is_err());
    let replaced = hubs::save(
        &format!("{url}/management.html"),
        "Team hub",
        key(),
        Some(&original),
    )
    .unwrap()
    .remove(0);
    assert!(hubs::remove(&original).is_err());
    hubs::save("https://fixture.example/another", "Other hub", key(), None).unwrap();
    assert_eq!(hubs::list().unwrap().len(), 2);
    assert_eq!(hubs::source_consumers("test").unwrap().len(), 2);
    secret_sources::change(Change::RemoveSource("test".into())).unwrap();
    secret_sources::change(Change::SaveSource("test".into(), source)).unwrap();
    assert!(hubs::source_consumers("test").unwrap().is_empty());
    assert!(hubs::remove(&replaced).is_err());
    let rt = tokio::runtime::Runtime::new().unwrap();
    for hub in hubs::list().unwrap() {
        assert!(rt
            .block_on(hubs::report(&hub))
            .err()
            .unwrap()
            .to_string()
            .contains("management key"));
        hubs::remove(&hub).unwrap();
    }
    assert!(hubs::list().unwrap().is_empty());
}
