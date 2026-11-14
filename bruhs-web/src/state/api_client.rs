//! One in-memory API workspace. Private credential entry is deliberately excluded.
use bruhs_models::api_client::{ApiCollectionSnapshot, ApiResponse};
use leptos::prelude::*;
use std::collections::BTreeSet;
use uuid::Uuid;
#[derive(Clone)]
pub(crate) struct ApiClientDraft {
    pub project_path: String,
    pub snapshot: Option<ApiCollectionSnapshot>,
    pub id: Uuid,
    pub name: String,
    pub method: String,
    pub url: String,
    pub headers: String,
    pub body: String,
    pub values: String,
    pub secrets: String,
    pub curl: String,
    pub response: Option<ApiResponse>,
    pub response_tab: &'static str,
    pub expanded: BTreeSet<String>,
    pub error: Option<String>,
    pub notice: Option<String>,
}
#[derive(Clone, Copy)]
pub(crate) struct ApiClientSession(pub StoredValue<Option<ApiClientDraft>>);
pub(super) fn provide() {
    provide_context(ApiClientSession(StoredValue::new(None)));
}
