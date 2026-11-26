//! Workbench access to model prices on an explicitly authorized linked environment.
use crate::error::AppError;
use axum::{
    extract::Path,
    http::{header, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
use bruhs_core::{
    delivery::writer::desktop_state_file,
    remote_hosts::RemoteHostManager,
    usage_hubs::linked::{self, PriceDestination},
};
use bruhs_models::{agent_extensions::UsageModelPricingChange, remote_hosts::RemoteHostSnapshot};
use std::path::PathBuf;

pub async fn script() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("prices.js"),
    )
}

#[derive(serde::Serialize)]
struct Destination {
    key: String,
    name: String,
}

pub async fn destinations() -> Result<impl IntoResponse, AppError> {
    let path = desktop_state_file();
    let links = linked::price_destinations(path.as_deref())
        .await
        .map_err(AppError::internal)?;
    let mut seen = std::collections::BTreeSet::new();
    let mut choices = Vec::with_capacity(links.len());
    for link in links {
        let key = link.request_key().map_err(AppError::internal)?;
        if seen.insert(key.clone()) {
            choices.push(Destination {
                key,
                name: link.name,
            });
        }
    }
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(choices)))
}

async fn connection(
    key: &str,
    hosts: Option<Extension<RemoteHostManager>>,
) -> Result<(PathBuf, PriceDestination, Option<RemoteHostSnapshot>), AppError> {
    let missing = || {
        AppError::NotFound(
            "The environment link changed. Reopen Model prices to use the current link.".into(),
        )
    };
    let path = desktop_state_file().ok_or_else(missing)?;
    let links = linked::price_destinations(Some(&path))
        .await
        .map_err(AppError::internal)?;
    for link in links {
        if link.request_key().map_err(AppError::internal)? != key {
            continue;
        }
        let tunnels = if let Some(Extension(manager)) = hosts.filter(|_| link.needs_tunnel()) {
            Some(manager.snapshot().await)
        } else {
            None
        };
        return Ok((path, link, tunnels));
    }
    Err(missing())
}

pub async fn read(
    Path(key): Path<String>,
    hosts: Option<Extension<RemoteHostManager>>,
) -> Result<impl IntoResponse, AppError> {
    let (path, link, tunnels) = connection(&key, hosts).await?;
    let rows = linked::read_model_prices(&path, &link, tunnels.as_ref())
        .await
        .map_err(AppError::Unavailable)?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(rows)))
}

pub async fn save(
    Path(key): Path<String>,
    hosts: Option<Extension<RemoteHostManager>>,
    Json(changes): Json<Vec<UsageModelPricingChange>>,
) -> Result<StatusCode, AppError> {
    let (path, link, tunnels) = connection(&key, hosts).await?;
    linked::save_model_prices(&path, &link, tunnels.as_ref(), &changes)
        .await
        .map_err(AppError::Unavailable)?;
    Ok(StatusCode::NO_CONTENT)
}
