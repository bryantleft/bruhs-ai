use super::*;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
            // A background `git maintenance` started by a commit would
            // count as a program working in the folder.
            "-c",
            "maintenance.auto=false",
            "-c",
            "gc.auto=0",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A project with one commit and a worktrees folder holding one chat
/// worktree with a commit of its own.
struct Host {
    _dir: tempfile::TempDir,
    root: PathBuf,
    project: PathBuf,
    chat: PathBuf,
}

fn host() -> Host {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    let root = dir.path().join("worktrees");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(root.join("project")).unwrap();
    git(&project, &["init", "-q", "-b", "main"]);
    std::fs::write(project.join(".gitignore"), "target/\n").unwrap();
    std::fs::write(project.join("README.md"), "hello\n").unwrap();
    git(&project, &["add", "-A"]);
    git(&project, &["commit", "-q", "-m", "init"]);
    let chat = root.join("project/bruhs-chat");
    git(
        &project,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "bruhs/chat",
            chat.to_str().unwrap(),
            "main",
        ],
    );
    std::fs::write(chat.join("idea.txt"), "kept\n").unwrap();
    git(&chat, &["add", "idea.txt"]);
    git(&chat, &["commit", "-q", "-m", "idea"]);
    std::fs::create_dir_all(chat.join("target")).unwrap();
    std::fs::write(chat.join("target/out"), "build").unwrap();
    Host {
        _dir: dir,
        root,
        project,
        chat,
    }
}

async fn remove(host: &Host) -> Result<(), String> {
    let _lock = lock(&host.root).await?;
    let branch = check(&host.root, &host.project, &host.chat).await?;
    unregister(&host.root, &host.project, &host.chat, &branch).await
}

/// A chat worktree is listed, removed with its branch kept, listed as
/// removed, and checked out again by the next turn in that folder.
#[tokio::test]
async fn a_chat_worktree_is_removed_and_comes_back_for_the_next_turn() {
    let host = host();
    let rows = list(&host.root).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].state, WorktreeState::Present);
    assert_eq!(rows[0].branch.as_deref(), Some("bruhs/chat"));
    assert_eq!(
        rows[0].project.as_deref().map(|p| resolve(Path::new(p))),
        Some(resolve(&host.project))
    );
    assert_eq!(rows[0].blocked, None);

    remove(&host).await.unwrap();
    assert!(!host.chat.exists());
    let rows = list(&host.root).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].state, WorktreeState::Removed);
    assert!(!rows[0].branch_missing);

    // The next turn there, even in a subfolder, checks the branch out again.
    restore_for_turn_in(&host.root, &host.chat.join("src/deep"))
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(host.chat.join("idea.txt")).unwrap(),
        "kept\n"
    );
    assert!(record(&host.root, &host.chat).await.unwrap().is_none());
    // A turn in a folder bruhs never removed is left alone.
    restore_for_turn_in(&host.root, &host.root.join("project/never"))
        .await
        .unwrap();
    assert!(!host.root.join("project/never").exists());
}

/// A records file that cannot be parsed is set aside, never overwritten,
/// and the removal that found it is refused.
#[tokio::test]
async fn corrupt_records_are_set_aside_not_overwritten() {
    let host = host();
    std::fs::write(host.root.join(".bruhs-removed.json"), "{ not json").unwrap();
    let error = remove(&host).await.unwrap_err();
    assert!(error.contains("set aside"), "{error}");
    assert!(host.chat.exists());
    let aside = std::fs::read_dir(&host.root)
        .unwrap()
        .flatten()
        .any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".bruhs-removed.json.corrupt-")
        });
    assert!(aside);
    // The next removal starts a fresh file.
    remove(&host).await.unwrap();
}

/// The main checkout, or a plain folder, inside the worktrees folder is
/// never removed.
#[tokio::test]
async fn only_linked_worktrees_are_removed() {
    let host = host();
    let plain = host.root.join("project/plain");
    std::fs::create_dir_all(&plain).unwrap();
    let error = check(&host.root, &host.project, &plain).await.unwrap_err();
    assert!(error.contains("not a linked worktree"), "{error}");
}

/// A deleted branch cannot come back on its own; it starts again from
/// another base, keeping the recorded branch name.
#[tokio::test]
async fn a_deleted_branch_starts_again_from_another_base() {
    let host = host();
    remove(&host).await.unwrap();
    git(&host.project, &["branch", "-D", "bruhs/chat"]);
    let rows = list(&host.root).await;
    assert!(rows[0].branch_missing, "{rows:?}");
    let _lock = lock(&host.root).await.unwrap();
    let error = restore_branch(&host.root, &host.project, &host.chat, "bruhs/chat", None)
        .await
        .unwrap_err();
    assert!(error.contains("no longer exists"), "{error}");
    restore_branch(
        &host.root,
        &host.project,
        &host.chat,
        "bruhs/chat",
        Some("main"),
    )
    .await
    .unwrap();
    assert_eq!(
        git(&host.chat, &["branch", "--show-current"]).trim(),
        "bruhs/chat"
    );
    assert!(!host.chat.join("idea.txt").exists());
}

