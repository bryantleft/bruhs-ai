//! Backup where it doesn't run (D27): every route of D20 still answers,
//! owner only, with 503 and [`UNSUPPORTED_PLATFORM`]. The server builds no
//! backup host and sweeps nothing.

use std::sync::Arc;

use axum::{
    extract::State,
    http::HeaderMap,
    routing::{on, MethodFilter},
    Router,
};
use bruhs_models::backup::{OWNER_ONLY_BACKUP, UNSUPPORTED_PLATFORM};

use crate::config::{Config, ServerState};
use crate::error::AppError;

/// Every route of D20 and the methods it takes.
const ROUTES: [(&str, MethodFilter); 12] = [
    ("/api/backup", MethodFilter::GET.or(MethodFilter::POST)),
    ("/api/backup/web-parts", MethodFilter::POST),
    (
        "/api/backup/jobs/{id}",
        MethodFilter::GET.or(MethodFilter::DELETE),
    ),
    ("/api/backup/files/{id}", MethodFilter::GET),
    ("/api/backup/restore/uploads", MethodFilter::POST),
    ("/api/backup/restore", MethodFilter::POST),
    ("/api/backup/restore/{id}", MethodFilter::DELETE),
    ("/api/backup/restore/apply", MethodFilter::POST),
    ("/api/backup/restore/status", MethodFilter::GET),
    (
        "/api/backup/restore/web-part",
        MethodFilter::GET.or(MethodFilter::DELETE),
    ),
    ("/api/backup/restore/web-part/previous", MethodFilter::POST),
    ("/api/backup/pre-restore/{stamp}", MethodFilter::DELETE),
];

/// Every backup route, each answering [`unsupported`].
pub fn router() -> Router<ServerState> {
    ROUTES
        .into_iter()
        .fold(Router::new(), |router, (path, methods)| {
            router.route(path, on(methods, unsupported))
        })
}

/// Other pages get the owner-only refusal, as where backup runs; the
/// owner learns it doesn't run here.
async fn unsupported(State(config): State<Arc<Config>>, headers: HeaderMap) -> AppError {
    if config.access.owner_request(&headers) {
        AppError::Unavailable(UNSUPPORTED_PLATFORM.into())
    } else {
        AppError::Forbidden(OWNER_ONLY_BACKUP.into())
    }
}

/// Stands in for the backup host `main` layers in: there is none.
#[cfg(not(unix))]
#[derive(Clone, Copy)]
pub struct NoBackup;

#[cfg(not(unix))]
impl NoBackup {
    /// Stands in for the host's roots, which the restore control takes.
    pub fn roots(&self) -> &Self {
        self
    }
}

/// Builds no backup host and runs no sweep.
#[cfg(not(unix))]
pub async fn start(
    _store: bruhs_core::secrets::SecretStore,
    _tools: Arc<bruhs_core::local_tools::LocalToolsHost>,
    _reactive: bruhs_core::reactive::ReactiveRuntime,
) -> std::io::Result<NoBackup> {
    Ok(NoBackup)
}
