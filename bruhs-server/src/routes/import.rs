//! Threads from the other agent tools on this machine, and sending one to
//! T3 Code. `GET /api/import/sessions?source=&q=&limit=` lists a tool's
//! threads; `GET /api/import/thread?source=&id=` reads one with its
//! messages; `POST /api/export/t3` starts a T3 thread carrying a bruhs
//! transcript. See `bruhs_core::import`.

use axum::extract::Query;
use axum::Json;
use bruhs_core::import::{self, ImportError, ImportRoots};
use bruhs_models::logic::portable::{
    ExportT3Request, ExportT3Response, ImportCandidate, ImportListQuery, ImportSource,
    ImportThreadQuery, ImportedThread,
};

use crate::error::AppError;
use crate::routes::paths::HomePath;

fn source(name: Option<&str>) -> Result<ImportSource, AppError> {
    let name = name.unwrap_or_default();
    ImportSource::parse(name).ok_or_else(|| {
        AppError::BadRequest(format!(
            "unknown source {name:?} (claude_code, codex, t3 or cursor)"
        ))
    })
}

fn app_error(error: ImportError) -> AppError {
    match error {
        ImportError::NotFound { .. } => AppError::NotFound(error.to_string()),
        other => AppError::internal_msg(other.to_string()),
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, ImportError> + Send + 'static,
) -> Result<T, AppError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| AppError::internal_in("import task", error.to_string()))?
        .map_err(app_error)
}

pub async fn get_sessions(
    Query(query): Query<ImportListQuery>,
) -> Result<Json<Vec<ImportCandidate>>, AppError> {
    let source = source(query.source.as_deref())?;
    let found = blocking(move || {
        import::list(
            &ImportRoots::from_env(),
            source,
            query.q.as_deref(),
            query.limit,
        )
    })
    .await?;
    Ok(Json(found))
}

pub async fn get_thread(
    Query(query): Query<ImportThreadQuery>,
) -> Result<Json<ImportedThread>, AppError> {
    let source = source(Some(&query.source))?;
    let thread =
        blocking(move || import::load(&ImportRoots::from_env(), source, &query.id)).await?;
    Ok(Json(thread))
}

pub async fn post_export_t3(
    Json(body): Json<ExportT3Request>,
) -> Result<Json<ExportT3Response>, AppError> {
    let project = HomePath::resolve(&body.project_path).await?.into_path_buf();
    let thread_id =
        blocking(move || import::export_to_t3(&project, &body.title, &body.model, &body.markdown))
            .await?;
    Ok(Json(ExportT3Response { thread_id }))
}
