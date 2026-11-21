//! OpenClaw and Hermes Agent personas (Task 8). Every home is a temp
//! folder; only the ignored `live_clawd_names_only` looks at a real one,
//! and only at names.

use super::{openclaw, scan, AgentRoots};
use crate::import::ImportError;
use bruhs_models::imports::{
    AgentScan, FileUse, PersonaCandidate, SkipReason, FOLDER_MISSING, FOLDER_PRIVATE,
    MAX_DAILY_LOGS, MAX_PERSONAS, MAX_PERSONA_FILE_BYTES,
};
use bruhs_models::logic::portable::ImportSource;
use bruhs_models::reactive::MAX_TEXT_CHARS;
use std::path::{Path, PathBuf};

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn canary() -> String {
    format!("canary-{:032x}", rand::random::<u128>())
}

/// A home in a temp folder, and the roots under it with no overrides.
fn home() -> (tempfile::TempDir, AgentRoots) {
    let dir = tempfile::tempdir().unwrap();
    let roots = AgentRoots::under(dir.path());
    (dir, roots)
}

fn scan_of(roots: &AgentRoots, source: ImportSource, picked: Option<&Path>) -> AgentScan {
    scan(roots, source, picked).unwrap()
}

fn folders(scan: &AgentScan) -> Vec<PathBuf> {
    scan.personas
        .iter()
        .map(|persona| PathBuf::from(&persona.folder))
        .collect()
}

/// `path` with its links resolved, as a scan answers folders.
fn canon(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap()
}

fn real<P: AsRef<Path>>(paths: impl IntoIterator<Item = P>) -> Vec<PathBuf> {
    paths.into_iter().map(|path| canon(path.as_ref())).collect()
}

fn use_of(persona: &PersonaCandidate, name: &str) -> Option<FileUse> {
    persona
        .files
        .iter()
        .find(|file| file.name == name)
        .map(|file| file.use_)
}

const NOT_IMPORTED: FileUse = FileUse::Skipped {
    reason: SkipReason::NotImported,
};

fn skipped(reason: SkipReason) -> FileUse {
    FileUse::Skipped { reason }
}

/// A clawd-era workspace (R§4) with invented text.
fn clawd(dir: &Path) {
    write(
        &dir.join("IDENTITY.md"),
        "# IDENTITY.md\n\n- **Name:** Pinch\n- **Creature:** a small lobster\n\
         - **Vibe:** _(how do you come across?)_\n- **Emoji:** 🦞\n\
         - **Avatar:** https://example.com/pinch.png\n",
    );
    write(&dir.join("SOUL.md"), "Soul text: be curious.");
    write(&dir.join("AGENTS.md"), "Agents text: check the inbox.");
    write(
        &dir.join("TOOLS.md"),
        "Tools text: the printer is upstairs.",
    );
    write(&dir.join("USER.md"), "User text: prefers tea.");
    write(&dir.join("HEARTBEAT.md"), "Heartbeat text.");
    write(&dir.join("canvas/index.html"), "<p>canvas</p>");
    write(
        &dir.join("memory/2026-01-26.md"),
        "Log 26: shipped the thing.",
    );
    write(
        &dir.join("memory/2026-01-25.md"),
        "Log 25: planned the thing.",
    );
    write(&dir.join("memory/discord-mention-state.json"), "{}");
    write(&dir.join("memory/\u{202e}gnp.md"), "Disguised.");
}

