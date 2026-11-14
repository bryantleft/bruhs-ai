use super::{ServerClient, ServerError};
use bruhs_models::coordination::{CoordinationCommand, CoordinationSnapshot};

impl ServerClient {
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
    pub fn workbench_client() -> Self {
        static CLIENT: std::sync::OnceLock<ServerClient> = std::sync::OnceLock::new();
        CLIENT
            .get_or_init(|| {
                ServerClient::from_env().unwrap_or_else(|error| ServerClient::unavailable(&error))
            })
            .clone()
    }
}
