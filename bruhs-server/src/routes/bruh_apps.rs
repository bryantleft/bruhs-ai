//! A bruh's app and mail calls over HTTP (D1, D15, D17).
//!
//! `POST /api/reactive/agents/{id}/apps`, with `{"tool": "app_request" |
//! "list_apps" | "send_mail", "input": {…}}`, answers only the bruh's own
//! helper: a request with an `Origin` comes from a page and is refused
//! ([`CALL_FROM_A_PAGE`]). Each call first uses up the one-shot allowance
//! the gate left for that tool and that input exactly as the helper sent
//! it ([`CALL_NOT_APPROVED`] without one). Then:
//!
//! - `list_apps` answers the bruh's apps and mailboxes.
//! - `app_request` looks its account up live, which must be the one the
//!   gate picked (ruling F10), runs the gate again at the bruh's stored
//!   access (only a Deny refuses: the allowance is the answer), reads the
//!   token, looks the account up once more, and sends the call. Any status
//!   the app answers is the value.
//! - `send_mail` does the same with its mailbox and sends the email.
//!
//! The whole route answers within [`ROUTE_BUDGET`], under the helper's own
//! 60 s request, and never cuts a write it sent (ruling F11): the lookups
//! and the token read share [`APP_READ_BUDGET`], and a call starts only
//! when its whole timeout still fits. No handler logs a body, a token or
//! an email's content or recipients (D26).

use std::time::Duration;

use axum::{
    extract::{rejection::JsonRejection, Path},
    http::{header::ORIGIN, HeaderMap, StatusCode},
    Extension, Json,
};
use bruhs_core::apps::{AppsError, AppsHost};
use bruhs_core::reactive::ReactiveRuntime;
use bruhs_core::secrets::SecretError;
use bruhs_models::agent::AgentProvider;
use bruhs_models::apps::{
    AppCall, APPS_NOT_ON_SERVER, APP_CALL_TIMEOUT_MS, APP_MAY_HAVE_LANDED, APP_UNREACHABLE,
};
use bruhs_models::crew::{
    app_gate, list_apps_gate, mail_gate, AppLookup, BruhAccess, Gate, MailLookup, MailSendInput,
    APPS_UNREADABLE, APP_CALL_SHAPE, APP_NOT_GRANTED, APP_REQUEST_TOOL, BRUH_NO_TOOLS,
    CALL_FROM_A_PAGE, CALL_NOT_APPROVED, KEYCHAIN_NEEDS_OWNER, LIST_APPS_TOOL, MAIL_CALL_SHAPE,
    MAIL_NOT_GRANTED, MAIL_NOT_SENT, MAIL_SEND_BUDGET_MS, SEND_MAIL_TOOL,
};
use bruhs_models::local_tools::ToolReply;
use serde_json::Value;
use tokio::time::Instant;

use super::apps::{app_status, RouteError};

/// How long the whole route may take: under the helper's own request
/// deadline (`capture_client::REQUEST_TIMEOUT`, 60 s), so the bruh never
/// gives up on a call that then lands.
pub const ROUTE_BUDGET: Duration = Duration::from_secs(55);
/// How long the lookups and the token read may take together (F11).
pub const APP_READ_BUDGET: Duration = Duration::from_secs(20);
/// What must be left of the route for a call to start besides its own
/// timeout.
const CALL_MARGIN: Duration = Duration::from_secs(2);
/// How long a `send_mail` can take once handed to the host: its send
/// budget, the 5 s the host may take to settle a spent one, and the margin.
const MAIL_WINDOW: Duration =
    Duration::from_millis(MAIL_SEND_BUDGET_MS).saturating_add(Duration::from_secs(7));

#[cfg(test)]
#[cfg(unix)]
mod tests;

/// The route's time: the whole, the reads, what a call needs left to
/// start, and what an email needs.
#[derive(Clone, Copy)]
struct Budget {
    total: Duration,
    read: Duration,
    call: Duration,
    mail: Duration,
}

impl Budget {
    const ROUTE: Self = Self {
        total: ROUTE_BUDGET,
        read: APP_READ_BUDGET,
        call: Duration::from_millis(APP_CALL_TIMEOUT_MS).saturating_add(CALL_MARGIN),
        mail: MAIL_WINDOW,
    };
}

/// Whether a call that needs `needed` may still start at `now`, for a
/// route that must answer by `deadline`.
fn fits(now: Instant, deadline: Instant, needed: Duration) -> bool {
    deadline.saturating_duration_since(now) >= needed
}

/// The model-facing copy for an app or mail error. An account the vault
/// no longer has is no longer granted.
fn bruh_error(error: AppsError) -> RouteError {
    if let Some((status, message)) = app_status(&error) {
        return RouteError::status(status, message);
    }
    match error {
        AppsError::Vault(SecretError::NotSaved) => RouteError::forbidden(APP_NOT_GRANTED),
        _ => RouteError::unavailable(APPS_UNREADABLE),
    }
}

