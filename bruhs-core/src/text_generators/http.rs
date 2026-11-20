use super::*;
use base64::Engine;
use reqwest::{Client, RequestBuilder, Url};
use serde_json::{json, Value};
use std::{collections::BTreeSet, time::Duration};

pub(super) fn base_url(value: &str) -> Result<String, String> {
    let url = Url::parse(value.trim()).map_err(|_| "Enter a complete HTTP or HTTPS base URL.")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.query().is_some()
    {
        return Err(
            "Enter an HTTP or HTTPS base URL without credentials, query or fragment.".into(),
        );
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}
fn client() -> Result<Client, String> {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(90))
        .build()
        .map_err(|_| "Could not start the HTTP client.".into())
}
fn auth(row: &Generator, key: Option<&str>, mut request: RequestBuilder) -> RequestBuilder {
    if row.preset == Preset::Anthropic {
        request = request.header("anthropic-version", "2023-06-01");
        if let Some(k) = key {
            request = request.header("x-api-key", k);
        }
    } else if let Some(k) = key {
        request = request.bearer_auth(k);
    }
    request
}
async fn response(request: RequestBuilder) -> Result<Value, String> {
    let mut r = request.send().await.map_err(|e| {
        if e.is_timeout() {
            "The text generator timed out."
        } else {
            "Could not reach the text generator. Check the base URL and network connection."
        }
    })?;
    if !r.status().is_success() {
        return Err(match r.status().as_u16() {
            401 | 403 => "The endpoint rejected the API key.".into(),
            429 => "The endpoint rate limit was reached. Try again later.".into(),
            n => format!("The text generator returned HTTP {n}."),
        });
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = r
        .chunk()
        .await
        .map_err(|_| "The endpoint response was interrupted.")?
    {
        if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
            return Err("The endpoint response exceeded 8 MiB.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "The endpoint returned invalid JSON.".into())
}
pub(super) async fn models(row: &Generator, key: Option<&str>) -> Result<Vec<Model>, String> {
    let client = client()?;
    let mut result = Vec::new();
    let mut seen = BTreeSet::new();
    let mut cursors = BTreeSet::new();
    let mut cursor = None::<String>;
    loop {
        let mut request = client.get(format!("{}/models", row.url));
        if let Some(id) = &cursor {
            request = request.query(&[("after_id", id)]);
        }
        let data = response(auth(row, key, request)).await?;
        for value in data["data"]
            .as_array()
            .ok_or("The endpoint did not return a model list.")?
        {
            if let Some(id) = value["id"].as_str().filter(|id| {
                !id.trim().is_empty() && id.len() <= 512 && !id.chars().any(char::is_control)
            }) {
                if seen.insert(id.to_owned()) {
                    let name = value["display_name"]
                        .as_str()
                        .or(value["name"].as_str())
                        .filter(|n| !n.trim().is_empty())
                        .unwrap_or(id);
                    result.push(Model {
                        id: id.into(),
                        name: name.into(),
                    });
                }
            }
        }
        if row.preset != Preset::Anthropic || data["has_more"] != true {
            break;
        }
        let last = data["last_id"]
            .as_str()
            .ok_or("Incomplete model list from Anthropic.")?
            .to_owned();
        if !cursors.insert(last.clone()) || cursors.len() > 100 {
            return Err("The endpoint did not finish listing models.".into());
        }
        cursor = Some(last);
    }
    result.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(result)
}
pub(super) async fn generate(
    row: &Generator,
    key: Option<&str>,
    model: &str,
    instructions: &str,
    prompt: &str,
    images: &[NamingImage],
) -> Result<String, String> {
    let anthropic = row.preset == Preset::Anthropic;
    let mut content = vec![json!({"type":"text","text":prompt})];
    for image in images {
        let data = base64::engine::general_purpose::STANDARD.encode(&image.bytes);
        content.push(if anthropic {json!({"type":"image","source":{"type":"base64","media_type":image.mime,"data":data}})} else {json!({"type":"image_url","image_url":{"url":format!("data:{};base64,{data}",image.mime)}})});
    }
    let (path, body) = if anthropic {
        (
            "messages",
            json!({"model":model,"max_tokens":4096,"system":instructions,"messages":[{"role":"user","content":content}]}),
        )
    } else {
        (
            "chat/completions",
            json!({"model":model,"messages":[{"role":"system","content":instructions},{"role":"user","content":if images.is_empty(){Value::String(prompt.into())}else{Value::Array(content)}}]}),
        )
    };
    let data =
        response(auth(row, key, client()?.post(format!("{}/{path}", row.url))).json(&body)).await?;
    let output = if anthropic {
        &data["content"]
    } else {
        &data["choices"][0]["message"]["content"]
    };
    let text = match output {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter(|p| p["type"] == "text")
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    };
    if text.trim().is_empty() {
        Err("The endpoint returned no text for this model.".into())
    } else {
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        matchers::{body_partial_json, header, method, path, query_param},
        Mock, MockServer, ResponseTemplate,
    };
    fn fixture(url: String, preset: Preset) -> Generator {
        Generator {
            id: Uuid::new_v4(),
            revision: 0,
            preset,
            label: String::new(),
            url,
            enabled: true,
            models: vec![],
            key: None,
            reference: None,
        }
    }
    #[tokio::test]
    async fn anthropic_paginates_with_auth_and_accepts_attached_images() {
        let server = MockServer::start().await;
        let row = fixture(server.uri(), Preset::Anthropic);
        let authenticated = || {
            Mock::given(header("x-api-key", "fixture-key"))
                .and(header("anthropic-version", "2023-06-01"))
        };
        authenticated()
            .and(method("GET"))
            .and(path("/models"))
            .and(query_param("after_id", "a"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!({"data":[{"id":"b","display_name":"Beta"}],"has_more":false}),
                ),
            )
            .expect(1)
            .mount(&server)
            .await;
        authenticated()
            .and(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"data":[{"id":"a","display_name":"Alpha"}],"last_id":"a","has_more":true}),
            ))
            .expect(1)
            .mount(&server)
            .await;
        let result = models(&row, Some("fixture-key")).await.unwrap();
        assert_eq!(
            result.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
            ["Alpha", "Beta"]
        );
        authenticated().and(method("POST")).and(path("/messages")).and(body_partial_json(json!({
            "system":"Name this image","model":"b","messages":[{"role":"user","content":[{"type":"text","text":"Diagram"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AQI="}}]}]
        }))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"content":[{"type":"text","text":"A "},{"type":"text","text":"diagram"}]}))).expect(1).mount(&server).await;
        let image = NamingImage {
            mime: "image/png",
            bytes: vec![1, 2],
        };
        assert_eq!(
            generate(
                &row,
                Some("fixture-key"),
                "b",
                "Name this image",
                "Diagram",
                &[image]
            )
            .await
            .unwrap(),
            "A diagram"
        );
    }
    #[tokio::test]
    async fn bearer_probe_reports_authentication_failure_and_refuses_redirects() {
        let origin = MockServer::start().await;
        let target = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&target)
            .await;
        let row = fixture(origin.uri(), Preset::OpenRouter);
        Mock::given(header("authorization", "Bearer invalid"))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount(&origin)
            .await;
        Mock::given(header("authorization", "Bearer secret"))
            .respond_with(ResponseTemplate::new(307).insert_header("location", target.uri()))
            .expect(1)
            .mount(&origin)
            .await;
        assert!(models(&row, Some("invalid"))
            .await
            .unwrap_err()
            .contains("API key"));
        assert!(models(&row, Some("secret"))
            .await
            .unwrap_err()
            .contains("307"));
    }
}
