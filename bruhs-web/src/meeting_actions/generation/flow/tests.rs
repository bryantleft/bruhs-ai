use super::*;
use bruhs_models::{SummaryGenerationRequest, SummaryGenerationState};
use std::{
    cell::{Cell, RefCell},
    future::Future,
    task::{Context, Poll},
};

struct Fixture {
    events: RefCell<Vec<&'static str>>,
    saved: RefCell<Option<Checkpoint>>,
    save_ready: Cell<bool>,
    fail: Option<&'static str>,
    existing: Option<Checkpoint>,
    current: bool,
    receipt: Option<SummaryGenerationReceipt>,
    cached_expectations: RefCell<Vec<Option<MeetingSummary>>>,
}

fn checkpoint() -> Checkpoint {
    Checkpoint {
        id: "owned-web-meeting".into(),
        minimum_version: 20,
        request: SummaryGenerationRequest::new("owned-web-meeting".into(), true, false),
    }
}

fn summary() -> MeetingSummary {
    MeetingSummary {
        tldr: "Saved original".into(),
        action_items: vec![],
        decisions: vec![],
        topics: vec![],
        generated_by: "synthetic".into(),
        generated_at: 21,
    }
}

impl Fixture {
    fn new(checkpoint: &Checkpoint) -> Self {
        Self {
            events: RefCell::new(vec![]),
            saved: RefCell::new(None),
            save_ready: Cell::new(true),
            fail: None,
            existing: None,
            current: true,
            cached_expectations: RefCell::new(vec![]),
            receipt: Some(SummaryGenerationReceipt {
                request: checkpoint.request.clone(),
                state: SummaryGenerationState::Succeeded { summary: summary() },
            }),
        }
    }
    fn event(&self, event: &'static str) -> Result<(), String> {
        self.events.borrow_mut().push(event);
        if self.fail == Some(event) {
            Err(format!("Owned {event} failure"))
        } else {
            Ok(())
        }
    }
}

