use super::{ServerClient, ServerError};
use bruhs_models::api_client::*;
use uuid::Uuid;
impl ServerClient {
    pub async fn save_api_credential(&self, input: &ApiCredentialInput) -> Result<(), ServerError> {
        let _: serde_json::Value = self
            .send_json(
                reqwest::Method::POST,
                "/api/api-client/credential",
                Some(input),
            )
            .await?;
        Ok(())
    }
    pub async fn api_collection(
        &self,
        project: &str,
    ) -> Result<ApiCollectionSnapshot, ServerError> {
        self.api_project_get("collection", project).await
    }
    pub async fn api_proposals(&self, project: &str) -> Result<Vec<ApiProposal>, ServerError> {
        self.api_project_get("proposals", project).await
    }
    async fn api_project_get<T: serde::de::DeserializeOwned>(
        &self,
        route: &str,
        project: &str,
    ) -> Result<T, ServerError> {
        let mut url = reqwest::Url::parse(&format!("{}/api/api-client/{route}", self.base))
            .map_err(|_| ServerError::Invalid("Invalid local server URL"))?;
        url.query_pairs_mut().append_pair("project_path", project);
        let response =
            Self::check(self.http()?.get(url).send().await, "Read API client state").await?;
        Self::decode(response, "Read API client state").await
    }
    pub async fn save_api_collection(
        &self,
        request: &SaveApiCollection,
    ) -> Result<ApiCollectionSnapshot, ServerError> {
        self.send_json(
            reqwest::Method::POST,
            "/api/api-client/collection",
            Some(request),
        )
        .await
    }
    pub async fn send_api_request(
        &self,
        request: &ApiExecution,
    ) -> Result<ApiResponse, ServerError> {
        self.send_json(reqwest::Method::POST, "/api/api-client/send", Some(request))
            .await
    }
    pub async fn decide_api_proposal(
        &self,
        request: &ApiProposalDecision,
    ) -> Result<ApiProposal, ServerError> {
        self.send_json(
            reqwest::Method::POST,
            "/api/api-client/decision",
            Some(request),
        )
        .await
    }
    pub async fn cancel_api_request(&self, id: Uuid) -> Result<(), ServerError> {
        Self::check(
            self.http()?
                .post(format!("{}/api/api-client/cancel/{id}", self.base))
                .send()
                .await,
            "Cancel API request",
        )
        .await
        .map(|_| ())
    }
}