#[test]
fn an_openclaw_workspace_becomes_a_persona() {
    let (dir, roots) = home();
    let workspace = dir.path().join("clawd");
    clawd(&workspace);

    let scan = scan_of(&roots, ImportSource::OpenClaw, None);
    assert_eq!(scan.personas.len(), 1);
    let persona = &scan.personas[0];
    assert_eq!(persona.source, ImportSource::OpenClaw);
    assert_eq!(persona.name, "Pinch");
    assert!(persona.id.starts_with("p-") && persona.id.len() == 18);
    assert_eq!(PathBuf::from(&persona.folder), canon(&workspace));

    let at = |text: &str| persona.instructions.find(text).unwrap();
    assert!(at("Soul text") < at("Agents text") && at("Agents text") < at("Tools text"));
    assert!(persona
        .instructions
        .starts_with("Identity from IDENTITY.md\nName: Pinch"));
    assert!(persona.instructions.contains("Creature: a small lobster"));
    assert!(!persona.instructions.contains("Vibe"));

    let at = |text: &str| persona.memory.find(text).unwrap();
    assert!(at("## OpenClaw: USER.md") < at("## OpenClaw: memory/2026-01-26.md"));
    assert!(at("Log 26") < at("Log 25") && at("User text") < at("Log 26"));
    assert!(!persona.memory.contains("Heartbeat") && !persona.instructions.contains("Heartbeat"));

    for name in [
        "HEARTBEAT.md",
        "canvas/",
        "memory/discord-mention-state.json",
    ] {
        assert_eq!(use_of(persona, name), Some(NOT_IMPORTED), "{name}");
    }
    assert_eq!(use_of(persona, "IDENTITY.md"), Some(FileUse::Identity));
    assert_eq!(use_of(persona, "TOOLS.md"), Some(FileUse::Instructions));
    assert_eq!(
        use_of(persona, "memory/2026-01-25.md"),
        Some(FileUse::Memory)
    );
    assert_eq!(persona.dropped_identity, vec!["Emoji", "Avatar"]);
    assert!(!persona.clipped);

    // The inventory names the same files without opening one.
    let names: Vec<String> = openclaw::inventory(&workspace)
        .into_iter()
        .map(|file| file.name)
        .collect();
    assert_eq!(
        names,
        [
            "IDENTITY.md",
            "SOUL.md",
            "AGENTS.md",
            "TOOLS.md",
            "USER.md",
            "memory/2026-01-26.md",
            "memory/2026-01-25.md",
            "HEARTBEAT.md",
            "canvas/",
            "memory/discord-mention-state.json",
            "memory/gnp.md",
        ]
    );
    assert_eq!(
        openclaw::identity_labels(&workspace),
        ["Name", "Creature", "Vibe", "Emoji", "Avatar"]
    );
}

#[test]
fn the_default_paths_are_searched_in_order() {
    let (dir, roots) = home();
    let home = dir.path();
    let canary = canary();
    let order = [
        home.join(".openclaw/workspace"),
        home.join(".openclaw/workspace-ops"),
        home.join(".openclaw-work/workspace"),
        home.join("clawd"),
    ];
    for (n, workspace) in order.iter().enumerate() {
        write(&workspace.join("SOUL.md"), &format!("Soul {n}."));
    }
    write(&home.join(".openclaw/openclaw.json"), &canary);
    write(&order[0].join("openclaw.json"), &canary);

    let scan = scan_of(&roots, ImportSource::OpenClaw, None);
    assert_eq!(folders(&scan), real(&order));
    assert!(!scan.looked_in.is_empty());
    for place in &scan.looked_in {
        assert!(place.starts_with("~/"), "{place}");
    }
    assert!(scan.looked_in.contains(&"~/clawd".to_string()));
    assert!(!serde_json::to_string(&scan).unwrap().contains(&canary));

    // The workspace override comes first; one that is also a default
    // place is answered once.
    let elsewhere = home.join("elsewhere/agent");
    write(&elsewhere.join("AGENTS.md"), "Elsewhere.");
    let first = AgentRoots {
        openclaw_workspace: Some(elsewhere.clone()),
        ..roots.clone()
    };
    let scan = scan_of(&first, ImportSource::OpenClaw, None);
    let mut expected = vec![elsewhere];
    expected.extend(order.iter().cloned());
    assert_eq!(folders(&scan), real(&expected));
    assert!(scan.looked_in[0].starts_with("~/elsewhere"));
    let again = AgentRoots {
        openclaw_workspace: Some(order[2].clone()),
        ..roots
    };
    let scan = scan_of(&again, ImportSource::OpenClaw, None);
    assert_eq!(
        folders(&scan),
        real([&order[2], &order[0], &order[1], &order[3]])
    );
}

