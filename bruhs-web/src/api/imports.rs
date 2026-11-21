//! The import routes for chat service exports and local agents (D3, D10).
//! Every call takes the dialog's `AbortSignal`, so closing it drops an
//! upload in flight, a poll or a thread read. An upload is a multipart
//! body with one `file` field per chosen `File`, which the browser streams
//! from disk; the server never reads the file names (F33).

use bruhs_models::imports::{valid_batch_id, AgentScan, ImportJob, IMPORT_FAILED, IMPORT_GONE};
use bruhs_models::logic::portable::{ImportSource, ImportedThread};
use gloo_net::http::{Method, RequestBuilder, Response};
use web_sys::AbortSignal;

use super::http::{decode, segment, send_request, url};
use super::ApiError;

const UPLOADS_PATH: &str = "/api/import/uploads";

/// `/api/import/uploads/{id}`, for a batch id the server would take.
fn job_path(id: &str) -> Option<String> {
    valid_batch_id(id).then(|| format!("{UPLOADS_PATH}/{id}"))
}

/// One conversation of a read batch.
fn thread_path(source: ImportSource, upload: &str, id: &str) -> Option<String> {
    valid_batch_id(upload).then(|| {
        format!(
            "/api/import/thread?source={}&id={}&upload={upload}",
            source.as_str(),
            segment(id)
        )
    })
}

/// An agent scan, of `path` when one was typed.
fn agents_path(source: ImportSource, path: Option<&str>) -> String {
    let mut route = format!("/api/import/agents?source={}", source.as_str());
    if let Some(path) = path.map(str::trim).filter(|path| !path.is_empty()) {
        route.push_str("&path=");
        route.push_str(&segment(path));
    }
    route
}

/// A batch id the server would refuse is refused here the same way: 404
/// with `IMPORT_GONE`, before any request.
fn checked(path: Option<String>) -> Result<String, ApiError> {
    path.ok_or_else(|| ApiError::Http {
        status: 404,
        detail: Some(IMPORT_GONE.into()),
    })
}

fn builder(method: Method, route: &str, signal: &AbortSignal) -> RequestBuilder {
    RequestBuilder::new(&url(route))
        .method(method)
        .abort_signal(Some(signal))
}

async fn get(route: &str, signal: &AbortSignal) -> Result<Response, ApiError> {
    send_request(builder(Method::GET, route, signal).build()?).await
}

/// The browser couldn't build the upload: shown as the import's own
/// failure copy, never the browser's text (M9).
fn browser_error(_: wasm_bindgen::JsValue) -> ApiError {
    upload_not_built()
}

fn upload_not_built() -> ApiError {
    ApiError::Http {
        status: 400,
        detail: Some(IMPORT_FAILED.into()),
    }
}

/// `POST /api/import/uploads`: the chosen export files as one `file` field
/// each. Answers 202 with the read; poll it with [`import_job`].
pub async fn upload_exports(
    files: Vec<web_sys::File>,
    signal: &AbortSignal,
) -> Result<ImportJob, ApiError> {
    let form = web_sys::FormData::new().map_err(browser_error)?;
    for file in &files {
        form.append_with_blob("file", file).map_err(browser_error)?;
    }
    // No Content-Type: the browser sets the multipart boundary.
    let request = builder(Method::POST, UPLOADS_PATH, signal).body(form)?;
    decode(send_request(request).await?).await
}

/// `GET /api/import/uploads/{id}`: where the read stands.
pub async fn import_job(id: &str, signal: &AbortSignal) -> Result<ImportJob, ApiError> {
    decode(get(&checked(job_path(id))?, signal).await?).await
}

/// `DELETE /api/import/uploads/{id}`: ends the read, or discards its batch.
pub async fn discard_import(id: &str, signal: &AbortSignal) -> Result<(), ApiError> {
    let route = checked(job_path(id))?;
    send_request(builder(Method::DELETE, &route, signal).build()?).await?;
    Ok(())
}

/// `GET /api/import/thread?source=&id=&upload=`: one conversation of a
/// read batch, ready to become a thread.
pub async fn import_export_thread(
    source: ImportSource,
    upload: &str,
    id: &str,
    signal: &AbortSignal,
) -> Result<ImportedThread, ApiError> {
    decode(get(&checked(thread_path(source, upload, id))?, signal).await?).await
}

/// `GET /api/import/agents?source=&path=`: OpenClaw or Hermes Agent
/// personas, in `path` or the agent's default places.
#[expect(dead_code, reason = "used from Task 11")]
pub async fn scan_agents(
    source: ImportSource,
    path: Option<&str>,
    signal: &AbortSignal,
) -> Result<AgentScan, ApiError> {
    decode(get(&agents_path(source, path), signal).await?).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn paths_are_api_shaped() {
        assert_eq!(
            thread_path(ImportSource::ChatGpt, ID, "a b/c").as_deref(),
            Some(format!("/api/import/thread?source=chatgpt&id=a%20b%2Fc&upload={ID}").as_str())
        );
        assert_eq!(
            job_path(ID).as_deref(),
            Some(format!("/api/import/uploads/{ID}").as_str())
        );
        assert_eq!(
            agents_path(ImportSource::OpenClaw, None),
            "/api/import/agents?source=openclaw"
        );
        assert_eq!(
            agents_path(ImportSource::Hermes, Some(" ~/my agent ")),
            "/api/import/agents?source=hermes&path=~%2Fmy%20agent"
        );
        assert_eq!(
            agents_path(ImportSource::Hermes, Some("  ")),
            "/api/import/agents?source=hermes"
        );
        for bad in [
            "",
            "0123456789ABCDEF0123456789ABCDEF",
            "0123456789abcdef0123456789abcde",
            "0123456789abcdef0123456789abcdef0",
            "../../api/vault/0123456789abcdef",
            "0123456789abcdef&upload=0123456789abcdef",
        ] {
            assert_eq!(thread_path(ImportSource::ChatGpt, bad, "x"), None, "{bad}");
            assert_eq!(job_path(bad), None, "{bad}");
        }
    }

    #[test]
    fn an_upload_the_browser_cannot_build_says_so_plainly() {
        assert_eq!(upload_not_built().to_string(), IMPORT_FAILED);
    }

    #[test]
    fn a_refused_id_is_refused_before_any_request() {
        // `checked` runs before a request is built, so a bad id never
        // reaches the network.
        assert_eq!(
            checked(thread_path(ImportSource::Grok, "../x", "g1")),
            Err(ApiError::Http {
                status: 404,
                detail: Some(IMPORT_GONE.into())
            })
        );
    }
}
