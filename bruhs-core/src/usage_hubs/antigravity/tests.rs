use super::*;
use wiremock::{
    matchers::{body_json, header, method, path},
    Mock, MockServer, ResponseTemplate,
};

#[test]
fn summary_handles_rolling_resets_duplicate_buckets_and_shared_labels() {
    let now = 1893456000000;
    let reset = "2030-01-01T05:00:00Z";
    let groups = json!({"groups":[{"displayName":"  Argon  ","buckets":[
        {"bucketId":"gemini-argon-a","remainingFraction":0.1},
        {"bucketId":"gemini-argon-a","window":"5h","remainingFraction":1,"resetTime":reset},
        {"bucketId":"gemini-argon-b","window":"5h","remainingFraction":1,"resetTime":reset,"description":"Scheduled reset"},
        {"bucketId":"3p-b","remainingFraction":-1,"window":"weekly"},
        {"bucketId":"off","remainingFraction":0.1,"disabled":true},
        {"bucketId":"missing"}
    ]}],"buckets":[{"bucketId":"outside","remainingFraction":0.5}]});
    let rows = windows::summary(&groups, "a", now);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].window_id.as_deref(), Some("gemini-argon-a"));
    assert_eq!(rows[0].name, "Argon · 5-hour");
    assert_eq!(rows[0].resets_at, None);
    assert_eq!(rows[1].name, rows[0].name);
    assert_eq!(rows[1].resets_at, Some(now / 1000 + 18000));
    assert_eq!(rows[2].used_percent, 100.);
    assert_eq!(rows[2].name, "Claude & GPT (shared) · Weekly");
}
#[test]
fn uses_the_active_acp_profile_and_skips_non_subscription_auth() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("antigravity-acp");
    std::fs::create_dir(&dir).unwrap();
    let settings = dir.join("settings.json");
    assert!(open_profile(root.path()).unwrap().is_none());
    std::fs::write(dir.join("acp_token.json"), "invalid secret fixture").unwrap();
    for method in ["gemini-api-key", "agent-platform", "other"] {
        std::fs::write(&settings, json!({"auth":{"type":method}}).to_string()).unwrap();
        assert!(open_profile(root.path()).unwrap().is_none());
    }
    std::fs::write(&settings, r#"{"auth":{"type":"oauth-business"}}"#).unwrap();
    assert!(open_profile(root.path()).is_err());
    std::fs::write(
        dir.join("acp_token.json"),
        r#"{"client_id":"id","client_secret":"key","refresh_token":"refresh"}"#,
    )
    .unwrap();
    assert!(open_profile(root.path()).unwrap().unwrap().business);
}
#[tokio::test]
async fn subscription_caches_the_exchange_and_falls_back_to_model_limits() {
    let s = MockServer::start().await;
    let base = s.uri();
    let token = format!("{base}/token");
    let user = format!("{base}/identity");
    let daily = format!("{base}/daily");
    let prod = format!("{base}/prod");
    let endpoints = Endpoints {
        token: &token,
        identity: &user,
        daily: &daily,
        prod: &prod,
    };
    let mut profile = Profile {
        business: true,
        credential: Credential {
            client_id: "client".into(),
            client_secret: "secret".into(),
            refresh_token: "refresh".into(),
            project_id: Some("project".into()),
        },
    };
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"access_token":"access","expires_in":3600})),
        )
        .expect(2)
        .mount(&s)
        .await;
    Mock::given(method("GET"))
        .and(path("/identity"))
        .and(header("authorization", "Bearer access"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"shared"})))
        .expect(2)
        .mount(&s)
        .await;
    for endpoint in ["prod", "daily"] {
        Mock::given(method("POST"))
            .and(path(format!(
                "/{endpoint}/v1internal:retrieveUserQuotaSummary"
            )))
            .and(header("authorization", "Bearer access"))
            .and(body_json(json!({"project":"project"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"buckets":[]})))
            .expect(3)
            .mount(&s)
            .await;
    }
    Mock::given(path("/daily/v1internal:fetchAvailableModels")).and(header("authorization","Bearer access")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"models":{"gemini-a":{"quotaInfo":{"remainingFraction":0.5}},"other":{"quotaInfo":{"remainingFraction":0}}}}))).expect(3).mount(&s).await;
    Mock::given(path("/prod/v1internal:fetchAvailableModels")).and(header("authorization","Bearer access")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"models":{"gemini-b":{"quotaInfo":{"remainingFraction":0.25}},"gemini-a":{"quotaInfo":{"remainingFraction":0.95}},"claude-a":{"weeklyQuotaInfo":{"remainingFraction":0.5}}}}))).expect(3).mount(&s).await;
    let cache = Cache::new(None);
    for n in 0..3 {
        if n == 2 {
            profile.credential.refresh_token = "rotated".into();
            profile.business = false;
        }
        let (account, rows) = read_profile(&profile, &endpoints, &cache).await.unwrap();
        assert_eq!(account, google_account("shared"));
        let gemini = rows
            .iter()
            .find(|w| w.name == "Gemini · Reported quota")
            .unwrap();
        assert_eq!(gemini.used_percent, if n == 2 { 50. } else { 75. });
        assert!(gemini.duration_minutes.is_none());
        assert_eq!(
            rows.iter()
                .find(|w| w.name == "Claude · Weekly")
                .unwrap()
                .duration_minutes,
            Some(10080)
        );
        assert_eq!(rows.len(), 2);
    }
    let requests = s.received_requests().await.unwrap();
    assert!(requests
        .iter()
        .filter(|r| r.url.path() == "/token")
        .all(|r| std::str::from_utf8(&r.body)
            .unwrap()
            .contains("grant_type=refresh_token")));
    assert!(requests
        .iter()
        .filter(|r| r.url.path() != "/token")
        .all(|r| !String::from_utf8_lossy(&r.body).contains("secret")));
}

