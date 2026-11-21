//! The isolated test executable shares one on-disk store across the feature's entry points.
use bruhs_core::{dictation_cleanup, text_generators as generators, thread_title};
use generators::{Draft, Preset};
use serde_json::json;
use wiremock::{
    matchers::{body_partial_json, method, path},
    Mock, MockServer, ResponseTemplate,
};

#[tokio::test]
async fn configured_endpoint_drives_titles_and_cleanup_through_its_saved_lifecycle() {
    let temp = tempfile::tempdir().unwrap();
    std::env::set_var(
        "BRUHS_GENERATORS_DB",
        temp.path().join("generators.sqlite3"),
    );
    let server = MockServer::start().await;
    let url = format!("{}/v1", server.uri());
    let draft = |label: &str| Draft {
        label: label.into(),
        url: url.clone(),
        credential: generators::CredentialEdit::Keep,
        preset: Preset::Custom,
    };
    assert!(generators::connect(None, draft("Unavailable"))
        .await
        .is_err());
    assert!(generators::list().await.unwrap().is_empty());
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"data":[{"id":"vendor/editor","name":"Editor"},{"id":"vendor/editor"}]}),
        ))
        .mount(&server)
        .await;
    let row = generators::connect(None, draft("Private endpoint"))
        .await
        .unwrap();
    let model = row.selection("vendor/editor");
    assert_eq!(generators::list().await.unwrap()[0].models.len(), 1);
    assert_eq!(
        generators::split_selection(&model),
        Some((row.id, "vendor/editor"))
    );
    let stale = row.clone();
    let row = generators::connect(Some(row), draft("Team endpoint"))
        .await
        .unwrap();
    assert!(generators::remove(stale).await.is_err());
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_partial_json(json!({"model":"vendor/editor"})))
        .respond_with(|r: &wiremock::Request| {
            let body: serde_json::Value = r.body_json().unwrap();
            let system = body["messages"][0]["content"].as_str().unwrap();
            let reply = if system.contains("transcript") {
                r#"{"text":"Take a break."}"#
            } else if system.contains("title") {
                r#"{"title":"Settings refresh"}"#
            } else {
                "OK"
            };
            ResponseTemplate::new(200)
                .set_body_json(json!({"choices":[{"message":{"content":reply}}]}))
        })
        .expect(3)
        .mount(&server)
        .await;
    generators::test(row.clone(), "vendor/editor".into())
        .await
        .unwrap();
    assert_eq!(
        thread_title::generate(&model, "Refresh Settings", "")
            .await
            .unwrap(),
        "Settings refresh"
    );
    assert_eq!(
        dictation_cleanup::clean(&model, "take a break", &Default::default(), None, "sorry")
            .await
            .unwrap(),
        "Take a break."
    );
    let row = generators::refresh(row).await.unwrap();
    let row = generators::enable(row, false).await.unwrap();
    assert!(!generators::active(&model).await.unwrap());
    assert!(generators::test(row.clone(), "vendor/editor".into())
        .await
        .is_err());
    let row = generators::enable(row, true).await.unwrap();
    assert!(generators::active(&model).await.unwrap());
    generators::remove(row).await.unwrap();
    assert!(!generators::active(&model).await.unwrap());
    assert!(generators::list().await.unwrap().is_empty());
}
