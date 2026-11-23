//! Reactive automation shares the local server's durable state with the web app.
use super::{ServerClient, ServerError};
use bruhs_models::api::{BackgroundService, BackgroundServiceAction};
use bruhs_models::reactive::{ReactiveCommand, ReactiveRun, ReactiveSnapshot};
use bruhs_models::weekly_reports::{EngineeringReport, WeeklyReportCommand, WeeklyReportsSnapshot};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct ReactiveHookToken {
    pub token: String,
    pub path: String,
}

impl ServerClient {
    pub async fn task_threads(
        &self,
        project: &str,
        query: &str,
    ) -> Result<Vec<bruhs_models::reactive::TaskThreadChoice>, ServerError> {
        let request = "Search project threads";
        let response = Self::check(
            self.http()?
                .get(format!(
                    "{}/api/reactive/task-threads",
                    self.base.trim_end_matches('/')
                ))
                .query(&[("project", project), ("query", query), ("desktop", "true")])
                .send()
                .await,
            request,
        )
        .await?;
        Self::decode(response, request).await
    }

    pub async fn desktop_task_turns(
        &self,
        thread: &str,
        after: i64,
    ) -> Result<Vec<bruhs_models::reactive::DesktopTaskTurn>, ServerError> {
        let response = Self::check(
            self.http()?
                .get(format!(
                    "{}/api/reactive/desktop-task-turns",
                    self.base.trim_end_matches('/')
                ))
                .query(&[("thread", thread), ("after", &after.to_string())])
                .send()
                .await,
            "Read scheduled replies",
        )
        .await?;
        Self::decode(response, "Read scheduled replies").await
    }

    pub async fn task_models(
        &self,
        project: &str,
    ) -> Result<bruhs_models::delivery::writer_catalog::WriterCatalog, ServerError> {
        let scope = bruhs_models::delivery::DeliveryPreferenceScope {
            path: (!project.is_empty()).then(|| project.to_owned()),
        };
        let response = self.http()?.get(self.url("/api/reactive/task-models")).query(&scope).send().await;
        let response = match Self::check(response, "Read scheduled task models").await {
            Err(ServerError::Status { code: reqwest::StatusCode::NOT_FOUND, .. }) => {
                return match self.writer_models(project).await {
                    Err(ServerError::Status { code: reqwest::StatusCode::NOT_FOUND, .. }) => Ok(Default::default()),
                    result => result,
                };
            }
            other => other?,
        };
        Self::decode(response, "Read scheduled task models").await
    }

    pub async fn task_base_branches(&self, project: &str) -> Result<bruhs_core::git::Branches, ServerError> {
        self.send_json(reqwest::Method::POST, "/api/agent/git-branches", Some(&serde_json::json!({"path":project}))).await
    }

    pub async fn weekly_reports(
        &self,
        before: Option<&str>,
    ) -> Result<WeeklyReportsSnapshot, ServerError> {
        let mut url = reqwest::Url::parse(&format!("{}/api/weekly-reports", self.base))
            .map_err(|_| ServerError::Invalid("Invalid local server URL"))?;
        if let Some(before) = before {
            url.query_pairs_mut().append_pair("before", before);
        }
        let request = "Read weekly reports";
        let response = Self::check(self.http()?.get(url).send().await, request).await?;
        Self::decode(response, request).await
    }

    pub async fn weekly_report(&self, id: &str) -> Result<EngineeringReport, ServerError> {
        let mut url = reqwest::Url::parse(&self.base)
            .map_err(|_| ServerError::Invalid("Invalid local server URL"))?;
        url.path_segments_mut()
            .map_err(|()| ServerError::Invalid("Invalid local server URL"))?
            .pop_if_empty()
            .extend(["api", "weekly-reports", id]);
        let request = "Read engineering report";
        let response = Self::check(self.http()?.get(url).send().await, request).await?;
        Self::decode(response, request).await
    }

    pub async fn weekly_report_command(
        &self,
        command: &WeeklyReportCommand,
    ) -> Result<WeeklyReportsSnapshot, ServerError> {
        self.send_json(
            reqwest::Method::POST,
            "/api/weekly-reports/commands",
            Some(command),
        )
        .await
    }

    pub async fn reactive_snapshot(&self) -> Result<ReactiveSnapshot, ServerError> {
        self.get_json("/api/reactive").await
    }

    pub async fn webhook_deliveries(
        &self,
        task: &str,
    ) -> Result<Vec<bruhs_models::reactive::WebhookDeliverySummary>, ServerError> {
        self.read_webhook_history(task, None).await
    }

    pub async fn webhook_delivery(
        &self,
        task: &str,
        id: &str,
    ) -> Result<bruhs_models::reactive::WebhookDelivery, ServerError> {
        self.read_webhook_history(task, Some(id)).await
    }