#[test]
fn a_hermes_home_and_its_profiles_become_personas() {
    let (dir, roots) = home();
    let hermes = dir.path().join(".hermes");
    write(&hermes.join("SOUL.md"), "Hermes soul.");
    write(
        &hermes.join("memories/USER.md"),
        "Likes tea\n§\nLives in Lisbon\nOn weekdays\n§\nLikes tea\n",
    );
    write(
        &hermes.join("memories/MEMORY.md"),
        "Ships on Fridays\n§\nLives in Lisbon\nOn weekdays\n",
    );
    write(&hermes.join("skills/notes/SKILL.md"), "Skill.");
    write(&hermes.join("profiles/coach/SOUL.md"), "Coach soul.");

    let scan = scan_of(&roots, ImportSource::Hermes, None);
    assert_eq!(scan.personas.len(), 2);
    let (main, coach) = (&scan.personas[0], &scan.personas[1]);
    assert_eq!(main.name, "Hermes Agent");
    assert_eq!(coach.name, "coach");
    assert_eq!(
        (main.source, coach.source),
        (ImportSource::Hermes, ImportSource::Hermes)
    );
    assert_ne!(main.id, coach.id);
    assert_eq!(main.instructions, "## SOUL.md\n\nHermes soul.");
    assert_eq!(coach.instructions, "## SOUL.md\n\nCoach soul.");
    assert_eq!(
        main.memory,
        "## Hermes Agent: memories/USER.md\n\nLikes tea\n\nLives in Lisbon\nOn weekdays\n\n\
         ## Hermes Agent: memories/MEMORY.md\n\nShips on Fridays"
    );
    assert_eq!(use_of(main, "memories/USER.md"), Some(FileUse::Memory));
    assert_eq!(use_of(main, "skills/"), Some(NOT_IMPORTED));
    assert_eq!(use_of(main, "SOUL.md"), Some(FileUse::Instructions));
    assert!(coach.memory.is_empty());
    assert_eq!(
        scan.looked_in,
        ["~/.hermes", "~/.hermes/profiles/*"].map(String::from)
    );

    // HERMES_HOME moves the home.
    let moved = dir.path().join("elsewhere");
    write(&moved.join("SOUL.md"), "Moved soul.");
    let other = AgentRoots {
        hermes_home: Some(moved.clone()),
        ..roots
    };
    let scan = scan_of(&other, ImportSource::Hermes, None);
    assert_eq!(folders(&scan), real([moved]));
}

