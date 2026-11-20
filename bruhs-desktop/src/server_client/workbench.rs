use super::{ServerClient, ServerError};
use bruhs_models::coordination::{CoordinationCommand, CoordinationSnapshot};

impl ServerClient {
    pub async fn prepare_work_item_handoff(
        &self,
        request: &bruhs_models::workbench_handoff::WorkItemHandoffRequest,
    ) -> Result<bruhs_models::workbench_handoff::WorkItemHandoff, ServerError> {
        self.send_json(
            reqwest::Method::POST,
            "/api/coordination/handoffs",
            Some(request),
        )
        .await
    }

    pub async fn coordination_command(
        &self,
        command: &CoordinationCommand,
    ) -> Result<CoordinationSnapshot, ServerError> {
        self.send_json(
            reqwest::Method::POST,
            "/api/coordination/commands",
            Some(command),
        )
        .await
    }
    pub async fn browser_capture_retention(&self, days: Option<u16>) -> Result<u16, ServerError> {
        let path = "/api/maintenance/browser-captures";
        match days {
            None => self.get_json(path).await,
            Some(days) => self.send_json(reqwest::Method::POST, path, Some(&days)).await,
        }
    }
    pub async fn prune_browser_captures(&self) -> Result<usize, ServerError> {
        self.send_json(reqwest::Method::POST, "/api/maintenance/browser-captures/prune", None::<&()>).await
    }
    pub async fn delivery_preferences(
        &self,
        path: Option<String>,
    ) -> Result<bruhs_models::delivery::DeliveryPreferences, ServerError> {
        self.send_json(
            reqwest::Method::POST,
            "/api/delivery/preferences/read",
            Some(&bruhs_models::delivery::DeliveryPreferenceScope { path }),
        )
        .await
    }
    pub async fn set_delivery_preference(
        &self,
        request: bruhs_models::delivery::SetDeliveryPreference,
    ) -> Result<bruhs_models::delivery::DeliveryPreferences, ServerError> {
        self.send_json(
            reqwest::Method::POST,
            "/api/delivery/preferences",
            Some(&request),
        )
        .await
    }

    pub async fn remote_hosts(
        &self,
        command: Option<&bruhs_models::remote_hosts::RemoteHostCommand>,
    ) -> Result<bruhs_models::remote_hosts::RemoteHostSnapshot, ServerError> {
        let mut url = self.local_owner_base()?;
        let http = self.local_owner_http().await?;
        let request = if let Some(command) = command {
            url.set_path("/api/remote-hosts/commands");
            http.post(url).json(command)
        } else {
            url.set_path("/api/remote-hosts");
            http.get(url)
        };
        let response = Self::check(request.send().await, "Remote environments").await?;
        Self::decode(response, "Remote environments").await
    }

    pub async fn export_settings(
        &self,
        change: Option<&bruhs_models::telemetry::SetEndpoint>,
    ) -> Result<bruhs_models::telemetry::ExportSettings, ServerError> {
        let url = self
            .local_owner_base()?
            .join("api/diagnostics/export")
            .expect("static URL path");
        let http = self.local_owner_http().await?;
        let req = match change {
            Some(change) => http.post(url).json(change),
            None => http.get(url),
        };
        let response = Self::check(req.send().await, "Diagnostic export settings").await?;
        Self::decode(response, "Diagnostic export settings").await
    }
    pub async fn check_export_endpoint(
        &self,
        endpoint: &bruhs_models::telemetry::SetEndpoint,
    ) -> Result<(), ServerError> {
        let url = self
            .local_owner_base()?
            .join("api/diagnostics/export/check")
            .expect("static URL path");
        let req = self.local_owner_http().await?.post(url).json(endpoint);
        Self::check(req.send().await, "Check diagnostic receiver").await?;
        Ok(())
    }

    pub async fn tool_snapshot(
        &self,
    ) -> Result<bruhs_models::local_tools::LocalToolsSnapshot, ServerError> {
        let mut url = self.local_owner_base()?;
        url.set_path("/api/local-tools");
        let r = Self::check(
            self.local_owner_http().await?.get(url).send().await,
            "Local tool permissions",
        )
        .await?;
        Self::decode(r, "Local tool permissions").await
    }
    pub async fn set_tool_grant(
        &self,
        request: &bruhs_models::local_tools::GrantRequest,
        revoke: Option<&str>,
    ) -> Result<(), ServerError> {
        let mut url = self.local_owner_base()?;
        url.set_path("/api/local-tools/grants");
        let http = self.local_owner_http().await?;
        let action = if let Some(id) = revoke {
            url.path_segments_mut().expect("HTTP URL").push(id);
            http.delete(url)
        } else {
            http.post(url).json(request)
        };
        Self::check(action.send().await, "Change local tool access").await?;
        Ok(())
    }
    pub async fn device_inventory(
        &self,
        grant: &str,
    ) -> Result<bruhs_models::local_tools::DeviceInventory, ServerError> {
        let mut url = self.local_owner_base()?;
        url.set_path("/api/local-tools/devices");
        let response = Self::check(
            self.local_owner_http()
                .await?
                .get(url)
                .query(&[("grant_id", grant)])
                .send()
                .await,
            "Discover devices",
        )
        .await?;
        Self::decode(response, "Discover devices").await
    }
    pub async fn device_power(
        &self,
        command: &bruhs_models::local_tools::DeviceCommand,
    ) -> Result<(), ServerError> {
        let mut url = self.local_owner_base()?;
        url.set_path("/api/local-tools/devices/command");
        Self::check(
            self.local_owner_http()
                .await?
                .post(url)
                .json(command)
                .send()
                .await,
            "Simulator power",
        )
        .await?;
        Ok(())
    }

    pub async fn clean_deleted_thread(
        &self,
        path: &str,
        branch: Option<&str>,
        thread_id: &str,
        keep_when: bruhs_models::coordination::worktree::KeepWhen,
    ) -> Result<(), ServerError> {
        let mut url = self.local_owner_base()?;
        // Older hosts ignore extra JSON fields. Use a distinct route for an
        // explicit policy so they cannot accidentally delete protected files.
        url.set_path(if branch.is_none() {
            "/api/worktrees/cleanup-deleted-scratch"
        } else if keep_when == Default::default() {
            "/api/worktrees/cleanup-deleted-thread"
        } else {
            "/api/worktrees/cleanup-deleted-thread/with-file-policy"
        });
        let body = match branch {
            Some(branch) => serde_json::json!({"path":path,"branch":branch,"keep_when":keep_when}),
            None => serde_json::json!({"path":path,"thread_id":thread_id,"keep_when":keep_when}),
        };
        let response = Self::check(
            self.local_owner_http()
                .await?
                .post(url)
                .json(&body)
                .send()
                .await,
            "Worktree cleanup",
        )
        .await?;
        if response.status() != reqwest::StatusCode::NO_CONTENT {
            return Err(ServerError::Invalid("The host did not confirm worktree cleanup."));
        }
        Ok(())
    }

    pub fn workbench_client() -> Self {
        static CLIENT: std::sync::OnceLock<ServerClient> = std::sync::OnceLock::new();
        CLIENT
            .get_or_init(|| {
                ServerClient::from_env().unwrap_or_else(|error| ServerClient::unavailable(&error))
            })
            .clone()
    }
}
