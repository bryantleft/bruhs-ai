//! Scoped, host-managed CLI updates for paired desktops.
use super::*;
use bruhs_core::updates::{StartError, UpdateChecker};
use bruhs_models::agent::AgentProvider;
use bruhs_models::updates::{LinkedProviderRun, LinkedProviderTarget, LinkedProviderUpdate};

pub(super) fn routes() -> Router<Arc<RemoteState>> {
    Router::new()
        .route("/api/projects/provider-updates", get(report))
        .route("/api/projects/provider-updates/{provider}", post(start))
        .route(
            "/api/projects/provider-updates/{provider}/runs/{id}",
            get(progress),
        )
}

async fn authorize(state: &RemoteState, headers: &HeaderMap) -> Result<(), AppError> {
    if !state
        .runtime
        .authorize_grant(&token(headers)?)
        .await?
        .provider_updates
    {
        return Err(CoordinationError::Unauthorized.into());
    }
    Ok(())
}

#[derive(Deserialize, Default)]
struct CheckQuery {
    #[serde(default)]
    refresh: bool,
}

async fn report(
    State(state): State<Arc<RemoteState>>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<CheckQuery>,
    Extension(checker): Extension<UpdateChecker>,
) -> Result<Json<Vec<LinkedProviderUpdate>>, AppError> {
    authorize(&state, &headers).await?;
    let report = checker.report(query.refresh, true).await;
    let available = report.updatable_providers();
    Ok(Json(
        report
            .providers
            .iter()
            .map(|p| LinkedProviderUpdate {
                provider: p.provider,
                state: p.state,
                installed: p.installed.clone(),
                latest: p.latest.clone(),
                can_update: available.contains(&p.provider),
                run: report
                    .runs
                    .iter()
                    .filter(|r| r.provider == p.provider)
                    .max_by_key(|r| r.id)
                    .map(Into::into),
            })
            .collect(),
    ))
}

async fn start(
    State(state): State<Arc<RemoteState>>,
    headers: HeaderMap,
    Extension(checker): Extension<UpdateChecker>,
    Path(provider): Path<AgentProvider>,
    Json(target): Json<LinkedProviderTarget>,
) -> Result<(StatusCode, Json<LinkedProviderRun>), AppError> {
    authorize(&state, &headers).await?;
    let report = checker.report(false, true).await;
    if !report.updatable_providers().contains(&provider)
        || !report.provider(provider).is_some_and(|p| {
            p.installed.as_ref() == Some(&target.installed)
                && p.latest.as_ref() == Some(&target.latest)
        })
    {
        return Err(AppError::Conflict(
            "The available update changed. Check again.".into(),
        ));
    }
    authorize(&state, &headers).await?;
    let run = checker.start_update(provider).map_err(|err| match err {
        StartError::AlreadyRunning => AppError::Conflict(err.to_string()),
        StartError::UnmanagedProvider => AppError::BadRequest(err.to_string()),
    })?;
    Ok((StatusCode::ACCEPTED, Json((&run).into())))
}

async fn progress(
    State(state): State<Arc<RemoteState>>,
    headers: HeaderMap,
    Extension(checker): Extension<UpdateChecker>,
    Path((provider, id)): Path<(AgentProvider, u64)>,
) -> Result<Json<LinkedProviderRun>, AppError> {
    authorize(&state, &headers).await?;
    let run = checker
        .runs()
        .iter()
        .find(|r| r.id == id && r.provider == provider)
        .map(Into::into)
        .ok_or_else(|| AppError::NotFound("Provider update run unavailable.".into()))?;
    Ok(Json(run))
}
