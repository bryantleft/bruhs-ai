use super::*;
use futures_util::StreamExt;
use reqwest::Client;
use uuid::Uuid;

struct Fixture {
    _root: tempfile::TempDir,
    host: MobileHost,
    client: Client,
    url: String,
}
impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let authority = MobileAuthority::open(
            root.path().join("private/devices.json"),
            MobileLaunch::isolated(),
        )
        .await
        .unwrap();
        let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mobile");
        let (host, _) = MobileHost::start(
            MobileHostConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                origin: Url::parse("https://localhost:0/").unwrap(),
                transport: MobileTransport::DirectTls {
                    certificate: fixtures.join("localhost.pem"),
                    private_key: fixtures.join("localhost-test-key.pem"),
                },
            },
            authority,
        )
        .await
        .unwrap();
        let ca =
            reqwest::Certificate::from_pem(include_bytes!("../../../tests/fixtures/mobile/ca.pem"))
                .unwrap();
        let client = Client::builder()
            .no_proxy()
            .tls_built_in_root_certs(false)
            .add_root_certificate(ca)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(REQUEST_TIMEOUT)
            .build()
            .unwrap();
        let url = host.origin().to_string().trim_end_matches('/').to_owned();
        Self {
            _root: root,
            host,
            client,
            url,
        }
    }
    fn offer(&self, scopes: MobileScopes, lifetime: Duration) -> String {
        let offer = self
            .host
            .authority()
            .begin_pairing(self.host.origin(), scopes, lifetime)
            .unwrap();
        Url::parse(&offer.url)
            .unwrap()
            .fragment()
            .unwrap()
            .strip_prefix("pair=")
            .unwrap()
            .into()
    }
    async fn pair(&self, scopes: MobileScopes, lifetime: Duration) -> MobilePairResult {
        self.client
            .post(format!("{}/api/mobile/pair", self.url))
            .json(&MobilePairInput {
                secret: self.offer(scopes, lifetime),
                name: "Isolated HTTPS phone".into(),
            })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap()
    }
}
#[tokio::test]
async fn mobile_real_https_trust_pairing_headers_and_isolated_routes() {
    let f = Fixture::new().await;
    assert!(
        Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(&f.url)
            .send()
            .await
            .is_err(),
        "the test CA must be explicitly trusted"
    );
    let root = f.client.get(&f.url).send().await.unwrap();
    assert_eq!(root.status(), StatusCode::OK);
    assert_eq!(root.headers()["cache-control"], "no-store");
    assert!(root.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .contains("frame-ancestors 'none'"));
    assert!(root
        .text()
        .await
        .unwrap()
        .contains("The laptop must stay awake"));
    let secret = f.offer([MobileScope::Read].into(), Duration::from_secs(60));
    let (a, b) = tokio::join!(
        f.client
            .post(format!("{}/api/mobile/pair", f.url))
            .json(&MobilePairInput {
                secret: secret.clone(),
                name: "First phone".into()
            })
            .send(),
        f.client
            .post(format!("{}/api/mobile/pair", f.url))
            .json(&MobilePairInput {
                secret,
                name: "Second phone".into()
            })
            .send()
    );
    let a = a.unwrap();
    let b = b.unwrap();
    assert_eq!(
        usize::from(a.status().is_success()) + usize::from(b.status().is_success()),
        1
    );
    let issued: MobilePairResult = if a.status().is_success() {
        a.json().await.unwrap()
    } else {
        b.json().await.unwrap()
    };
    assert_eq!(
        f.client
            .get(format!("{}/api/mobile/session", f.url))
            .bearer_auth(&issued.token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    for route in [
        "/api/mobile/session",
        "/api/threads",
        "/api/secrets",
        "/api/database/query",
    ] {
        assert_eq!(
            f.client
                .get(format!("{}{route}", f.url))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        f.client
            .get(format!("{}/api/threads", f.url))
            .bearer_auth(&issued.token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    for (header, value) in [
        ("host", "attacker.invalid"),
        ("origin", "https://attacker.invalid"),
    ] {
        assert_eq!(
            f.client
                .get(format!("{}/api/mobile/session", f.url))
                .bearer_auth(&issued.token)
                .header(header, value)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    let mut headers = reqwest::header::HeaderMap::new();
    headers.append(
        "authorization",
        format!("Bearer {}", issued.token).parse().unwrap(),
    );
    headers.append("authorization", "Bearer duplicate".parse().unwrap());
    assert_eq!(
        f.client
            .get(format!("{}/api/mobile/session", f.url))
            .headers(headers)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let oversized = MobilePairInput {
        secret: "x".repeat(REQUEST_BYTES),
        name: "not paired".into(),
    };
    assert_eq!(
        f.client
            .post(format!("{}/api/mobile/pair", f.url))
            .json(&oversized)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    f.host.shutdown().await;
}
#[tokio::test]
async fn mobile_scopes_apply_before_broker_and_revocation_closes_idle_sse() {
    let f = Fixture::new().await;
    let issued = f
        .pair([MobileScope::Read].into(), Duration::from_secs(60))
        .await;
    let command = MobileCommand {
        id: Uuid::new_v4(),
        sequence: 1,
        action: MobileAction::Prompt {
            thread_id: "fixture".into(),
            text: "No write grant".into(),
        },
    };
    assert_eq!(
        f.client
            .post(format!("{}/api/mobile/command", f.url))
            .bearer_auth(&issued.token)
            .json(&command)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.client
            .get(format!("{}/api/mobile/diff?thread_id=fixture", f.url))
            .bearer_auth(&issued.token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let lease = f.host.authority().authenticate(&issued.token).unwrap();
    assert_eq!(f.host.authority().session(&lease).unwrap().next_sequence, 1);
    let mut events = f
        .client
        .get(format!("{}/api/mobile/events", f.url))
        .bearer_auth(&issued.token)
        .send()
        .await
        .unwrap()
        .bytes_stream();
    assert!(String::from_utf8_lossy(&events.next().await.unwrap().unwrap()).contains("refresh"));
    f.host.authority().revoke_now(issued.device.id).unwrap();
    f.host.authority().persist_revocation().await.unwrap();
    let closed = tokio::time::timeout(Duration::from_secs(1), async {
        let mut result = String::new();
        while let Some(chunk) = events.next().await {
            result.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
        }
        result
    })
    .await
    .unwrap();
    assert!(closed.contains("revoked"));
    assert_eq!(
        f.client
            .get(format!("{}/api/mobile/session", f.url))
            .bearer_auth(&issued.token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    f.host.shutdown().await;
}
#[tokio::test]
async fn mobile_sse_expires_without_any_followup_request() {
    let f = Fixture::new().await;
    let issued = f
        .pair([MobileScope::Read].into(), Duration::from_millis(500))
        .await;
    let response = f
        .client
        .get(format!("{}/api/mobile/events", f.url))
        .bearer_auth(&issued.token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let events = tokio::time::timeout(Duration::from_secs(2), response.text())
        .await
        .unwrap()
        .unwrap();
    assert!(events.contains("refresh"));
    assert!(events.contains("revoked"));
    f.host.shutdown().await;
}
#[test]
fn mobile_plain_proxy_cannot_listen_on_the_network() {
    let config = MobileHostConfig {
        bind: "0.0.0.0:7411".parse().unwrap(),
        origin: Url::parse("https://phone.example/").unwrap(),
        transport: MobileTransport::HttpsProxy,
    };
    assert!(validate_config(&config).is_err());
}
