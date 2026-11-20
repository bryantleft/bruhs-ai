//! Owner-declared duration and a terminal signal independent of output backpressure.

use super::{OrchestratorEvent, SendMessageError};
use std::task::{Context, Poll};
use tokio::sync::{mpsc, oneshot};

pub(super) fn deadline(
    seconds: Option<u64>,
) -> Result<Option<(tokio::time::Instant, u64)>, SendMessageError> {
    seconds
        .map(|seconds| {
            if seconds == 0 {
                return Err(SendMessageError::InvalidExecutionBudget);
            }
            std::time::Instant::now()
                .checked_add(std::time::Duration::from_secs(seconds))
                .map(|at| (tokio::time::Instant::from_std(at), seconds))
                .ok_or(SendMessageError::InvalidExecutionBudget)
        })
        .transpose()
}

pub(super) fn poll_next(
    output: &mut mpsc::Receiver<OrchestratorEvent>,
    terminal: &mut Option<oneshot::Receiver<OrchestratorEvent>>,
    finished: &mut bool,
    cx: &mut Context<'_>,
) -> Poll<Option<OrchestratorEvent>> {
    if *finished {
        return Poll::Ready(None);
    }
    let output_poll = output.poll_recv(cx);
    if let Poll::Ready(Some(event)) = output_poll {
        return Poll::Ready(Some(event));
    }
    if let Some(receiver) = terminal {
        use std::future::Future;
        match std::pin::Pin::new(receiver).poll(cx) {
            Poll::Ready(Ok(event)) => {
                *terminal = None;
                *finished = true;
                output.close();
                return Poll::Ready(Some(event));
            }
            Poll::Ready(Err(_)) => *terminal = None,
            Poll::Pending => return Poll::Pending,
        }
    }
    if matches!(output_poll, Poll::Ready(None)) {
        *finished = true;
        Poll::Ready(None)
    } else {
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_is_explicit_and_invalid_budgets_fail_before_start() {
        assert!(deadline(None).unwrap().is_none());
        assert!(deadline(Some(0)).is_err());
        assert!(deadline(Some(u64::MAX)).is_err());
        assert_eq!(deadline(Some(1)).unwrap().unwrap().1, 1);
    }

    #[tokio::test]
    async fn partial_output_is_drained_before_one_terminal_budget_event() {
        let (sender, mut output) = mpsc::channel(2);
        let (terminal_sender, terminal_receiver) = oneshot::channel();
        sender
            .send(OrchestratorEvent::Query(super::super::QueryEvent::Text(
                "retained partial".into(),
            )))
            .await
            .unwrap();
        terminal_sender
            .send(OrchestratorEvent::ExecutionBudgetExceeded { seconds: 1 })
            .unwrap();
        let mut terminal = Some(terminal_receiver);
        let mut finished = false;
        let first =
            std::future::poll_fn(|cx| poll_next(&mut output, &mut terminal, &mut finished, cx))
                .await;
        assert!(matches!(
            first,
            Some(OrchestratorEvent::Query(super::super::QueryEvent::Text(_)))
        ));
        let last =
            std::future::poll_fn(|cx| poll_next(&mut output, &mut terminal, &mut finished, cx))
                .await;
        assert!(matches!(
            last,
            Some(OrchestratorEvent::ExecutionBudgetExceeded { seconds: 1 })
        ));
        assert!(sender
            .send(OrchestratorEvent::Query(super::super::QueryEvent::Text(
                "late".into()
            )))
            .await
            .is_err());
        assert!(std::future::poll_fn(|cx| poll_next(
            &mut output,
            &mut terminal,
            &mut finished,
            cx
        ))
        .await
        .is_none());
    }

    #[tokio::test]
    async fn dropping_an_expired_producer_settles_only_its_registered_tasks() {
        use super::super::{tests::turn_context, BackgroundTaskRegistry, QueryEvent, Sessions};
        use bruhs_models::ids::ThreadId;
        use std::{
            collections::HashMap,
            sync::{Arc, Mutex},
        };
        let sessions: Sessions = Arc::new(Mutex::new(HashMap::new()));
        let registry = Arc::new(BackgroundTaskRegistry::new());
        let thread = ThreadId::new_v4();
        let (turn, _receiver) = turn_context(&sessions, &registry, &thread, 1);
        assert!(
            turn.forward(QueryEvent::ToolCall {
                id: "owned-budget-task".into(),
                tool: "Bash".into(),
                input: serde_json::json!({"command":"fixture","run_in_background":true})
            })
            .await
        );
        let ids = turn.tasks.lock().unwrap().clone();
        assert_eq!(ids.len(), 1);
        drop(turn);
        assert_eq!(
            registry.get(&ids[0]).unwrap().status,
            crate::models::TaskStatus::Killed
        );
    }
}
