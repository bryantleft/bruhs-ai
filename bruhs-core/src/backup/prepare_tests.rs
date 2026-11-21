//! Preparing staged data for this computer, over temporary roots and
//! file-test keychains only (ruling F13). Most tests back up home A, one
//! root as on Linux, and open the backup on home B, two roots as on macOS,
//! each home under its own temp folder with its own keychain. Keys derive
//! at `KdfParams::FLOOR`.
#![cfg(unix)]

use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use bruhs_models::backup::*;
use bruhs_models::crew::BruhStatus;
use bruhs_models::reactive::{
    ReactiveAgent, ReactiveChange, ReactiveCommand, ReactiveRun, RunStatus,
};
use bruhs_models::vault::{
    CredentialScope, LoginInput, SecretField, TotpInput, VaultCreate, VaultSecretInput,
};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::open::{extract, staging_dir};
use super::prepare::{prepare, reactive_io, rewrite_sqlite};
use super::*;
use crate::reactive::{ReactiveError, ReactiveRuntime};
use crate::secrets::{read_seal_key, SecretError, SecretManager, SecretStore, SECRETS_SERVICE};

const PASSWORD: &str = "correct horse 1";
const BRUH: &str = "bruh-1";
const APP_TOKEN: &str = "ghp_restoredtoken1";

/// Home A (one root) and home B (two roots), each with its own keychain.
struct Homes {
    _a: tempfile::TempDir,
    _b: tempfile::TempDir,
    a: Roots,
    b: Roots,
    store_a: SecretStore,
    store_b: SecretStore,
    /// A folder outside every root for backup files.
    out: PathBuf,
}

fn homes() -> Homes {
    let a_dir = tempfile::tempdir().unwrap();
    let b_dir = tempfile::tempdir().unwrap();
    let a_base = std::fs::canonicalize(a_dir.path()).unwrap();
    let b_base = std::fs::canonicalize(b_dir.path()).unwrap();
    let home_a = a_base.join("home-a");
    let core_a = home_a.join("data");
    let a = Roots {
        home: home_a,
        core: core_a.clone(),
        desktop: core_a.clone(),
        config: core_a,
    };
    let home_b = b_base.join("home-b");
    let b = Roots {
        core: home_b.join("core"),
        desktop: home_b.join("desktop"),
        config: home_b.join("core"),
        home: home_b,
    };
    for folder in [&a.core, &b.core, &b.desktop] {
        std::fs::create_dir_all(folder).unwrap();
    }
    let out = a_base.join("out");
    std::fs::create_dir_all(&out).unwrap();
    Homes {
        store_a: SecretStore::file_test(a.core.join("credentials")),
        store_b: SecretStore::file_test(b.core.join("credentials")),
        _a: a_dir,
        _b: b_dir,
        a,
        b,
        out,
    }
}

fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn text(path: &Path) -> String {
    path.to_str().unwrap().to_owned()
}

