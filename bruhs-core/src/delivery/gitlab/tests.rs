use super::*;
use std::{path::PathBuf, process::Command as Process, sync::Arc};

struct Fixture {
    directory: tempfile::TempDir,
    repository: tempfile::TempDir,
    service: DeliveryService,
    state: PathBuf,
    log: PathBuf,
}
impl Fixture {
    async fn new() -> Self {
        use std::os::unix::fs::PermissionsExt as _;
        let directory = tempfile::tempdir().unwrap();
        let repository = crate::delivery::tests::repository();
        assert!(Process::new("git")
            .args(["remote", "add", "origin", "git@gitlab.com:team/repo.git"])
            .current_dir(repository.path())
            .status()
            .unwrap()
            .success());
        let executable = directory.path().join("glab.py");
        let state = executable.with_extension("json");
        let log = executable.with_extension("log");
        std::fs::write(&executable,r#"#!/usr/bin/env python3
import json, sys, os
from urllib.parse import unquote
from pathlib import Path
assert not any(name in os.environ for name in ['GITLAB_REPO','GLAB_REPO','GITLAB_HOST','GITLAB_URI','GITLAB_URL','GLAB_DEBUG_HTTP'])
p=Path(__file__)
state=json.loads(p.with_suffix('.json').read_text())
args=sys.argv[1:]
assert args[0]=='api' and args[args.index('--hostname')+1]=='gitlab.com'
endpoint=args[1]
mrs=state.get('mrs',[state])
mr=next((mr for mr in mrs if '/merge_requests/'+str(mr['iid']) in endpoint),mrs[0])
if '--input' in args:
    body=json.load(sys.stdin)
    with p.with_suffix('.log').open('a') as log: log.write(json.dumps({'args':args,'body':body})+'\n')
    if 'mrs' in state and endpoint.endswith('/merge'):
        assert body['sha']==mr['sha']
        state['root']=('d' if mr['iid']==1 else 'e')*40;mr['state']='merged';mr['merge_commit_sha']=state['root']
        p.with_suffix('.json').write_text(json.dumps(state))
    elif 'mrs' in state and 'target_branch' in body:
        mr['target_branch']=body['target_branch'];mr['diff_refs']['start_sha']=state['root']
        p.with_suffix('.json').write_text(json.dumps(state))
    print(json.dumps({'data':{'mergeRequestRequestChanges':{'errors':[],'mergeRequest':{'iid':'42'}}}} if endpoint=='graphql' else {'ok':True}))
elif endpoint=='user': print(json.dumps({'username':'owner'}))
elif endpoint.endswith('/approvals'): print(json.dumps({'approved':False,'approved_by':[]}))
elif '/notes?' in endpoint or '/discussions?' in endpoint: print('[]')
elif '/merge_requests?' in endpoint: print(json.dumps([mr for mr in mrs if mr['state']=='opened']))
elif '/repository/branches/' in endpoint:
    branch=unquote(endpoint.split('/repository/branches/')[1]); oid=state['root'] if branch=='main' else next(mr['sha'] for mr in mrs if mr['source_branch']==branch)
    print(json.dumps({'commit':{'id':oid}}))
else: print(json.dumps(mr))
"#).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(&state,json!({"iid":42,"title":"Fixture MR","description":"Reviewed body","web_url":"https://gitlab.com/team/repo/-/merge_requests/42","state":"opened","source_branch":"feature","target_branch":"main","sha":"a".repeat(40),"updated_at":"today","source_project_id":1,"target_project_id":1,"detailed_merge_status":"mergeable","diff_refs":{"base_sha":"b".repeat(40),"start_sha":"b".repeat(40),"head_sha":"a".repeat(40)},"head_pipeline":{"status":"failed","web_url":"https://gitlab.com/team/repo/-/pipelines/1"}}).to_string()).unwrap();
        let mut service = DeliveryService::open(directory.path().join("store"))
            .await
            .unwrap();
        service.glab = Arc::new(executable);
        Self {
            directory,
            repository,
            service,
            state,
            log,
        }
    }
    async fn prepare(&self, action: DeliveryAction) -> PreparedAction {
        self.service
            .prepare(PrepareRequest {
                forge: Forge::Auto,
                path: self.repository.path().to_string_lossy().into_owned(),
                action,
                // A stack merge runs dozens of git commands; under a loaded
                // gate a five second budget timed out on a healthy run.
                timeout_seconds: 60,
            })
            .await
            .unwrap()
    }
    async fn completed(&self, id: &str) -> ActionReceipt {
        tokio::time::timeout(std::time::Duration::from_secs(60), async {
            loop {
                let receipt = self
                    .service
                    .snapshot()
                    .await
                    .actions
                    .into_iter()
                    .find(|receipt| receipt.prepared.id == id)
                    .unwrap();
                if receipt.status != ActionStatus::Running {
                    return receipt;
                }
                // Sleep rather than spin, so the git processes being awaited get the CPU.
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap()
    }
}
#[tokio::test]
async fn gitlab_mutations_freeze_identity_literal_stdin_and_execute_once() {
    let fixture = Fixture::new().await;
    let body = "First line\n$(touch NEVER) `touch NEVER`\nlast line";
    let prepared = fixture
        .prepare(DeliveryAction::Comment {
            number: 42,
            body: body.into(),
        })
        .await;
    assert_eq!(prepared.forge, Forge::GitLab);
    assert_eq!(prepared.repository.as_ref().unwrap().project, "team/repo");
    assert_eq!(
        prepared.commands[0].args,
        [
            "api",
            "projects/team%2Frepo/merge_requests/42/notes",
            "--hostname",
            "gitlab.com",
            "--method",
            "POST",
            "--input",
            "-"
        ]
    );
    assert!(!fixture.log.exists());
    fixture.service.execute(&prepared.id).await.unwrap();
    assert!(fixture.service.execute(&prepared.id).await.is_err());
    let receipt = fixture.completed(&prepared.id).await;
    assert_eq!(
        receipt.status,
        ActionStatus::Completed,
        "{:?}",
        receipt.error
    );
    let log = std::fs::read_to_string(&fixture.log).unwrap();
    assert_eq!(log.lines().count(), 1);
    let write: Value = serde_json::from_str(log.trim()).unwrap();
    assert_eq!(write["body"]["body"], body);
    assert!(!fixture.repository.path().join("NEVER").exists());
    let stale = fixture
        .prepare(DeliveryAction::Comment {
            number: 42,
            body: "stale".into(),
        })
        .await;
    assert!(Process::new("git")
        .args([
            "remote",
            "set-url",
            "origin",
            "git@gitlab.com:other/repository.git"
        ])
        .current_dir(fixture.repository.path())
        .status()
        .unwrap()
        .success());
    fixture.service.execute(&stale.id).await.unwrap();
    assert_eq!(
        fixture.completed(&stale.id).await.status,
        ActionStatus::Failed
    );
    assert_eq!(
        std::fs::read_to_string(&fixture.log)
            .unwrap()
            .lines()
            .count(),
        1
    );
}
#[tokio::test]
async fn gitlab_ci_merge_leases_and_request_changes_use_real_contracts() {
    let fixture = Fixture::new().await;
    let pull = fixture
        .service
        .pull(fixture.repository.path(), 42)
        .await
        .unwrap();
    assert!(pull.checks[0].failed());
    assert!(pull.head_in_repository);
    let prepared = fixture
        .prepare(DeliveryAction::Merge {
            number: 42,
            head_oid: pull.head_oid.clone(),
            method: MergeMethod::Squash,
            auto: true,
        })
        .await;
    let input: Value = serde_json::from_str(prepared.commands[0].stdin.as_ref().unwrap()).unwrap();
    assert_eq!(input["sha"], pull.head_oid);
    assert_eq!(input["auto_merge"], true);
    assert_eq!(input["squash"], true);
    let review = fixture
        .prepare(DeliveryAction::Review {
            number: 42,
            head_oid: pull.head_oid,
            event: ReviewEvent::RequestChanges,
            body: "Please fix the failing pipeline".into(),
            comments: Vec::new(),
        })
        .await;
    assert_eq!(review.commands.len(), 2);
    assert_eq!(review.commands[1].args[1], "graphql");
    assert!(review.commands.iter().all(|command| !command
        .args
        .iter()
        .any(|argument| argument.contains("draft_notes"))));
    assert!(validate_mutation_output(
        &review.commands[1],
        r#"{"data":{"mergeRequestRequestChanges":{"errors":["denied"],"mergeRequest":null}}}"#
    )
    .is_err());
    fixture.service.execute(&review.id).await.unwrap();
    assert_eq!(
        fixture.completed(&review.id).await.status,
        ActionStatus::Completed
    );
    assert!(fixture.directory.path().exists());
}
#[tokio::test]
async fn gitlab_watches_keep_the_reviewed_origin_and_detect_ci_changes() {
    let fixture = Fixture::new().await;
    let watch = fixture
        .service
        .watch_pull(WatchRequest {
            forge: Forge::Auto,
            path: fixture.repository.path().to_string_lossy().into_owned(),
            number: 42,
            poll_seconds: 60,
            reactive_source_id: None,
        })
        .await
        .unwrap();
    assert_eq!(watch.request.forge, Forge::GitLab);
    assert!(watch.repository.is_some());
    let mut data: Value = serde_json::from_slice(&std::fs::read(&fixture.state).unwrap()).unwrap();
    data["sha"] = json!("c".repeat(40));
    std::fs::write(&fixture.state, data.to_string()).unwrap();
    fixture.service.poll_watches().await.unwrap();
    let snapshot = fixture.service.snapshot().await;
    assert_eq!(snapshot.events.len(), 1);
    assert!(snapshot.events[0]
        .reasons
        .iter()
        .any(|reason| reason.contains("Head commit")));
    fixture
        .service
        .deliver_watch_events(None, None)
        .await
        .unwrap();
    assert!(fixture.service.snapshot().await.events[0].delivered);
}
#[test]
fn hostile_origins_and_hidden_quick_actions_are_rejected() {
    for remote in [
        "https://token:secret@gitlab.com/team/repo.git",
        "https://gitlab.com/team/../repo.git",
        "https://gitlab.com/team/repo.git?token=secret",
        "-evil:team/repo",
    ] {
        assert!(
            crate::delivery::forge::repository(remote, Forge::GitLab).is_err(),
            "{remote}"
        );
    }
    assert!(no_quick_actions("Normal text\n /merge").is_err());
    assert!(no_quick_actions("Literal \\/merge").is_ok());
}

#[tokio::test]
async fn gitlab_branch_stack_merges_bottom_up_with_sha_leases_and_retargeting() {
    let fixture = Fixture::new().await;
    let first: Value = serde_json::from_slice(&std::fs::read(&fixture.state).unwrap()).unwrap();
    let mut one = first.clone();
    one["iid"] = json!(1);
    one["source_branch"] = json!("one");
    one["head_pipeline"]["status"] = json!("success");
    one["diff_refs"]["start_sha"] = json!("0".repeat(40));
    let mut two = one.clone();
    two["iid"] = json!(2);
    two["source_branch"] = json!("two");
    two["target_branch"] = json!("one");
    two["sha"] = json!("b".repeat(40));
    two["diff_refs"]["head_sha"] = json!("b".repeat(40));
    two["diff_refs"]["start_sha"] = json!("a".repeat(40));
    std::fs::write(
        &fixture.state,
        json!({"root":"0".repeat(40),"mrs":[one,two]}).to_string(),
    )
    .unwrap();
    let prepared = fixture
        .prepare(DeliveryAction::MergeStack {
            number: 2,
            method: MergeMethod::Squash,
        })
        .await;
    assert_eq!(prepared.stack.as_ref().unwrap().layers.len(), 2);
    assert!(!fixture.log.exists());
    fixture.service.execute(&prepared.id).await.unwrap();
    let receipt = fixture.completed(&prepared.id).await;
    assert_eq!(
        receipt.status,
        ActionStatus::Completed,
        "{:?}",
        receipt.error
    );
    assert_eq!(receipt.completed_commands, 2);
    let writes = std::fs::read_to_string(&fixture.log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(writes.len(), 3);
    assert_eq!(writes[0]["body"]["sha"], "a".repeat(40));
    assert_eq!(writes[1]["body"]["target_branch"], "main");
    assert_eq!(writes[2]["body"]["sha"], "b".repeat(40));
}

#[tokio::test]
async fn replacing_a_watched_origin_cannot_reuse_an_approved_task_subscription_id() {
    let fixture = Fixture::new().await;
    let request = WatchRequest {
        forge: Forge::Auto,
        path: fixture.repository.path().to_string_lossy().into_owned(),
        number: 42,
        poll_seconds: 60,
        reactive_source_id: None,
    };
    let first = fixture.service.watch_pull(request.clone()).await.unwrap();
    assert_eq!(
        fixture
            .service
            .watch_pull(request.clone())
            .await
            .unwrap()
            .id,
        first.id
    );
    assert!(Process::new("git")
        .args([
            "remote",
            "set-url",
            "origin",
            "git@gitlab.com:other/repository.git"
        ])
        .current_dir(fixture.repository.path())
        .status()
        .unwrap()
        .success());
    fixture.service.poll_watches().await.unwrap();
    assert!(!fixture.service.snapshot().await.watches[0].enabled);
    let replacement = fixture.service.watch_pull(request).await.unwrap();
    assert_ne!(replacement.id, first.id);
    assert_eq!(replacement.repository.unwrap().project, "other/repository");
}