#[cfg(unix)]
#[test]
fn secrets_and_links_are_never_read() {
    use super::files::{read_text, FileRead, NEVER_OPEN};
    use std::os::unix::fs::symlink;
    let (dir, roots) = home();
    let home = dir.path();
    let canaries: Vec<String> = (0..12).map(|_| canary()).collect();
    let workspace = home.join(".openclaw/workspace");
    let hermes = home.join(".hermes");
    let secrets = [
        ".env",
        ".env.local",
        "auth.json",
        "auth-profiles.json",
        "credentials/token",
        "config.yaml",
        "openclaw.json",
        "state.db",
        "agent.sqlite",
    ];
    for (name, canary) in secrets.iter().zip(&canaries) {
        write(&workspace.join(name), canary);
        write(&hermes.join(name), canary);
    }
    // Outside both: a canary file and a canary folder.
    let outside = home.join("outside");
    write(&outside.join("soul.md"), &canaries[9]);
    write(&outside.join("logs/2026-01-01.md"), &canaries[10]);
    write(&outside.join("logs/USER.md"), &canaries[11]);
    write(&outside.join("logs/MEMORY.md"), &canaries[11]);
    symlink(outside.join("soul.md"), workspace.join("SOUL.md")).unwrap();
    symlink(outside.join("logs"), workspace.join("memory")).unwrap();
    let fifo = workspace.join("USER.md");
    assert!(std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap()
        .success());
    let big = usize::try_from(MAX_PERSONA_FILE_BYTES).unwrap() + 44 * 1024;
    write(&workspace.join("MEMORY.md"), &"m".repeat(big));
    write(&workspace.join("AGENTS.md"), "Agents text.");
    write(&hermes.join("SOUL.md"), "Hermes soul.");
    symlink(outside.join("logs"), hermes.join("memories")).unwrap();

    // A FIFO never blocks the scan: a hang guard, not a time bound.
    let (sent, got) = std::sync::mpsc::channel();
    let guarded = roots;
    std::thread::spawn(move || {
        let scans = (
            scan(&guarded, ImportSource::OpenClaw, None).unwrap(),
            scan(&guarded, ImportSource::Hermes, None).unwrap(),
        );
        sent.send(scans).unwrap();
    });
    let (claw, herm) = got
        .recv_timeout(std::time::Duration::from_secs(60))
        .expect("the scan never waits on a FIFO");

    let persona = &claw.personas[0];
    assert_eq!(PathBuf::from(&persona.folder), canon(&workspace));
    assert_eq!(use_of(persona, "SOUL.md"), Some(skipped(SkipReason::Link)));
    assert_eq!(use_of(persona, "memory/"), Some(skipped(SkipReason::Link)));
    assert_eq!(
        use_of(persona, "USER.md"),
        Some(skipped(SkipReason::NotAFile))
    );
    assert_eq!(
        use_of(persona, "MEMORY.md"),
        Some(skipped(SkipReason::TooLarge))
    );
    assert_eq!(persona.instructions, "## AGENTS.md\n\nAgents text.");
    assert!(persona.memory.is_empty());
    let persona = &herm.personas[0];
    assert_eq!(
        use_of(persona, "memories/"),
        Some(skipped(SkipReason::Link))
    );
    assert!(persona.memory.is_empty());
    for scan in [&claw, &herm] {
        let json = serde_json::to_string(scan).unwrap();
        for canary in &canaries {
            assert!(!json.contains(canary.as_str()));
        }
        for name in secrets {
            assert!(!json.contains(&format!("\"{name}\"")), "{name}");
        }
    }

    // The handle checks answer the same when a file is read by name.
    assert!(matches!(
        read_text(&workspace, "SOUL.md"),
        FileRead::Skipped(SkipReason::Link)
    ));
    assert!(matches!(
        read_text(&workspace, "USER.md"),
        FileRead::Skipped(SkipReason::NotAFile)
    ));
    assert!(matches!(
        read_text(&workspace, "MEMORY.md"),
        FileRead::Skipped(SkipReason::TooLarge)
    ));
    // A linked folder is never gone through (F22).
    for (dir, name) in [
        (&workspace, "memory/2026-01-01.md"),
        (&hermes, "memories/USER.md"),
        (&hermes, "memories/MEMORY.md"),
    ] {
        assert!(
            matches!(read_text(dir, name), FileRead::Skipped(SkipReason::Link)),
            "{name}"
        );
    }
    assert!(matches!(
        read_text(&workspace, "AGENTS.md"),
        FileRead::Text(_)
    ));
    // Never-opened names answer as missing, whatever is there.
    for name in NEVER_OPEN.iter().copied().chain([
        ".env.local",
        "agent.sqlite",
        "credentials/token",
        "AUTH.JSON",
    ]) {
        assert!(
            matches!(read_text(&workspace, name), FileRead::Missing),
            "{name}"
        );
        assert!(
            matches!(read_text(&hermes, name), FileRead::Missing),
            "{name}"
        );
    }
    // Even through a FIFO that would block an open without O_NONBLOCK.
    let held = workspace.join("credentials/held");
    assert!(std::process::Command::new("mkfifo")
        .arg(&held)
        .status()
        .unwrap()
        .success());
    assert!(matches!(
        read_text(&workspace, "credentials/held"),
        FileRead::Missing
    ));
}

