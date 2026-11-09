use bruhs_models::{PersonProfile, PersonVersionPage};
use gloo_net::http::Method;

use super::http::{get_json, send_json};
use super::ApiError;

pub async fn person_versions(after: Option<&str>) -> Result<PersonVersionPage, ApiError> {
    let mut route = "/api/people/versions".to_string();
    if let Some(after) = after {
        let encoded: String = js_sys::encode_uri_component(after).into();
        route.push_str(&format!("?after={encoded}"));
    }
    let page: PersonVersionPage = get_json(&route).await?;
    page.validate(after)
        .map_err(|error| ApiError::Decode(error.to_string()))?;
    Ok(page)
}

/// Profiles for `ids`. Callers send at most
/// [`bruhs_models::PERSON_PROFILE_BATCH_SIZE`]
/// ids per call; the server rejects a larger batch (an `Http` 400).
pub async fn get_people(ids: &[String]) -> Result<Vec<PersonProfile>, ApiError> {
    let body = serde_json::json!({ "ids": ids });
    let profiles: Vec<PersonProfile> =
        send_json(Method::POST, "/api/people/profiles", Some(&body)).await?;
    let requested = ids.iter().collect::<std::collections::HashSet<_>>();
    let mut seen = std::collections::HashSet::new();
    if profiles.len() > ids.len()
        || profiles
            .iter()
            .any(|p| !requested.contains(&p.id) || !seen.insert(&p.id))
    {
        return Err(ApiError::Decode(
            "contact profile batch contains unexpected or duplicate identities".into(),
        ));
    }
    Ok(profiles)
}
