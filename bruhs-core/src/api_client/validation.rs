use super::{ApiClientError, ApiResult};
use bruhs_models::api_client::*;

pub(crate) fn plain_template(input: &str, environment: &ApiEnvironment) -> ApiResult<String> {
    template(input,|key|{
        if environment.secrets.contains_key(key){return Err("Keychain variables cannot appear in URLs. Put credentials in a request header or body.".into());}
        environment.values.get(key).cloned().ok_or_else(||format!("Define environment variable {key} before sending."))
    }).map_err(ApiClientError::Invalid)
}
pub(crate) fn request(
    request: &ApiRequest,
    environment: &ApiEnvironment,
    forbidden_ports: &[u16],
    forbidden_origins: &[String],
) -> ApiResult<()> {
    let invalid = |message: &str| ApiClientError::Invalid(message.into());
    if !valid_method(&request.method) {
        return Err(invalid(
            "Choose GET, POST, PUT, PATCH, DELETE, HEAD or OPTIONS.",
        ));
    }
    if request.name.trim().is_empty()
        || bruhs_models::secrets::review_text(&request.name) != request.name
    {
        return Err(invalid(
            "Give the request a visible name without control characters.",
        ));
    }
    let bytes = serde_json::to_vec(&(request, environment))
        .map_err(|_| invalid("Invalid request input."))?;
    if bytes.len() > PAYLOAD_BYTES {
        return Err(invalid(
            "Request and environment exceed the 32 KiB payload budget. Narrow the request.",
        ));
    }
    for (key, value) in &environment.values {
        valid_key(key)?;
        if environment.secrets.contains_key(key) {
            return Err(invalid(
                "A variable cannot have both an ordinary value and a keychain reference.",
            ));
        }
        if value.contains('\0') {
            return Err(invalid("Environment values cannot contain NUL characters."));
        }
    }
    for (key, name) in &environment.secrets {
        valid_key(key)?;
        if name.trim().is_empty() || bruhs_models::secrets::review_text(name) != *name {
            return Err(invalid(
                "Use a saved credential name without control characters.",
            ));
        }
    }
    let destination = plain_template(&request.url, environment)?;
    let url = reqwest::Url::parse(&destination).map_err(|_| {
        invalid("Use a complete http:// or https:// destination after variable substitution.")
    })?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(invalid("Use a complete http:// or https:// destination."));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid(
            "URL credentials are unsupported. Use a keychain variable in Authorization instead.",
        ));
    }
    if url.fragment().is_some() {
        return Err(invalid(
            "Remove the URL fragment; it is not sent to an HTTP server.",
        ));
    }
    if matches!(
        url.host_str(),
        Some("api.bruhs.localhost" | "bruhs.localhost" | "capture.bruhs.localhost")
    ) || forbidden_origins
        .iter()
        .filter_map(|origin| reqwest::Url::parse(origin).ok())
        .any(|origin| origin.origin() == url.origin())
    {
        return Err(invalid("This is a Bruhs app or proxy origin. Use the app's own controls for Bruhs data and settings."));
    }
    // Restrict by port independently of host text/DNS aliases. These are private app listeners.
    if url
        .port_or_known_default()
        .is_some_and(|port| forbidden_ports.contains(&port))
    {
        return Err(invalid("This is a private Bruhs server port. Use the app's own controls for Bruhs data and settings."));
    }
    let mut names = std::collections::HashSet::new();
    for header in &request.headers {
        let name = reqwest::header::HeaderName::from_bytes(header.name.as_bytes())
            .map_err(|_| invalid("A header name is invalid. Use HTTP header-name syntax."))?;
        if matches!(
            name.as_str(),
            "host"
                | "content-length"
                | "connection"
                | "transfer-encoding"
                | "upgrade"
                | "te"
                | "trailer"
                | "proxy-authorization"
                | "proxy-connection"
                | "expect"
        ) {
            return Err(invalid("Host, framing, connection and proxy headers are controlled by the native HTTP client."));
        }
        if !names.insert(name.clone()) {
            return Err(invalid(
                "A header name is repeated. Combine its values explicitly before sending.",
            ));
        }
        let mut has_secret = false;
        let value = template(&header.value, |key| {
            if let Some(name) = environment.secrets.get(key) {
                has_secret = true;
                Ok(format!("[keychain:{name}]"))
            } else {
                environment
                    .values
                    .get(key)
                    .cloned()
                    .ok_or_else(|| format!("Define environment variable {key} before sending."))
            }
        })
        .map_err(ApiClientError::Invalid)?;
        reqwest::header::HeaderValue::from_str(&value)
            .map_err(|_| invalid("A header value contains invalid characters or a line break."))?;
        if sensitive_header(name.as_str()) && !has_secret {
            return Err(invalid("Sensitive headers require a keychain variable, for example Authorization: Bearer {{api_key}} with api_key listed under Keychain references. Save its value in Settings, Vault."));
        }
    }
    template(&request.body, |key| {
        if environment.secrets.contains_key(key) {
            Ok("[keychain reference]".into())
        } else {
            environment
                .values
                .get(key)
                .cloned()
                .ok_or_else(|| format!("Define environment variable {key} before sending."))
        }
    })
    .map_err(ApiClientError::Invalid)?;
    Ok(())
}
pub(crate) fn collection(collection: &ApiCollection) -> ApiResult<()> {
    if collection.version != 1 {
        return Err(ApiClientError::Invalid(
            "This collection version is unsupported. Use the app version that wrote it.".into(),
        ));
    }
    if serde_json::to_vec(collection)
        .map_err(|_| ApiClientError::Invalid("Invalid collection.".into()))?
        .len()
        > PAYLOAD_BYTES
    {
        return Err(ApiClientError::Invalid(
            "The collection exceeds 32 KiB. Split or reduce its requests before saving.".into(),
        ));
    }
    let mut ids = std::collections::HashSet::new();
    for item in &collection.requests {
        if !ids.insert(item.id) {
            return Err(ApiClientError::Invalid(
                "A saved request ID is repeated. Give each request a unique ID.".into(),
            ));
        }
        request(item, &collection.environment, &[], &[])?;
    }
    Ok(())
}
fn valid_key(key: &str) -> ApiResult<()> {
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(ApiClientError::Invalid(
            "Variable names use letters, numbers and underscores.".into(),
        ));
    }
    Ok(())
}