#[test]
fn a_picked_folder_is_tried_as_workspace_then_state_dir() {
    let (dir, roots) = home();
    let home = dir.path();
    let state = home.join(".openclaw");
    write(&state.join("workspace/SOUL.md"), "Main.");
    write(&state.join("workspace-ops/SOUL.md"), "Ops.");
    write(&state.join("openclaw.json"), &canary());

    let found = scan_of(&roots, ImportSource::OpenClaw, Some(&state));
    assert_eq!(
        folders(&found),
        real([state.join("workspace"), state.join("workspace-ops")])
    );
    assert_eq!(found.looked_in, ["~/.openclaw"]);

    let workspace = state.join("workspace-ops");
    let found = scan_of(&roots, ImportSource::OpenClaw, Some(&workspace));
    assert_eq!(folders(&found), real([workspace]));

    // `~/` is read against the home.
    let found = scan_of(
        &roots,
        ImportSource::OpenClaw,
        Some(Path::new("~/.openclaw")),
    );
    assert_eq!(found.personas.len(), 2);

    let empty = home.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    for source in [ImportSource::OpenClaw, ImportSource::Hermes] {
        let found = scan_of(&roots, source, Some(&empty));
        assert!(found.personas.is_empty());
        assert_eq!(found.looked_in, ["~/empty"]);
    }

    for missing in [
        home.join("missing"),
        PathBuf::from("relative/folder"),
        state.join("workspace/SOUL.md"),
    ] {
        assert!(
            matches!(
                scan(&roots, ImportSource::OpenClaw, Some(&missing)),
                Err(ImportError::Refused(copy)) if copy == FOLDER_MISSING
            ),
            "{}",
            missing.display()
        );
    }
    // A Hermes home picked by its folder.
    let hermes = home.join("h");
    write(&hermes.join("SOUL.md"), "Picked.");
    write(&hermes.join("profiles/writer/SOUL.md"), "Writer.");
    let found = scan_of(&roots, ImportSource::Hermes, Some(&hermes));
    assert_eq!(
        folders(&found),
        real([hermes.clone(), hermes.join("profiles/writer")])
    );
    // Nothing under a folder named like a secret is opened.
    let hidden = state.join("credentials");
    write(&hidden.join("SOUL.md"), &canary());
    write(&hidden.join("workspace/SOUL.md"), &canary());
    for source in [ImportSource::OpenClaw, ImportSource::Hermes] {
        assert!(refused(&roots, source, Some(&hidden), FOLDER_PRIVATE));
    }
    // Other sources aren't agents.
    assert!(scan_is_refused(&roots, ImportSource::ChatGpt));
    assert!(scan_is_refused(&roots, ImportSource::ClaudeCode));
}

fn refused(roots: &AgentRoots, source: ImportSource, picked: Option<&Path>, copy: &str) -> bool {
    matches!(scan(roots, source, picked), Err(ImportError::Refused(got)) if got == copy)
}

fn scan_is_refused(roots: &AgentRoots, source: ImportSource) -> bool {
    matches!(scan(roots, source, None), Err(ImportError::Refused(_)))
}

#[test]
fn long_personas_are_cut_to_fit() {
    let (dir, roots) = home();
    let workspace = dir.path().join("clawd");
    let soul = "s".repeat(20_000);
    assert!(u64::try_from(soul.len()).unwrap() < MAX_PERSONA_FILE_BYTES);
    write(&workspace.join("SOUL.md"), &soul);
    write(&workspace.join("AGENTS.md"), "Agents text.");
    let scan = scan_of(&roots, ImportSource::OpenClaw, None);
    let persona = &scan.personas[0];
    assert!(persona.clipped);
    assert!(persona.instructions.chars().count() <= MAX_TEXT_CHARS);
    assert!(!persona.instructions.contains("Agents text."));
}

#[test]
fn many_daily_logs_stop_once_the_memory_is_full() {
    let (dir, roots) = home();
    let workspace = dir.path().join("clawd");
    write(&workspace.join("SOUL.md"), "Soul.");
    for day in 1..=28 {
        write(
            &workspace.join(format!("memory/2026-02-{day:02}.md")),
            &format!("day {day} {}", "x".repeat(2_000)),
        );
    }
    let scan = scan_of(&roots, ImportSource::OpenClaw, None);
    let persona = &scan.personas[0];
    assert!(persona.clipped);
    assert!(persona.memory.chars().count() <= MAX_TEXT_CHARS);
    assert!(persona.memory.contains("day 28 ") && !persona.memory.contains("day 1 "));
    // Logs past the budget aren't opened, so they aren't named as memory.
    assert_eq!(use_of(persona, "memory/2026-02-01.md"), None);
    assert_eq!(
        use_of(persona, "memory/2026-02-28.md"),
        Some(FileUse::Memory)
    );
    let read = persona
        .files
        .iter()
        .filter(|f| f.name.starts_with("memory/"))
        .count();
    assert_eq!(usize::try_from(persona.left_out_logs).unwrap() + read, 28);
}