/// The tool a helper names, as the gate knows it.
fn tool_named(name: Option<&str>) -> Option<&'static str> {
    match name? {
        "app_request" => Some(APP_REQUEST_TOOL),
        "list_apps" => Some(LIST_APPS_TOOL),
        "send_mail" => Some(SEND_MAIL_TOOL),
        _ => None,
    }
}

/// `POST /api/reactive/agents/{id}/apps`: one call from the bruh's helper.
pub async fn call(
    runtime: Option<Extension<ReactiveRuntime>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Result<Json<Value>, JsonRejection>,
) -> Result<Json<ToolReply>, RouteError> {
    let started = Instant::now();
    if headers.contains_key(ORIGIN) {
        return Err(RouteError::forbidden(CALL_FROM_A_PAGE));
    }
    let runtime = super::reactive::runtime(runtime)?;
    let Json(body) = body.map_err(|_| RouteError::forbidden(CALL_NOT_APPROVED))?;
    let tool = tool_named(body["tool"].as_str())
        .ok_or_else(|| RouteError::forbidden(CALL_NOT_APPROVED))?;
    // Used up first, matched against the input exactly as the helper sent
    // it (parsing would turn 1 into 1.0), so a call refused below can't be
    // sent again without the gate.
    let input = body.get("input").cloned().unwrap_or(Value::Null);
    let allowance = runtime
        .consume_app_allowance(&id, tool, &input)
        .await?
        .ok_or_else(|| RouteError::forbidden(CALL_NOT_APPROVED))?;
    let value = answer(
        &runtime,
        &id,
        tool,
        &input,
        allowance.target.as_deref(),
        started,
        Budget::ROUTE,
    )
    .await?;
    Ok(Json(ToolReply { value, image: None }))
}

/// The bruh's stored access and the provider its model runs on, from the
/// runtime: [`BRUH_NO_TOOLS`] for an id no bruh has.
async fn bruh_access(
    runtime: &ReactiveRuntime,
    id: &str,
) -> Result<(BruhAccess, AgentProvider), RouteError> {
    let snapshot = runtime.snapshot().await?;
    let missing = || RouteError::Status(StatusCode::NOT_FOUND, BRUH_NO_TOOLS.into());
    let bruh = snapshot
        .bruhs
        .into_iter()
        .find(|bruh| bruh.agent_id == id)
        .ok_or_else(missing)?;
    let agent = snapshot
        .agents
        .iter()
        .find(|agent| agent.id == id)
        .ok_or_else(missing)?;
    Ok((bruh.access, AgentProvider::for_model(&agent.model)))
}

/// Refuse what the gate, run again, denies. Allow and Approve go on: the
/// allowance was the gate's or the owner's answer.
fn regated(gate: Gate) -> Result<(), RouteError> {
    match gate {
        Gate::Deny(reason) => Err(RouteError::forbidden(reason)),
        Gate::Allow | Gate::Approve => Ok(()),
    }
}

/// One allowed call of `tool` with `input`, which the gate let through for
/// `target`, within `budget` of `started`.
async fn answer(
    runtime: &ReactiveRuntime,
    id: &str,
    tool: &'static str,
    input: &Value,
    target: Option<&str>,
    started: Instant,
    budget: Budget,
) -> Result<Value, RouteError> {
    let reads_by = started + budget.read;
    let (access, provider) = tokio::time::timeout_at(reads_by, bruh_access(runtime, id))
        .await
        .map_err(|_| RouteError::unavailable(APPS_UNREADABLE))??;
    match tool {
        LIST_APPS_TOOL => {
            regated(list_apps_gate(&access, provider))?;
            let app = input.get("app").and_then(Value::as_str);
            Ok(
                tokio::time::timeout_at(reads_by, runtime.list_apps(id, app))
                    .await
                    .map_err(|_| RouteError::unavailable(APPS_UNREADABLE))??,
            )
        }
        APP_REQUEST_TOOL => {
            let apps = runtime
                .apps()
                .ok_or_else(|| RouteError::unavailable(APPS_NOT_ON_SERVER))?;
            app_request(
                runtime,
                apps,
                id,
                (&access, provider),
                input,
                target,
                started,
                budget,
            )
            .await
        }
        SEND_MAIL_TOOL => {
            let apps = runtime
                .apps()
                .ok_or_else(|| RouteError::unavailable(APPS_NOT_ON_SERVER))?;
            send_mail(
                runtime,
                apps,
                id,
                (&access, provider),
                input,
                target,
                started,
                budget,
            )
            .await
        }
        // A tool added later never falls into a send.
        _ => Err(RouteError::forbidden(CALL_NOT_APPROVED)),
    }
}