impl Transport for Fixture {
    async fn reserve(&self, checkpoint: &Checkpoint) -> Result<generations::Reservation, String> {
        self.event("reserve")?;
        futures_util::future::poll_fn(|_| {
            if self.save_ready.get() {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await;
        if let Some(existing) = &self.existing {
            return Ok(generations::Reservation::Existing(existing.clone()));
        }
        *self.saved.borrow_mut() = Some(checkpoint.clone());
        Ok(generations::Reservation::Started)
    }
    fn current_matches(&self, _: &Checkpoint) -> bool {
        self.events.borrow_mut().push("preflight");
        self.current
    }
    async fn send(&self, checkpoint: &Checkpoint) -> Result<MeetingSummary, String> {
        assert_eq!(
            self.saved.borrow().as_ref(),
            Some(checkpoint),
            "exact intent acknowledged before POST"
        );
        self.event("send")?;
        Ok(summary())
    }
    async fn receipt(&self, _: &Checkpoint) -> Result<Option<SummaryGenerationReceipt>, String> {
        self.event("receipt")?;
        Ok(self.receipt.clone())
    }
    async fn cache(&self, _: &Checkpoint, expected: Option<MeetingSummary>) -> Result<(), String> {
        self.event("cache")?;
        self.cached_expectations.borrow_mut().push(expected);
        Ok(())
    }
    async fn clear(&self, checkpoint: &Checkpoint) -> Result<(), String> {
        self.event("clear")?;
        assert_eq!(self.saved.borrow().as_ref(), Some(checkpoint));
        *self.saved.borrow_mut() = None;
        Ok(())
    }
    async fn cancel(&self, _: &Checkpoint) -> Result<Option<SummaryGenerationReceipt>, String> {
        self.event("cancel")?;
        Ok(self.receipt.clone())
    }
}

fn run<T>(future: impl Future<Output = T>) -> T {
    let mut future = std::pin::pin!(future);
    let waker = futures_util::task::noop_waker();
    match future.as_mut().poll(&mut Context::from_waker(&waker)) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("Fixture future unexpectedly pending"),
    }
}

#[test]
fn generation_waits_for_exact_intent_acknowledgment_before_post() {
    let checkpoint = checkpoint();
    let io = Fixture::new(&checkpoint);
    io.save_ready.set(false);
    let mut operation = std::pin::pin!(generate(&io, &checkpoint));
    let waker = futures_util::task::noop_waker();
    let mut context = Context::from_waker(&waker);
    assert!(operation.as_mut().poll(&mut context).is_pending());
    assert_eq!(*io.events.borrow(), ["reserve"]);
    assert!(io.saved.borrow().is_none());
    io.save_ready.set(true);
    assert!(matches!(
        operation.as_mut().poll(&mut context),
        Poll::Ready(Ok(Generated::Saved))
    ));
    assert_eq!(
        *io.events.borrow(),
        ["reserve", "preflight", "send", "cache", "clear"]
    );
    assert_eq!(*io.cached_expectations.borrow(), [Some(summary())]);
}

#[test]
fn failed_intent_save_never_contacts_provider() {
    let checkpoint = checkpoint();
    let mut io = Fixture::new(&checkpoint);
    io.fail = Some("reserve");
    let failure = run(generate(&io, &checkpoint)).err().unwrap();
    assert!(failure.checkpoint.is_none());
    assert_eq!(*io.events.borrow(), ["reserve"]);
}

#[test]
fn existing_intent_is_recovered_instead_of_replayed_or_replaced() {
    let checkpoint = checkpoint();
    let mut io = Fixture::new(&checkpoint);
    let existing = super::tests::checkpoint();
    io.existing = Some(existing.clone());
    let failure = run(generate(&io, &checkpoint)).err().unwrap();
    assert_eq!(failure.checkpoint, Some(existing));
    assert_eq!(*io.events.borrow(), ["reserve"]);
}

#[test]
fn changed_preflight_clears_only_known_unsent_intent_without_post() {
    let checkpoint = checkpoint();
    let mut io = Fixture::new(&checkpoint);
    io.current = false;
    assert!(matches!(
        run(generate(&io, &checkpoint)),
        Ok(Generated::NotSent)
    ));
    assert_eq!(*io.events.borrow(), ["reserve", "preflight", "clear"]);
    assert!(io.saved.borrow().is_none());
}

#[test]
fn uncertain_generation_and_cache_or_clear_failures_preserve_same_request() {
    for fail in ["send", "cache", "clear"] {
        let checkpoint = checkpoint();
        let mut io = Fixture::new(&checkpoint);
        io.fail = Some(fail);
        let failure = run(generate(&io, &checkpoint)).err().unwrap();
        assert_eq!(failure.checkpoint, Some(checkpoint.clone()));
        assert_eq!(*io.saved.borrow(), Some(checkpoint.clone()));
        assert_eq!(
            io.events
                .borrow()
                .iter()
                .filter(|event| **event == "send")
                .count(),
            1
        );
        io.fail = None;
        run(recover(&io, &checkpoint)).unwrap();
        assert_eq!(
            io.events
                .borrow()
                .iter()
                .filter(|event| **event == "send")
                .count(),
            1
        );
        assert!(io.saved.borrow().is_none());
    }
}

#[test]
fn unresolved_receipts_never_cache_or_clear_recovery_intent() {
    for scenario in 0..4 {
        let checkpoint = checkpoint();
        let mut io = Fixture::new(&checkpoint);
        *io.saved.borrow_mut() = Some(checkpoint.clone());
        match scenario {
            0 => io.receipt = None,
            1 => io.receipt.as_mut().unwrap().state = SummaryGenerationState::Pending,
            2 => io.receipt.as_mut().unwrap().request.request_id = uuid::Uuid::new_v4().to_string(),
            _ => io.fail = Some("receipt"),
        }
        assert!(run(recover(&io, &checkpoint)).is_err());
        assert_eq!(*io.events.borrow(), ["receipt"]);
        assert_eq!(*io.saved.borrow(), Some(checkpoint));
    }
}

#[test]
fn recovery_reads_latest_instead_of_overwriting_with_historical_summary() {
    for state in [
        SummaryGenerationState::Failed,
        SummaryGenerationState::Cancelled,
        SummaryGenerationState::Succeeded { summary: summary() },
    ] {
        let checkpoint = checkpoint();
        let mut io = Fixture::new(&checkpoint);
        *io.saved.borrow_mut() = Some(checkpoint.clone());
        io.receipt.as_mut().unwrap().state = state;
        run(recover(&io, &checkpoint)).unwrap();
        assert_eq!(*io.events.borrow(), ["receipt", "cache", "clear"]);
        assert_eq!(*io.cached_expectations.borrow(), [None]);
    }
}

#[test]
fn failed_recovery_cache_never_clears_intent() {
    let checkpoint = checkpoint();
    let mut io = Fixture::new(&checkpoint);
    io.fail = Some("cache");
    *io.saved.borrow_mut() = Some(checkpoint.clone());
    assert!(run(recover(&io, &checkpoint)).is_err());
    assert_eq!(*io.events.borrow(), ["receipt", "cache"]);
    assert_eq!(*io.saved.borrow(), Some(checkpoint));
}

#[test]
fn cancelled_receipt_clears_intent_without_reading_the_meeting() {
    let checkpoint = checkpoint();
    let mut io = Fixture::new(&checkpoint);
    *io.saved.borrow_mut() = Some(checkpoint.clone());
    io.receipt.as_mut().unwrap().state = SummaryGenerationState::Cancelled;
    assert_eq!(run(cancel(&io, &checkpoint)).unwrap(), Cancelled::Fenced);
    assert_eq!(*io.events.borrow(), ["cancel", "clear"]);
    assert!(io.saved.borrow().is_none());
}

#[test]
fn cancelling_a_finished_request_recovers_latest_instead_of_discarding_it() {
    for state in [
        SummaryGenerationState::Failed,
        SummaryGenerationState::Succeeded { summary: summary() },
    ] {
        let checkpoint = checkpoint();
        let mut io = Fixture::new(&checkpoint);
        *io.saved.borrow_mut() = Some(checkpoint.clone());
        io.receipt.as_mut().unwrap().state = state;
        assert_eq!(
            run(cancel(&io, &checkpoint)).unwrap(),
            Cancelled::AlreadyFinished
        );
        assert_eq!(*io.events.borrow(), ["cancel", "cache", "clear"]);
        assert_eq!(*io.cached_expectations.borrow(), [None]);
        assert!(io.saved.borrow().is_none());
    }
}

#[test]
fn unconfirmed_cancellations_retain_the_saved_request() {
    for scenario in 0..5 {
        let checkpoint = checkpoint();
        let mut io = Fixture::new(&checkpoint);
        *io.saved.borrow_mut() = Some(checkpoint.clone());
        io.receipt.as_mut().unwrap().state = SummaryGenerationState::Cancelled;
        match scenario {
            0 => io.receipt = None,
            1 => io.receipt.as_mut().unwrap().state = SummaryGenerationState::Pending,
            2 => io.receipt.as_mut().unwrap().request.request_id = uuid::Uuid::new_v4().to_string(),
            3 => io.fail = Some("cancel"),
            _ => io.fail = Some("clear"),
        }
        assert!(run(cancel(&io, &checkpoint)).is_err(), "{scenario}");
        let expected: &[&str] = if scenario == 4 {
            &["cancel", "clear"]
        } else {
            &["cancel"]
        };
        assert_eq!(*io.events.borrow(), expected, "{scenario}");
        assert_eq!(*io.saved.borrow(), Some(checkpoint), "{scenario}");
    }
}
