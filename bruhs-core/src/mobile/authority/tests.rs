use super::*;
use std::sync::atomic::{AtomicI64, AtomicUsize};

struct Clock(AtomicI64);
impl MobileClock for Clock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::Acquire)
    }
}
struct Fixture {
    root: tempfile::TempDir,
    path: PathBuf,
    clock: Arc<Clock>,
    launch: MobileLaunch,
    authority: MobileAuthority,
}
impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("private/devices.json");
        let clock = Arc::new(Clock(AtomicI64::new(1_000_000)));
        let launch = MobileLaunch::isolated();
        let authority =
            MobileAuthority::open_with_clock(path.clone(), launch.clone(), clock.clone())
                .await
                .unwrap();
        Self {
            root,
            path,
            clock,
            launch,
            authority,
        }
    }
    fn offer(&self, scopes: MobileScopes, lifetime: Duration) -> MobilePairingOffer {
        self.authority
            .begin_pairing(
                &url::Url::parse("https://localhost/").unwrap(),
                scopes,
                lifetime,
            )
            .unwrap()
    }
    async fn pair(&self, scopes: MobileScopes) -> (MobilePairResult, MobileLease) {
        let offer = self.offer(scopes, Duration::from_secs(60));
        let input = MobilePairInput {
            secret: secret(&offer),
            name: "Synthetic phone".into(),
        };
        let result = self.authority.pair(input).await.unwrap();
        let lease = self.authority.authenticate(&result.token).unwrap();
        (result, lease)
    }
    async fn reopen(&self) -> MobileAuthority {
        self.authority.release().await;
        MobileAuthority::open_with_clock(self.path.clone(), self.launch.clone(), self.clock.clone())
            .await
            .unwrap()
    }
}
fn scopes() -> MobileScopes {
    [
        MobileScope::Read,
        MobileScope::Prompt,
        MobileScope::Approve,
        MobileScope::Diff,
    ]
    .into_iter()
    .collect()
}
fn secret(offer: &MobilePairingOffer) -> String {
    url::Url::parse(&offer.url)
        .unwrap()
        .fragment()
        .unwrap()
        .strip_prefix("pair=")
        .unwrap()
        .to_string()
}
fn command(sequence: u64) -> MobileCommand {
    MobileCommand {
        id: Uuid::new_v4(),
        sequence,
        action: MobileAction::Prompt {
            thread_id: "synthetic-thread".into(),
            text: "Inspect the synthetic fixture".into(),
        },
    }
}
fn ticket(reservation: MobileReservation) -> MobileCommandTicket {
    match reservation {
        MobileReservation::New(ticket) => ticket,
        MobileReservation::Existing(_) => panic!("Expected a new command reservation"),
    }
}

#[tokio::test]
async fn mobile_window_owners_never_inherit_devices_but_share_the_fail_closed_boundary() {
    let f = Fixture::new().await;
    let (issued, _) = f.pair(scopes()).await;
    let second = MobileLaunch(Arc::new(LaunchState {
        id: Uuid::new_v4(),
        failure: f.launch.0.failure.clone(),
    }));
    f.authority.release().await;
    let other = MobileAuthority::open_with_clock(f.path.clone(), second.clone(), f.clock.clone())
        .await
        .unwrap();
    assert!(other.authenticate(&issued.token).is_err());
    second.poison();
    assert!(matches!(f.launch.require(), Err(MobileError::Storage)));
    assert!(other.healthy().is_err());
}

#[tokio::test]
async fn mobile_completion_is_saved_before_a_later_sequence_and_uuid_reuse_cannot_clear_its_guard()
{
    let f = Fixture::new().await;
    let (_, lease) = f.pair(scopes()).await;
    let first = command(1);
    let reserved = ticket(f.authority.reserve(&lease, &first).await.unwrap());
    f.authority
        .commit(&lease, &reserved, |_| {
            Ok(MobileCommandOutcome::Accepted { queued: false })
        })
        .unwrap();
    let (entered, resume) = storage::pause_next(&f.path);
    let authority = f.authority.clone();
    let finish_lease = lease.clone();
    let finishing = reserved.clone();
    let task = tokio::spawn(async move { authority.finish(&finish_lease, &finishing).await });
    entered.await.unwrap();
    let mut second = command(2);
    second.id = first.id;
    assert!(matches!(
        f.authority.reserve(&lease, &second).await,
        Err(MobileError::Conflict(_))
    ));
    resume.send(()).unwrap();
    let receipt = task.await.unwrap().unwrap();
    assert_eq!(receipt.id, first.id);
    assert!(matches!(
        receipt.outcome,
        MobileCommandOutcome::Accepted { .. }
    ));
    let next = ticket(f.authority.reserve(&lease, &second).await.unwrap());
    drop(reserved);
    f.authority
        .commit(&lease, &next, |_| {
            Ok(MobileCommandOutcome::Accepted { queued: true })
        })
        .unwrap();
    assert!(matches!(
        f.authority.reserve(&lease, &command(3)).await,
        Err(MobileError::Conflict(_))
    ));
    assert_eq!(f.authority.finish(&lease, &next).await.unwrap().sequence, 2);
    assert!(matches!(
        f.authority.reserve(&lease, &command(3)).await,
        Ok(MobileReservation::New(_))
    ));
}

