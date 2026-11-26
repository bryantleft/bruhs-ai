pub mod pools;
pub mod prices;
use crate::error::AppError;
use axum::{response::Html, Json};
use bruhs_models::agent_extensions::{
    AcpInstance, ExtensionConfig, ImportedUsageSnapshot, UsageHistoryPreview, UsageHistoryRequest,
    UsageModelPricing, UsageModelPricingChange, UsageSnapshot,
};

pub async fn edit_usage_model_pricing(
    Json(request): Json<UsageModelPricingChange>,
) -> Result<Json<UsageModelPricing>, AppError> {
    let edit = request.edit.clone();
    persist_usage_model_prices(vec![request]).await?;
    Ok(Json(edit))
}

pub async fn edit_usage_model_prices(
    Json(changes): Json<Vec<UsageModelPricingChange>>,
) -> Result<axum::http::StatusCode, AppError> {
    persist_usage_model_prices(changes).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

pub async fn usage_model_prices(
) -> Result<Json<std::collections::BTreeMap<String, UsageModelPricing>>, AppError> {
    tokio::task::spawn_blocking(|| {
        bruhs_core::agent_extensions::usage_model_pricing_at(
            &bruhs_core::agent_extensions::directory()?,
        )
    })
    .await
    .map_err(AppError::internal)?
    .map(Json)
    .map_err(AppError::internal)
}

async fn persist_usage_model_prices(changes: Vec<UsageModelPricingChange>) -> Result<(), AppError> {
    tokio::task::spawn_blocking(move || {
        bruhs_core::agent_extensions::edit_usage_model_prices_at(
            &bruhs_core::agent_extensions::directory()?,
            changes,
        )
    })
    .await
    .map_err(AppError::internal)?
    .map_err(|error| AppError::BadRequest(error.to_string()))
}

pub async fn history_preview(
    Json(mut request): Json<UsageHistoryRequest>,
) -> Result<Json<UsageHistoryPreview>, AppError> {
    let path = bruhs_core::agent_extensions::history::source_root(&request);
    let path = crate::routes::paths::HomePath::resolve(&path.to_string_lossy()).await?;
    request.root = Some(path.as_path().to_string_lossy().into_owned());
    tokio::task::spawn_blocking(move || {
        bruhs_core::agent_extensions::history::preview_at(
            &bruhs_core::agent_extensions::directory()?,
            request,
        )
    })
    .await
    .map_err(AppError::internal)?
    .map(Json)
    .map_err(|error| AppError::BadRequest(error.to_string()))
}
pub async fn history_commit(
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<ImportedUsageSnapshot>, AppError> {
    tokio::task::spawn_blocking(move || {
        bruhs_core::agent_extensions::history::commit_at(
            &bruhs_core::agent_extensions::directory()?,
            &id,
        )
    })
    .await
    .map_err(AppError::internal)?
    .map(Json)
    .map_err(|error| AppError::BadRequest(error.to_string()))
}

pub async fn config() -> Result<Json<ExtensionConfig>, AppError> {
    tokio::task::spawn_blocking(bruhs_core::agent_extensions::load_config)
        .await
        .map_err(AppError::internal)?
        .map(Json)
        .map_err(AppError::internal)
}
pub async fn save(Json(config): Json<ExtensionConfig>) -> Result<Json<ExtensionConfig>, AppError> {
    config.validate().map_err(AppError::BadRequest)?;
    let copy = config.clone();
    tokio::task::spawn_blocking(move || {
        bruhs_core::agent_extensions::save_config_at(
            &bruhs_core::agent_extensions::directory()?,
            &copy,
        )
    })
    .await
    .map_err(AppError::internal)?
    .map_err(AppError::internal)?;
    Ok(Json(config))
}

#[derive(serde::Deserialize)]
pub struct AgentEdit {
    expected: Option<AcpInstance>,
    next: AcpInstance,
}

pub async fn edit_agent(Json(edit): Json<AgentEdit>) -> Result<Json<Vec<AcpInstance>>, AppError> {
    tokio::task::spawn_blocking(move || {
        bruhs_core::agent_extensions::instances::edit(edit.expected.as_ref(), Some(&edit.next))
    })
    .await
    .map_err(AppError::internal)?
    .map_err(|error| AppError::BadRequest(error.to_string()))?;
    Ok(Json(config().await?.0.acp))
}
pub async fn quota_windows(
    axum::extract::Path(provider): axum::extract::Path<String>,
) -> Result<Json<Vec<bruhs_models::agent_extensions::QuotaWindow>>, AppError> {
    use bruhs_models::agent::AgentProvider;
    let provider = match provider.as_str() {
        "codex" => AgentProvider::Codex,
        "claude" => AgentProvider::Claude,
        "opencode" => AgentProvider::OpenCode,
        "grok" => AgentProvider::Grok,
        "antigravity" => AgentProvider::Antigravity,
        "cursor" => AgentProvider::Cursor,
        _ => return Err(AppError::BadRequest("Unknown usage provider".into())),
    };
    bruhs_core::agent_extensions::quota::provider_windows(provider)
        .await
        .map(Json)
        .map_err(|e| AppError::bad_gateway("Could not read provider usage limits", e))
}

pub async fn usage() -> Result<Json<UsageSnapshot>, AppError> {
    tokio::task::spawn_blocking(|| {
        bruhs_core::agent_extensions::snapshot_at(&bruhs_core::agent_extensions::directory()?)
    })
    .await
    .map_err(AppError::internal)?
    .map(Json)
    .map_err(AppError::internal)
}
pub async fn refresh() -> Result<Json<UsageSnapshot>, AppError> {
    bruhs_core::agent_extensions::refresh_codex()
        .await
        .map_err(|e| AppError::bad_gateway("Could not refresh Codex usage limits", e))?;
    usage().await
}
pub async fn ui() -> Html<&'static str> {
    Html(include_str!("agent_extensions.html"))
}
pub async fn pools_script() -> impl axum::response::IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/javascript; charset=utf-8",
        )],
        concat!(
            include_str!("agent_extensions/pool_account.js"),
            "\n",
            include_str!("agent_extensions/pools.js")
        ),
    )
}
pub async fn workbench() -> Html<&'static str> {
    Html(include_str!("workbench.html"))
}
