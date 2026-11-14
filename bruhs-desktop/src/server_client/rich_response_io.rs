use super::{ServerClient, ServerError};
use bruhs_models::logic::rich_response::{media_is_remote, media_url, WidgetState};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::io::AsyncWriteExt as _;

#[derive(Clone, Deserialize)]
pub struct RichArtifact {
    pub id: String,
    pub title: String,
    pub html: String,
}

#[derive(Deserialize)]
pub struct CapturedRichMedia {
    pub src: String,
    pub signed_url: String,
}

impl ServerClient {
    pub async fn proxy_rich_media(&self, url: &str) -> Result<CapturedRichMedia, ServerError> {
        self.send_json(
            reqwest::Method::POST,
            "/api/agent/rich-response/remote-media",
            Some(&serde_json::json!({"url":url})),
        )
        .await
    }
    fn widget_state_url(&self, key: &str) -> Result<reqwest::Url, ServerError> {
        let mut url = reqwest::Url::parse(&self.url("/api/agent/rich-response/state"))
            .map_err(|_| ServerError::Invalid("Invalid widget state URL"))?;
        url.query_pairs_mut().append_pair("key", key);
        Ok(url)
    }

    pub async fn widget_state(&self, key: &str) -> Result<WidgetState, ServerError> {
        let response = Self::check(
            self.http()?.get(self.widget_state_url(key)?).send().await,
            "Read widget state",
        )
        .await?;
        Self::decode(response, "Read widget state").await
    }

    pub async fn save_widget_state(
        &self,
        key: &str,
        state: &WidgetState,
    ) -> Result<(), ServerError> {
        Self::check(
            self.http()?
                .put(self.widget_state_url(key)?)
                .json(state)
                .send()
                .await,
            "Save widget state",
        )
        .await?;
        Ok(())
    }

    pub async fn capture_rich_media(
        &self,
        src: &str,
        project: &str,
    ) -> Result<String, ServerError> {
        Ok(self.capture_rich_asset(src, project).await?.src)
    }

    pub async fn capture_rich_asset(
        &self,
        src: &str,
        project: &str,
    ) -> Result<CapturedRichMedia, ServerError> {
        let captured: CapturedRichMedia = self
            .send_json(
                reqwest::Method::POST,
                "/api/agent/rich-response/media",
                Some(&serde_json::json!({"src":src,"project":project})),
            )
            .await?;
        Ok(captured)
    }

    fn artifact_path(&self, id: &str, diagnostics: bool) -> Result<String, ServerError> {
        let mut url = reqwest::Url::parse(&self.url("/api/agent/artifacts/"))
            .map_err(|_| ServerError::Invalid("Invalid artifact URL"))?;
        let mut segments = url
            .path_segments_mut()
            .map_err(|()| ServerError::Invalid("Invalid artifact URL"))?;
        segments.pop_if_empty().push(id);
        if diagnostics {
            segments.push("diagnostics");
        }
        drop(segments);
        Ok(url.to_string())
    }

    pub async fn rich_artifact(&self, id: &str) -> Result<RichArtifact, ServerError> {
        let response = Self::check(
            self.http()?
                .get(self.artifact_path(id, false)?)
                .send()
                .await,
            "Read artifact",
        )
        .await?;
        Self::decode(response, "Read artifact").await
    }

    pub async fn report_artifact(&self, id: &str, errors: &[String]) -> Result<(), ServerError> {
        #[derive(Serialize)]
        struct Report<'a> {
            state: &'a str,
            errors: &'a [String],
        }
        Self::check(
            self.http()?
                .post(self.artifact_path(id, true)?)
                .json(&Report {
                    state: if errors.is_empty() { "ready" } else { "error" },
                    errors,
                })
                .send()
                .await,
            "Report artifact diagnostics",
        )
        .await?;
        Ok(())
    }

    pub async fn save_rich_media(
        &self,
        src: &str,
        project: &str,
        path: &Path,
    ) -> Result<(), ServerError> {
        let request = if media_is_remote(src) {
            self.http()?
                .post(self.url("/api/agent/rich-response/download"))
                .json(&serde_json::json!({"url":src}))
        } else {
            self.http()?.get(media_url(&self.base, src, Some(project)))
        };
        let response = Self::check(request.send().await, "Download response media").await?;
        save_download(response, path).await
    }

    pub async fn export_rich_reply(
        &self,
        markdown: &str,
        project: &str,
        state_keys: &[String],
        include_state: bool,
        include_remote: bool,
        path: &Path,
    ) -> Result<(), ServerError> {
        let response = Self::check(self.http()?.post(self.url("/api/agent/rich-response/export")).json(&serde_json::json!({"markdown":markdown,"project":project,"state_keys":state_keys,"include_state":include_state,"include_remote":include_remote})).send().await, "Export response bundle").await?;
        save_download(response, path).await
    }
}