/// A legacy id, as `secrets::storage_key` makes it.
fn legacy_id(project: &str, name: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(project.as_bytes());
    hash.update([0]);
    hash.update(name.as_bytes());
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn host(roots: &Roots, store: &SecretStore) -> BackupHost {
    BackupHost::new(roots.clone(), store.clone()).for_tests(false)
}

async fn finished(host: &BackupHost, id: &str) -> JobState {
    for _ in 0..3000 {
        let state = host.job(id).unwrap().state;
        if !matches!(state, JobState::Running { .. }) {
            return state;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the job never finished");
}

/// Backs up `roots` to `out/<name>.bruhsbak`.
async fn make_backup(homes: &Homes, roots: &Roots, store: &SecretStore, name: &str) -> PathBuf {
    let host = host(roots, store);
    let path = homes.out.join(format!("{name}.bruhsbak"));
    let job = host
        .start_create(CreateRequest {
            password: Zeroizing::new(PASSWORD.into()),
            options: BackupOptions::default(),
            app: BackupApp::Desktop,
            web_part: None,
            web_db_version: None,
            output: Output::File {
                path: path.clone(),
                overwrite: false,
            },
        })
        .await
        .unwrap();
    match finished(&host, &job.id).await {
        JobState::Created { .. } => path,
        other => panic!("{other:?}"),
    }
}

/// Backs up home A and opens the backup on home B.
async fn a_onto_b(homes: &Homes) -> (String, RestorePreview) {
    let backup = make_backup(homes, &homes.a, &homes.store_a, "a").await;
    let host = host(&homes.b, &homes.store_b);
    let job = host
        .start_open(backup, Zeroizing::new(PASSWORD.into()), false)
        .unwrap();
    match finished(&host, &job.id).await {
        JobState::Opened { preview } => (job.id, *preview),
        other => panic!("{other:?}"),
    }
}

fn json(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// A bruh at home A, its project its own work folder.
fn bruh_agent(roots: &Roots, id: &str) -> ReactiveAgent {
    let work = roots.core.join(format!("server/bruhs/{id}/work"));
    std::fs::create_dir_all(&work).unwrap();
    ReactiveAgent {
        id: id.into(),
        name: format!("Bruh {id}"),
        instructions: "Prepare drafts for review.".into(),
        memory: String::new(),
        model: "claude-sonnet-4-6".into(),
        project_path: text(&work),
        enabled: true,
    }
}

/// `roots`' reactive database with `BRUH` and its queued chat run.
async fn crew(roots: &Roots) -> PathBuf {
    let path = roots.core.join("server/reactive.sqlite3");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let runtime = ReactiveRuntime::open(&path).await.unwrap();
    runtime
        .command(ReactiveCommand::Apply {
            change: ReactiveChange::SaveAgent {
                agent: bruh_agent(roots, BRUH),
            },
        })
        .await
        .unwrap();
    runtime
        .command(ReactiveCommand::Chat {
            agent_id: BRUH.into(),
            message: "Draft the weekly notes".into(),
            run_id: None,
        })
        .await
        .unwrap();
    drop(runtime);
    path
}

/// Every file under `folder`, `meta` aside, by path with its bytes.
fn files_under(folder: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut folders = vec![folder.to_path_buf()];
    while let Some(at) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&at) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let rel = path.strip_prefix(folder).unwrap().to_path_buf();
            if rel == Path::new("meta") {
                continue;
            }
            if entry.file_type().unwrap().is_dir() {
                folders.push(path);
            } else {
                files.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    files
}

/// D12: every project item follows its project to the new home, through
/// the catalogs and the keychain, and reads back through the normal path.
#[tokio::test]
async fn a_restore_on_another_home_rekeys_every_project_item() {
    let homes = homes();
    let project_a = homes.a.home.join("src/app");
    let project_b = homes.b.home.join("src/app");
    std::fs::create_dir_all(&project_a).unwrap();
    let project = text(&project_a);
    let store = &homes.store_a;
    store
        .save(&project, "API_TOKEN", Zeroizing::new("legacy value".into()))
        .await
        .unwrap();
    let database = crate::database::password_name(uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
    store
        .save(&project, &database, Zeroizing::new("db password".into()))
        .await
        .unwrap();
    let manager = SecretManager::new(store.clone());
    let scope = CredentialScope::Project {
        path: project.clone(),
    };
    let key = manager
        .vault_create(VaultCreate {
            name: "VAULT_KEY".into(),
            scope: scope.clone(),
            secret: VaultSecretInput::Value {
                value: Some("vault value".into()),
            },
        })
        .await
        .unwrap();
    let login = manager
        .vault_create(VaultCreate {
            name: "Shop".into(),
            scope,
            secret: VaultSecretInput::Login {
                url: "https://shop.test/in".into(),
                fields: LoginInput::Saved {
                    username: "owner@shop.test".into(),
                    password: Some("login password".into()),
                    totp: TotpInput::Keep,
                },
            },
        })
        .await
        .unwrap();
    let token = manager
        .vault_create(VaultCreate {
            name: "GITHUB_TOKEN".into(),
            scope: CredentialScope::Global,
            secret: VaultSecretInput::AppToken {
                app: "github".into(),
                account: "octocat".into(),
                params: BTreeMap::new(),
                token: Some(APP_TOKEN.into()),
                key: None,
            },
        })
        .await
        .unwrap();
    drop(manager);

    let (id, preview) = a_onto_b(&homes).await;
    assert!(!preview.rewrites.is_empty());
    assert_eq!(preview.counts.vault_entries, 5);
    // The staged catalog's ids are the new home's (map key and id alike).
    let new_project = text(&project_b);
    let catalog = json(&staging_dir(&homes.b, RootKey::Core, &id).join("credentials/catalog.json"));
    let entries = catalog["entries"].as_object().unwrap();
    let names: BTreeSet<&str> = entries
        .values()
        .map(|entry| entry["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        BTreeSet::from(["API_TOKEN", database.as_str(), "VAULT_KEY"])
    );
    for (map_key, entry) in entries {
        let name = entry["name"].as_str().unwrap();
        assert_eq!(map_key, &legacy_id(&new_project, name));
        assert_eq!(entry["id"], *map_key);
        assert_eq!(entry["project_path"], new_project);
    }
    // The map in `meta` names each moved account, the login's included,
    // and no value.
    let moved: BTreeMap<String, String> = serde_json::from_slice(
        &std::fs::read(staging_dir(&homes.b, RootKey::Core, &id).join("meta/accounts.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(moved.len(), 4);
    assert!(moved.contains_key(&format!(
        "vault:{}:{}",
        CredentialScope::Project {
            path: project.clone()
        }
        .key(),
        login.id
    )));

    schedule(&homes.b, &RestoreSource::Staged { id: id.clone() }, false).unwrap();
    assert!(matches!(
        apply_pending(&homes.b, &homes.store_b, None).unwrap(),
        ApplyOutcome::Applied { .. }
    ));
    let manager = SecretManager::new(homes.store_b.clone());
    let listed = manager.vault_entries().await.unwrap();
    assert_eq!(listed.len(), 5);
    for entry in &listed {
        assert_eq!(
            entry.state,
            bruhs_models::secrets::SavedCredentialState::Saved,
            "{}",
            entry.name
        );
        if let CredentialScope::Project { path } = &entry.scope {
            assert_eq!(path, &new_project);
        }
    }
    let id_of = |name: &str| {
        listed
            .iter()
            .find(|entry| entry.name == name)
            .unwrap()
            .id
            .clone()
    };
    for (name, value) in [
        ("API_TOKEN", "legacy value"),
        (database.as_str(), "db password"),
        ("VAULT_KEY", "vault value"),
    ] {
        let id = id_of(name);
        assert_eq!(id, legacy_id(&new_project, name));
        assert_eq!(manager.resolve_value(&id).await.unwrap().as_str(), value);
    }
    // The vault's Project key was a legacy entry under home A's id.
    assert_eq!(key.id, legacy_id(&project, "VAULT_KEY"));
    assert_eq!(
        manager
            .resolve_login(&login.id, SecretField::Password)
            .await
            .unwrap()
            .as_str(),
        "login password"
    );
    assert_eq!(
        manager
            .app_token_for_tests(&token.id)
            .await
            .unwrap()
            .as_str(),
        APP_TOKEN
    );
    // A Global account hashes no path, so the app token's is the same.
    let account = format!("vault:global:{}", token.id);
    let item = homes
        .store_b
        .read_item(SECRETS_SERVICE, &account)
        .unwrap()
        .unwrap();
    assert!(item.contains(APP_TOKEN));
    assert!(!moved.contains_key(&account));
}

/// D10, D11: bruh folders and projects point at this computer's roots and
/// home, the owner's own files stay byte for byte, and a one-root backup's
/// desktop entries land in the desktop root.
#[tokio::test]
async fn bruh_folders_and_projects_point_here() {
    let homes = homes();
    let (home_a, core_a) = (text(&homes.a.home), text(&homes.a.core));
    let (home_b, core_b, desktop_b) = (
        text(&homes.b.home),
        text(&homes.b.core),
        text(&homes.b.desktop),
    );
    crew(&homes.a).await;
    let core = &homes.a.core;
    write(
        &core.join("server/task-workspaces.json"),
        format!(
            "{{\n  \"workspaces\": [{{ \"repoRoot\": \"{home_a}/src/app\", \"worktreePath\": \"{home_a}/src/app-wt\" }}]\n}}\n"
        )
        .as_bytes(),
    );
    write(
        &core.join("desktop-state.json"),
        format!(
            "{{\"projects\":[{{\"projectPath\":\"{home_a}/src/app\"}}],\"attachments\":[\"{core_a}/knowledge-attachments/k1/doc.txt\"]}}"
        )
        .as_bytes(),
    );
    // The desktop's backup copy of its state moves too (M4).
    write(
        &core.join("desktop-state.json.bak"),
        format!("{{\"projects\":[{{\"projectPath\":\"{home_a}/src/old\"}}]}}").as_bytes(),
    );
    // A `.json` file that isn't JSON stays byte for byte (M4).
    let not_json = format!("{home_a}/src/app is not JSON");
    write(&core.join("server/notes.json"), not_json.as_bytes());
    let doc = format!("A note about {home_a}/src/app");
    write(
        &core.join("knowledge-attachments/k1/doc.txt"),
        doc.as_bytes(),
    );
    let notes = format!("{{\"project_path\":\"{home_a}/src/app\"}}");
    write(
        &core.join(format!("server/bruhs/{BRUH}/work/notes.json")),
        notes.as_bytes(),
    );

    let (id, preview) = a_onto_b(&homes).await;
    assert_eq!(preview.counts.bruhs, 1);
    let staged_core = staging_dir(&homes.b, RootKey::Core, &id);
    let staged_desktop = staging_dir(&homes.b, RootKey::Desktop, &id);

    let db = Connection::open(staged_core.join("server/reactive.sqlite3")).unwrap();
    let project: String = db
        .query_row(
            "SELECT json_extract(payload,'$.project_path') FROM reactive_agents WHERE id=?1",
            [BRUH],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(project, format!("{core_b}/server/bruhs/{BRUH}/work"));
    drop(db);

    let workspaces = json(&staged_core.join("server/task-workspaces.json"));
    assert_eq!(
        workspaces["workspaces"][0]["repoRoot"],
        format!("{home_b}/src/app")
    );
    assert_eq!(
        workspaces["workspaces"][0]["worktreePath"],
        format!("{home_b}/src/app-wt")
    );
    // An indented file stays indented.
    assert!(
        std::fs::read_to_string(staged_core.join("server/task-workspaces.json"))
            .unwrap()
            .contains('\n')
    );

    let state = json(&staged_desktop.join("desktop-state.json"));
    assert_eq!(
        state["projects"][0]["projectPath"],
        format!("{home_b}/src/app")
    );
    assert_eq!(
        state["attachments"][0],
        format!("{desktop_b}/knowledge-attachments/k1/doc.txt")
    );
    assert_eq!(
        json(&staged_desktop.join("desktop-state.json.bak"))["projects"][0]["projectPath"],
        format!("{home_b}/src/old")
    );
    assert_eq!(
        std::fs::read(staged_core.join("server/notes.json")).unwrap(),
        not_json.as_bytes()
    );
    assert!(!staged_core.join("desktop-state.json").exists());
    assert!(!staged_core.join("knowledge-attachments").exists());
    assert_eq!(
        std::fs::read(staged_desktop.join("knowledge-attachments/k1/doc.txt")).unwrap(),
        doc.as_bytes()
    );
    // A bruh's work folder holds the owner's files: never rewritten.
    assert_eq!(
        std::fs::read(staged_core.join(format!("server/bruhs/{BRUH}/work/notes.json"))).unwrap(),
        notes.as_bytes()
    );
    // No rewrite leaves a file behind.
    for folder in [&staged_core, &staged_desktop] {
        assert!(files_under(folder)
            .keys()
            .all(|path| !path.to_string_lossy().contains(".bruhs-rewrite-")));
    }
}

/// D10: a path outside both homes is kept as it is and named, by its first
/// two components, from the data and from the catalogs.
#[tokio::test]
async fn outside_paths_are_kept_and_flagged() {
    let homes = homes();
    let state = "{\"projects\":[{\"projectPath\":\"/Volumes/Work/repo\"}]}";
    write(&homes.a.core.join("desktop-state.json"), state.as_bytes());
    homes
        .store_a
        .save(
            "/Volumes/Other/repo",
            "API_KEY",
            Zeroizing::new("outside value".into()),
        )
        .await
        .unwrap();

    let (id, preview) = a_onto_b(&homes).await;
    assert_eq!(
        preview.outside_home,
        vec!["/Volumes/Other".to_owned(), "/Volumes/Work".to_owned()]
    );
    assert_eq!(
        std::fs::read_to_string(
            staging_dir(&homes.b, RootKey::Desktop, &id).join("desktop-state.json")
        )
        .unwrap(),
        state
    );
    let catalog = json(&staging_dir(&homes.b, RootKey::Core, &id).join("credentials/catalog.json"));
    let kept = legacy_id("/Volumes/Other/repo", "API_KEY");
    assert_eq!(
        catalog["entries"][&kept]["project_path"],
        "/Volumes/Other/repo"
    );
}

/// D14, ruling F18: the staged crew is paused, managed presets included,
/// with its runs cancelled and its sessions gone; grants stay.
#[tokio::test]
async fn restored_bruhs_are_paused_with_new_sessions() {
    let homes = homes();
    let live = crew(&homes.a).await;
    let preset = format!("weekly-report-{}", uuid::Uuid::new_v4());
    {
        let db = Connection::open(&live).unwrap();
        let agent = bruh_agent(&homes.a, &preset);
        db.execute(
            "INSERT INTO reactive_agents(id,payload,thread_id,session_id) VALUES(?1,?2,?3,'s-preset')",
            [
                preset.as_str(),
                &serde_json::to_string(&agent).unwrap(),
                &uuid::Uuid::new_v4().to_string(),
            ],
        )
        .unwrap();
        db.execute(
            "UPDATE reactive_agents SET session_id='s-bruh' WHERE id=?1",
            [BRUH],
        )
        .unwrap();
        for agent in [BRUH, preset.as_str()] {
            db.execute(
                "INSERT INTO reactive_session_stamps(agent_id,stamp) VALUES(?1,'stamp')",
                [agent],
            )
            .unwrap();
            db.execute(
                "INSERT INTO reactive_approval_grants(agent_id,fingerprint,created) VALUES(?1,'fp',1)",
                [agent],
            )
            .unwrap();
        }
    }

    let (id, preview) = a_onto_b(&homes).await;
    // The preset isn't counted as a bruh.
    assert_eq!(preview.counts.bruhs, 1);
    let staged = staging_dir(&homes.b, RootKey::Core, &id).join("server/reactive.sqlite3");
    // Looked at in a copy, so the staging holds nothing a runtime adds.
    let scratch = tempfile::tempdir().unwrap();
    let copy = scratch.path().join("reactive.sqlite3");
    std::fs::copy(&staged, &copy).unwrap();
    {
        let db = Connection::open(&copy).unwrap();
        let sessions: i64 = db
            .query_row(
                "SELECT count(*) FROM reactive_agents WHERE session_id IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(sessions, 0);
        let stamps: i64 = db
            .query_row("SELECT count(*) FROM reactive_session_stamps", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(stamps, 0);
        let grants: Vec<String> = db
            .prepare("SELECT agent_id FROM reactive_approval_grants ORDER BY agent_id")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let mut expected = vec![BRUH.to_owned(), preset.clone()];
        expected.sort();
        assert_eq!(grants, expected);
        for agent in [BRUH, preset.as_str()] {
            let paused: bool = db
                .query_row(
                    "SELECT json_extract(payload,'$.paused_at') IS NOT NULL FROM reactive_bruhs WHERE agent_id=?1",
                    [agent],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(paused, "{agent}");
        }
        let runs: Vec<ReactiveRun> = db
            .prepare("SELECT payload FROM reactive_runs")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|payload| serde_json::from_str(&payload.unwrap()).unwrap())
            .collect();
        assert!(!runs.is_empty());
        assert!(runs.iter().all(|run| run.status == RunStatus::Cancelled));
    }
    let runtime = ReactiveRuntime::open(&copy).await.unwrap();
    let snapshot = runtime.snapshot().await.unwrap();
    assert!(!snapshot.bruhs.is_empty());
    assert!(snapshot
        .bruhs
        .iter()
        .all(|bruh| bruh.status == BruhStatus::Paused));
}

/// D10, ruling F17: text cells are rewritten in one transaction with
/// triggers off; virtual tables and their shadow tables are left alone; a
/// database that fails its check is damaged.
#[test]
fn sqlite_rewrites_skip_virtual_tables() {
    let dir = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(dir.path()).unwrap();
    let rules = vec![PathRewrite {
        from: "/a/home".into(),
        to: "/b/home".into(),
    }];
    let path = base.join("notes.sqlite3");
    let db = Connection::open(&path).unwrap();
    db.execute_batch(
        "CREATE TABLE notes(id INTEGER PRIMARY KEY, body TEXT, project_path TEXT);
         INSERT INTO notes(body, project_path) VALUES
           ('{\"cwd\":\"/a/home/x\",\"n\":1}', '/a/home/y'),
           ('/a/home/z', '/Volumes/Work/repo'),
           ('prose about /a/home/q', NULL);
         CREATE VIRTUAL TABLE docs USING fts5(body);
         INSERT INTO docs(body) VALUES ('/a/home/doc');
         CREATE TRIGGER refuse BEFORE UPDATE ON notes BEGIN SELECT RAISE(ABORT, 'refused'); END;",
    )
    .unwrap();
    let virtual_rows = |db: &Connection| -> BTreeMap<String, Vec<String>> {
        let tables: Vec<String> = db
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'docs%'")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        tables
            .into_iter()
            .map(|table| {
                let mut statement = db.prepare(&format!("SELECT * FROM \"{table}\"")).unwrap();
                let width = statement.column_count();
                let rows = statement
                    .query_map([], |row| {
                        Ok((0..width)
                            .map(|at| format!("{:?}", row.get_ref(at).unwrap()))
                            .collect::<Vec<_>>()
                            .join("|"))
                    })
                    .unwrap()
                    .map(Result::unwrap)
                    .collect();
                (table, rows)
            })
            .collect()
    };
    let before = virtual_rows(&db);
    assert!(before.len() > 1, "fts5 makes shadow tables");
    drop(db);

    let mut outside = BTreeSet::new();
    assert_eq!(
        rewrite_sqlite(&path, &rules, &mut outside, &CancelFlag::default()).unwrap(),
        3
    );
    assert_eq!(outside, BTreeSet::from(["/Volumes/Work".to_owned()]));
    let db = Connection::open(&path).unwrap();
    let rows: Vec<(String, Option<String>)> = db
        .prepare("SELECT body, project_path FROM notes ORDER BY id")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        rows,
        vec![
            (
                "{\"cwd\":\"/b/home/x\",\"n\":1}".to_owned(),
                Some("/b/home/y".to_owned())
            ),
            (
                "/b/home/z".to_owned(),
                Some("/Volumes/Work/repo".to_owned())
            ),
            ("prose about /a/home/q".to_owned(), None),
        ]
    );
    assert_eq!(virtual_rows(&db), before);
    // Triggers are off for the rewrite's connection only: the schema keeps
    // them (M4).
    let triggers: i64 = db
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='trigger' AND name='refuse'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(triggers, 1);
    drop(db);

    // One transaction: a later table that can't take its rewrite undoes
    // the earlier one, and the clash is a fit problem, not damage (M6).
    let path = base.join("clash.sqlite3");
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "CREATE TABLE first(p TEXT); INSERT INTO first VALUES ('/a/home/1');
             CREATE TABLE second(p TEXT UNIQUE); INSERT INTO second VALUES ('/a/home/q'), ('/b/home/q');",
        )
        .unwrap();
    let error =
        rewrite_sqlite(&path, &rules, &mut BTreeSet::new(), &CancelFlag::default()).unwrap_err();
    assert_eq!(error.message(), RESTORE_CANT_MAP);
    let first: String = Connection::open(&path)
        .unwrap()
        .query_row("SELECT p FROM first", [], |row| row.get(0))
        .unwrap();
    assert_eq!(first, "/a/home/1");

    // A database that fails its check is never written.
    let path = base.join("broken.sqlite3");
    let mut bytes = b"SQLite format 3\0".to_vec();
    bytes.extend_from_slice(&[0xa5; 4080]);
    std::fs::write(&path, &bytes).unwrap();
    let error =
        rewrite_sqlite(&path, &rules, &mut BTreeSet::new(), &CancelFlag::default()).unwrap_err();
    assert_eq!(error.message(), DAMAGED);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

fn column(path: &Path, sql: &str) -> Vec<Option<String>> {
    Connection::open(path)
        .unwrap()
        .prepare(sql)
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// B Task 9 I1: each cell moves once, by its old text and all the rules,
/// even when one rule's target sits under another rule's source; a
/// column's own collation never widens the match. A table without row
/// ids moves by value too. (A UNIQUE column whose chained moves only fit
/// in a different order answers `RESTORE_CANT_MAP`: SQLite checks each
/// row as it goes.)
#[test]
fn nested_rules_move_each_cell_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = std::fs::canonicalize(dir.path())
        .unwrap()
        .join("nested.sqlite3");
    // Longest first, as `rewrite_rules` sorts them: `/h/b/core` lies
    // under the second rule's `/h/b`.
    let rules = vec![
        PathRewrite {
            from: "/h/a/core".into(),
            to: "/h/b/core".into(),
        },
        PathRewrite {
            from: "/h/b".into(),
            to: "/h/c".into(),
        },
    ];
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "CREATE TABLE t(id INTEGER PRIMARY KEY, p TEXT COLLATE NOCASE);
             INSERT INTO t(p) VALUES ('/h/a/core/x'), ('/h/b/core/x'), ('/H/B/CORE/X'),
               ('{\"cwd\":\"/h/a/core/x\"}'), ('{\"cwd\":\"/h/b/core/x\"}');
             CREATE TABLE w(p TEXT PRIMARY KEY) WITHOUT ROWID;
             INSERT INTO w VALUES ('/h/a/core/w'), ('/h/c/w');",
        )
        .unwrap();
    let changed =
        rewrite_sqlite(&path, &rules, &mut BTreeSet::new(), &CancelFlag::default()).unwrap();
    assert_eq!(changed, 5);
    assert_eq!(
        column(&path, "SELECT p FROM t ORDER BY id"),
        [
            "/h/b/core/x",
            "/h/c/core/x",
            "/H/B/CORE/X",
            "{\"cwd\":\"/h/b/core/x\"}",
            "{\"cwd\":\"/h/c/core/x\"}",
        ]
        .map(|text| Some(text.to_owned()))
    );
    assert_eq!(
        column(&path, "SELECT p FROM w ORDER BY p"),
        ["/h/b/core/w", "/h/c/w"].map(|text| Some(text.to_owned()))
    );
}

/// B Task 9 N1: staged tables named like the map, its old alias or its
/// columns are rewritten as any other, and none is taken for the map.
#[test]
fn staged_names_never_meet_the_map() {
    let dir = tempfile::tempdir().unwrap();
    let path = std::fs::canonicalize(dir.path())
        .unwrap()
        .join("names.sqlite3");
    let db = Connection::open(&path).unwrap();
    db.execute_batch(
        "CREATE TABLE bruhs_rewrite_map(old TEXT, new TEXT);
         INSERT INTO bruhs_rewrite_map VALUES ('/a/home/1', '/a/home/2');
         CREATE TABLE m(old TEXT, new TEXT);
         INSERT INTO m VALUES ('/a/home/3', 'kept');
         CREATE TABLE \"we\"\"ird\"(p TEXT);
         INSERT INTO \"we\"\"ird\" VALUES ('/a/home/4');",
    )
    .unwrap();
    // No staged table can take the alias.
    assert!(db
        .execute("CREATE TABLE sqlite_bruhs_map(p TEXT)", [])
        .is_err());
    drop(db);
    let rules = vec![PathRewrite {
        from: "/a/home".into(),
        to: "/b/home".into(),
    }];
    let changed =
        rewrite_sqlite(&path, &rules, &mut BTreeSet::new(), &CancelFlag::default()).unwrap();
    assert_eq!(changed, 4);
    let rows = |sql: &str| -> Vec<(String, String)> {
        Connection::open(&path)
            .unwrap()
            .prepare(sql)
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(
        rows("SELECT old, new FROM bruhs_rewrite_map"),
        [("/b/home/1".to_owned(), "/b/home/2".to_owned())]
    );
    assert_eq!(
        rows("SELECT old, new FROM m"),
        [("/b/home/3".to_owned(), "kept".to_owned())]
    );
    assert_eq!(
        column(&path, "SELECT p FROM \"we\"\"ird\""),
        [Some("/b/home/4".to_owned())]
    );
}

/// B Task 9 I1: a large column is one pass, not one scan per value.
#[test]
fn a_large_table_rewrites_in_one_pass() {
    let dir = tempfile::tempdir().unwrap();
    let path = std::fs::canonicalize(dir.path())
        .unwrap()
        .join("large.sqlite3");
    let mut db = Connection::open(&path).unwrap();
    db.execute("CREATE TABLE runs(payload TEXT)", []).unwrap();
    let transaction = db.transaction().unwrap();
    for at in 0..20_000 {
        transaction
            .execute(
                "INSERT INTO runs VALUES (?1)",
                [format!("{{\"cwd\":\"/a/home/p{at}\"}}")],
            )
            .unwrap();
    }
    transaction.commit().unwrap();
    drop(db);
    let rules = vec![PathRewrite {
        from: "/a/home".into(),
        to: "/b/home".into(),
    }];
    let started = std::time::Instant::now();
    let changed =
        rewrite_sqlite(&path, &rules, &mut BTreeSet::new(), &CancelFlag::default()).unwrap();
    assert_eq!(changed, 20_000);
    // Loose: one scan per value took minutes at this size.
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "{:?}",
        started.elapsed()
    );
    let moved: i64 = Connection::open(&path)
        .unwrap()
        .query_row(
            "SELECT count(*) FROM runs WHERE payload LIKE '{\"cwd\":\"/b/home/p%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(moved, 20_000);
}

/// A cancelled job stops before the next table, and nothing is written.
#[test]
fn a_cancelled_rewrite_stops_between_tables() {
    let dir = tempfile::tempdir().unwrap();
    let path = std::fs::canonicalize(dir.path())
        .unwrap()
        .join("cancel.sqlite3");
    Connection::open(&path)
        .unwrap()
        .execute_batch("CREATE TABLE t(p TEXT); INSERT INTO t VALUES ('/a/home/x');")
        .unwrap();
    let cancel = CancelFlag::default();
    cancel.cancel();
    let rules = vec![PathRewrite {
        from: "/a/home".into(),
        to: "/b/home".into(),
    }];
    let error = rewrite_sqlite(&path, &rules, &mut BTreeSet::new(), &cancel).unwrap_err();
    assert_eq!(error.message(), BACKUP_STOPPED);
    assert_eq!(
        column(&path, "SELECT p FROM t"),
        [Some("/a/home/x".to_owned())]
    );
}

/// B Task 9 M3, M6: a backup whose paths clash once moved fails its open
/// with `RESTORE_CANT_MAP`, and leaves no staging and no seal key.
#[tokio::test]
async fn a_failed_prepare_removes_the_staging_and_its_key() {
    let homes = homes();
    let (home_a, home_b) = (text(&homes.a.home), text(&homes.b.home));
    let db = Connection::open(homes.a.core.join("clash.sqlite3")).unwrap();
    db.execute("CREATE TABLE seen(p TEXT UNIQUE)", []).unwrap();
    for path in [format!("{home_a}/src/x"), format!("{home_b}/src/x")] {
        db.execute("INSERT INTO seen VALUES (?1)", [path]).unwrap();
    }
    drop(db);
    let backup = make_backup(&homes, &homes.a, &homes.store_a, "clash").await;
    let host = host(&homes.b, &homes.store_b);
    let job = host
        .start_open(backup, Zeroizing::new(PASSWORD.into()), false)
        .unwrap();
    match finished(&host, &job.id).await {
        JobState::Failed { message } => assert_eq!(message, RESTORE_CANT_MAP),
        other => panic!("{other:?}"),
    }
    for root in [RootKey::Core, RootKey::Desktop] {
        let staging = homes.b.backups(root).join("staging");
        let left: Vec<_> = std::fs::read_dir(&staging)
            .map(|entries| entries.flatten().map(|entry| entry.file_name()).collect())
            .unwrap_or_default();
        assert!(left.is_empty(), "{left:?}");
    }
    assert!(matches!(
        read_seal_key(&homes.store_b, &format!("staged:{}", job.id)),
        Err(SecretError::NotSaved)
    ));
}

/// B Task 9 M1: a catalog that can't be written answers the io copy, and
/// a store error is io only for SQLite's full disk and io codes.
#[tokio::test]
async fn io_failures_answer_the_io_copy() {
    let homes = homes();
    let project = format!("{}/src/app", text(&homes.a.home));
    homes
        .store_a
        .save(&project, "API_KEY", Zeroizing::new("moving value".into()))
        .await
        .unwrap();
    let backup = make_backup(&homes, &homes.a, &homes.store_a, "io").await;
    let id = host(&homes.b, &homes.store_b).random_id();
    let mut staged = extract(
        &homes.b,
        &id,
        &backup,
        PASSWORD,
        &CancelFlag::default(),
        &|_, _| {},
    )
    .unwrap();
    let credentials = staging_dir(&homes.b, RootKey::Core, &id).join("credentials");
    std::fs::set_permissions(&credentials, std::fs::Permissions::from_mode(0o500)).unwrap();
    let error = prepare(&homes.b, &mut staged, &CancelFlag::default())
        .await
        .err()
        .unwrap();
    std::fs::set_permissions(&credentials, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(matches!(error, BackupError::Io(_)), "{error:?}");

    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE t(v TEXT UNIQUE); INSERT INTO t VALUES ('x');")
        .unwrap();
    let clash = db.execute("INSERT INTO t VALUES ('x')", []).unwrap_err();
    assert!(!reactive_io(&ReactiveError::from(clash)));
    let full =
        rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_FULL), None);
    assert!(reactive_io(&ReactiveError::from(full)));
}

/// Final review B-I2: the staged catalogs are read on the same computer
/// too, where nothing is re-keyed, so catalogs an apply couldn't read fail
/// the open (`DAMAGED`) instead of the apply.
#[tokio::test]
async fn staged_catalogs_are_read_on_the_same_home() {
    let homes = homes();
    let roots = &homes.b;
    let backup = make_backup(&homes, roots, &homes.store_b, "catalogs").await;
    let id = host(roots, &homes.store_b).random_id();
    let mut staged = extract(
        roots,
        &id,
        &backup,
        PASSWORD,
        &CancelFlag::default(),
        &|_, _| {},
    )
    .unwrap();
    let core = staging_dir(roots, RootKey::Core, &id);
    let _ = std::fs::remove_dir_all(core.join("connections"));
    std::fs::write(core.join("connections"), b"not a folder").unwrap();
    match prepare(roots, &mut staged, &CancelFlag::default()).await {
        Err(error) => assert_eq!(error.message(), DAMAGED),
        Ok(_) => panic!("catalogs that don't read were prepared"),
    }
}

/// The same computer gets no rules, and preparing changes no staged byte
/// but the reactive database's sessions and pause.
#[tokio::test]
async fn the_same_home_rewrites_nothing() {
    let homes = homes();
    let roots = &homes.b;
    let live = crew(roots).await;
    {
        let db = Connection::open(&live).unwrap();
        db.execute("UPDATE reactive_agents SET session_id='s-bruh'", [])
            .unwrap();
    }
    let home = text(&roots.home);
    write(
        &roots.core.join("server/task-workspaces.json"),
        format!("{{\"repoRoot\":\"{home}/src/app\"}}").as_bytes(),
    );
    write(
        &roots.desktop.join("desktop-state.json"),
        format!("{{\"projectPath\":\"{home}/src/app\"}}").as_bytes(),
    );
    let mail = Connection::open(roots.core.join("server/mail.sqlite3")).unwrap();
    mail.execute("CREATE TABLE notes(project_path TEXT)", [])
        .unwrap();
    mail.execute("INSERT INTO notes VALUES (?1)", [format!("{home}/src/app")])
        .unwrap();
    drop(mail);
    homes
        .store_b
        .save(
            &format!("{home}/src/app"),
            "API_KEY",
            Zeroizing::new("same value".into()),
        )
        .await
        .unwrap();

    let backup = make_backup(&homes, roots, &homes.store_b, "same").await;
    let id = host(roots, &homes.store_b).random_id();
    let mut staged = extract(
        roots,
        &id,
        &backup,
        PASSWORD,
        &CancelFlag::default(),
        &|_, _| {},
    )
    .unwrap();
    let core = staging_dir(roots, RootKey::Core, &id);
    let desktop = staging_dir(roots, RootKey::Desktop, &id);
    let before = [files_under(&core), files_under(&desktop)];

    let prepared = prepare(roots, &mut staged, &CancelFlag::default())
        .await
        .unwrap();
    assert_eq!(prepared.rewrites, Vec::<PathRewrite>::new());
    assert_eq!(prepared.outside_home, Vec::<String>::new());
    assert_eq!(prepared.bruhs, 1);
    assert_eq!(prepared.vault_entries, 1);
    let reactive = PathBuf::from("server/reactive.sqlite3");
    let after = [files_under(&core), files_under(&desktop)];
    for (before, after) in before.iter().zip(&after) {
        let unchanged = |files: &BTreeMap<PathBuf, Vec<u8>>| {
            files
                .iter()
                .filter(|(path, _)| **path != reactive)
                .map(|(path, bytes)| (path.clone(), bytes.clone()))
                .collect::<BTreeMap<_, _>>()
        };
        assert_eq!(unchanged(before), unchanged(after));
    }
    assert!(
        before[0][&reactive] != after[0][&reactive],
        "the pause changed the reactive database"
    );
    let db = Connection::open(core.join(&reactive)).unwrap();
    let (sessions, paused): (i64, bool) = db
        .query_row(
            "SELECT (SELECT count(*) FROM reactive_agents WHERE session_id IS NOT NULL),
                    (SELECT json_extract(payload,'$.paused_at') IS NOT NULL FROM reactive_bruhs WHERE agent_id=?1)",
            [BRUH],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((sessions, paused), (0, true));
    // The account map is stored even when nothing moved.
    assert_eq!(
        std::fs::read(core.join("meta/accounts.json")).unwrap(),
        b"{}"
    );
}
