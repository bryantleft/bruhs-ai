use super::{DeliveryError, DeliveryResult, DeliveryService};
use bruhs_models::delivery::*;
use std::{
    path::Path,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

pub(super) fn reasons(before: &Pull, after: &Pull, owner: &str) -> Vec<String> {
    let mut reasons = Vec::new();
    if before.head_oid != after.head_oid {
        reasons.push("Head commit changed".into());
    }
    if before.state != after.state {
        reasons.push(format!("Pull request is {}", after.state.to_lowercase()));
    }
    if before.mergeable != after.mergeable && after.mergeable == "CONFLICTING" {
        reasons.push("Branch has merge conflicts".into());
    }
    for check in &after.checks {
        if check.failed()
            && !before.checks.iter().any(|previous| {
                previous.name == check.name
                    && previous.state == check.state
                    && previous.conclusion == check.conclusion
            })
        {
            reasons.push(format!("Check needs attention: {}", check.name));
        }
    }
    if !after.checks.is_empty()
        && after
            .checks
            .iter()
            .all(|check| !check.failed() && !check.pending())
        && before.checks.iter().any(Check::pending)
    {
        reasons.push("Checks finished without failures".into());
    }
    if after.comments.iter().any(|comment| {
        comment.author != owner
            && !before
                .comments
                .iter()
                .any(|previous| previous.id == comment.id)
    }) {
        reasons.push("New pull request comments".into());
    }
    if after.reviews.iter().any(|review| {
        review.author != owner
            && !before
                .reviews
                .iter()
                .any(|previous| previous.id == review.id)
    }) {
        reasons.push("New review".into());
    }
    reasons
}

impl DeliveryService {
    pub async fn watch_pull(&self, mut request: WatchRequest) -> DeliveryResult<PullWatch> {
        if request.poll_seconds == 0
            || request.poll_seconds > u64::try_from(i64::MAX / 1000).unwrap_or(u64::MAX)
        {
            return Err(DeliveryError::Invalid(
                "Choose a positive polling interval that fits a millisecond timestamp.".into(),
            ));
        }
        let path = tokio::fs::canonicalize(&request.path)
            .await
            .map_err(|error| DeliveryError::Invalid(error.to_string()))?;
        request.path = path.to_string_lossy().into_owned();
        let repository = self.forge_repository(&path, request.forge).await?;
        if repository.is_none() {
            return Err(DeliveryError::Invalid("A durable watch requires a credential-free repository origin. Add remote.origin.url, then review the watch again.".into()));
        }
        request.forge = repository
            .as_ref()
            .map_or(Forge::GitHub, |repository| repository.forge);
        let (pull, login) = tokio::try_join!(
            self.watch_snapshot(&path, request.number, request.forge),
            self.watch_login(&path, repository.as_ref())
        )?;
        let record = PullWatch {
            id: uuid::Uuid::new_v4().to_string(),
            request,
            repository,
            enabled: true,
            next_poll_at_ms: super::now(),
            previous: Some(pull),
            owner_login: login.trim().to_owned(),
            error: None,
        };
        self.mutate(|state| {
            if let Some(existing) = state.watches.iter_mut().find(|watch| {
                watch.request.path == record.request.path
                    && watch.request.number == record.request.number
            }) {
                let mut updated = record.clone();
                if existing.repository == record.repository
                    && existing.request.forge == record.request.forge
                {
                    updated.id.clone_from(&existing.id);
                }
                *existing = updated.clone();
                return Ok(updated);
            }
            if state.watches.len() >= bruhs_models::reactive::HISTORY_LIMIT {
                return Err(DeliveryError::Conflict(
                    "The watch page is full. Remove an existing watch before adding another."
                        .into(),
                ));
            }
            state.watches.push(record.clone());
            Ok(record)
        })
        .await
    }

    pub async fn remove_watch(&self, id: &str) -> DeliveryResult<()> {
        self.mutate(|state| {
            let index = state
                .watches
                .iter()
                .position(|watch| watch.id == id)
                .ok_or_else(|| DeliveryError::NotFound("Pull request watch not found.".into()))?;
            state.watches.remove(index);
            Ok(())
        })
        .await
    }

    pub async fn acknowledge_event(&self, id: &str) -> DeliveryResult<()> {
        self.mutate(|state| {
            let event = state
                .events
                .iter_mut()
                .find(|event| event.id == id)
                .ok_or_else(|| DeliveryError::NotFound("PR event not found.".into()))?;
            event.acknowledged = true;
            Ok(())
        })
        .await
    }

    async fn watch_login(
        &self,
        path: &Path,
        repository: Option<&ForgeRepository>,
    ) -> DeliveryResult<String> {
        if let Some(repository) = repository.filter(|repository| repository.forge == Forge::GitLab)
        {
            let text = self
                .read(
                    path,
                    "glab",
                    &["api", "user", "--hostname", &repository.host],
                )
                .await?;
            let value: serde_json::Value = serde_json::from_str(&text).map_err(|_| {
                DeliveryError::Command("GitLab returned an unreadable signed-in user.".into())
            })?;
            return value["username"]
                .as_str()
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| {
                    DeliveryError::Command(
                        "Sign in using glab auth login before adding a GitLab watch.".into(),
                    )
                });
        }
        self.github_read(path, &["api", "user", "--jq", ".login"])
            .await
    }

    async fn watch_snapshot(&self, path: &Path, number: u64, forge: Forge) -> DeliveryResult<Pull> {
        let (mut pull, comments) = tokio::try_join!(
            self.pull_for(path, number, forge),
            self.comments_for(path, number, forge)
        )?;
        pull.comments.extend(comments);
        Ok(pull)
    }

    pub async fn poll_watches(&self) -> DeliveryResult<()> {
        let watches = self
            .state
            .lock()
            .await
            .watches
            .iter()
            .filter(|watch| watch.enabled && watch.next_poll_at_ms <= super::now())
            .cloned()
            .collect::<Vec<_>>();
        for watch in watches {
            let path = Path::new(&watch.request.path);
            let result = async {
                let repository = watch.repository.as_ref().ok_or_else(|| DeliveryError::Conflict("This legacy watch has no frozen repository identity. Remove it and review a new watch before resuming task events.".into()))?;
                if self.forge_repository(path, repository.forge).await?.as_ref() != Some(repository) {
                    return Err(DeliveryError::Conflict("The watched repository origin changed. Remove this watch and review a new one.".into()));
                }
                self.watch_snapshot(path, watch.request.number, watch.request.forge).await
            }.await;
            self.mutate(|state| {
                let Some(current) = state.watches.iter_mut().find(|current| current.id == watch.id && current.request == watch.request) else { return Ok(()); };
                current.next_poll_at_ms = super::now().saturating_add(i64::try_from(watch.request.poll_seconds.saturating_mul(1000)).unwrap_or(i64::MAX));
                match result {
                    Ok(pull) => {
                        let changes = current.previous.as_ref().map(|previous| reasons(previous, &pull, &current.owner_login)).unwrap_or_default();
                        if !changes.is_empty() {
                            if state.events.iter().filter(|event| !event.acknowledged).count() >= bruhs_models::reactive::HISTORY_LIMIT {
                                current.enabled = false;
                                current.error = Some("Unacknowledged PR events fill the review page. Read/dismiss them, then enable this watch again.".into());
                                return Ok(());
                            }
                            state.events.push(WatchEvent {
                                id: uuid::Uuid::new_v4().to_string(), watch_id: watch.id.clone(), number: pull.number,
                                title: pull.title.clone(), reasons: changes, url: pull.url.clone(), created_at_ms: super::now(), delivered: false, acknowledged: false,
                                reactive_source_id: watch.request.reactive_source_id.clone(),
                            });
                        }
                        if matches!(pull.state.as_str(), "MERGED" | "CLOSED") { current.enabled = false; }
                        current.previous = Some(pull); current.error = None;
                    }
                    Err(error) => { if matches!(error, DeliveryError::Conflict(_)) { current.enabled = false; } current.error = Some(error.to_string()); },
                }
                Ok(())
            }).await?;
        }
        Ok(())
    }

    pub async fn deliver_watch_events(
        &self,
        reactive: Option<&crate::reactive::ReactiveRuntime>,
        coordination: Option<&crate::coordination::CoordinationRuntime>,
    ) -> DeliveryResult<()> {
        let events = self
            .state
            .lock()
            .await
            .events
            .iter()
            .filter(|event| !event.delivered)
            .cloned()
            .collect::<Vec<_>>();
        for event in events {
            if let Some(source) = event.reactive_source_id.as_deref() {
                let Some(reactive) = reactive else {
                    continue;
                };
                let payload = serde_json::to_value(&event)
                    .map_err(|error| DeliveryError::Storage(error.to_string()))?;
                if let Err(error) = reactive.publish(source, &event.id, payload).await {
                    tracing::warn!(%error, event = event.id, "PR event remains in durable outbox; its configured reactive source did not accept it");
                    continue;
                }
            }
            if let Some(coordination) = coordination {
                let project = self
                    .state
                    .lock()
                    .await
                    .watches
                    .iter()
                    .find(|watch| watch.id == event.watch_id)
                    .map(|watch| watch.request.path.clone());
                if let Some(project) = project {
                    if let Err(error) = coordination.ingest_delivery_event(&event, &project).await {
                        tracing::warn!(%error, event = event.id, "PR event remains in durable outbox; its approved task subscription did not accept it");
                        continue;
                    }
                }
            }
            self.mutate(|state| {
                if let Some(saved) = state.events.iter_mut().find(|saved| saved.id == event.id) {
                    saved.delivered = true;
                }
                Ok(())
            })
            .await?;
        }
        Ok(())
    }

    /// The caller owns this handle. Aborting it stops polling; reviewed watch configuration stays saved.
    pub fn start_watcher(
        &self,
        app: Arc<crate::AppState>,
        coordination: Option<crate::coordination::CoordinationRuntime>,
    ) -> DeliveryResult<tokio::task::JoinHandle<()>> {
        if self.watching.swap(true, Ordering::AcqRel) {
            return Err(DeliveryError::Conflict(
                "The delivery watch worker is already running.".into(),
            ));
        }
        let service = self.clone();
        Ok(tokio::spawn(async move {
            struct WorkerFlag(Arc<std::sync::atomic::AtomicBool>);
            impl Drop for WorkerFlag {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::Release);
                }
            }
            let _flag = WorkerFlag(service.watching.clone());
            loop {
                if let Err(error) = service.poll_watches().await {
                    tracing::error!(%error, "PR watch persistence stopped; restart after restoring the delivery store");
                    return;
                }
                if let Err(error) = service
                    .deliver_watch_events(app.reactive(), coordination.as_ref())
                    .await
                {
                    tracing::error!(%error, "PR event delivery stopped");
                    return;
                }
                let notified = service.changed.notified();
                let next = {
                    let state = service.state.lock().await;
                    let polling = state
                        .watches
                        .iter()
                        .filter(|watch| watch.enabled)
                        .map(|watch| watch.next_poll_at_ms)
                        .min();
                    // Delivery retries use that watch's owner-selected cadence, including
                    // a terminal MR whose final event has not reached its subscription.
                    let retry = state
                        .events
                        .iter()
                        .filter(|event| !event.delivered)
                        .filter_map(|event| {
                            state
                                .watches
                                .iter()
                                .find(|watch| watch.id == event.watch_id)
                        })
                        .map(|watch| {
                            super::now().saturating_add(
                                i64::try_from(watch.request.poll_seconds.saturating_mul(1000))
                                    .unwrap_or(i64::MAX),
                            )
                        })
                        .min();
                    drop(state);
                    polling.into_iter().chain(retry).min()
                };
                match next {
                    Some(next) => {
                        let delay = u64::try_from(next.saturating_sub(super::now())).unwrap_or(0);
                        tokio::select! { () = tokio::time::sleep(Duration::from_millis(delay)) => {}, () = notified => {} }
                    }
                    None => notified.await,
                }
            }
        }))
    }
}