#[tokio::test]
async fn mobile_pairing_is_atomic_one_use_and_catalog_contains_no_raw_token() {
    let f = Fixture::new().await;
    let offer = f.offer(scopes(), Duration::from_secs(60));
    let nonce = secret(&offer);
    let (a, b) = tokio::join!(
        f.authority.pair(MobilePairInput {
            secret: nonce.clone(),
            name: "Phone A".into()
        }),
        f.authority.pair(MobilePairInput {
            secret: nonce.clone(),
            name: "Phone B".into()
        })
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    let issued = if let Ok(issued) = a {
        issued
    } else {
        b.unwrap()
    };
    assert_ne!(issued.token, nonce);
    assert!(f.authority.authenticate(&issued.token).is_ok());
    assert!(f
        .authority
        .pair(MobilePairInput {
            secret: nonce.clone(),
            name: "Replay".into()
        })
        .await
        .is_err());
    assert_eq!(f.authority.devices().unwrap().len(), 1);
    let bytes = std::fs::read_to_string(&f.path).unwrap();
    assert!(!bytes.contains(&issued.token));
    assert!(!bytes.contains(&nonce));
    assert!(bytes.contains("token_digest"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&f.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(f.path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }
}
#[tokio::test]
async fn mobile_offers_expire_replace_and_reject_unsafe_origin_or_name() {
    let f = Fixture::new().await;
    let old = f.offer(scopes(), Duration::from_secs(60));
    let offer = f.offer(scopes(), Duration::from_secs(60));
    assert!(!f.authority.pairing_pending(old.id));
    assert!(f
        .authority
        .pair(MobilePairInput {
            secret: secret(&old),
            name: "Old".into()
        })
        .await
        .is_err());
    assert!(f
        .authority
        .pair(MobilePairInput {
            secret: secret(&offer),
            name: "hidden\u{202e}name".into()
        })
        .await
        .is_err());
    assert!(f.authority.pairing_pending(offer.id));
    f.clock
        .0
        .fetch_add(duration_ms(PAIRING_LIFETIME).unwrap(), Ordering::AcqRel);
    assert!(!f.authority.pairing_pending(offer.id));
    assert!(f
        .authority
        .pair(MobilePairInput {
            secret: secret(&offer),
            name: "Expired".into()
        })
        .await
        .is_err());
    assert!(f
        .authority
        .begin_pairing(
            &url::Url::parse("http://localhost/").unwrap(),
            scopes(),
            Duration::from_secs(1)
        )
        .is_err());
    assert!(f
        .authority
        .begin_pairing(
            &url::Url::parse("https://localhost/").unwrap(),
            scopes(),
            Duration::from_nanos(1)
        )
        .is_err());
}
#[tokio::test]
async fn mobile_scopes_expiry_and_revocation_are_rechecked_before_dispatch() {
    let f = Fixture::new().await;
    let (issued, lease) = f.pair([MobileScope::Read].into_iter().collect()).await;
    assert!(matches!(
        lease.with_scope(MobileScope::Diff, || panic!(
            "A missing scope must not touch native state"
        )),
        Err(MobileError::Forbidden)
    ));
    assert!(matches!(
        f.authority.reserve(&lease, &command(1)).await,
        Err(MobileError::Forbidden)
    ));
    assert_eq!(f.authority.session(&lease).unwrap().next_sequence, 1);
    f.clock.0.fetch_add(60_000, Ordering::AcqRel);
    assert!(f.authority.authenticate(&issued.token).is_err());
    assert!(lease.with_scope(MobileScope::Read, || ()).is_err());
    let (_, lease) = f.pair(scopes()).await;
    let queued = ticket(f.authority.reserve(&lease, &command(1)).await.unwrap());
    f.authority.revoke_now(lease.id()).unwrap();
    assert!(f
        .authority
        .commit(&lease, &queued, |_| panic!(
            "A queued command must not run after revocation"
        ))
        .is_err());
    f.authority.persist_revocation().await.unwrap();
    assert!(lease.check(MobileScope::Read).is_err());
}
#[tokio::test]
async fn mobile_mutations_are_immutable_and_retries_never_dispatch_twice() {
    let f = Fixture::new().await;
    let (_, lease) = f.pair(scopes()).await;
    let original = command(1);
    let mut changed = original.clone();
    let reserved = ticket(f.authority.reserve(&lease, &original).await.unwrap());
    assert!(matches!(
        f.authority.reserve(&lease, &original).await.unwrap(),
        MobileReservation::Existing(MobileCommandReceipt {
            outcome: MobileCommandOutcome::Pending,
            ..
        })
    ));
    if let MobileAction::Prompt { text, .. } = &mut changed.action {
        *text = "A different payload".into();
    }
    assert!(f.authority.reserve(&lease, &changed).await.is_err());
    let calls = AtomicUsize::new(0);
    let receipt = f
        .authority
        .commit(&lease, &reserved, |action| {
            assert_eq!(action, &original.action);
            calls.fetch_add(1, Ordering::AcqRel);
            Ok(MobileCommandOutcome::Accepted { queued: true })
        })
        .unwrap();
    assert!(matches!(
        receipt.outcome,
        MobileCommandOutcome::Accepted { queued: true }
    ));
    f.authority.finish(&lease, &reserved).await.unwrap();
    assert!(f
        .authority
        .commit(&lease, &reserved, |_| panic!(
            "Never replay an accepted command"
        ))
        .is_err());
    assert!(matches!(
        f.authority.reserve(&lease, &original).await.unwrap(),
        MobileReservation::Existing(MobileCommandReceipt {
            outcome: MobileCommandOutcome::Accepted { .. },
            ..
        })
    ));
    assert_eq!(calls.load(Ordering::Acquire), 1);
    assert!(f.authority.reserve(&lease, &command(3)).await.is_err());
    let next = command(2);
    let reserved = ticket(f.authority.reserve(&lease, &next).await.unwrap());
    f.authority
        .commit(&lease, &reserved, |_| {
            Ok(MobileCommandOutcome::Accepted { queued: false })
        })
        .unwrap();
    f.authority.finish(&lease, &reserved).await.unwrap();
    assert!(f.authority.reserve(&lease, &original).await.is_err());
}
#[tokio::test]
async fn mobile_listener_restart_preserves_receipts_and_process_restart_invalidates_tokens() {
    let f = Fixture::new().await;
    let (issued, lease) = f.pair(scopes()).await;
    let request = command(1);
    let reserved = ticket(f.authority.reserve(&lease, &request).await.unwrap());
    f.authority
        .commit(&lease, &reserved, |_| {
            Ok(MobileCommandOutcome::Accepted { queued: false })
        })
        .unwrap();
    f.authority.finish(&lease, &reserved).await.unwrap();
    let reopened = f.reopen().await;
    let lease = reopened.authenticate(&issued.token).unwrap();
    assert!(matches!(
        reopened.reserve(&lease, &request).await.unwrap(),
        MobileReservation::Existing(MobileCommandReceipt {
            outcome: MobileCommandOutcome::Accepted { .. },
            ..
        })
    ));
    reopened.release().await;
    let restart =
        MobileAuthority::open_with_clock(f.path.clone(), MobileLaunch::isolated(), f.clock.clone())
            .await
            .unwrap();
    assert!(restart.authenticate(&issued.token).is_err());
    assert!(restart.devices().unwrap()[0].revoked);
}
#[tokio::test]
async fn mobile_abandoned_reservation_recovers_as_unconfirmed_without_replay() {
    let f = Fixture::new().await;
    let (issued, lease) = f.pair(scopes()).await;
    let request = command(1);
    let reserved = ticket(f.authority.reserve(&lease, &request).await.unwrap());
    drop(reserved);
    assert!(matches!(
        f.authority
            .session(&lease)
            .unwrap()
            .last_command
            .unwrap()
            .outcome,
        MobileCommandOutcome::Unconfirmed
    ));
    let reopened = f.reopen().await;
    let lease = reopened.authenticate(&issued.token).unwrap();
    assert!(matches!(
        reopened.reserve(&lease, &request).await.unwrap(),
        MobileReservation::Existing(MobileCommandReceipt {
            outcome: MobileCommandOutcome::Unconfirmed,
            ..
        })
    ));
    assert!(matches!(
        reopened.reserve(&lease, &command(2)).await.unwrap(),
        MobileReservation::New(_)
    ));
}
#[tokio::test]
async fn mobile_storage_failure_blocks_the_launch_and_old_disk_tokens_cannot_recover() {
    let f = Fixture::new().await;
    let (issued, lease) = f.pair(scopes()).await;
    let old = std::fs::read(&f.path).unwrap();
    f.authority.revoke_now(lease.id()).unwrap();
    std::fs::remove_file(&f.path).unwrap();
    std::fs::create_dir(&f.path).unwrap();
    assert!(f.authority.persist_revocation().await.is_err());
    assert!(f.authority.healthy().is_err());
    assert!(f.authority.authenticate(&issued.token).is_err());
    f.authority.release().await;
    std::fs::remove_dir(&f.path).unwrap();
    std::fs::write(&f.path, old).unwrap();
    assert!(
        MobileAuthority::open_with_clock(f.path.clone(), f.launch.clone(), f.clock.clone())
            .await
            .is_err()
    );
    let restart =
        MobileAuthority::open_with_clock(f.path.clone(), MobileLaunch::isolated(), f.clock.clone())
            .await
            .unwrap();
    assert!(restart.authenticate(&issued.token).is_err());
}
#[tokio::test]
async fn mobile_unflushed_revocation_cannot_resurrect_in_the_same_launch() {
    let f = Fixture::new().await;
    let (_, lease) = f.pair(scopes()).await;
    f.authority.revoke_now(lease.id()).unwrap();
    f.authority.release().await;
    assert!(
        MobileAuthority::open_with_clock(f.path.clone(), f.launch.clone(), f.clock.clone())
            .await
            .is_err()
    );
}
#[tokio::test]
async fn mobile_cancelled_writer_stays_serialized_until_its_atomic_write_finishes() {
    let f = Fixture::new().await;
    let (issued, lease) = f.pair(scopes()).await;
    let (entered, resume) = storage::pause_next(&f.path);
    let first = f.authority.clone();
    let first = tokio::spawn(async move { first.persist().await });
    tokio::time::timeout(Duration::from_secs(2), entered)
        .await
        .unwrap()
        .unwrap();
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    f.authority.revoke_now(lease.id()).unwrap();
    let second = f.authority.clone();
    let mut second = tokio::spawn(async move { second.persist_revocation().await });
    assert!(tokio::time::timeout(Duration::from_millis(50), &mut second)
        .await
        .is_err());
    resume.send(()).unwrap();
    second.await.unwrap().unwrap();
    let reopened = f.reopen().await;
    assert!(reopened.authenticate(&issued.token).is_err());
    assert!(reopened.devices().unwrap()[0].revoked);
}
#[tokio::test]
async fn mobile_owner_lock_and_corrupt_catalog_fail_closed() {
    let f = Fixture::new().await;
    let (_, lease) = f.pair(scopes()).await;
    assert!(
        MobileAuthority::open_with_clock(f.path.clone(), f.launch.clone(), f.clock.clone())
            .await
            .is_err()
    );
    f.authority.revoke_now(lease.id()).unwrap();
    f.authority.persist_revocation().await.unwrap();
    f.authority.release().await;
    std::fs::write(&f.path, b"{ broken private catalog").unwrap();
    assert!(
        MobileAuthority::open_with_clock(f.path.clone(), f.launch.clone(), f.clock.clone())
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(&f.path).unwrap(), b"{ broken private catalog");
    assert!(f.root.path().is_dir());
}

#[cfg(unix)]
#[tokio::test]
async fn mobile_listener_release_unlocks_an_inherited_open_file_description() {
    let f = Fixture::new().await;
    let (issued, _) = f.pair(scopes()).await;
    let inherited = f
        .authority
        .0
        .owner
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .try_clone()
        .unwrap();
    let reopened = f.reopen().await;
    assert!(reopened.authenticate(&issued.token).is_ok());
    drop(inherited);
}
