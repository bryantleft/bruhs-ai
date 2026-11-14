//! Owner-reviewed HTTP execution and repository collections for both interfaces.
use bruhs_models::api_client::*;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::{watch, Semaphore};
use uuid::Uuid;

mod collection;
#[cfg(test)]
mod tests;
pub(crate) mod validation;

/// Leaves half the native HTTP caller's 20s budget for transport and cleanup.
pub(crate) const SEND_TIMEOUT: Duration = Duration::from_secs(10);
#[derive(Debug, thiserror::Error)]
pub enum ApiClientError {
    #[error("{0}")]
    Invalid(String),
    #[error("The collection changed on disk. Reload it, review your draft, and save again.")]
    Conflict,
    #[error("Another API request is running. Cancel it or wait for it to finish.")]
    Busy,
    #[error("API request cancelled or timed out. A remote server may already have applied it; inspect its state before retrying.")]
    Cancelled,
    #[error(
        "This API proposal is no longer pending. A new send requires a new proposal and approval."
    )]
    NotPending,
    #[error("{0}")]
    Storage(String),
    #[error("{0}")]
    Network(String),
}
pub(crate) type ApiResult<T> = Result<T, ApiClientError>;
struct ActiveSend {
    id: Uuid,
    cancel: watch::Sender<bool>,
}
struct ScopedProposal {
    scope: Uuid,
    proposal: ApiProposal,
}
struct Inner {
    secrets: crate::secrets::SecretStore,
    client: reqwest::Client,
    forbidden_ports: Vec<u16>,
    forbidden_origins: Vec<String>,
    active: Mutex<Option<ActiveSend>>,
    cancelled_before_start: Mutex<HashMap<Uuid, Instant>>,
    send_gate: Arc<Semaphore>,
    proposals: Mutex<Vec<ScopedProposal>>,
}
#[derive(Clone)]
pub struct ApiClientHost(Arc<Inner>);
struct SendGuard(ApiClientHost, Uuid);
struct ProposalGuard(ApiClientHost, Uuid);
impl Drop for ProposalGuard {
    fn drop(&mut self) {
        if let Ok(mut proposals) = self.0 .0.proposals.lock() {
            if let Some(entry) = proposals.iter_mut().find(|entry| {
                entry.proposal.id == self.1
                    && matches!(entry.proposal.state, ApiProposalState::Running)
            }) {
                entry.proposal.state = ApiProposalState::Failed {
                    error: ApiClientError::Cancelled.to_string(),
                };
            }
        }
    }
}
impl Drop for SendGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = self.0 .0.active.lock() {
            if active.as_ref().is_some_and(|active| active.id == self.1) {
                active.take();
            }
        }
    }
}
impl ApiClientHost {
    pub async fn save_credential(&self, input: ApiCredentialInput) -> ApiResult<()> {
        if input.name.trim().is_empty()
            || bruhs_models::secrets::review_text(&input.name) != input.name
            || input.name.len() + input.value.len() > PAYLOAD_BYTES
        {
            return Err(ApiClientError::Invalid(
                "Choose a visible credential name and a value within 32 KiB.".into(),
            ));
        }
        let project = canonical_project(input.project_path)
            .await?
            .to_string_lossy()
            .into_owned();
        self.0
            .secrets
            .save(&project, &input.name, zeroize::Zeroizing::new(input.value))
            .await
            .map_err(|_| {
                ApiClientError::Storage(
                    "Could not save the credential. Unlock the OS keychain and retry.".into(),
                )
            })
    }
    pub fn new(secrets: crate::secrets::SecretStore, forbidden_ports: Vec<u16>) -> ApiResult<Self> {
        Self::with_origins(secrets, forbidden_ports, Vec::new())
    }
    pub fn with_origins(
        secrets: crate::secrets::SecretStore,
        forbidden_ports: Vec<u16>,
        forbidden_origins: Vec<String>,
    ) -> ApiResult<Self> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .referer(false)
            .retry(reqwest::retry::never())
            .timeout(SEND_TIMEOUT)
            .build()
            .map_err(|_| {
                ApiClientError::Network(
                    "Could not initialize the native HTTP client. Restart the companion server."
                        .into(),
                )
            })?;
        Ok(Self(Arc::new(Inner {
            secrets,
            client,
            forbidden_ports,
            forbidden_origins,
            active: Mutex::default(),
            cancelled_before_start: Mutex::default(),
            send_gate: Arc::new(Semaphore::new(1)),
            proposals: Mutex::default(),
        })))
    }
    pub async fn collection(&self, project: String) -> ApiResult<ApiCollectionSnapshot> {
        let root = canonical_project(project).await?;
        tokio::task::spawn_blocking(move || collection::load(&root))
            .await
            .map_err(|_| worker_error())?
    }
    pub async fn save(&self, request: SaveApiCollection) -> ApiResult<ApiCollectionSnapshot> {
        validation::collection(&request.collection)?;
        let root = canonical_project(request.project_path).await?;
        tokio::task::spawn_blocking(move || {
            collection::save(&root, request.collection, request.expected_revision)
        })
        .await
        .map_err(|_| worker_error())?
    }
    pub async fn send(&self, mut execution: ApiExecution) -> ApiResult<ApiResponse> {
        execution.project_path = canonical_project(execution.project_path)
            .await?
            .to_string_lossy()
            .into_owned();
        validation::request(
            &execution.request,
            &execution.environment,
            &self.0.forbidden_ports,
            &self.0.forbidden_origins,
        )?;
        let _permit = Arc::clone(&self.0.send_gate)
            .try_acquire_owned()
            .map_err(|_| ApiClientError::Busy)?;
        let (cancel, mut cancelled) = watch::channel(false);
        {
            let mut active = self.0.active.lock().map_err(|_| worker_error())?;
            let mut pending = self
                .0
                .cancelled_before_start
                .lock()
                .map_err(|_| worker_error())?;
            pending.retain(|_, time| time.elapsed() < SEND_TIMEOUT * 2);
            if pending.remove(&execution.request_id).is_some() {
                return Err(ApiClientError::Cancelled);
            }
            drop(pending);
            *active = Some(ActiveSend {
                id: execution.request_id,
                cancel,
            });
            drop(active);
        }
        let _guard = SendGuard(self.clone(), execution.request_id);
        tokio::select! {biased;
            _=cancelled.changed()=>Err(ApiClientError::Cancelled),
            result=tokio::time::timeout(SEND_TIMEOUT,self.0.secrets.execute_api(&self.0.client,&execution))=>result.map_err(|_|ApiClientError::Cancelled)?
        }
    }
    pub fn cancel(&self, id: Uuid) -> ApiResult<()> {
        let active = self.0.active.lock().map_err(|_| worker_error())?;
        if let Some(active) = active.as_ref().filter(|active| active.id == id) {
            let _ = active.cancel.send(true);
            return Ok(());
        }
        let mut pending = self
            .0
            .cancelled_before_start
            .lock()
            .map_err(|_| worker_error())?;
        pending.retain(|_, time| time.elapsed() < SEND_TIMEOUT * 2);
        if pending.len() >= PAYLOAD_BYTES / size_of::<Uuid>() {
            return Err(ApiClientError::Invalid(
                "Too many cancellation IDs are pending. Wait for current requests to settle."
                    .into(),
            ));
        }
        pending.insert(id, Instant::now());
        drop(pending);
        // Retain active through tombstone insertion so cancel cannot miss a concurrent send.
        drop(active);
        Ok(())
    }
    pub async fn propose(&self, request: ProposeApiRequest) -> ApiResult<ApiProposal> {
        let project = canonical_project(request.project_path)
            .await?
            .to_string_lossy()
            .into_owned();
        if request.thread_id.trim().is_empty() {
            return Err(ApiClientError::Invalid(
                "API proposals require the originating turn's thread.".into(),
            ));
        }
        validation::request(
            &request.request,
            &request.environment,
            &self.0.forbidden_ports,
            &self.0.forbidden_origins,
        )?;
        let mut proposals = self.0.proposals.lock().map_err(|_| worker_error())?;
        if proposals.len() >= PROPOSAL_LIMIT {
            if let Some(index) = proposals.iter().position(|entry| {
                !matches!(
                    entry.proposal.state,
                    ApiProposalState::Pending | ApiProposalState::Running
                )
            }) {
                proposals.remove(index);
            } else {
                return Err(ApiClientError::Invalid("The API approval queue is full. Review or decline pending requests before proposing another.".into()));
            }
        }
        let proposal = ApiProposal {
            id: Uuid::new_v4(),
            project_path: project,
            thread_id: request.thread_id,
            destination: reqwest::Url::parse(&validation::plain_template(
                &request.request.url,
                &request.environment,
            )?)
            .map_err(|_| ApiClientError::Invalid("Invalid request destination.".into()))?
            .to_string(),
            request: request.request,
            environment: request.environment,
            state: ApiProposalState::Pending,
        };
        proposals.push(ScopedProposal {
            scope: request.scope,
            proposal: proposal.clone(),
        });
        drop(proposals);
        Ok(proposal)
    }
    pub async fn proposals(&self, project: String) -> ApiResult<Vec<ApiProposal>> {
        let project = canonical_project(project)
            .await?
            .to_string_lossy()
            .into_owned();
        Ok(self
            .0
            .proposals
            .lock()
            .map_err(|_| worker_error())?
            .iter()
            .filter(|entry| entry.proposal.project_path == project)
            .map(|entry| entry.proposal.clone())
            .collect())
    }
    pub async fn proposal_result(
        &self,
        project: String,
        scope: Uuid,
        thread: &str,
        id: Uuid,
    ) -> ApiResult<ApiProposal> {
        let project = canonical_project(project)
            .await?
            .to_string_lossy()
            .into_owned();
        self.0
            .proposals
            .lock()
            .map_err(|_| worker_error())?
            .iter()
            .find(|entry| {
                entry.scope == scope
                    && entry.proposal.project_path == project
                    && entry.proposal.thread_id == thread
                    && entry.proposal.id == id
            })
            .map(|entry| entry.proposal.clone())
            .ok_or(ApiClientError::NotPending)
    }
    pub async fn decide(&self, decision: ApiProposalDecision) -> ApiResult<ApiProposal> {
        let project = canonical_project(decision.project_path)
            .await?
            .to_string_lossy()
            .into_owned();
        // Consume the immutable reviewed proposal before any await or network effect.
        let mut proposal = {
            let mut proposals = self.0.proposals.lock().map_err(|_| worker_error())?;
            let entry = proposals
                .iter_mut()
                .find(|entry| {
                    entry.proposal.id == decision.id && entry.proposal.project_path == project
                })
                .ok_or(ApiClientError::NotPending)?;
            if !matches!(entry.proposal.state, ApiProposalState::Pending) {
                return Err(ApiClientError::NotPending);
            }
            entry.proposal.state = if decision.approve {
                ApiProposalState::Running
            } else {
                ApiProposalState::Declined
            };
            let proposal = entry.proposal.clone();
            drop(proposals);
            proposal
        };
        if decision.approve {
            let _guard = ProposalGuard(self.clone(), proposal.id);
            let result = self
                .send(ApiExecution {
                    project_path: project,
                    request_id: proposal.id,
                    request: proposal.request.clone(),
                    environment: proposal.environment.clone(),
                })
                .await;
            proposal.state = match result {
                Ok(response) => ApiProposalState::Finished { response },
                Err(error) => ApiProposalState::Failed {
                    error: error.to_string(),
                },
            };
            {
                let mut proposals = self.0.proposals.lock().map_err(|_| worker_error())?;
                if let Some(entry) = proposals
                    .iter_mut()
                    .find(|entry| entry.proposal.id == proposal.id)
                {
                    entry.proposal = proposal.clone();
                }
            }
        }
        Ok(proposal)
    }
}
fn worker_error() -> ApiClientError {
    ApiClientError::Storage(
        "The API client worker is unavailable. Restart the companion server.".into(),
    )
}
async fn canonical_project(project: String) -> ApiResult<PathBuf> {
    let root = tokio::fs::canonicalize(project).await.map_err(|_| {
        ApiClientError::Invalid("Choose an existing project working folder.".into())
    })?;
    if !root.is_dir() {
        return Err(ApiClientError::Invalid(
            "Choose a project folder, not a file.".into(),
        ));
    }
    Ok(root)
}
