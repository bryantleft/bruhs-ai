//! Browser-origin checks run before API handlers, not just on CORS preflights.
//! This is a loopback browser boundary, not authentication against local code.
use axum::{
    extract::{Request, State},
    http::{header, HeaderMap, HeaderValue, Method, Uri},
    middleware::Next,
    response::Response,
};
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

use crate::error::AppError;

const OAUTH_CALLBACK: &str = "/api/calendar/google/oauth/callback";
/// The policy of every API response that brings none of its own: opened as
/// a document (an uploaded SVG, an HTML file, an error page) it runs no
/// script and loads nothing, and its sandbox gives it an opaque origin, so
/// it can't call this API as the app's own page. Inline styles keep the
/// app's plain HTML pages (the OAuth return) readable. `<img>`, media and
/// `fetch` from the app ignore a subresource's policy.
const INERT_CSP: &str = "sandbox; default-src 'none'; style-src 'unsafe-inline'";
/// App-owned pages that run their own script against this API, besides
/// the framed ones in `inline_response`.
const APP_PAGES: [&str; 1] = ["/api/worktrees/ui"];

#[derive(Debug, Clone, Default)]
pub struct Policy {
    /// Exact browser origins allowed besides the loopback literals.
    configured_origins: Vec<HeaderValue>,
}

/// The settings that name an extra allowed browser origin.
#[derive(Clone, Copy)]
enum OriginSetting {
    ApiUrl,
    WebOrigin,
}

impl OriginSetting {
    /// Parse an explicitly configured origin: HTTP(S), no credentials, path
    /// or query (a trailing `/` is ignored).
    fn parse(self, value: &str) -> Result<HeaderValue, &'static str> {
        let value = value.trim().trim_end_matches('/');
        let malformed = match self {
            Self::ApiUrl => {
                "BRUHS_API_URL must be an HTTP(S) origin without credentials, path or query"
            }
            Self::WebOrigin => {
                "BRUHS_WEB_ORIGIN must be an HTTP(S) origin without credentials, path or query"
            }
        };
        if origin_authority(value).is_none() {
            return Err(malformed);
        }
        HeaderValue::from_str(value).map_err(|_| malformed)
    }
}

impl Policy {
    /// Built once at startup from [`crate::config::Config`]. Browser origins
    /// allowed besides `http(s)://{localhost,127.0.0.1,[::1]}`:
    /// the runtime `BRUHS_API_URL` origin and the web app's origin
    /// (`BRUHS_WEB_ORIGIN`, e.g. `https://bruhs.localhost` behind portless).
    /// Other `*.localhost` names are not trusted implicitly: any local dev
    /// site can use one, and this API drives the capture daemon.
    pub fn from_config(
        api_url: Option<&str>,
        web_origin: Option<&str>,
    ) -> Result<Self, &'static str> {
        let mut configured_origins = Vec::new();
        for (setting, value) in [
            (OriginSetting::ApiUrl, api_url),
            (OriginSetting::WebOrigin, web_origin),
        ] {
            if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
                configured_origins.push(setting.parse(value)?);
            }
        }
        Ok(Self { configured_origins })
    }

    pub fn allows_origin(&self, value: &HeaderValue) -> bool {
        value.to_str().ok().is_some_and(|origin| {
            origin_authority(origin).is_some_and(|authority| {
                loopback_literal(&authority) || self.configured_origins.contains(value)
            })
        })
    }

    /// Whether a request may change owner-only state, the vault and a
    /// bruh's grants (ruling F26), which the shared guard's every-loopback
    /// trust is too wide for: no `Origin` (the desktop's native client), an
    /// `http` `Origin` naming this request's own `Host` (the web app this
    /// server serves), or one of the configured origins.
    pub fn owner_request(&self, headers: &HeaderMap) -> bool {
        let mut origins = headers.get_all(header::ORIGIN).iter();
        let Some(origin) = origins.next() else {
            return true;
        };
        if origins.next().is_some() {
            return false;
        }
        if self.configured_origins.contains(origin) {
            return true;
        }
        let mut hosts = headers.get_all(header::HOST).iter();
        let (Some(host), None) = (hosts.next(), hosts.next()) else {
            return false;
        };
        let (Ok(origin), Ok(host)) = (origin.to_str(), host.to_str()) else {
            return false;
        };
        origin.starts_with("http://")
            && origin_authority(origin)
                .is_some_and(|authority| authority.as_str().eq_ignore_ascii_case(host))
    }

    pub fn cors(&self) -> CorsLayer {
        let policy = self.clone();
        CorsLayer::new()
            .allow_origin(AllowOrigin::predicate(move |origin, _| {
                policy.allows_origin(origin)
            }))
            .allow_methods(Any)
            .allow_headers(Any)
            .expose_headers([
                header::CONTENT_DISPOSITION,
                header::CONTENT_LENGTH,
                header::CONTENT_RANGE,
            ])
    }

    fn frame_ancestors(&self) -> HeaderValue {
        let mut policy = String::from("frame-ancestors 'self' http://localhost:* https://localhost:* http://127.0.0.1:* https://127.0.0.1:*");
        for origin in &self.configured_origins {
            if let Ok(origin) = origin.to_str() {
                policy.push(' ');
                policy.push_str(origin);
            }
        }
        // Configured origins were validated as HTTP header values at startup.
        HeaderValue::from_str(&policy)
            .unwrap_or_else(|_| HeaderValue::from_static("frame-ancestors 'none'"))
    }
}