/// A program working in the folder (an agent CLI, a shell) keeps it.
#[cfg(unix)]
#[tokio::test]
async fn a_program_working_in_the_folder_keeps_it() {
    let host = host();
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .current_dir(host.chat.join("target"))
        .spawn()
        .unwrap();
    // lsof sees the process once it has started.
    let mut refused = None;
    for _ in 0..50 {
        if let Err(error) = check(&host.root, &host.project, &host.chat).await {
            refused = Some(error);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    child.kill().unwrap();
    child.wait().unwrap();
    let refused = refused.expect("a working program never blocked removal");
    assert!(refused.contains("working in this folder"), "{refused}");
    assert!(refused.contains("sleep"), "{refused}");
}

#[test]
fn a_folder_resolves_the_same_before_and_after_it_is_gone() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("w");
    std::fs::create_dir(&folder).unwrap();
    let present = resolve(&folder);
    std::fs::remove_dir(&folder).unwrap();
    assert_eq!(present, resolve(&folder));
    assert!(!present.to_string_lossy().ends_with('/'));
}

#[test]
fn only_folders_inside_the_root_qualify() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("worktrees");
    std::fs::create_dir_all(root.join("p")).unwrap();
    assert!(inside(&root, &root.join("p/w")));
    assert!(!inside(&root, &root));
    assert!(!inside(&root, &root.join("p/../../escape")));
    assert!(!inside(&root, Path::new("relative/p")));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/tmp", root.join("link")).unwrap();
        assert!(!inside(&root, &root.join("link/w")));
    }
}

#[cfg(not(target_os = "linux"))]
#[test]
fn lsof_records_pair_each_process_with_its_directory() {
    let out = b"p12\ncclaude\nn/w/one\np34\ncnode\nn/w/two dir\n";
    assert_eq!(
        parse_lsof(out),
        [
            (12, "claude".to_string(), PathBuf::from("/w/one")),
            (34, "node".to_string(), PathBuf::from("/w/two dir")),
        ]
    );
}

#[tokio::test]
async fn cleanup_protects_local_files_according_to_the_selected_rule() {
    use KeepWhen::{AnyLocalFiles, TrackedChanges, UncommittedChanges};
    let host = host();
    let path = &host.chat;
    let protected = |rule| crate::git::has_protected_worktree_files(path, rule);
    // The fixture has one ignored build artifact.
    assert!(protected(AnyLocalFiles).await.unwrap());
    assert!(!protected(UncommittedChanges).await.unwrap());
    std::fs::remove_dir_all(path.join("target")).unwrap();
    std::fs::create_dir_all(path.join("packages/a/node_modules/pkg")).unwrap();
    let dependency = path.join("packages/a/node_modules/pkg/index.js");
    std::fs::write(&dependency, "module").unwrap();
    assert!(!protected(AnyLocalFiles).await.unwrap());
    std::fs::write(path.join("notes.txt"), "notes").unwrap();
    assert!(protected(AnyLocalFiles).await.unwrap());
    assert!(protected(UncommittedChanges).await.unwrap());
    assert!(!protected(TrackedChanges).await.unwrap());
    git(path, &["add", "-f", "packages/a/node_modules/pkg/index.js"]);
    for rule in [AnyLocalFiles, UncommittedChanges, TrackedChanges] {
        assert!(protected(rule).await.unwrap());
    }
    git(path, &["commit", "-qm", "track a dependency"]);
    std::fs::write(&dependency, "modified tracked dependency").unwrap();
    assert!(protected(TrackedChanges).await.unwrap());
    git(path, &["restore", "."]);
    let _guard = lock(&host.root).await.unwrap();
    let cleanup =
        |branch, rule| cleanup_thread_checkout(&host.root, &host.project, path, branch, rule);
    assert!(cleanup("bruhs/chat", UncommittedChanges).await.is_err());
    assert!(cleanup("wrong-branch", TrackedChanges).await.is_err());
    assert!(path.join("notes.txt").exists());
    assert!(record(&host.root, path).await.unwrap().is_none());
    cleanup("bruhs/chat", TrackedChanges).await.unwrap();
    assert!(!path.exists());
    restore_branch(&host.root, &host.project, path, "bruhs/chat", None)
        .await
        .unwrap();
    assert_eq!(std::fs::read_to_string(dependency).unwrap(), "module");
    assert!(!path.join("notes.txt").exists());
}

#[tokio::test]
async fn cleanup_rechecks_moved_files_and_recovers_interrupted_inspection() {
    let host = host();
    let _guard = lock(&host.root).await.unwrap();
    // A file appeared between the initial check and the rename.
    std::fs::write(host.chat.join("local"), "save me").unwrap();
    let refused = move_and_unregister(
        &host.project,
        &host.chat,
        "bruhs/chat",
        Some(KeepWhen::UncommittedChanges),
        None,
    )
    .await;
    assert!(refused.unwrap_err().contains("cleanup protection"));
    let pending = pending_path(&host.chat).unwrap();
    std::fs::rename(&host.chat, &pending).unwrap();
    sweep_staging(pending.parent().unwrap());
    restore_branch(&host.root, &host.project, &host.chat, "bruhs/chat", None)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(host.chat.join("local")).unwrap(),
        "save me"
    );
    assert_eq!(
        git(&host.chat, &["branch", "--show-current"]).trim(),
        "bruhs/chat"
    );
}
