use serde::Deserialize;
use serde_json::{json, Value};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{io::AsyncReadExt, process::Command};
use url::Url;

struct Link {
    url: Url,
    endpoint: String,
    github: bool,
}

fn link(candidate: &str) -> Option<Link> {
    let mut url = Url::parse(candidate).ok()?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return None;
    }
    url.set_query(None);
    url.set_fragment(None);
    let parts: Vec<_> = url.path_segments()?.collect();
    let valid = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    };
    let id =
        |s: &str| !s.is_empty() && !s.starts_with('0') && s.bytes().all(|b| b.is_ascii_digit());
    let github = url.host_str()? == "github.com";
    let endpoint = match (url.host_str()?, parts.as_slice()) {
        ("github.com", [org, repo, "issues" | "pull", n, ..])
            if valid(org) && valid(repo) && id(n) =>
        {
            format!("repos/{org}/{repo}/issues/{n}")
        }
        ("gitlab.com", _) => {
            let marker = parts.iter().position(|s| *s == "-")?;
            let project = &parts[..marker];
            let [kind @ ("merge_requests" | "issues"), n, ..] = &parts[marker + 1..] else {
                return None;
            };
            if project.is_empty() || project.iter().any(|p| p.is_empty()) || !id(n) {
                return None;
            }
            let project: String =
                url::form_urlencoded::byte_serialize(project.join("/").as_bytes()).collect();
            format!("projects/{project}/{kind}/{n}")
        }
        _ => return None,
    };
    Some(Link {
        url,
        endpoint,
        github,
    })
}

fn links(text: &str) -> Vec<Link> {
    let mut found = Vec::new();
    for (start, _) in text.match_indices("https://") {
        let tail = &text[start..];
        let end = tail
            .find(|ch: char| ch.is_whitespace() || "<>\"')]\x60".contains(ch))
            .unwrap_or(tail.len());
        let candidate = tail[..end].trim_end_matches(['.', ',', ':', ';', '!', '?', '*']);
        if let Some(link) = link(candidate) {
            if found.iter().any(|item: &Link| item.url == link.url) {
                continue;
            }
            found.push(link);
            if found.len() == 2 {
                break;
            }
        }
    }
    found
}

#[derive(Deserialize)]
struct Subject {
    title: String,
    #[serde(alias = "description")]
    body: Option<String>,
}

const MAX_SUBJECT_BYTES: usize = 1_000_000;

/// An environment token also works on installations without the GitHub CLI.
/// Keep it on the fixed API origin, including when a repository has moved.
fn github_request(endpoint: &str, token: &str) -> Option<reqwest::RequestBuilder> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("bruhs/", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()?;
    Some(
        client
            .get(format!("https://api.github.com/{endpoint}"))
            .bearer_auth(token),
    )
}

async fn github_subject(request: reqwest::RequestBuilder) -> Option<Subject> {
    let mut response = request.send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let mut data = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if chunk.len() > MAX_SUBJECT_BYTES - data.len() {
            return None;
        }
        data.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&data).ok()
}

async fn read(link: &Link, cwd: &Path) -> Option<Subject> {
    if link.github {
        let token = ["GH_TOKEN", "GITHUB_TOKEN"]
            .into_iter()
            .filter_map(|key| std::env::var(key).ok())
            .find(|value| !value.trim().is_empty());
        if let Some(token) = token {
            return github_subject(github_request(&link.endpoint, token.trim())?).await;
        }
    }
    let mut cmd = Command::new(if link.github { "gh" } else { "glab" });
    if cwd.is_dir() {
        cmd.current_dir(cwd);
    }
    cmd.args(["api", "--hostname", link.url.host_str()?, &link.endpoint])
        .env("GH_PROMPT_DISABLED", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    crate::connections::protect_command(&mut cmd);
    let mut child = cmd.spawn().ok()?;
    let mut data = Vec::new();
    child
        .stdout
        .take()?
        .take(MAX_SUBJECT_BYTES as u64 + 1)
        .read_to_end(&mut data)
        .await
        .ok()?;
    if data.len() > MAX_SUBJECT_BYTES || !child.wait().await.ok()?.success() {
        return None;
    }
    serde_json::from_slice(&data).ok()
}

pub(super) async fn subjects(text: &str, cwd: &Path) -> Vec<Value> {
    futures_util::future::join_all(links(text).into_iter().map(|link| async move {
        let subject = tokio::time::timeout(Duration::from_secs(3), read(&link, cwd)).await.ok().flatten();
        match subject {
            Some(s) => json!({"url":link.url.as_str(),"title":s.title.chars().take(300).collect::<String>(),"body":s.body.unwrap_or_default().chars().take(1200).collect::<String>()}),
            None => json!({"url":link.url.as_str(),"available":false}),
        }
    })).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn token_lookup_targets_the_fixed_api_and_bounds_the_response() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let link = link("https://github.com/acme/board/pull/23").unwrap();
        let req = github_request(&link.endpoint, "fixture-only")
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(
            req.url().as_str(),
            "https://api.github.com/repos/acme/board/issues/23"
        );
        assert_eq!(req.headers()["authorization"], "Bearer fixture-only");
        for (body, accepted) in [
            (r#"{"title":"Radar","body":null}"#.to_owned(), true),
            (
                format!(
                    r#"{{"title":"Radar","body":"{}"}}"#,
                    "x".repeat(MAX_SUBJECT_BYTES)
                ),
                false,
            ),
        ] {
            let server = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .unwrap();
            let addr = server.local_addr().unwrap();
            let task = tokio::spawn(async move {
                let (mut stream, _) = server.accept().await.unwrap();
                let mut buf = [0; 2048];
                stream.read(&mut buf).await.unwrap();
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(header.as_bytes()).await.unwrap();
                let _ = stream.write_all(body.as_bytes()).await;
            });
            let request = reqwest::Client::builder()
                .no_proxy()
                .build()
                .unwrap()
                .get(format!("http://{addr}/"));
            assert_eq!(github_subject(request).await.is_some(), accepted);
            task.await.unwrap();
        }
    }
    #[test]
    fn link_subjects_are_pinned_to_supported_forges_and_deduplicated() {
        let text = "https://github.com.evil.test/a/b/pull/2 https://me@github.com/a/b/pull/2 https://gitlab.com:8443/x/y/-/issues/8 https://github.com/a/b/pull/3?view=1#review, https://github.com/a/b/pull/3 https://gitlab.com/g/sub/proj/-/merge_requests/42/diffs). https://github.com/a/b/issues/4";
        let found = links(text);
        assert_eq!(
            found
                .iter()
                .map(|l| l.endpoint.as_str())
                .collect::<Vec<_>>(),
            [
                "repos/a/b/issues/3",
                "projects/g%2Fsub%2Fproj/merge_requests/42"
            ]
        );
        assert_eq!(found[0].url.as_str(), "https://github.com/a/b/pull/3");
    }
}