fn origin_authority(value: &str) -> Option<axum::http::uri::Authority> {
    let uri: Uri = value.parse().ok()?;
    let authority = uri.authority()?;
    (matches!(uri.scheme_str(), Some("http" | "https"))
        && !value.contains('@')
        && !value.contains('#')
        && !value.ends_with('/')
        && uri.path() == "/"
        && uri.query().is_none()
        && (authority.as_str() == authority.host() || authority.port_u16().is_some()))
    .then(|| authority.clone())
}

fn local_authority(value: &str) -> bool {
    value
        .parse::<axum::http::uri::Authority>()
        .is_ok_and(|authority| loopback_authority(&authority))
}

/// A loopback Host: the literals plus `*.localhost` names (a local proxy
/// such as portless forwards those). Host checks stop DNS rebinding; which
/// pages may call the API is decided by the Origin policy.
fn loopback_authority(authority: &axum::http::uri::Authority) -> bool {
    let host = authority.host();
    let loopback = matches!(host, "127.0.0.1" | "localhost" | "[::1]")
        || (host.ends_with(".localhost")
            && host.split('.').all(|label| {
                !label.is_empty()
                    && label
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-')
            }));
    valid_port_form(authority) && loopback
}

/// A loopback browser origin needs no configuration only for the literal
/// loopback names.
fn loopback_literal(authority: &axum::http::uri::Authority) -> bool {
    valid_port_form(authority) && matches!(authority.host(), "127.0.0.1" | "localhost" | "[::1]")
}

fn valid_port_form(authority: &axum::http::uri::Authority) -> bool {
    !authority.as_str().contains('@')
        && (authority.as_str() == authority.host() || authority.port_u16().is_some())
}

fn requires_owner_origin(path: &str, method: &Method) -> bool {
    matches!(
        path,
        "/api/agent"
            | "/api/agent/steer"
            | "/api/agent/approve"
            | "/api/agent/answer-question"
            | "/api/agent/question-files"
            | bruhs_models::logic::attachments::RETIRE_QUESTION_FILES_PATH
            | "/api/agent/second-opinion"
            | "/api/agent/secret"
            | "/api/agent/thread-reply"
            | "/api/agent/login"
            | "/api/agent/git-checkout"
            | "/api/agent/worktree"
            | "/api/agent/worktree-snapshot"
            | "/api/agent/worktree-restore"
            | "/api/agent/prompt-pull-checkout"
            | "/api/agent/prompt-clone"
            | "/api/open-in-editor"
            | "/api/open-in-terminal"
            | "/api/terminal/ws"
            | "/api/terminal/sessions"
    ) || path.starts_with("/api/terminal/sessions/")
        || path.starts_with("/api/agent/login/")
        || (path == bruhs_models::workspace_trust::PATH && *method != Method::POST)
}