/// Names only: run with `BRUHS_IMPORT_LIVE_CLAWD=1 cargo test -p
/// bruhs-core -- --ignored live_clawd_names_only`. It lists file names
/// and the `IDENTITY.md` labels it knows, and never reads a value.
#[test]
#[ignore = "reads the names in the owner's ~/clawd"]
fn live_clawd_names_only() {
    if std::env::var_os("BRUHS_IMPORT_LIVE_CLAWD").is_none_or(|value| value != "1") {
        return;
    }
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let clawd = PathBuf::from(home).join("clawd");
    let inventory = openclaw::inventory(&clawd);
    for file in &inventory {
        eprintln!("{} {:?}", file.name, file.use_);
    }
    let labels = openclaw::identity_labels(&clawd);
    eprintln!("labels: {labels:?}");
    assert_eq!(labels, ["Name", "Creature", "Vibe", "Emoji", "Avatar"]);
    for name in ["SOUL.md", "IDENTITY.md", "USER.md", "memory/2026-01-26.md"] {
        assert!(inventory.iter().any(|file| file.name == name), "{name}");
    }
}

#[test]
fn the_import_host_scans_only_the_agent_roots_it_was_given() {
    let (dir, roots) = home();
    write(&dir.path().join("clawd/SOUL.md"), "Soul.");
    let imports = dir.path().join("core/imports");
    let host = crate::import::ImportHost::new(imports.clone());
    let none = host.scan(ImportSource::OpenClaw, None).unwrap();
    assert!(none.personas.is_empty());
    let host = host.with_agent_roots(roots);
    let found = host.scan(ImportSource::OpenClaw, None).unwrap();
    assert_eq!(folders(&found), real([dir.path().join("clawd")]));
    assert!(!imports.exists(), "a scan writes nothing");
}

#[cfg(unix)]
#[test]
fn named_places_are_resolved_and_listed_ones_must_be_real() {
    use std::os::unix::fs::symlink;
    let (dir, roots) = home();
    let home = dir.path();
    // A linked ~/clawd is the owner's own layout: read at its target.
    let real_clawd = home.join("projects/clawd");
    write(&real_clawd.join("SOUL.md"), "Clawd.");
    symlink(&real_clawd, home.join("clawd")).unwrap();
    // A linked ~/.openclaw/workspace too.
    let real_main = home.join("projects/main");
    write(&real_main.join("SOUL.md"), "Main.");
    std::fs::create_dir_all(home.join(".openclaw")).unwrap();
    symlink(&real_main, home.join(".openclaw/workspace")).unwrap();
    // A linked workspace-ops was found by listing: never read.
    let real_ops = home.join("projects/ops");
    write(&real_ops.join("SOUL.md"), "Ops.");
    symlink(&real_ops, home.join(".openclaw/workspace-ops")).unwrap();
    // A link to nothing is missing.
    symlink(home.join("gone"), home.join("openclaw")).unwrap();

    let found = scan_of(&roots, ImportSource::OpenClaw, None);
    assert_eq!(folders(&found), real([&real_main, &real_clawd]));
    // looked_in names the places, the folders are where they lead.
    assert!(found.looked_in.contains(&"~/clawd".to_string()));
    assert!(found
        .looked_in
        .contains(&"~/.openclaw/workspace".to_string()));
    assert!(found.looked_in.contains(&"~/openclaw".to_string()));
    let ids: Vec<&str> = found.personas.iter().map(|p| p.id.as_str()).collect();
    let again = scan_of(&roots, ImportSource::OpenClaw, Some(&real_clawd));
    assert_eq!(again.personas[0].id, ids[1], "one id however it is reached");

    // A linked home still finds ~/.openclaw-* folders.
    write(&home.join(".openclaw-work/workspace/SOUL.md"), "Work.");
    let linked_home = dir.path().join("linked-home");
    symlink(home, &linked_home).unwrap();
    let through = scan_of(
        &AgentRoots::under(&linked_home),
        ImportSource::OpenClaw,
        None,
    );
    assert!(folders(&through).contains(&canon(&home.join(".openclaw-work/workspace"))));
    assert!(through
        .looked_in
        .iter()
        .all(|place| place.starts_with("~/")));
}

