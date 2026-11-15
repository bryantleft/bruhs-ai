use super::*;
use serde_json::{json, Value};
use std::{path::PathBuf, process::Command as Process, sync::Arc};

struct Fixture {
    directory: tempfile::TempDir,
    repository: tempfile::TempDir,
    service: DeliveryService,
    state: PathBuf,
    log: PathBuf,
}
impl Fixture {
    async fn new(mode: &str) -> Self {
        use std::os::unix::fs::PermissionsExt as _;
        let directory = tempfile::tempdir().unwrap();
        let repository = crate::delivery::tests::repository();
        assert!(Process::new("git")
            .args(["remote", "add", "origin", "git@github.com:fixture/repo.git"])
            .current_dir(repository.path())
            .status()
            .unwrap()
            .success());
        let executable = directory.path().join("gh.py");
        let state = executable.with_extension("json");
        let log = executable.with_extension("log");
        std::fs::write(&state,json!({"mode":mode,"head":"b".repeat(40),"extra":false,"blocked":false,"submitted":false}).to_string()).unwrap();
        std::fs::write(&executable,r#"#!/usr/bin/env python3
import json,sys
from pathlib import Path
p=Path(__file__); file=p.with_suffix('.json'); state=json.loads(file.read_text()); args=sys.argv[1:]
numbers=[1,2]+([3] if state['extra'] else [])
def head(number): return 'a'*40 if number==1 else state['head'] if number==2 else 'c'*40
def merged(number): return state['submitted'] and (state['mode']=='merged' or state['mode']=='failed' and number==1)
if args[:2]==['pr','view']:
 n=int(args[2]); print(json.dumps({'number':n,'title':'Native fixture','body':'Body','url':f'https://github.com/fixture/repo/pull/{n}','state':'MERGED' if merged(n) else 'OPEN','isDraft':False,'isCrossRepository':False,'baseRefName':'main' if n==1 else f'branch-{n-1}','baseRefOid':'0'*40 if n==1 else head(n-1),'headRefName':f'branch-{n}','headRefOid':head(n),'mergeCommit':{'oid':'d'*40} if merged(n) else None,'updatedAt':'today','mergeable':'MERGEABLE','reviewDecision':'APPROVED','statusCheckRollup':[{'name':'tests','status':'COMPLETED','conclusion':'FAILURE' if state['blocked'] else 'SUCCESS'}]}))
elif args[0]=='api':
 endpoint=args[1]
 assert args[args.index('--hostname')+1]=='github.com'
 if '/stacks?' in endpoint: print(json.dumps([{'number':7,'open':True,'base':{'ref':'main','sha':'0'*40},'pull_requests':[{'number':n,'state':'open','draft':False,'merged_at':None,'head':{'ref':f'branch-{n}','sha':head(n)}} for n in numbers]}]))
 elif '/git/ref/' in endpoint: print(json.dumps({'object':{'sha':'0'*40}}))
 elif '/merge-async' in endpoint:
  if '--input' in args:
   body=json.load(sys.stdin)
   with p.with_suffix('.log').open('a') as log: log.write(json.dumps({'endpoint':endpoint,'body':body})+'\n')
   state['submitted']=True;file.write_text(json.dumps(state))
   if state['mode']=='unknown': sys.exit(1)
  mode=state['mode']; status='merged' if mode=='merged' else 'failed' if mode=='failed' else 'enqueued' if mode=='enqueued' else 'pending'
  print(json.dumps({'status':status,'details':{'uuid':'3a31b937-d5c4-4ddb-a87c-e534603282f2','expected_head_sha':state['head'],'message':'Fixture '+status,'sha':'d'*40 if status=='merged' else ''}}))
 else: raise Exception(endpoint)
else: raise Exception(args)
"#).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut service = DeliveryService::open(directory.path().join("store"))
            .await
            .unwrap();
        service.gh = Arc::new(executable);
        Self {
            directory,
            repository,
            service,
            state,
            log,
        }
    }
    async fn prepare(&self) -> PreparedAction {
        self.service
            .prepare(PrepareRequest {
                forge: Forge::Auto,
                path: self.repository.path().to_string_lossy().into_owned(),
                action: DeliveryAction::MergeNativeStack {
                    number: 1,
                    method: MergeMethod::Squash,
                },
                timeout_seconds: 10,
            })
            .await
            .unwrap()
    }
    fn change(&self, change: impl FnOnce(&mut Value)) {
        let mut state: Value =
            serde_json::from_slice(&std::fs::read(&self.state).unwrap()).unwrap();
        change(&mut state);
        std::fs::write(&self.state, state.to_string()).unwrap();
    }
    async fn finished(&self, id: &str) -> ActionReceipt {
        // Wider than the action's own 10 second budget, which decides the
        // outcome; this only catches a receipt that never settles.
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
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap()
    }
}
#[tokio::test]
async fn native_stack_requires_owner_ack_and_observes_saved_uuid_without_resubmission() {
    let fixture = Fixture::new("pending").await;
    let prepared = fixture.prepare().await;
    assert_eq!(prepared.native_stack.as_ref().unwrap().merge_through, 2);
    assert!(fixture.service.execute(&prepared.id).await.is_err());
    assert!(fixture
        .service
        .execute_native_stack(&prepared.id, false)
        .await
        .is_err());
    assert!(!fixture.log.exists());
    // Explicit owner authority permits server-current heads after a fresh full-stack check.
    fixture.change(|state| state["head"] = json!("e".repeat(40)));
    fixture
        .service
        .execute_native_stack(&prepared.id, true)
        .await
        .unwrap();
    let receipt = fixture.finished(&prepared.id).await;
    assert_eq!(receipt.status, ActionStatus::Interrupted);
    let operation = receipt.native_operation.unwrap();
    assert!(operation.acknowledged_server_heads);
    assert_eq!(operation.state, NativeMergeState::Pending);
    assert_eq!(operation.preflight.layers[1].head_oid, "e".repeat(40));
    assert!(operation.operation_id.is_some());
    let writes = std::fs::read_to_string(&fixture.log).unwrap();
    let write: Value = serde_json::from_str(writes.trim()).unwrap();
    assert_eq!(write["endpoint"], "repos/fixture/repo/pulls/2/merge-async");
    assert_eq!(
        write["body"],
        json!({"merge_method":"squash","merge_action":"default"})
    );
    assert!(fixture
        .service
        .execute_native_stack(&prepared.id, true)
        .await
        .is_err());
    fixture.change(|state| state["mode"] = json!("merged"));
    let completed = fixture
        .service
        .refresh_native_stack(&prepared.id)
        .await
        .unwrap();
    assert_eq!(completed.status, ActionStatus::Completed);
    assert_eq!(completed.completed_commands, 2);
    assert_eq!(
        std::fs::read_to_string(&fixture.log)
            .unwrap()
            .lines()
            .count(),
        1
    );
    let saved =
        std::fs::read_to_string(fixture.directory.path().join("store/journal.json")).unwrap();
    assert!(saved.contains("3a31b937-d5c4-4ddb-a87c-e534603282f2"));
}
#[tokio::test]
async fn native_membership_changes_stop_before_submission_and_partial_failures_remain_visible() {
    let fixture = Fixture::new("failed").await;
    let prepared = fixture.prepare().await;
    fixture.change(|state| state["extra"] = json!(true));
    fixture
        .service
        .execute_native_stack(&prepared.id, true)
        .await
        .unwrap();
    assert_eq!(
        fixture.finished(&prepared.id).await.status,
        ActionStatus::Failed
    );
    assert!(!fixture.log.exists());
    fixture.change(|state| state["extra"] = json!(false));
    let partial = fixture.prepare().await;
    fixture
        .service
        .execute_native_stack(&partial.id, true)
        .await
        .unwrap();
    let receipt = fixture.finished(&partial.id).await;
    assert_eq!(receipt.status, ActionStatus::Failed);
    assert_eq!(receipt.completed_commands, 1);
    assert_eq!(
        receipt.native_operation.unwrap().state,
        NativeMergeState::Failed
    );
}
#[tokio::test]
async fn unknown_native_submit_is_never_replayed_and_enqueued_is_not_completed() {
    for (mode, state) in [
        ("unknown", NativeMergeState::Unknown),
        ("enqueued", NativeMergeState::Enqueued),
    ] {
        let fixture = Fixture::new(mode).await;
        let prepared = fixture.prepare().await;
        fixture
            .service
            .execute_native_stack(&prepared.id, true)
            .await
            .unwrap();
        let receipt = fixture.finished(&prepared.id).await;
        assert_eq!(receipt.status, ActionStatus::Interrupted);
        assert_eq!(receipt.native_operation.unwrap().state, state);
        assert_eq!(receipt.completed_commands, 0);
        assert!(fixture
            .service
            .execute_native_stack(&prepared.id, true)
            .await
            .is_err());
        assert_eq!(
            std::fs::read_to_string(&fixture.log)
                .unwrap()
                .lines()
                .count(),
            1
        );
    }
}
