use super::*;
use wiremock::{
    matchers::{body_json, body_partial_json, header, method, path},
    Mock, MockServer, ResponseTemplate,
};

const KEY: &str = "fixture";
fn file() -> Value {
    json!({"id":"pool-account","provider":"codex","auth_index":"current-slot","id_token":{"chatgpt_account_id":"workspace"}})
}
fn management(s: &MockServer) -> Url {
    Url::parse(&format!("{}/team/v0/management/", s.uri())).unwrap()
}
fn credit(s: &MockServer) -> ResetCredit {
    ResetCredit {
        expires_at: Utc::now() + chrono::Duration::days(1),
        id: "oldest".into(),
        account: "pool-account".into(),
        identity: "workspace".into(),
        endpoint: management(s).join("api-call").unwrap(),
    }
}
async fn listed(s: &MockServer, file: Value, reads: u64) {
    Mock::given(method("GET"))
        .and(path("/team/v0/management/auth-files"))
        .and(header("authorization", "Bearer fixture"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"files":[file]})))
        .expect(reads)
        .mount(s)
        .await;
}
fn wrapper(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({"status_code":200,"body":data.to_string()}))
}
fn consume() -> wiremock::MockBuilder {
    let data =
        json!({"credit_id":"oldest","redeem_request_id":"e777f91b-f807-5d99-a8e9-7a97d0a6bf9f"});
    Mock::given(method("POST")).and(path("/team/v0/management/api-call")).and(header("authorization","Bearer fixture")).and(body_json(json!({"auth_index":"current-slot","method":"POST","url":CONSUME,"header":codex_headers(&file()),"data":data.to_string()})))
}

#[tokio::test]
async fn availability_filters_sorts_and_never_redeems_on_refresh() {
    let s = MockServer::start().await;
    let mut base = json!({"id":"later","status":"available","reset_type":"codex_rate_limits","expires_at":"2100-01-01T00:00:00Z"});
    let mut options = vec![base.clone()];
    for (field, value) in [
        ("status", "spent"),
        ("reset_type", "other"),
        ("expires_at", "2020-01-01T00:00:00Z"),
        ("expires_at", "invalid"),
    ] {
        let mut skipped = base.clone();
        skipped["id"] = json!(format!("{field}-{value}"));
        skipped[field] = json!(value);
        options.push(skipped);
    }
    base["expires_at"] = json!("2099-01-01T00:00:00Z");
    base["id"] = json!("oldest");
    options.extend([base.clone(), base]);
    Mock::given(method("POST"))
        .and(path("/team/v0/management/api-call"))
        .and(body_partial_json(json!({"url":CREDITS,"method":"GET"})))
        .respond_with(wrapper(json!({"credits":options})))
        .expect(1)
        .mount(&s)
        .await;
    let bank = available(
        &client().unwrap(),
        &management(&s).join("api-call").unwrap(),
        KEY,
        &file(),
    )
    .await
    .unwrap();
    assert_eq!(bank.count, 2);
    let next = bank.next.unwrap();
    assert_eq!(next.id, "oldest");
    assert_eq!(next.identity, "workspace");
    assert_eq!(s.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn retries_reuse_the_confirmed_request_and_refresh_the_current_slot() {
    let s = MockServer::start().await;
    listed(&s, file(), 2).await;
    consume()
        .respond_with(ResponseTemplate::new(502))
        .up_to_n_times(1)
        .expect(1)
        .mount(&s)
        .await;
    consume()
        .respond_with(wrapper(json!({"code":"already_redeemed"})))
        .expect(1)
        .mount(&s)
        .await;
    Mock::given(method("POST"))
        .and(path("/team/v0/management/reset-quota"))
        .and(header("authorization", "Bearer fixture"))
        .and(body_json(json!({"auth_index":"current-slot"})))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&s)
        .await;
    let c = credit(&s);
    let m = management(&s);
    let client = client().unwrap();
    assert!(redeem_at(&client, &m, KEY, &c).await.is_err());
    let receipt = redeem_at(&client, &m, KEY, &c).await.unwrap();
    assert!(receipt.message.contains("already redeemed"));
    assert!(receipt.cooldown_warning);
    assert_eq!(s.received_requests().await.unwrap().len(), 5);
}

#[tokio::test]
async fn outcomes_clear_only_the_account_that_was_reset() {
    for (code, expected, clear) in [
        ("reset", "Reset applied.", true),
        ("no_credit", "No reset credit left.", false),
        ("nothing_to_reset", "Nothing to reset right now.", false),
    ] {
        let s = MockServer::start().await;
        listed(&s, file(), 1).await;
        consume()
            .respond_with(wrapper(json!({"code":code})))
            .expect(1)
            .mount(&s)
            .await;
        Mock::given(method("POST"))
            .and(path("/team/v0/management/reset-quota"))
            .and(body_json(json!({"auth_index":"current-slot"})))
            .respond_with(ResponseTemplate::new(204))
            .expect(u64::from(clear))
            .mount(&s)
            .await;
        let receipt = redeem_at(&client().unwrap(), &management(&s), KEY, &credit(&s))
            .await
            .unwrap();
        assert!(receipt.message.starts_with(expected));
        assert!(!receipt.cooldown_warning);
        assert_eq!(
            s.received_requests().await.unwrap().len(),
            2 + usize::from(clear)
        );
    }
}

#[tokio::test]
async fn a_changed_account_or_hub_cannot_redeem_the_displayed_credit() {
    for field in ["disabled", "provider", "id_token", "id"] {
        let s = MockServer::start().await;
        let mut current = file();
        current[field] = match field {
            "disabled" => json!(true),
            "id_token" => json!({"chatgpt_account_id":"different"}),
            _ => json!("other"),
        };
        listed(&s, current, 1).await;
        let other = MockServer::start().await;
        let c = credit(&s);
        let client = client().unwrap();
        assert!(redeem_at(&client, &management(&other), KEY, &c)
            .await
            .is_err());
        assert!(other.received_requests().await.unwrap().is_empty());
        assert!(redeem_at(&client, &management(&s), KEY, &c).await.is_err());
        assert_eq!(s.received_requests().await.unwrap().len(), 1);
    }
}