/// An `app_request`: the account looked up live and checked against the
/// gate's, the gate again, the entry and its token read within the read
/// budget, then the call, only if its whole timeout fits.
#[expect(
    clippy::too_many_arguments,
    reason = "the route's fixed context, passed once from `answer`"
)]
async fn app_request(
    runtime: &ReactiveRuntime,
    apps: &AppsHost,
    id: &str,
    (access, provider): (&BruhAccess, AgentProvider),
    input: &Value,
    target: Option<&str>,
    started: Instant,
    budget: Budget,
) -> Result<Value, RouteError> {
    let reads_by = started + budget.read;
    let lookup = tokio::time::timeout_at(reads_by, runtime.app_lookup(id, input))
        .await
        .map_err(|_| RouteError::unavailable(APP_UNREACHABLE))?;
    let entry_id = match &lookup {
        AppLookup::Found(found) => found.entry_id.clone(),
        AppLookup::Missing(reason) => return Err(RouteError::forbidden(*reason)),
    };
    // The account may differ once grants change after the card (F10).
    if target != Some(entry_id.as_str()) {
        return Err(RouteError::forbidden(APP_NOT_GRANTED));
    }
    regated(app_gate(access, provider, input, &lookup))?;
    let call: AppCall = serde_json::from_value(input.clone())
        .map_err(|_| RouteError::status(StatusCode::BAD_REQUEST, APP_CALL_SHAPE))?;
    let keychain = || RouteError::unavailable(KEYCHAIN_NEEDS_OWNER);
    // The entry the live lookup found, never one the input names.
    let entry = tokio::time::timeout_at(reads_by, apps.vault().vault_entry(&entry_id))
        .await
        .map_err(|_| keychain())?
        .map_err(|error| match error {
            SecretError::Storage => keychain(),
            _ => RouteError::unavailable(APPS_UNREADABLE),
        })?
        .ok_or_else(|| RouteError::forbidden(APP_NOT_GRANTED))?;
    let ready = tokio::time::timeout_at(reads_by, apps.ready(&entry, &call))
        .await
        .map_err(|_| keychain())?
        .map_err(bruh_error)?;
    // The token read can wait on the keychain: a grant revoked meanwhile
    // stops the call (ruling A Task 13 minors).
    let again = tokio::time::timeout_at(reads_by, runtime.app_lookup(id, input))
        .await
        .map_err(|_| RouteError::unavailable(APP_UNREACHABLE))?;
    if !matches!(&again, AppLookup::Found(found) if found.entry_id == entry_id) {
        return Err(RouteError::forbidden(APP_NOT_GRANTED));
    }
    if !fits(Instant::now(), started + budget.total, budget.call) {
        // Nothing was sent: a retry is safe.
        return Err(keychain());
    }
    // Once sent, the call runs to its own timeout.
    let response = apps.send_ready(ready).await.map_err(bruh_error)?;
    serde_json::to_value(response).map_err(|_| RouteError::unavailable(APP_MAY_HAVE_LANDED))
}

/// A `send_mail`: the mailbox looked up live and checked against the
/// gate's, the gate again, then the send, only if its whole window fits.
#[expect(
    clippy::too_many_arguments,
    reason = "the route's fixed context, passed once from `answer`"
)]
async fn send_mail(
    runtime: &ReactiveRuntime,
    apps: &AppsHost,
    id: &str,
    (access, provider): (&BruhAccess, AgentProvider),
    input: &Value,
    target: Option<&str>,
    started: Instant,
    budget: Budget,
) -> Result<Value, RouteError> {
    let deadline = started + budget.total;
    // The lookup gets what the send leaves, at most the read budget.
    let reads_by = deadline
        .checked_sub(budget.mail)
        .unwrap_or(started)
        .min(started + budget.read);
    let not_sent = || RouteError::unavailable(MAIL_NOT_SENT);
    let lookup = tokio::time::timeout_at(reads_by, runtime.mail_lookup(id, input))
        .await
        .map_err(|_| not_sent())?;
    let mailbox_id = match &lookup {
        MailLookup::Found(found) => found.mailbox_id.clone(),
        MailLookup::Missing(reason) => return Err(RouteError::forbidden(*reason)),
    };
    if target != Some(mailbox_id.as_str()) {
        return Err(RouteError::forbidden(MAIL_NOT_GRANTED));
    }
    regated(mail_gate(access, provider, input, &lookup))?;
    let mail: MailSendInput = serde_json::from_value(input.clone())
        .map_err(|_| RouteError::status(StatusCode::BAD_REQUEST, MAIL_CALL_SHAPE))?;
    if !fits(Instant::now(), deadline, budget.mail) {
        return Err(not_sent());
    }
    let outcome = apps
        .send_mail(&mailbox_id, mail)
        .await
        .map_err(bruh_error)?;
    serde_json::to_value(outcome).map_err(|_| RouteError::unavailable(APPS_UNREADABLE))
}