#[cfg(unix)]
#[test]
fn a_link_never_leads_into_a_secret_folder() {
    use std::os::unix::fs::symlink;
    let (dir, roots) = home();
    let home = dir.path();
    let canary = canary();
    let credentials = home.join(".openclaw/credentials");
    write(&credentials.join("agent/SOUL.md"), &canary);
    write(&credentials.join("SOUL.md"), &canary);
    symlink(&credentials, home.join("notes")).unwrap();
    for picked in ["~/notes/agent", "~/notes"] {
        for source in [ImportSource::OpenClaw, ImportSource::Hermes] {
            assert!(
                refused(&roots, source, Some(Path::new(picked)), FOLDER_PRIVATE),
                "{picked}"
            );
        }
    }
    // A default place that leads there is passed over quietly.
    symlink(credentials.join("agent"), home.join("clawd")).unwrap();
    let found = scan_of(&roots, ImportSource::OpenClaw, None);
    assert!(found.personas.is_empty());
    assert!(!serde_json::to_string(&found).unwrap().contains(&canary));
    // An override that leads there is refused out loud.
    let chosen = AgentRoots {
        hermes_home: Some(home.join("notes")),
        ..roots.clone()
    };
    assert!(refused(&chosen, ImportSource::Hermes, None, FOLDER_PRIVATE));
    let chosen = AgentRoots {
        openclaw_workspace: Some(home.join("notes/agent")),
        ..roots
    };
    assert!(refused(
        &chosen,
        ImportSource::OpenClaw,
        None,
        FOLDER_PRIVATE
    ));
}

#[test]
fn overrides_and_legacy_places_keep_their_order() {
    let (dir, roots) = home();
    let home = dir.path();
    let state = home.join("state");
    let order = [
        state.join("workspace"),
        state.join("workspace-b"),
        home.join(".openclaw-team/workspace"),
        home.join(".openclaw-alpha/workspace"),
        home.join(".clawdbot/workspace"),
        home.join(".moltbot/workspace"),
        home.join("clawd"),
        home.join("openclaw"),
    ];
    for workspace in &order {
        write(&workspace.join("SOUL.md"), "Soul.");
    }
    // Ignored once OPENCLAW_STATE_DIR names another state folder.
    write(&home.join(".openclaw/workspace/SOUL.md"), "Default state.");
    let roots = AgentRoots {
        openclaw_state: Some(state),
        openclaw_profile: Some("team".into()),
        ..roots
    };
    let found = scan_of(&roots, ImportSource::OpenClaw, None);
    assert_eq!(folders(&found), real(&order));
    assert_eq!(
        found.looked_in,
        [
            "~/state/workspace",
            "~/state/workspace-*",
            "~/.openclaw-team/workspace",
            "~/.openclaw-*/workspace",
            "~/.clawdbot/workspace",
            "~/.moltbot/workspace",
            "~/clawd",
            "~/openclaw",
        ]
    );
    // ~/clawd picked by hand.
    let picked = scan_of(&roots, ImportSource::OpenClaw, Some(Path::new("~/clawd")));
    assert_eq!(folders(&picked), real([home.join("clawd")]));
    assert_eq!(picked.looked_in, ["~/clawd"]);
}