async fn save_download(mut response: reqwest::Response, path: &Path) -> Result<(), ServerError> {
    let temporary = path.with_file_name(format!(".bruhs-download-{}.part", uuid::Uuid::new_v4()));
    let result = async {
        let mut file = tokio::fs::File::create(&temporary).await.map_err(|_| {
            ServerError::Invalid(
                "Could not create a temporary download file beside the selected file",
            )
        })?;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|source| ServerError::Transport {
                request: "Download response".into(),
                source: source.without_url(),
            })?
        {
            file.write_all(&chunk)
                .await
                .map_err(|_| ServerError::Invalid("Could not write the selected download file"))?;
        }
        file.flush()
            .await
            .map_err(|_| ServerError::Invalid("Could not finish the selected download file"))?;
        drop(file);
        tokio::fs::rename(&temporary, path).await.map_err(|_| {
            ServerError::Invalid(
                "Could not save the completed response download to the selected location",
            )
        })
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temporary).await;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::ServerClient;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn mock_client(body: &'static str) -> (ServerClient, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = vec![0; 8192];
            let read = socket.read(&mut bytes).await.unwrap();
            let response=format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",body.len(),body);
            socket.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8(bytes[..read].to_vec()).unwrap()
        });
        (
            ServerClient::new(base, "http://127.0.0.1:9").unwrap(),
            server,
        )
    }

    #[tokio::test]
    async fn widget_state_uses_bound_key_and_typed_snapshot() {
        let (client, request) = mock_client(r#"{"custom":{"count":7}}"#).await;
        let state = client.widget_state("thread/widget").await.unwrap();
        assert_eq!(state.custom["count"], 7);
        assert!(request
            .await
            .unwrap()
            .starts_with("GET /api/agent/rich-response/state?key=thread%2Fwidget "));
        let (client, request) = mock_client("{}").await;
        client.save_widget_state("bound-key", &state).await.unwrap();
        let request = request.await.unwrap();
        assert!(request.starts_with("PUT /api/agent/rich-response/state?key=bound-key "));
        assert!(request.contains(r#""custom":{"count":7}"#));
    }

    #[tokio::test]
    async fn explicit_remote_download_uses_the_public_download_proxy() {
        let (client, request) = mock_client("image bytes").await;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("image.png");
        client
            .save_rich_media("https://example.com/image.png", "/project", &path)
            .await
            .unwrap();
        let request = request.await.unwrap();
        assert!(request.starts_with("POST /api/agent/rich-response/download "));
        assert!(request.contains("https://example.com/image.png"));
        assert_eq!(std::fs::read(path).unwrap(), b"image bytes");
    }

    #[tokio::test]
    async fn reply_export_explicitly_controls_widget_state() {
        let (client, request) = mock_client("zip bytes").await;
        let directory = tempfile::tempdir().unwrap();
        client
            .export_rich_reply(
                "Reply",
                "/project",
                &["widget-key".into()],
                true,
                false,
                &directory.path().join("response.zip"),
            )
            .await
            .unwrap();
        let request = request.await.unwrap();
        assert!(request.contains(r#""include_state":true"#));
        assert!(request.contains(r#""state_keys":["widget-key"]"#));
    }

    #[tokio::test]
    async fn failed_download_preserves_an_existing_selected_file() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 4096];
            assert!(socket.read(&mut bytes).await.unwrap() > 0);
            socket
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 100\r\nconnection: close\r\n\r\nx")
                .await
                .unwrap();
        });
        let client = ServerClient::new(base, "http://127.0.0.1:9").unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("existing.png");
        std::fs::write(&path, b"keep existing").unwrap();
        assert!(client
            .save_rich_media("/project/image.png", "/project", &path)
            .await
            .is_err());
        server.await.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"keep existing");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
