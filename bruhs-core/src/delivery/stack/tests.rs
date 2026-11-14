use super::*;
use serde_json::{json, Value};
use std::{path::PathBuf, process::Command as Process, sync::Arc};

fn node(number: u64, head: &str, base: &str, head_oid: &str, base_oid: &str) -> GraphPull {
    GraphPull {
        number,
        head: head.into(),
        base: base.into(),
        head_oid: head_oid.into(),
        base_oid: base_oid.into(),
        head_repository: "fixture/repo".into(),
        base_repository: "fixture/repo".into(),
    }
}

fn graph() -> Vec<GraphPull> {
    vec![
        node(1, "one", "main", &"a".repeat(40), &"0".repeat(40)),
        node(2, "two", "one", &"b".repeat(40), &"a".repeat(40)),
    ]
}

#[test]
fn selecting_a_tip_includes_all_ancestors_and_descendants() {
    let mut pulls = graph();
    pulls.push(node(3, "three", "two", &"c".repeat(40), &"b".repeat(40)));
    let layers = chain(&pulls, "fixture/repo", 2).unwrap();
    assert_eq!(
        layers.iter().map(|layer| layer.number).collect::<Vec<_>>(),
        [1, 2, 3]
    );
}

#[test]
fn fork_heads_cannot_collide_with_same_named_local_heads() {
    let mut pulls = graph();
    let mut fork = node(3, "one", "main", &"c".repeat(40), &"0".repeat(40));
    fork.head_repository = "outsider/repo".into();
    pulls.push(fork);
    assert_eq!(chain(&pulls, "fixture/repo", 2).unwrap().len(), 2);
    assert!(chain(&pulls, "fixture/repo", 3)
        .unwrap_err()
        .to_string()
        .contains("Fork branches"));
}

#[test]
fn cycles_duplicate_owners_and_branching_children_are_refused() {
    let mut cyclic = graph();
    cyclic[0].base = "two".into();
    assert!(chain(&cyclic, "fixture/repo", 2)
        .unwrap_err()
        .to_string()
        .contains("cycle"));
    let mut duplicate = graph();
    let mut extra = duplicate[0].clone();
    extra.number = 3;
    duplicate.push(extra);
    assert!(chain(&duplicate, "fixture/repo", 2)
        .unwrap_err()
        .to_string()
        .contains("ambiguous"));
    let mut branching = graph();
    branching.push(node(3, "three", "one", &"c".repeat(40), &"a".repeat(40)));
    assert!(chain(&branching, "fixture/repo", 2)
        .unwrap_err()
        .to_string()
        .contains("multiple child"));
}

fn git(path: &Path, args: &[&str]) -> String {
    let result = Process::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap().trim().to_owned()
}

fn commit(path: &Path, file: &str, contents: &str) {
    std::fs::write(path.join(file), contents).unwrap();
    git(path, &["add", "--", file]);
    git(path, &["commit", "-qm", file]);
}

struct Fixture {
    repository: tempfile::TempDir,
    directory: tempfile::TempDir,
    service: DeliveryService,
    pulls: Vec<GraphPull>,
}

fn pull_json(pull: &GraphPull, state: &str, root: &str, merged: bool) -> Value {
    json!({"number":pull.number,"title":format!("Layer {}",pull.number),"body":"fixture", "url":format!("https://github.com/fixture/repo/pull/{}",pull.number),
        "state":state,"isDraft":false,"isCrossRepository":false,"headRefName":pull.head,"headRefOid":pull.head_oid,
        "baseRefName":if merged {"main"} else {&pull.base},"baseRefOid":if merged {root} else {&pull.base_oid},
        "mergeCommit":if state == "MERGED" {json!({"oid":root})} else {Value::Null},
        "updatedAt":"fixture","mergeable":"MERGEABLE","reviewDecision":"APPROVED",
        "statusCheckRollup":[{"name":"unit","status":"COMPLETED","conclusion":"SUCCESS"}],"files":[],"comments":[],"reviews":[]})
}

fn write_json(path: impl AsRef<Path>, value: &Value) {
    std::fs::write(path, value.to_string()).unwrap();
}

