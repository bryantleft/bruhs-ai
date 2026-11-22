use super::*;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AccessScope {
    Owner,
    Keybindings { editable: bool },
    Projects,
    ProjectNames,
    ProjectUsage { #[serde(default)] rename: bool },
    ProjectSchedules {
        #[serde(default)]
        rename: bool,
        #[serde(default)]
        toggle: bool,
        #[serde(default)]
        run: bool,
        #[serde(default)]
        delete: bool,
        #[serde(default)]
        deliveries: bool,
        #[serde(default)]
        edit: bool,
        #[serde(default)]
        conversations: bool,
        #[serde(default)]
        read_threads: bool,
        #[serde(default)]
        reply_threads: bool,
        #[serde(default)]
        usage: bool,
    },
    Guest { task_id: ThreadId },
}

impl AccessScope {
    pub fn shares_usage(&self) -> bool {
        matches!(self, Self::ProjectUsage { .. } | Self::ProjectSchedules { usage: true, .. })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccessGrant {
    pub id: String,
    pub scope: AccessScope,
    #[serde(default)]
    pub provider_updates: bool,
    pub expires_at: EpochMs,
}

/// Returned only to the local owner that creates a grant. Never logged or
/// included in a task snapshot, and only a SHA-256 digest is saved on disk.
#[derive(Serialize)]
pub struct IssuedAccess {
    pub grant: AccessGrant,
    pub token: String,
}

impl CoordinationRuntime {
    pub async fn issue_access(
        &self,
        scope: AccessScope,
        lifetime_seconds: u32,
    ) -> Result<IssuedAccess, CoordinationError> {
        self.issue_access_with_updates(scope, lifetime_seconds, false).await
    }

    pub async fn issue_access_with_updates(
        &self,
        scope: AccessScope,
        lifetime_seconds: u32,
        provider_updates: bool,
    ) -> Result<IssuedAccess, CoordinationError> {
        if provider_updates && !matches!(&scope, AccessScope::Projects | AccessScope::ProjectNames | AccessScope::ProjectUsage { .. } | AccessScope::ProjectSchedules { .. }) {
            return Err(invalid("Provider updates require a Projects grant."));
        }
        if lifetime_seconds == 0 {
            return Err(invalid("Choose an access lifetime greater than zero."));
        }
        let expires_at = EpochMs::now()
            .as_millis()
            .checked_add(i64::from(lifetime_seconds) * 1000)
            .and_then(|time| EpochMs::new(time).ok())
            .ok_or_else(|| invalid("Access expiry is outside the supported date range."))?;
        let mut data = self.0.data.lock().await;
        if let AccessScope::Guest { task_id } = &scope {
            if !data.tasks.contains_key(task_id) {
                return Err(not_found(task_id));
            }
        }
        // Two independently random UUIDs supply 244 bits of OS-backed entropy.
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let grant = AccessGrant {
            id: uuid::Uuid::new_v4().to_string(),
            scope,
            provider_updates,
            expires_at,
        };
        data.grants.insert(digest(&token), grant.clone());
        self.persist(&mut data).await?;
        drop(data);
        Ok(IssuedAccess { grant, token })
    }

    pub async fn access_grants(&self) -> Vec<AccessGrant> {
        self.0.data.lock().await.grants.values().cloned().collect()
    }

    pub async fn revoke_access(&self, id: &str) -> Result<(), CoordinationError> {
        let mut data = self.0.data.lock().await;
        if !data.grants.values().any(|grant| grant.id == id) {
            return Err(CoordinationError::NotFound(id.into()));
        }
        data.grants.retain(|_, grant| grant.id != id);
        let result = self.persist(&mut data).await;
        drop(data);
        result
    }

    pub async fn authorize(&self, token: &str) -> Result<AccessScope, CoordinationError> {
        Ok(self.authorize_grant(token).await?.scope)
    }

    pub async fn authorize_grant(&self, token: &str) -> Result<AccessGrant, CoordinationError> {
        let data = self.0.data.lock().await;
        let grant = data
            .grants
            .get(&digest(token))
            .filter(|grant| grant.expires_at > EpochMs::now())
            .ok_or(CoordinationError::Unauthorized)?;
        if let AccessScope::Guest { task_id } = &grant.scope {
            if !data.tasks.contains_key(task_id) {
                return Err(CoordinationError::Unauthorized);
            }
        }
        Ok(grant.clone())
    }

    pub async fn scoped_snapshot(
        &self,
        scope: &AccessScope,
    ) -> Result<CoordinationSnapshot, CoordinationError> {
        match scope {
            AccessScope::Owner => Ok(self.snapshot().await),
            AccessScope::Projects | AccessScope::ProjectUsage { .. } | AccessScope::ProjectNames | AccessScope::ProjectSchedules { .. } | AccessScope::Keybindings { .. } => Err(CoordinationError::Unauthorized),
            AccessScope::Guest { task_id } => {
                let mut task = self.task(task_id).await?;
                task.approvals.clear();
                task.session_id = None;
                task.parent_id = None;
                task.source_context.clear();
                task.source_revision = None;
                task.work_item_origin = None;
                task.project_path.clear();
                task.working_path.clear();
                task.spec_id = None;
                task.removed_worktree = None;
                Ok(CoordinationSnapshot {
                    tasks: vec![task],
                    ..CoordinationSnapshot::default()
                })
            }
        }
    }

    pub async fn scoped_command(
        &self,
        scope: &AccessScope,
        command: CoordinationCommand,
    ) -> Result<CoordinationSnapshot, CoordinationError> {
        if *scope != AccessScope::Owner {
            return Err(CoordinationError::Unauthorized);
        }
        self.command(command).await
    }

    pub async fn propose(
        &self,
        scope: &AccessScope,
        text: String,
    ) -> Result<GuestProposal, CoordinationError> {
        validate_text(&text, "Guest message")?;
        let AccessScope::Guest { task_id } = scope else {
            return Err(invalid(
                "Use the owner's message control to send a task message.",
            ));
        };
        let mut data = self.0.data.lock().await;
        if !data.tasks.contains_key(task_id) {
            return Err(CoordinationError::Unauthorized);
        }
        let proposal = GuestProposal {
            id: ThreadId::new_v4(),
            task_id: task_id.clone(),
            text,
            at: EpochMs::now(),
        };
        data.proposals.insert(proposal.id.clone(), proposal.clone());
        self.persist(&mut data).await?;
        drop(data);
        Ok(proposal)
    }
}

fn digest(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}
