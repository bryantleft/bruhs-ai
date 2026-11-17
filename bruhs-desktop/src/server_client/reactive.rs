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

    /// One agent's runs, newest first; `before` is the id of the last run already shown.
    pub async fn agent_runs(
        &self,
        agent_id: &str,
        before: Option<&str>,
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
        let request = "Read agent runs";
        let response = Self::check(self.http()?.get(url).send().await, request).await?;
        Self::decode(response, request).await
    }

    /// One room's messages, newest first; `before` is the id of the last message already shown.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the room transcript (Task 13) reads it")
    )]
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

/// Keeps `room_messages` live in test builds until the room transcript reads it.
#[cfg(test)]
const _: fn() = || {
    let _ = ServerClient::room_messages;
};