#[cfg(unix)]
impl Fixture {
    async fn new(conflicting: bool) -> Self {
        use std::os::unix::fs::PermissionsExt as _;
        let repository = super::super::tests::repository();
        let path = repository.path();
        let remote_root = git(path, &["rev-parse", "HEAD"]);
        git(path, &["switch", "-qc", "one"]);
        commit(
            path,
            if conflicting { "sample.txt" } else { "one.txt" },
            "parent old\n",
        );
        let remote_one = git(path, &["rev-parse", "HEAD"]);
        git(path, &["switch", "-qc", "two"]);
        commit(path, "two.txt", "child\n");
        let remote_two = git(path, &["rev-parse", "HEAD"]);
        git(path, &["switch", "-q", "one"]);
        if !conflicting {
            std::fs::write(path.join("one.txt"), "parent amended\n").unwrap();
            git(path, &["add", "one.txt"]);
            git(path, &["commit", "--amend", "--no-edit", "-q"]);
        }
        git(path, &["switch", "-q", "main"]);
        commit(
            path,
            if conflicting {
                "sample.txt"
            } else {
                "trunk.txt"
            },
            "root advanced\n",
        );
        let pulls = vec![
            node(1, "one", "main", &remote_one, &remote_root),
            node(2, "two", "one", &remote_two, &remote_one),
        ];
        let directory = tempfile::tempdir().unwrap();
        let files = directory.path();
        std::fs::write(files.join("repo"), "fixture/repo\n").unwrap();
        let graph = json!([pulls.iter().map(|pull| json!({"number":pull.number,"head":pull.head,"base":pull.base,
            "head_oid":pull.head_oid,"base_oid":pull.base_oid,"head_repository":pull.head_repository,"base_repository":pull.base_repository})).collect::<Vec<_>>()]);
        write_json(files.join("graph"), &graph);
        std::fs::write(files.join("native"), "[]").unwrap();
        std::fs::write(files.join("root0"), &remote_root).unwrap();
        std::fs::write(files.join("root1"), "d".repeat(40)).unwrap();
        std::fs::write(files.join("root2"), "e".repeat(40)).unwrap();
        for pull in &pulls {
            write_json(
                files.join(format!("pull{}", pull.number)),
                &pull_json(pull, "OPEN", "", false),
            );
            write_json(
                files.join(format!("pull{}-merged", pull.number)),
                &pull_json(
                    pull,
                    "MERGED",
                    &if pull.number == 1 {
                        "d".repeat(40)
                    } else {
                        "e".repeat(40)
                    },
                    true,
                ),
            );
        }
        write_json(
            files.join("pull2-retargeted"),
            &pull_json(&pulls[1], "OPEN", &"d".repeat(40), true),
        );
        let script = format!(
            "#!/bin/sh\ncd '{}' || exit 90\n{}",
            files.to_string_lossy().replace('\'', "'\\''"),
            r#"
case "$1 $2" in
  'repo view') cat repo ;;
  'api '*)
    case "$2" in
      */pulls\?*) cat graph ;;
      */stacks\?*) cat native ;;
      */git/ref/heads/*) if [ -f merged2 ]; then cat root2; elif [ -f merged1 ]; then cat root1; else cat root0; fi ;;
      *) echo 'unexpected API fixture' >&2; exit 91 ;;
    esac ;;
  'pr view')
    if [ -f "merged$3" ]; then cat "pull$3-merged";
    elif [ -f "retargeted$3" ]; then cat "pull$3-retargeted";
    else cat "pull$3"; fi ;;
  'pr edit') printf 'edit %s %s %s\n' "$3" "$4" "$5" >> mutations; touch "retargeted$3" ;;
  'pr merge') printf 'merge %s %s %s %s\n' "$3" "$4" "$5" "$6" >> mutations
    if [ -f "fail$3" ]; then echo 'fixture merge refused' >&2; exit 7; fi
    touch "merged$3" ;;
  *) echo 'unexpected GH fixture command' >&2; exit 92 ;;
esac
"#
        );
        let executable = files.join("gh-fixture");
        std::fs::write(&executable, script).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut service = DeliveryService::open(files.join("journal")).await.unwrap();
        service.gh = Arc::new(executable);
        Self {
            repository,
            directory,
            service,
            pulls,
        }
    }

    async fn prepare(&self, action: DeliveryAction) -> PreparedAction {
        self.service
            .prepare(PrepareRequest {
                forge: Forge::GitHub,
                path: self.repository.path().to_string_lossy().into_owned(),
                action,
                timeout_seconds: 30,
            })
            .await
            .unwrap()
    }

    async fn execute(&self, prepared: &PreparedAction) -> ActionReceipt {
        self.service.execute(&prepared.id).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(35), async {
            loop {
                let receipt = self
                    .service
                    .snapshot()
                    .await
                    .actions
                    .into_iter()
                    .find(|receipt| receipt.prepared.id == prepared.id)
                    .unwrap();
                if receipt.status != ActionStatus::Running {
                    return receipt;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap()
    }

    fn file(&self, name: &str) -> PathBuf {
        self.directory.path().join(name)
    }
    fn mutations(&self) -> String {
        std::fs::read_to_string(self.file("mutations")).unwrap_or_default()
    }
}

#[cfg(unix)]
#[tokio::test]
async fn amended_parent_and_child_replay_without_touching_the_source_checkout() {
    let fixture = Fixture::new(false).await;
    let source = fixture.repository.path();
    let root = git(source, &["rev-parse", "HEAD"]);
    let old_one = git(source, &["rev-parse", "one"]);
    let old_two = git(source, &["rev-parse", "two"]);
    let prepared = fixture
        .prepare(DeliveryAction::RebaseStack { number: 2 })
        .await;
    assert_eq!(
        git(source, &["rev-parse", "one"]),
        old_one,
        "preview must not mutate refs"
    );
    assert_eq!(
        prepared.stack.as_ref().unwrap().layers[1]
            .replay_after_oid
            .as_deref(),
        Some(fixture.pulls[0].head_oid.as_str())
    );
    let receipt = fixture.execute(&prepared).await;
    assert_eq!(
        receipt.status,
        ActionStatus::Completed,
        "{:?}",
        receipt.error
    );
    assert_eq!(receipt.completed_commands, 2);
    assert_ne!(git(source, &["rev-parse", "one"]), old_one);
    assert_ne!(git(source, &["rev-parse", "two"]), old_two);
    assert_eq!(git(source, &["rev-parse", "HEAD"]), root);
    assert_eq!(git(source, &["symbolic-ref", "--short", "HEAD"]), "main");
    assert_eq!(git(source, &["status", "--porcelain"]), "");
    assert_eq!(git(source, &["show", "two:one.txt"]), "parent amended");
    assert_eq!(git(source, &["show", "two:two.txt"]), "child");
    assert_eq!(git(source, &["rev-list", "--count", "main..two"]), "2");
    assert!(!Path::new(
        prepared
            .stack
            .as_ref()
            .unwrap()
            .temporary_worktree
            .as_ref()
            .unwrap()
    )
    .exists());
    assert!(
        fixture.mutations().is_empty(),
        "a local rebase must never write to GitHub"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn replay_conflicts_leave_every_source_branch_ref_unchanged() {
    let fixture = Fixture::new(true).await;
    let source = fixture.repository.path();
    let before = [
        git(source, &["rev-parse", "one"]),
        git(source, &["rev-parse", "two"]),
    ];
    let prepared = fixture
        .prepare(DeliveryAction::RebaseStack { number: 1 })
        .await;
    let receipt = fixture.execute(&prepared).await;
    assert_eq!(receipt.status, ActionStatus::Failed);
    assert_eq!(receipt.completed_commands, 0);
    assert_eq!(
        [
            git(source, &["rev-parse", "one"]),
            git(source, &["rev-parse", "two"])
        ],
        before
    );
    assert!(!Path::new(
        prepared
            .stack
            .as_ref()
            .unwrap()
            .temporary_worktree
            .as_ref()
            .unwrap()
    )
    .exists());
    assert_eq!(git(source, &["status", "--porcelain"]), "");
}

#[cfg(unix)]
#[tokio::test]
async fn a_changed_local_lease_stops_before_creating_a_scratch_worktree() {
    let fixture = Fixture::new(false).await;
    let prepared = fixture
        .prepare(DeliveryAction::RebaseStack { number: 2 })
        .await;
    git(
        fixture.repository.path(),
        &["update-ref", "refs/heads/one", &fixture.pulls[0].base_oid],
    );
    let receipt = fixture.execute(&prepared).await;
    assert_eq!(receipt.status, ActionStatus::Failed);
    assert!(receipt.error.unwrap().contains("changed after review"));
    assert!(!Path::new(
        prepared
            .stack
            .as_ref()
            .unwrap()
            .temporary_worktree
            .as_ref()
            .unwrap()
    )
    .exists());
}

#[cfg(unix)]
#[tokio::test]
async fn an_existing_scratch_path_is_never_cleaned_up_as_our_own() {
    let fixture = Fixture::new(false).await;
    let prepared = fixture
        .prepare(DeliveryAction::RebaseStack { number: 2 })
        .await;
    let scratch = Path::new(
        prepared
            .stack
            .as_ref()
            .unwrap()
            .temporary_worktree
            .as_ref()
            .unwrap(),
    );
    std::fs::create_dir_all(scratch).unwrap();
    std::fs::write(scratch.join("owner-work"), "preserve").unwrap();
    let receipt = fixture.execute(&prepared).await;
    assert_eq!(receipt.status, ActionStatus::Failed);
    assert_eq!(
        std::fs::read_to_string(scratch.join("owner-work")).unwrap(),
        "preserve"
    );
    assert_eq!(receipt.completed_commands, 0);
}

#[cfg(unix)]
#[tokio::test]
async fn symbolic_branch_refs_are_refused_instead_of_rewriting_an_unreviewed_target() {
    let fixture = Fixture::new(false).await;
    let source = fixture.repository.path();
    let before = git(source, &["rev-parse", "two"]);
    git(
        source,
        &["symbolic-ref", "refs/heads/one", "refs/heads/two"],
    );
    let error = fixture
        .service
        .prepare(PrepareRequest {
            forge: Forge::GitHub,
            path: source.to_string_lossy().into_owned(),
            action: DeliveryAction::RebaseStack { number: 1 },
            timeout_seconds: 30,
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("symbolic branch ref"));
    assert_eq!(git(source, &["rev-parse", "two"]), before);
    assert!(fixture.mutations().is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn cancelling_an_active_replay_cleans_its_worktree_without_publishing_refs() {
    use std::os::unix::fs::PermissionsExt as _;
    let mut fixture = Fixture::new(false).await;
    let shim = fixture.file("git-fixture");
    let marker = fixture.file("replay-started");
    std::fs::write(&shim, format!("#!/bin/sh\nfor arg in \"$@\"; do\nif [ \"$arg\" = rebase ]; then touch '{}'; sleep 30; fi\ndone\nexec git \"$@\"\n", marker.to_string_lossy().replace('\'', "'\\''"))).unwrap();
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o700)).unwrap();
    fixture.service.git = Arc::new(shim);
    let source = fixture.repository.path();
    let before = [
        git(source, &["rev-parse", "one"]),
        git(source, &["rev-parse", "two"]),
    ];
    let prepared = fixture
        .prepare(DeliveryAction::RebaseStack { number: 2 })
        .await;
    fixture.service.execute(&prepared.id).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        while !marker.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    fixture.service.cancel(&prepared.id).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        while fixture
            .service
            .cancellations
            .lock()
            .await
            .contains_key(&prepared.id)
        {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        fixture.service.snapshot().await.actions[0].status,
        ActionStatus::Cancelled
    );
    assert_eq!(
        [
            git(source, &["rev-parse", "one"]),
            git(source, &["rev-parse", "two"])
        ],
        before
    );
    assert!(!Path::new(
        prepared
            .stack
            .as_ref()
            .unwrap()
            .temporary_worktree
            .as_ref()
            .unwrap()
    )
    .exists());
    assert_eq!(git(source, &["status", "--porcelain"]), "");
}

#[cfg(unix)]
#[tokio::test]
async fn dirty_and_occupied_worktrees_are_refused_before_preparation() {
    let fixture = Fixture::new(false).await;
    let source = fixture.repository.path();
    let request = || PrepareRequest {
        forge: Forge::GitHub,
        path: source.to_string_lossy().into_owned(),
        action: DeliveryAction::RebaseStack { number: 1 },
        timeout_seconds: 30,
    };
    std::fs::write(source.join("untracked"), "owner work").unwrap();
    assert!(fixture
        .service
        .prepare(request())
        .await
        .unwrap_err()
        .to_string()
        .contains("untracked"));
    std::fs::remove_file(source.join("untracked")).unwrap();
    let occupied = fixture.file("occupied");
    git(
        source,
        &["worktree", "add", &occupied.to_string_lossy(), "one"],
    );
    assert!(fixture
        .service
        .prepare(request())
        .await
        .unwrap_err()
        .to_string()
        .contains("checked out"));
    git(source, &["worktree", "remove", &occupied.to_string_lossy()]);
    assert!(fixture.service.snapshot().await.actions.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn stack_merge_uses_bottom_up_head_leases_and_explicit_retargeting() {
    let fixture = Fixture::new(false).await;
    let prepared = fixture
        .prepare(DeliveryAction::MergeStack {
            number: 2,
            method: MergeMethod::Squash,
        })
        .await;
    assert!(fixture.mutations().is_empty());
    let receipt = fixture.execute(&prepared).await;
    assert_eq!(
        receipt.status,
        ActionStatus::Completed,
        "{:?}",
        receipt.error
    );
    assert_eq!(receipt.completed_commands, 2);
    assert_eq!(
        fixture.mutations(),
        format!(
            "merge 1 --squash --match-head-commit {}\nedit 2 --base main\nmerge 2 --squash --match-head-commit {}\n",
            fixture.pulls[0].head_oid, fixture.pulls[1].head_oid
        )
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_later_merge_failure_retains_the_confirmed_partial_result() {
    let fixture = Fixture::new(false).await;
    let prepared = fixture
        .prepare(DeliveryAction::MergeStack {
            number: 1,
            method: MergeMethod::Merge,
        })
        .await;
    std::fs::write(fixture.file("fail2"), "fixture failure").unwrap();
    let receipt = fixture.execute(&prepared).await;
    assert_eq!(receipt.status, ActionStatus::Failed);
    assert_eq!(receipt.completed_commands, 1);
    assert!(receipt.error.unwrap().contains("fixture merge refused"));
    assert!(fixture.file("merged1").exists());
    assert!(fixture.file("retargeted2").exists());
    assert!(!fixture.file("merged2").exists());
    assert!(
        fixture.service.execute(&prepared.id).await.is_err(),
        "a partial action is never replayed"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn refreshed_checks_after_retarget_stop_the_next_merge() {
    let fixture = Fixture::new(false).await;
    let prepared = fixture
        .prepare(DeliveryAction::MergeStack {
            number: 1,
            method: MergeMethod::Merge,
        })
        .await;
    let mut pending = pull_json(&fixture.pulls[1], "OPEN", &"d".repeat(40), true);
    pending["statusCheckRollup"] =
        json!([{"name":"new-base-ci","status":"QUEUED","conclusion":null}]);
    write_json(fixture.file("pull2-retargeted"), &pending);
    let receipt = fixture.execute(&prepared).await;
    assert_eq!(receipt.status, ActionStatus::Failed);
    assert_eq!(receipt.completed_commands, 1);
    assert!(receipt.error.unwrap().contains("pending"));
    assert_eq!(
        fixture.mutations().lines().count(),
        2,
        "only merge 1 and retarget 2 were submitted"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn stale_remote_heads_and_native_unpinned_merges_are_refused() {
    let fixture = Fixture::new(false).await;
    let prepared = fixture
        .prepare(DeliveryAction::MergeStack {
            number: 1,
            method: MergeMethod::Merge,
        })
        .await;
    let mut graph: Value =
        serde_json::from_str(&std::fs::read_to_string(fixture.file("graph")).unwrap()).unwrap();
    graph[0][1]["head_oid"] = json!("f".repeat(40));
    write_json(fixture.file("graph"), &graph);
    let receipt = fixture.execute(&prepared).await;
    assert_eq!(receipt.status, ActionStatus::Failed);
    assert!(fixture.mutations().is_empty());
    graph[0][1]["head_oid"] = json!(fixture.pulls[1].head_oid);
    write_json(fixture.file("graph"), &graph);
    write_json(fixture.file("native"), &json!([{"number":7}]));
    let error = fixture
        .service
        .prepare(PrepareRequest {
            forge: Forge::GitHub,
            path: fixture.repository.path().to_string_lossy().into_owned(),
            action: DeliveryAction::MergeStack {
                number: 2,
                method: MergeMethod::Merge,
            },
            timeout_seconds: 30,
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("expected-head leases"));
    assert!(fixture.mutations().is_empty());
}