#[test]
fn no_home_and_relative_places_search_nothing() {
    let (dir, _) = home();
    write(&dir.path().join("clawd/SOUL.md"), "Soul.");
    let none = AgentRoots::none();
    for source in [ImportSource::OpenClaw, ImportSource::Hermes] {
        let found = scan_of(&none, source, None);
        assert!(found.personas.is_empty() && found.looked_in.is_empty());
        assert!(refused(
            &none,
            source,
            Some(Path::new("~/clawd")),
            FOLDER_MISSING
        ));
    }
    // A picked absolute folder still works.
    let found = scan_of(
        &none,
        ImportSource::OpenClaw,
        Some(&dir.path().join("clawd")),
    );
    assert_eq!(found.personas.len(), 1);
    // Relative places never resolve against the working folder.
    let relative = AgentRoots {
        home: PathBuf::from("home"),
        openclaw_workspace: Some(PathBuf::from("clawd")),
        hermes_home: Some(PathBuf::from(".")),
        ..AgentRoots::none()
    };
    for source in [ImportSource::OpenClaw, ImportSource::Hermes] {
        assert!(scan_of(&relative, source, None).personas.is_empty());
    }
}

#[test]
fn a_picked_profiles_folder_lists_its_profiles() {
    let (dir, roots) = home();
    let hermes = dir.path().join(".hermes");
    write(&hermes.join("SOUL.md"), "Main.");
    write(&hermes.join("profiles/coach/SOUL.md"), "Coach.");
    write(&hermes.join("profiles/writer/memories/USER.md"), "Writes.");
    let found = scan_of(&roots, ImportSource::Hermes, Some(&hermes.join("profiles")));
    assert_eq!(
        folders(&found),
        real([
            hermes.join("profiles/coach"),
            hermes.join("profiles/writer")
        ])
    );
    assert_eq!(found.personas[0].name, "coach");
    assert_eq!(found.looked_in, ["~/.hermes/profiles"]);
}

#[test]
fn a_full_scan_stops_and_left_out_logs_are_counted() {
    let (dir, roots) = home();
    let state = dir.path().join(".openclaw");
    for n in 0..MAX_PERSONAS + 8 {
        write(&state.join(format!("workspace-{n:03}/SOUL.md")), "Soul.");
    }
    let found = scan_of(&roots, ImportSource::OpenClaw, None);
    assert_eq!(found.personas.len(), MAX_PERSONAS);
    assert!(PathBuf::from(&found.personas[0].folder).ends_with("workspace-000"));

    // Past MAX_DAILY_LOGS the oldest logs are left out and counted.
    let workspace = dir.path().join("logs");
    write(&workspace.join("SOUL.md"), "Soul.");
    let extra = 5;
    for i in 0..MAX_DAILY_LOGS + extra {
        let name = format!("memory/{:04}-01-{:02}.md", 2000 + i / 50, i % 50);
        write(&workspace.join(name), "x");
    }
    let picked = scan_of(&roots, ImportSource::OpenClaw, Some(&workspace));
    let persona = &picked.personas[0];
    assert_eq!(persona.left_out_logs, u32::try_from(extra).unwrap());
    assert!(persona.clipped);
    assert_eq!(use_of(persona, "memory/2000-01-00.md"), None);
    assert_eq!(
        use_of(persona, "memory/2008-01-04.md"),
        Some(FileUse::Memory)
    );
}

#[test]
fn listings_keep_the_same_names_whatever_order_the_folder_gives() {
    use super::files::{select, Kind, Pick};
    let (dir, _) = home();
    let folder = dir.path().join("many");
    // Made out of order, so the folder's own order differs from the names'.
    for n in [7, 2, 9, 0, 5, 3, 8, 1, 6, 4] {
        write(&folder.join(format!("{n}.md")), "x");
    }
    write(&folder.join(".env"), "never listed");
    let names = |pick| -> Vec<String> {
        let (kept, matched) = select(&folder, 3, |name| name.ends_with(".md"), pick);
        assert_eq!(matched, 10);
        assert!(kept
            .iter()
            .all(|(_, kind)| matches!(kind, Kind::File { .. })));
        kept.into_iter().map(|(name, _)| name).collect()
    };
    assert_eq!(names(Pick::First), ["0.md", "1.md", "2.md"]);
    assert_eq!(names(Pick::Last), ["7.md", "8.md", "9.md"]);
}
