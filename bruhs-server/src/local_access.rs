//! Browser-origin checks run before API handlers, not just on CORS preflights.
//! This is a loopback browser boundary, not authentication against local code.
use axum::{
    extract::{Request, State},
    http::{header, HeaderValue, Method, Uri},
    middleware::Next,
    response::Response,
};
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

use crate::error::AppError;

const OAUTH_CALLBACK: &str = "/api/calendar/google/oauth/callback";

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

    pub fn cors(&self) -> CorsLayer {
        let policy = self.clone();
        CorsLayer::new()
            .allow_origin(AllowOrigin::predicate(move |origin, _| {
                policy.allows_origin(origin)
            }))
            .allow_methods(Any)
            .allow_headers(Any)
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

pub async fn guard(
    State(policy): State<Policy>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let headers = request.headers();
    let is_api = request.uri().path() == "/api" || request.uri().path().starts_with("/api/");
    // The browser returns here from Google without Origin. The handler still
    // validates its one-use, expiring OAuth state before exchanging any code.
    let oauth_return = request.method() == Method::GET && request.uri().path() == OAUTH_CALLBACK;
    if headers.get_all(header::HOST).iter().count() != 1
        || !headers
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .is_some_and(local_authority)
        || headers.get_all(header::ORIGIN).iter().count() > 1
        || headers
            .get(header::ORIGIN)
            .is_some_and(|origin| !policy.allows_origin(origin))
        || (is_api
            && !oauth_return
            && !headers.contains_key(header::ORIGIN)
            && headers
                .get("sec-fetch-site")
                .is_some_and(|site| site == "cross-site"))
    {
        return Err(AppError::Forbidden(
            "Request origin is not allowed to use this local API.".into(),
        ));
    }
    let mut response = next.run(request).await;
    if is_api {
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    response
        .headers_mut()
        .insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

#[cfg(test)]
mod tests;
