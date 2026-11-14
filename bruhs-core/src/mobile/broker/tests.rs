use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use uuid::Uuid;
async fn lease() -> (tempfile::TempDir, MobileLease) {
    let root = tempfile::tempdir().unwrap();
    let authority = MobileAuthority::open(
        root.path().join("private/devices.json"),
        MobileLaunch::isolated(),
    )
    .await
    .unwrap();
    let offer = authority
        .begin_pairing(
            &"https://localhost/".parse().unwrap(),
            [MobileScope::Read, MobileScope::Prompt].into(),
            std::time::Duration::from_secs(60),
        )
        .unwrap();
    let secret = url::Url::parse(&offer.url)
        .unwrap()
        .fragment()
        .unwrap()
        .strip_prefix("pair=")
        .unwrap()
        .to_owned();
    let result = authority
        .pair(MobilePairInput {
            secret,
            name: "Broker fixture".into(),
        })
        .await
        .unwrap();
    (root, authority.authenticate(&result.token).unwrap())
}
fn command() -> MobileCommand {
    MobileCommand {
        id: Uuid::new_v4(),
        sequence: 1,
        action: MobileAction::Prompt {
            thread_id: "native-fixture".into(),
            text: "One desktop action".into(),
        },
    }
}
#[tokio::test]
async fn mobile_broker_exact_retries_dispatch_once_and_revocation_rejects_queued_work() {
    let (_root, lease) = lease().await;
    let (broker, mut receiver) = MobileBroker::new();
    let request = command();
    let task_broker = broker.clone();
    let task_lease = lease.clone();
    let task_request = request.clone();
    let task = tokio::spawn(async move { task_broker.command(task_lease, task_request).await });
    let MobilePending::Command {
        lease: queued,
        ticket,
        respond,
    } = receiver.recv().await.unwrap()
    else {
        panic!()
    };
    let calls = AtomicUsize::new(0);
    let receipt = queued.authority().commit(&queued, &ticket, |_| {
        calls.fetch_add(1, Ordering::AcqRel);
        Ok(MobileCommandOutcome::Accepted { queued: false })
    });
    respond.send(receipt).unwrap();
    drop(ticket);
    let first = task.await.unwrap().unwrap();
    assert_eq!(broker.command(lease.clone(), request).await.unwrap(), first);
    assert_eq!(calls.load(Ordering::Acquire), 1);
    assert!(receiver.try_recv().is_err());
    let mut next = command();
    next.sequence = 2;
    let next_broker = broker.clone();
    let next_lease = lease.clone();
    let task = tokio::spawn(async move { next_broker.command(next_lease, next).await });
    let MobilePending::Command {
        lease: queued,
        ticket,
        respond,
    } = receiver.recv().await.unwrap()
    else {
        panic!()
    };
    queued.authority().revoke_now(queued.id()).unwrap();
    queued.authority().persist_revocation().await.unwrap();
    let receipt = queued.authority().commit(&queued, &ticket, |_| {
        calls.fetch_add(1, Ordering::AcqRel);
        Ok(MobileCommandOutcome::Accepted { queued: false })
    });
    assert!(matches!(receipt, Err(MobileError::Unauthorized)));
    let _ = respond.send(receipt);
    drop(ticket);
    assert!(task.await.unwrap().is_err());
    assert_eq!(calls.load(Ordering::Acquire), 1);
}
#[tokio::test]
async fn mobile_broker_cancelled_callers_do_not_release_native_work_admission_early() {
    let (_root, lease) = lease().await;
    let (broker, mut receiver) = MobileBroker::new();
    let mut tasks = Vec::new();
    let mut native = Vec::new();
    for _ in 0..BROKER_CAPACITY {
        let broker = broker.clone();
        let lease = lease.clone();
        tasks.push(tokio::spawn(async move {
            broker.read(lease, MobileRead::Threads { offset: 0 }).await
        }));
        native.push(receiver.recv().await.unwrap());
    }
    for task in &tasks {
        task.abort();
    }
    for task in tasks {
        let _ = task.await;
    }
    assert!(native.iter().all(MobilePending::is_closed));
    assert!(matches!(
        broker
            .read(lease.clone(), MobileRead::Threads { offset: 0 })
            .await,
        Err(MobileError::Unavailable)
    ));
    drop(native);
    let task_broker = broker.clone();
    let task_lease = lease.clone();
    let task = tokio::spawn(async move {
        task_broker
            .read(task_lease, MobileRead::Threads { offset: 0 })
            .await
    });
    let MobilePending::Read { respond, .. } = receiver.recv().await.unwrap() else {
        panic!()
    };
    respond
        .send(Ok(MobileReadReply::Threads(MobileThreadPage {
            threads: vec![],
            next_offset: None,
        })))
        .unwrap();
    assert!(task.await.unwrap().is_ok());
}
#[tokio::test]
async fn mobile_broker_rechecks_revocation_after_native_read_before_release() {
    let (_root, lease) = lease().await;
    let (broker, mut receiver) = MobileBroker::new();
    let queued_lease = lease.clone();
    let task = tokio::spawn(async move {
        broker
            .read(queued_lease, MobileRead::Threads { offset: 0 })
            .await
    });
    let MobilePending::Read { respond, .. } = receiver.recv().await.unwrap() else {
        panic!()
    };
    lease.authority().revoke_now(lease.id()).unwrap();
    lease.authority().persist_revocation().await.unwrap();
    respond
        .send(Ok(MobileReadReply::Threads(MobileThreadPage {
            threads: vec![],
            next_offset: None,
        })))
        .unwrap();
    assert!(matches!(
        task.await.unwrap(),
        Err(MobileError::Unauthorized)
    ));
}
