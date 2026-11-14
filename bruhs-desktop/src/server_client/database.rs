use super::{ServerClient, ServerError};
use bruhs_models::database::*;
use uuid::Uuid;
impl ServerClient {
    pub async fn databases(&self, project: &str) -> Result<Vec<DatabaseConnection>, ServerError> {
        let mut url = reqwest::Url::parse(&format!("{}/api/databases", self.base))
            .map_err(|_| ServerError::Invalid("Invalid local server URL"))?;
        url.query_pairs_mut().append_pair("project_path", project);
        let response = Self::check(
            self.http()?.get(url).send().await,
            "Read database connections",
        )
        .await?;
        Self::decode(response, "Read database connections").await
    }
    pub async fn save_database(
        &self,
        request: &SaveDatabase,
    ) -> Result<DatabaseConnection, ServerError> {
        self.send_json(reqwest::Method::POST, "/api/databases", Some(request))
            .await
    }
    pub async fn database_grant(
        &self,
        request: &DatabaseGrant,
    ) -> Result<DatabaseConnection, ServerError> {
        self.send_json(reqwest::Method::POST, "/api/databases/grant", Some(request))
            .await
    }
    pub async fn database_tables(
        &self,
        project: &str,
        connection: &DatabaseConnection,
    ) -> Result<Vec<DatabaseTable>, ServerError> {
        let mut url = reqwest::Url::parse(&format!(
            "{}/api/databases/{}/tables",
            self.base, connection.id
        ))
        .map_err(|_| ServerError::Invalid("Invalid local server URL"))?;
        url.query_pairs_mut()
            .append_pair("project_path", project)
            .append_pair("revision", &connection.revision.to_string());
        let response =
            Self::check(self.http()?.get(url).send().await, "Read database schema").await?;
        Self::decode(response, "Read database schema").await
    }
    pub async fn database_query(
        &self,
        request: &ProjectDatabaseQuery,
    ) -> Result<DatabaseResult, ServerError> {
        self.send_json(reqwest::Method::POST, "/api/databases/query", Some(request))
            .await
    }
    pub async fn database_browse(
        &self,
        request: &ProjectDatabaseBrowse,
    ) -> Result<DatabaseResult, ServerError> {
        self.send_json(
            reqwest::Method::POST,
            "/api/databases/browse",
            Some(request),
        )
        .await
    }
    pub async fn database_describe(
        &self,
        request: &ProjectDatabaseBrowse,
    ) -> Result<DatabaseResult, ServerError> {
        self.send_json(
            reqwest::Method::POST,
            "/api/databases/describe",
            Some(request),
        )
        .await
    }
    pub async fn cancel_database_query(&self, id: Uuid) -> Result<(), ServerError> {
        Self::check(
            self.http()?
                .post(format!("{}/api/databases/cancel/{id}", self.base))
                .send()
                .await,
            "Cancel database query",
        )
        .await
        .map(|_| ())
    }
}
