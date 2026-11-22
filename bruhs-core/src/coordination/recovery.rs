use super::*;
use bruhs_models::agent_extensions::QuotaWindow;

pub struct ResetWorker(tokio::task::JoinHandle<()>);
impl Drop for ResetWorker {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl CoordinationRuntime {
    /// Scheduling waits for exact provider timestamps, with no polling timer.
    pub fn start_reset_worker(&self) -> ResetWorker {
        let runtime = self.clone();
        ResetWorker(tokio::spawn(async move {
            let mut changes = runtime.changes();
            loop {
                let next = {
                    let data = runtime.0.data.lock().await;
                    data.tasks
                        .values()
                        .filter_map(|task| task.resume_at.map(|at| (at, task.id.clone())))
                        .min_by_key(|(at, _)| *at)
                };
                match next {
                    Some((at, id)) => {
                        let delay = at.as_millis().saturating_sub(EpochMs::now().as_millis());
                        tokio::select! {
                            _ = changes.changed() => continue,
                            _ = tokio::time::sleep(std::time::Duration::from_millis(u64::try_from(delay.max(0)).unwrap_or_default())) => {}
                        }
                        if let Err(error) = runtime.resume_due(&id, EpochMs::now()).await {
                            tracing::warn!(%error, task=%id, "scheduled task continuation did not run");
                            let mut data = runtime.0.data.lock().await;
                            if let Some(task) = data.tasks.get_mut(&id) {
                                task.resume_at = None;
                                task.error = Some(format!(
                                    "Scheduled continuation failed: {error}. Send a message to retry."
                                ));
                            }
                            let result = runtime.persist(&mut data).await;
                            drop(data);
                            if let Err(error) = result {
                                tracing::error!(%error, "could not save continuation failure");
                                break;
                            }
                        }
                    }
                    None => {
                        if changes.changed().await.is_err() {
                            break;
                        }
                    }
                }
            }
        }))
    }

    pub async fn schedule_reset(&self, id: &ThreadId) -> Result<(), CoordinationError> {
        let task = self.task(id).await?;
        if task.state.active() || task.state == TaskState::Ready {
            return Err(conflict(
                "Wait for this task to stop before scheduling a quota continuation.",
            ));
        }
        let provider = provider_for_model_id(&task.model).as_str();
        let at = provider_quota_reset(provider).await.map_err(storage)?
            .ok_or_else(|| invalid("This provider has no exhausted quota with a future reset time. Refresh Usage or resume manually."))?;
        let mut data = self.0.data.lock().await;
        let task = data.tasks.get_mut(id).ok_or_else(|| not_found(id))?;
        if task.state.active() || task.state == TaskState::Ready {
            return Err(conflict(
                "The task resumed while checking its quota. No continuation was scheduled.",
            ));
        }
        task.resume_at = Some(at);
        let result = self.persist(&mut data).await;
        drop(data);
        result
    }

    async fn resume_due(&self, id: &ThreadId, now: EpochMs) -> Result<(), CoordinationError> {
        let task = self.task(id).await?;
        if task.resume_at.is_none_or(|at| at > now) {
            return Ok(());
        }
        self.send(id, "Continue the task after the provider-reported usage reset. First inspect the current files and prior result before repeating a command.".into()).await
    }
}

/// The last exhausted window for the selected account must have a reported future reset.
pub async fn provider_quota_reset(provider: &str) -> std::io::Result<Option<EpochMs>> {
    let limits = match provider {
        "codex" => crate::agent_extensions::refresh_codex().await?,
        "claude" => tokio::task::spawn_blocking(|| {
            let root = crate::agent_extensions::directory()?;
            crate::agent_extensions::snapshot_at(&root).map(|snapshot| snapshot.limits)
        })
        .await
        .map_err(std::io::Error::other)??,
        _ => return Ok(None),
    };
    Ok(reset_time(
        &limits,
        provider,
        chrono::Utc::now().timestamp(),
    ))
}

fn reset_time(limits: &[QuotaWindow], provider: &str, now_seconds: i64) -> Option<EpochMs> {
    // All exhausted windows must refill before the current account is usable.
    // Multiple accounts cannot be safely routed by guessing which owns a turn.
    let accounts: std::collections::BTreeSet<_> = limits
        .iter()
        .filter(|window| window.provider == provider)
        .map(|window| &window.account)
        .collect();
    if accounts.len() != 1 {
        return None;
    }
    let exhausted: Vec<_> = limits
        .iter()
        .filter(|window| window.provider == provider && window.used_percent >= 100.0)
        .collect();
    if exhausted.is_empty()
        || exhausted
            .iter()
            .any(|window| window.resets_at.is_none_or(|at| at <= now_seconds))
    {
        return None;
    }
    exhausted
        .iter()
        .filter_map(|window| window.resets_at)
        .max()?
        .checked_mul(1000)
        .and_then(|at| EpochMs::new(at).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn window(name: &str, used_percent: f64, resets_at: Option<i64>) -> QuotaWindow {
        QuotaWindow {
            window_id: None,
            duration_minutes: None,
            provider: "claude".into(),
            account: "current".into(),
            name: name.into(),
            used_percent,
            resets_at,
            observed_at: 100,
        }
    }
    #[test]
    fn wait_for_every_exhausted_window_and_never_guess_missing_resets() {
        assert_eq!(
            reset_time(
                &[
                    window("session", 100., Some(200)),
                    window("week", 100., Some(400))
                ],
                "claude",
                100
            )
            .unwrap()
            .as_millis(),
            400_000
        );
        assert!(reset_time(&[window("session", 100., None)], "claude", 100).is_none());
        assert!(reset_time(&[window("session", 20., Some(200))], "claude", 100).is_none());
    }
}