#[tokio::test]
async fn redirect_cannot_forward_the_oauth_exchange_to_another_host() {
    let server = MockServer::start().await;
    let sink = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(307).insert_header("location", sink.uri()))
        .expect(1)
        .mount(&server)
        .await;
    let base = server.uri();
    let url = format!("{base}/token");
    let endpoints = Endpoints {
        token: &url,
        identity: &base,
        daily: &base,
        prod: &base,
    };
    let profile = Profile {
        business: false,
        credential: Credential {
            client_id: "client".into(),
            client_secret: "private".into(),
            refresh_token: "refresh".into(),
            project_id: None,
        },
    };
    let error = read_profile(&profile, &endpoints, &Cache::new(None))
        .await
        .unwrap_err()
        .to_string();
    assert!(sink.received_requests().await.unwrap().is_empty());
    assert!(!error.contains("private"));
}

#[test]
fn reads_os_keyring_only_sign_ins_and_falls_back_to_file() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("antigravity-acp");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(
        dir.join("settings.json"),
        r#"{"auth":{"type":"oauth-personal"}}"#,
    )
    .unwrap();
    // OS-keyring-only sign-in (no acp_token.json on disk, camelCase JSON fields).
    let profile = open_profile_with_keyring(root.path(), false, |service, account| {
        if service == "gemini" && account == "antigravity-acp" {
            Ok(Some(Zeroizing::new(
                r#"{"clientId":"cid","clientSecret":"csec","refreshToken":"rtok","projectId":"p1"}"#
                    .into(),
            )))
        } else {
            Ok(None)
        }
    })
    .unwrap()
    .expect("keyring credential should open");
    assert!(!profile.business);
    assert_eq!(profile.credential.client_id, "cid");
    assert_eq!(profile.credential.project_id.as_deref(), Some("p1"));

    // Falls back to acp_token.json when keyring has no entry.
    std::fs::write(
        dir.join("acp_token.json"),
        r#"{"client_id":"file-id","client_secret":"file-sec","refresh_token":"file-rt"}"#,
    )
    .unwrap();
    let fallback = open_profile_with_keyring(root.path(), false, |_, _| Ok(None))
        .unwrap()
        .expect("file fallback should open");
    assert_eq!(fallback.credential.client_id, "file-id");

    // Gemini Neon bucket label uses group display name or fallback.
    let neon = windows::summary(
        &json!({
            "buckets": [
                {"bucketId": "gemini-neon-5h", "window": "5h", "remainingFraction": 0.6}
            ]
        }),
        "a",
        1_893_456_000_000,
    );
    assert_eq!(neon.len(), 1);
    assert_eq!(neon[0].name, "Gemini Neon · 5-hour");
}
