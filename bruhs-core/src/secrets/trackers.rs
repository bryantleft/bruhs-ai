//! Purpose-bound tracker reads. Saved tokens never leave this credential boundary.
use super::SecretStore;
use crate::delivery::{DeliveryError, DeliveryResult};
use base64::Engine;
use bruhs_models::{
    api_client::PAYLOAD_BYTES,
    delivery::{Issue, TrackerConfig, TrackerProvider},
};
use serde_json::{json, Value};
use zeroize::Zeroizing;

impl SecretStore {
    pub(crate) async fn read_tracker_issue(
        &self,
        client: &reqwest::Client,
        project: &str,
        config: &TrackerConfig,
        credential_name: &str,
        key: &str,
    ) -> DeliveryResult<Issue> {
        let token = self.find(project, credential_name).await.map_err(|_| DeliveryError::Storage("Could not unlock this project's tracker credential. Unlock the OS keychain and retry.".into()))?.filter(|token| !token.is_empty()).ok_or_else(|| DeliveryError::Invalid("Save the tracker API token in this project's Delivery → Issue tracker settings.".into()))?;
        let authorization = Zeroizing::new(if config.oauth {
            format!("Bearer {}", token.as_str())
        } else if config.provider == TrackerProvider::Jira {
            format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode(format!(
                    "{}:{}",
                    config.email,
                    token.as_str()
                ))
            )
        } else {
            token.to_string()
        });
        let mut header = reqwest::header::HeaderValue::from_str(&authorization).map_err(|_| DeliveryError::Invalid("The saved tracker token is not a valid HTTP credential. Replace it in Settings → Credentials.".into()))?;
        header.set_sensitive(true);
        let api_origin = if config.provider == TrackerProvider::Jira && config.oauth {
            let response = client.get("https://api.atlassian.com/oauth/token/accessible-resources").header(reqwest::header::AUTHORIZATION, header.clone()).header(reqwest::header::ACCEPT, "application/json").send().await.map_err(|_| DeliveryError::Command("Atlassian site discovery failed. Check the OAuth token and network connection.".into()))?;
            let resources = tracker_json(response, &token, &authorization).await?;
            jira_oauth_origin(&config.origin, &resources)?
        } else {
            config.origin.clone()
        };
        let mut request = match config.provider {
            TrackerProvider::Linear => client.post(format!("{}/graphql",config.origin)).json(&json!({"query":"query BruhsIssue($id: String!) { issue(id: $id) { identifier title description url number } }","variables":{"id":key}})),
            TrackerProvider::Jira => client.get(format!("{}/rest/api/3/issue/{key}",api_origin)).query(&[("fields","summary,description")]),
        };
        request = request
            .header(reqwest::header::AUTHORIZATION, header)
            .header(reqwest::header::ACCEPT, "application/json");
        let response = request.send().await.map_err(|_| DeliveryError::Command("The tracker could not be reached. Check its origin, TLS certificate, sign-in and network connection.".into()))?;
        let value = tracker_json(response, &token, &authorization).await?;
        decode_issue(config, key, &value)
    }
}
fn jira_oauth_origin(site: &str, resources: &Value) -> DeliveryResult<String> {
    let mut matches = resources
        .as_array()
        .ok_or_else(|| DeliveryError::Command("Atlassian returned unreadable site access.".into()))?
        .iter()
        .filter(|resource| {
            resource["url"]
                .as_str()
                .is_some_and(|url| url.trim_end_matches('/') == site)
        });
    let resource=matches.next().ok_or_else(||DeliveryError::Invalid("The OAuth token does not grant access to this Jira site. Connect that site or use its account email and API token.".into()))?;
    let id = resource["id"]
        .as_str()
        .and_then(|id| uuid::Uuid::parse_str(id).ok())
        .ok_or_else(|| {
            DeliveryError::Command("Atlassian returned an invalid Jira cloud ID.".into())
        })?;
    if matches.any(|other| {
        other["id"]
            .as_str()
            .is_some_and(|other| other != id.to_string())
    }) {
        return Err(DeliveryError::Command(
            "Atlassian returned ambiguous site access. Connect this Jira site again.".into(),
        ));
    }
    Ok(format!("https://api.atlassian.com/ex/jira/{id}"))
}
async fn tracker_json(
    mut response: reqwest::Response,
    token: &str,
    authorization: &str,
) -> DeliveryResult<Value> {
    if !response.status().is_success() {
        return Err(DeliveryError::Command(format!("The tracker returned HTTP {}. Check the saved account's issue-read permission and issue identifier.",response.status().as_u16())));
    }
    // Reuse the local API client's bounded JSON allocation; never return a truncated issue.
    let mut bytes = Zeroizing::new(Vec::new());
    while let Some(chunk) = response.chunk().await.map_err(|_| {
        DeliveryError::Command("The tracker response was interrupted. Retry the issue read.".into())
    })? {
        if bytes.len().saturating_add(chunk.len()) > PAYLOAD_BYTES {
            return Err(DeliveryError::Command("The issue exceeds the local API client's 32 KiB response budget. Shorten its description or open it in the tracker.".into()));
        }
        bytes.extend_from_slice(&chunk);
    }
    let mut text = Zeroizing::new(
        String::from_utf8(bytes.to_vec())
            .map_err(|_| DeliveryError::Command("The tracker returned non-UTF-8 JSON.".into()))?,
    );
    // A hostile or misconfigured tracker must not reflect credentials into an issue receipt.
    let escaped = serde_json::to_string(token)
        .map_err(|_| DeliveryError::Command("Could not decode the tracker response.".into()))?;
    for pattern in [
        token.to_owned(),
        authorization.to_owned(),
        escaped[1..escaped.len() - 1].to_owned(),
        base64::engine::general_purpose::STANDARD.encode(token.as_bytes()),
        url::form_urlencoded::byte_serialize(token.as_bytes()).collect(),
    ] {
        *text = text.replace(&pattern, "[redacted]");
    }
    let value: Value = serde_json::from_str(&text)
        .map_err(|_| DeliveryError::Command("The tracker returned invalid JSON.".into()))?;
    Ok(value)
}
fn decode_issue(config: &TrackerConfig, key: &str, value: &Value) -> DeliveryResult<Issue> {
    let missing = || {
        DeliveryError::Command("The tracker returned no readable issue. Check the identifier and the account's issue-read permission.".into())
    };
    match config.provider {
        TrackerProvider::Linear => {
            if value
                .get("errors")
                .and_then(Value::as_array)
                .is_some_and(|errors| !errors.is_empty())
            {
                return Err(DeliveryError::Command("Linear reported a GraphQL error. Check issue access and the saved API token; partial data was discarded.".into()));
            }
            let issue = &value["data"]["issue"];
            if issue["identifier"].as_str() != Some(key) {
                return Err(missing());
            }
            Ok(Issue {
                number: issue["number"].as_u64().ok_or_else(missing)?,
                title: format!("{key}: {}", issue["title"].as_str().ok_or_else(missing)?),
                body: issue["description"].as_str().unwrap_or_default().into(),
                url: issue["url"].as_str().ok_or_else(missing)?.into(),
            })
        }
        TrackerProvider::Jira => {
            if value["key"].as_str() != Some(key) {
                return Err(missing());
            }
            let number = key
                .rsplit_once('-')
                .and_then(|(_, number)| number.parse::<u64>().ok())
                .ok_or_else(missing)?;
            let mut body = String::new();
            adf_text(&value["fields"]["description"], &mut body);
            Ok(Issue {
                number,
                title: format!(
                    "{key}: {}",
                    value["fields"]["summary"].as_str().ok_or_else(missing)?
                ),
                body,
                url: format!("{}/browse/{key}", config.origin),
            })
        }
    }
}
fn adf_text(node: &Value, output: &mut String) {
    if let Some(text) = node.as_str() {
        output.push_str(text);
        return;
    }
    if let Some(text) = node["text"].as_str() {
        output.push_str(text);
    }
    match node["type"].as_str() {
        Some("hardBreak") => output.push('\n'),
        Some("mention") => {
            if let Some(text) = node["attrs"]["text"].as_str() {
                output.push_str(text);
            }
        }
        Some("emoji") => {
            if let Some(text) = node["attrs"]["text"]
                .as_str()
                .or_else(|| node["attrs"]["shortName"].as_str())
            {
                output.push_str(text);
            }
        }
        _ => {}
    }
    if let Some(children) = node["content"].as_array() {
        for child in children {
            adf_text(child, output);
        }
    }
    if matches!(
        node["type"].as_str(),
        Some("paragraph" | "heading" | "listItem" | "blockquote" | "codeBlock" | "tableRow")
    ) && !output.ends_with('\n')
    {
        output.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    async fn fixture(body: Value) -> (String, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|value| value.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let body = body.to_string();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).as_bytes()).await.unwrap();
            String::from_utf8(bytes).unwrap()
        });
        (format!("http://{address}"), task)
    }

    #[tokio::test]
    async fn linear_http_contract_uses_graphql_variables_and_redacts_reflected_credentials() {
        let token = "fixture-key-do-not-export";
        let (origin, request) = fixture(json!({"data":{"issue":{"identifier":"ENG-42","number":42,"title":"Read an issue","description":format!("Reflected {token}"),"url":"https://linear.app/team/issue/ENG-42/read"}}})).await;
        let store = SecretStore::in_memory();
        store
            .save("/fixture", "linear", Zeroizing::new(token.into()))
            .await
            .unwrap();
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let config = TrackerConfig {
            provider: TrackerProvider::Linear,
            origin,
            email: String::new(),
            oauth: false,
        };
        let issue = store
            .read_tracker_issue(&client, "/fixture", &config, "linear", "ENG-42")
            .await
            .unwrap();
        assert_eq!(issue.body, "Reflected [redacted]");
        let request = request.await.unwrap();
        assert!(request.starts_with("POST /graphql HTTP/1.1"));
        assert!(request
            .to_ascii_lowercase()
            .contains(&format!("authorization: {token}")));
        let json: Value = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(json["variables"]["id"], "ENG-42");
        assert!(!serde_json::to_string(&issue).unwrap().contains(token));
    }

    #[tokio::test]
    async fn jira_http_contract_binds_basic_auth_to_the_configured_site_and_rejects_missing_context(
    ) {
        let (origin, request) = fixture(json!({"key":"OPS-7","fields":{"summary":"Fix issue intake","description":{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Issue context"}]}]}}})).await;
        let store = SecretStore::in_memory();
        store
            .save("/fixture", "jira", Zeroizing::new("fixture-token".into()))
            .await
            .unwrap();
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let config = TrackerConfig {
            provider: TrackerProvider::Jira,
            origin,
            email: "owner@example.com".into(),
            oauth: false,
        };
        let issue = store
            .read_tracker_issue(&client, "/fixture", &config, "jira", "OPS-7")
            .await
            .unwrap();
        assert_eq!(issue.title, "OPS-7: Fix issue intake");
        let request = request.await.unwrap();
        assert!(request
            .starts_with("GET /rest/api/3/issue/OPS-7?fields=summary%2Cdescription HTTP/1.1"));
        assert!(request.contains(
            &base64::engine::general_purpose::STANDARD.encode("owner@example.com:fixture-token")
        ));
        assert!(store
            .read_tracker_issue(&client, "/other-project", &config, "jira", "OPS-7")
            .await
            .is_err());
    }
    #[test]
    fn jira_adf_and_graphql_errors_are_not_silently_dropped() {
        let config = TrackerConfig {
            provider: TrackerProvider::Jira,
            origin: "https://team.atlassian.net".into(),
            email: "owner@example.com".into(),
            oauth: false,
        };
        let issue=decode_issue(&config,"ENG-42",&json!({"key":"ENG-42","fields":{"summary":"Finish intake","description":{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Hello "},{"type":"mention","attrs":{"text":"@owner"}}]}]}}})).unwrap();
        assert_eq!(issue.body, "Hello @owner\n");
        assert!(decode_issue(
            &TrackerConfig {
                provider: TrackerProvider::Linear,
                ..config
            },
            "ENG-42",
            &json!({"data":{"issue":{}},"errors":[{"message":"denied"}]})
        )
        .is_err());
    }
    #[test]
    fn jira_oauth_selects_only_the_owner_configured_site() {
        let resources = json!([{"url":"https://other.atlassian.net","id":"440527fc-033e-45a5-931b-a2244027d06a"},{"url":"https://team.atlassian.net","id":"d3f9fcd9-2741-4bc2-b34b-140917077a68"}]);
        assert_eq!(
            jira_oauth_origin("https://team.atlassian.net", &resources).unwrap(),
            "https://api.atlassian.com/ex/jira/d3f9fcd9-2741-4bc2-b34b-140917077a68"
        );
        assert!(jira_oauth_origin("https://missing.atlassian.net", &resources).is_err());
        assert!(jira_oauth_origin(
            "https://team.atlassian.net",
            &json!([{"url":"https://team.atlassian.net","id":"../../another"}])
        )
        .is_err());
    }
}