pub async fn guard(
    State(policy): State<Policy>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let headers = request.headers();
    let is_api = request.uri().path() == "/api" || request.uri().path().starts_with("/api/");
    let verification_browser = headers.contains_key("x-bruhs-verification-browser")
        || headers
            .get(header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains("BruhsVerification/1"));
    if is_api && verification_browser {
        return Err(AppError::Forbidden(
            "The verification browser cannot access Bruhs APIs. Use the scoped workbench tools."
                .into(),
        ));
    }
    let inline_response = matches!(
        request.uri().path(),
        "/api/agent/rich-response"
            | "/api/agent/rich-media"
            | "/api/agent/media"
            | "/api/workbench/ui"
            | "/api/coordination/ui"
            | "/api/delivery/ui"
            | "/api/local-tools/ui"
            | "/api/remote-hosts/ui"
            | "/api/agent-extensions/ui"
    ) || request
        .uri()
        .path()
        .strip_prefix("/api/agent/artifacts/")
        .and_then(|rest| rest.strip_suffix("/preview"))
        .is_some_and(|id| id.len() == 36 && !id.contains('/'));
    // This one read-only route verifies an immutable asset capability before
    // opening any file. Opaque media frames need CORS for subtitle tracks;
    // their grants do not authorize any other API or state operation.
    let scoped_asset = matches!(*request.method(), Method::GET | Method::HEAD)
        && request
            .uri()
            .path()
            .strip_prefix("/api/agent/rich-response/asset/")
            .is_some_and(|name| !name.is_empty() && !name.contains('/'));
    // The browser returns here from Google without Origin. The handler still
    // validates its one-use, expiring OAuth state before exchanging any code.
    let oauth_return = request.method() == Method::GET && request.uri().path() == OAUTH_CALLBACK;
    let app_page = inline_response || APP_PAGES.contains(&request.uri().path());
    if headers.get_all(header::HOST).iter().count() != 1
        || !headers
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .is_some_and(local_authority)
        || headers.get_all(header::ORIGIN).iter().count() > 1
        || headers
            .get(header::ORIGIN)
            .is_some_and(|origin| !policy.allows_origin(origin) && !scoped_asset)
        || (is_api
            && !oauth_return
            && !scoped_asset
            && !headers.contains_key(header::ORIGIN)
            && headers
                .get("sec-fetch-site")
                .is_some_and(|site| site == "cross-site"))
    {
        return Err(AppError::Forbidden(
            "Request origin is not allowed to use this local API.".into(),
        ));
    }
    if requires_owner_origin(request.uri().path(), request.method())
        && !policy.owner_request(headers)
    {
        return Err(AppError::Forbidden(
            "This action requires the Bruhs app origin.".into(),
        ));
    }
    let mut response = next.run(request).await;
    if is_api {
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        // Same-origin active content would pass as the app's own page
        // (ruling F26's owner check), so nothing else served here runs.
        if !app_page
            && !response
                .headers()
                .contains_key(header::CONTENT_SECURITY_POLICY)
        {
            response.headers_mut().insert(
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static(INERT_CSP),
            );
        }
    }
    if scoped_asset {
        // A capability intentionally allows this inert file to be displayed
        // in an opaque-origin media frame; no general API is frameable.
    } else if inline_response {
        // Only the inert shell and scoped media can be framed. Every other
        // API retains DENY; the exact same origin guard still runs above.
        response
            .headers_mut()
            .append(header::CONTENT_SECURITY_POLICY, policy.frame_ancestors());
    } else {
        response
            .headers_mut()
            .insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    }
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

#[cfg(test)]
mod tests;