    async fn read_webhook_history<T: serde::de::DeserializeOwned>(
        &self,
        task: &str,
        id: Option<&str>,
    ) -> Result<T, ServerError> {
        let mut url = reqwest::Url::parse(&self.base)
            .map_err(|_| ServerError::Invalid("Invalid local server URL"))?;
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|()| ServerError::Invalid("Invalid local server URL"))?;
            path.pop_if_empty()
                .extend(["api", "reactive", "tasks", task, "deliveries"]);
            if let Some(id) = id {
                path.push(id);
            }
        }
        let request = "Read task webhook deliveries";
        let response = Self::check(self.http()?.get(url).send().await, request).await?;
        Self::decode(response, request).await
    }

    /// One agent's runs, newest first; `before` is the id of the last run already shown.
    pub async fn agent_runs(
        &self,
        agent_id: &str,
        before: Option<&str>,
    ) -> Result<Vec<ReactiveRun>, ServerError> {
        self.agent_thread_runs(agent_id, before, None).await
    }

    pub async fn agent_thread_runs(
        &self, agent_id: &str, before: Option<&str>, thread: Option<&str>,
    ) -> Result<Vec<ReactiveRun>, ServerError> {
        let mut url = reqwest::Url::parse(&self.base)
            .map_err(|_| ServerError::Invalid("Invalid local server URL"))?;
        url.path_segments_mut()
            .map_err(|()| ServerError::Invalid("Invalid local server URL"))?
            .pop_if_empty()
            .extend(["api", "reactive", "agents", agent_id, "runs"]);
        if let Some(before) = before {
            url.query_pairs_mut().append_pair("before", before);
        }
        if let Some(thread) = thread {
            url.query_pairs_mut().append_pair("thread", thread);
        }
        let request = "Read agent runs";
        let response = Self::check(self.http()?.get(url).send().await, request).await?;
        Self::decode(response, request).await
    }

    /// One room's messages, newest first; `before` is the id of the last message already shown.
    pub async fn room_messages(
        &self,
        room_id: &str,
        before: Option<&str>,
    ) -> Result<Vec<bruhs_models::crew::room::RoomMessage>, ServerError> {
        let mut url = reqwest::Url::parse(&self.base)
            .map_err(|_| ServerError::Invalid("Invalid local server URL"))?;
        url.path_segments_mut()
            .map_err(|()| ServerError::Invalid("Invalid local server URL"))?
            .pop_if_empty()
            .extend(["api", "reactive", "rooms", room_id, "messages"]);
        if let Some(before) = before {
            url.query_pairs_mut().append_pair("before", before);
        }
        let request = "Read room messages";
        let response = Self::check(self.http()?.get(url).send().await, request).await?;
        Self::decode(response, request).await
    }

    #[expect(
        clippy::allow_attributes,
        reason = "shared via #[path] between bruhs-desktop and bruhs-server tests"
    )]
    #[allow(
        dead_code,
        reason = "called from bruhs-desktop; bruhs-server tests also include server_client"
    )]
    pub(crate) async fn remove_checkout_task_threads(
        &self,
        threads: std::collections::BTreeMap<
            bruhs_models::ids::ThreadId,
            std::collections::BTreeSet<String>,
        >,
    ) -> Result<(), ServerError> {
        let url = self
            .local_owner_base()?
            .join("api/reactive/commands")
            .expect("static URL path");
        let request = self
            .local_owner_http()
            .await?
            .post(url)
            .json(&ReactiveCommand::DeleteCheckoutTaskThreads { threads });
        Self::check(request.send().await, "Remove checkout schedules").await?;
        Ok(())
    }

    pub async fn reactive_command(
        &self,
        command: &ReactiveCommand,
    ) -> Result<ReactiveSnapshot, ServerError> {
        self.send_json(
            reqwest::Method::POST,
            "/api/reactive/commands",
            Some(command),
        )
        .await
    }

    /// Whether the local server keeps running automations after the app quits.
    pub async fn background_service(&self) -> Result<BackgroundService, ServerError> {
        self.get_json("/api/service").await
    }

    /// Install or remove the login service. The installer can take a while.
    pub async fn set_background_service(
        &self,
        action: BackgroundServiceAction,
    ) -> Result<BackgroundService, ServerError> {
        self.send_json_with_timeout(
            reqwest::Method::POST,
            "/api/service",
            Some(&action),
            std::time::Duration::from_secs(60),
        )
        .await
    }

    pub async fn task_webhook_address(
        &self,
        task: &str,
        previous: Option<&str>,
    ) -> Result<reqwest::Url, ServerError> {
        let base = reqwest::Url::parse(&self.base)
            .map_err(|_| ServerError::Invalid("Invalid local server URL"))?;
        let mut owner_url = base.clone();
        owner_url
            .path_segments_mut()
            .map_err(|()| ServerError::Invalid("Invalid local server URL"))?
            .pop_if_empty()
            .extend(["api", "reactive", "tasks", task, "webhook-url"]);
        let http = self.local_owner_http().await?;
        let request = match previous {
            Some(token) => http.post(owner_url).json(token),
            None => http.get(owner_url),
        };
        let action = "Access task webhook URL";
        let response = Self::check(request.send().await, action).await?;
        let token: String = Self::decode(response, action).await?;
        let mut url = base;
        url.path_segments_mut()
            .map_err(|()| ServerError::Invalid("Invalid local server URL"))?
            .pop_if_empty()
            .extend(["api", "reactive", "task-hooks", task, &token]);
        Ok(url)
    }

    pub async fn reactive_hook_token(
        &self,
        source_id: &str,
    ) -> Result<ReactiveHookToken, ServerError> {
        let mut url = reqwest::Url::parse(&self.base)
            .map_err(|_| ServerError::Invalid("Invalid local server URL"))?;
        url.path_segments_mut()
            .map_err(|()| ServerError::Invalid("Invalid local server URL"))?
            .pop_if_empty()
            .extend(["api", "reactive", "sources", source_id, "token"]);
        let request = "Read reactive webhook token";
        let response = Self::check(self.http()?.get(url).send().await, request).await?;
        Self::decode(response, request).await
    }
}
