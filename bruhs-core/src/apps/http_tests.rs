//! `AppHttp` against wiremock upstreams (D24's mapping) and its resolver,
//! URL and scrub rules with no network at all.

use super::*;
use base64::Engine;
use bruhs_models::apps::{
    connector, AppHost, APP_CUT_NOTE, APP_HOST_REFUSED, APP_NOT_TEXT, APP_RESPONSE_MAX_BYTES,
    APP_TIMED_OUT, APP_TOO_LARGE, APP_UNREACHABLE, CONNECTORS,
};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn app(id: &str) -> &'static AppConnector {
    connector(id).unwrap()
}

fn secret(token: &str) -> AppSecret {
    AppSecret {
        token: Zeroizing::new(token.to_owned()),
        key: None,
    }
}

fn params(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

/// A random token with a `/`, a `+`, an `=` and an `A`, so its JSON-escaped,
/// `\u`-escaped and percent-encoded forms all differ from it.
fn canary() -> (String, String) {
    let core = uuid::Uuid::new_v4().simple().to_string();
    (format!("tok/A{core}+="), core)
}

async fn upstream() -> (MockServer, AppHttp) {
    let server = MockServer::start().await;
    let http = AppHttp::with_upstream(*server.address()).unwrap();
    (server, http)
}

fn header<'a>(request: &'a wiremock::Request, name: &str) -> Option<&'a str> {
    request
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
}

#[tokio::test]
async fn each_auth_form_sends_its_header() {
    let (server, http) = upstream().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .mount(&server)
        .await;
    let (token, _) = canary();
    let cases: [(&str, BTreeMap<String, String>, Option<&str>); 5] = [
        ("github", params(&[]), None),
        (
            "jira",
            params(&[("site", "acme"), ("email", "me@acme.test")]),
            None,
        ),
        ("linear", params(&[]), None),
        ("shopify", params(&[("shop", "acme")]), None),
        ("trello", params(&[]), Some("trellokey1234")),
    ];
    for (id, account, key) in &cases {
        let mut app_secret = secret(&token);
        app_secret.key = key.map(|key| Zeroizing::new(key.to_owned()));
        let reply = http
            .raw(app(id), account, &app_secret, "GET", "/probe", None)
            .await
            .unwrap();
        assert_eq!(reply.status, 200, "{id}");
    }
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 5);
    for request in &requests {
        assert_eq!(header(request, "user-agent"), Some("bruhs-ai"));
        assert_eq!(request.url.path(), "/probe");
    }
    let github = &requests[0];
    assert_eq!(
        header(github, "authorization"),
        Some(format!("Bearer {token}").as_str())
    );
    assert_eq!(header(github, "x-github-api-version"), Some("2022-11-28"));
    assert_eq!(header(github, "x-bruhs-app-host"), Some("api.github.com"));

    let jira = &requests[1];
    let basic = base64::engine::general_purpose::STANDARD.encode(format!("me@acme.test:{token}"));
    assert_eq!(
        header(jira, "authorization"),
        Some(format!("Basic {basic}").as_str())
    );
    assert_eq!(header(jira, "x-bruhs-app-host"), Some("acme.atlassian.net"));

    let linear = &requests[2];
    assert_eq!(header(linear, "authorization"), Some(token.as_str()));
    assert_eq!(header(linear, "x-bruhs-app-host"), Some("api.linear.app"));

    let shopify = &requests[3];
    assert_eq!(
        header(shopify, "x-shopify-access-token"),
        Some(token.as_str())
    );
    assert_eq!(header(shopify, "authorization"), None);
    assert_eq!(
        header(shopify, "x-bruhs-app-host"),
        Some("acme.myshopify.com")
    );

    let trello = &requests[4];
    assert_eq!(
        header(trello, "authorization"),
        Some(
            format!(r#"OAuth oauth_consumer_key="trellokey1234", oauth_token="{token}""#).as_str()
        )
    );
}

#[tokio::test]
async fn echoed_tokens_never_come_back() {
    let (server, http) = upstream().await;
    let (token, core) = canary();
    let account = params(&[("site", "acme"), ("email", "me@acme.test")]);
    let basic = base64::engine::general_purpose::STANDARD.encode(format!("me@acme.test:{token}"));
    let json_escaped = |text: &str| text.replace('/', "\\/");
    let unicode_escaped = token.replacen('A', "\\u0041", 1);
    let encoded: String = url::form_urlencoded::byte_serialize(token.as_bytes()).collect();
    assert_ne!(encoded, token);
    // Raw text, so the escapes reach Bruhs exactly as an app would send them.
    let body = format!(
        r#"{{"error": "bad token {token}", "auth": "Bearer {token}", "basic": "{basic}", "slashed": "Basic {slashed}", "escaped": "{unicode_escaped}", "{token}": "as a key", "nested": [{{"deep": "x {escaped_token} y"}}]}}"#,
        slashed = json_escaped(&basic),
        escaped_token = json_escaped(&token),
    );
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_raw(body.into_bytes(), "application/json")
                .insert_header("www-authenticate", format!("Bearer {token}").as_str())
                .insert_header("set-cookie", format!("s={token}").as_str())
                .insert_header(
                    "link",
                    format!("<https://acme.atlassian.net/next?t={encoded}>; rel=\"next\"").as_str(),
                )
                .insert_header("location", format!("https://x.test/?b={basic}").as_str())
                .insert_header("x-ratelimit-remaining", "41"),
        )
        .mount(&server)
        .await;
    let reply = http
        .raw(
            app("jira"),
            &account,
            &secret(&token),
            "GET",
            "/rest/api/3/myself",
            None,
        )
        .await
        .unwrap();
    assert_eq!(reply.status, 401);
    assert_eq!(reply.body["error"], json!("bad token [redacted]"));
    for field in ["auth", "basic", "slashed", "escaped"] {
        let value = reply.body[field].as_str().unwrap();
        assert!(value.contains("[redacted]"), "{field}: {value}");
    }
    assert_eq!(reply.body["[redacted]"], json!("as a key"));
    assert_eq!(reply.body["nested"][0]["deep"], json!("x [redacted] y"));
    assert!(!reply.headers.contains_key("set-cookie"));
    assert!(!reply.headers.contains_key("www-authenticate"));
    assert!(reply.headers["link"].contains("[redacted]"));
    assert!(reply.headers["location"].contains("[redacted]"));
    assert_eq!(reply.headers["x-ratelimit-remaining"], "41");
    let wire = serde_json::to_string(&reply).unwrap();
    for leaked in [&token, &core, &basic, &encoded, &json_escaped(&token)] {
        assert!(!wire.contains(leaked.as_str()), "leaked {leaked}");
    }
}

#[tokio::test]
async fn replies_are_capped_and_typed() {
    let (server, http) = upstream().await;
    let big = json!({"items": "x".repeat(40 * 1024)});
    let routes = [
        ("/big", ResponseTemplate::new(200).set_body_json(&big)),
        (
            "/huge",
            ResponseTemplate::new(200).set_body_raw(vec![b'x'; 2 * 1024 * 1024], "text/plain"),
        ),
        (
            "/bin",
            ResponseTemplate::new(200).set_body_raw(vec![0xff, 0xfe, 0x00, 0x9f], "text/plain"),
        ),
        (
            "/text",
            ResponseTemplate::new(200).set_body_string("plain words"),
        ),
        (
            "/moved",
            ResponseTemplate::new(302).insert_header("location", "/user"),
        ),
        ("/user", ResponseTemplate::new(200).set_body_json(json!({}))),
    ];
    for (route, template) in routes {
        Mock::given(path(route))
            .respond_with(template)
            .mount(&server)
            .await;
    }
    let github = app("github");
    let account = params(&[]);
    let token = secret(&canary().0);
    let get = |route: &'static str| http.raw(github, &account, &token, "GET", route, None);

    let cut = get("/big").await.unwrap();
    assert!(cut.truncated);
    assert_eq!(cut.note.as_deref(), Some(APP_CUT_NOTE));
    assert_eq!(cut.body.as_str().unwrap().len(), APP_RESPONSE_MAX_BYTES);
    assert!(cut.body.as_str().unwrap().starts_with(r#"{"items":"xxx"#));

    assert!(matches!(get("/huge").await, Err(AppsError::TooLarge(m)) if m == APP_TOO_LARGE));
    assert!(matches!(get("/bin").await, Err(AppsError::Invalid(m)) if m == APP_NOT_TEXT));

    let text = get("/text").await.unwrap();
    assert_eq!(text.body, json!("plain words"));
    assert!(!text.truncated);
    assert_eq!(text.headers["content-type"], "text/plain");

    let moved = get("/moved").await.unwrap();
    assert_eq!(moved.status, 302);
    assert_eq!(moved.headers["location"], "/user");
    let requests = server.received_requests().await.unwrap();
    let paths: Vec<&str> = requests.iter().map(|request| request.url.path()).collect();
    assert_eq!(paths.iter().filter(|seen| **seen == "/moved").count(), 1);
    assert!(!paths.contains(&"/user"), "the redirect was followed");
}

#[tokio::test]
async fn queries_and_bodies_go_as_sent() {
    let (server, http) = upstream().await;
    Mock::given(path("/repos/o/r/pulls"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"merged": true})))
        .mount(&server)
        .await;
    Mock::given(path("/v1/customers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    let account = params(&[]);
    let token = secret(&canary().0);
    let call = |method: &str, path: &str, query: Value, body: Option<Value>| AppCall {
        app: "github".into(),
        account: None,
        method: method.into(),
        path: path.into(),
        query: serde_json::from_value(query).unwrap(),
        body,
    };

    let list = call(
        "GET",
        "/repos/o/r/pulls",
        json!({"state": "open", "per_page": 50, "draft": false}),
        None,
    );
    http.call(app("github"), &account, &token, &list)
        .await
        .unwrap();
    let body = json!({"merge_method": "squash", "commit_title": "a/b \u{2028} c"});
    let merge = call(
        "PUT",
        "/repos/o/r/pulls/1/%6Derge",
        json!({}),
        Some(body.clone()),
    );
    http.call(app("github"), &account, &token, &merge)
        .await
        .unwrap();
    // A key that only looks like `_method` or `balance` once decoded is sent
    // as the literal text the gate saw.
    let mut sneaky = call(
        "GET",
        "/v1/customers",
        json!({"%5Fmethod": "DELETE", "%62alance": "-100"}),
        None,
    );
    sneaky.app = "stripe".into();
    http.call(app("stripe"), &account, &token, &sneaky)
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let [list, merge, sneaky] = requests.as_slice() else {
        panic!("{} requests", requests.len());
    };
    assert_eq!(list.url.query(), Some("draft=false&per_page=50&state=open"));
    let pairs: BTreeMap<String, String> = list.url.query_pairs().into_owned().collect();
    assert_eq!(
        pairs,
        params(&[("state", "open"), ("per_page", "50"), ("draft", "false")])
    );
    assert!(list.body.is_empty());
    assert_eq!(header(list, "content-type"), None);

    assert_eq!(merge.url.path(), "/repos/o/r/pulls/1/merge");
    assert_eq!(merge.url.query(), None);
    assert_eq!(header(merge, "content-type"), Some("application/json"));
    assert_eq!(merge.body, serde_json::to_vec(&body).unwrap());

    let pairs: Vec<(String, String)> = sneaky.url.query_pairs().into_owned().collect();
    assert_eq!(
        pairs,
        vec![
            ("%5Fmethod".to_owned(), "DELETE".to_owned()),
            ("%62alance".to_owned(), "-100".to_owned()),
        ]
    );
}

#[tokio::test]
async fn paths_go_exactly_as_normalized() {
    let (server, http) = upstream().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    let account = params(&[]);
    let token = secret(&canary().0);
    for (sent, seen) in [
        (
            "/repos/o/r/contents/caf%c3%a9",
            "/repos/o/r/contents/caf%C3%A9",
        ),
        ("/repos/o/r/contents/%7Ea%2Db", "/repos/o/r/contents/~a-b"),
        ("/repos/o/r/contents/a+b", "/repos/o/r/contents/a+b"),
    ] {
        http.raw(app("github"), &account, &token, "GET", sent, None)
            .await
            .unwrap();
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.last().unwrap().url.path(), seen, "{sent}");
    }
    for refused in ["/a/%2e%2E/x", "/a/b%2Fc", "//evil.test/x", "/a?b=1", "/a@b"] {
        assert!(
            matches!(
                http.raw(app("github"), &account, &token, "GET", refused, None)
                    .await,
                Err(AppsError::Invalid(_))
            ),
            "{refused}"
        );
    }
    assert_eq!(server.received_requests().await.unwrap().len(), 3);
}

#[tokio::test]
async fn failures_say_nothing_of_the_transport() {
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = closed.local_addr().unwrap();
    drop(closed);
    let account = params(&[]);
    let token = secret(&canary().0);
    let unreachable = AppHttp::with_upstream(addr)
        .unwrap()
        .raw(app("github"), &account, &token, "GET", "/user", None)
        .await
        .unwrap_err();
    assert!(matches!(unreachable, AppsError::Unreachable(m) if m == APP_UNREACHABLE));

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(5)))
        .mount(&server)
        .await;
    let slow = AppHttp::with_upstream(*server.address())
        .unwrap()
        .with_timeout(Duration::from_millis(200))
        .raw(app("github"), &account, &token, "GET", "/user", None)
        .await
        .unwrap_err();
    assert!(matches!(slow, AppsError::TimedOut(m) if m == APP_TIMED_OUT));

    for (error, at) in [(unreachable, addr), (slow, *server.address())] {
        let message = error.to_string();
        assert!(!message.contains(&at.ip().to_string()), "{message}");
        assert!(!message.contains(&at.port().to_string()), "{message}");
    }
}

/// A lookup that answers every name with `answer` and counts its calls.
fn stub(answer: &'static str) -> (GuardedResolver, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let resolver = GuardedResolver::with_lookup(move |_name: String| {
        counted.fetch_add(1, Ordering::SeqCst);
        let ip: IpAddr = answer.parse().unwrap();
        async move { Ok(vec![SocketAddr::new(ip, 443)]) }
    });
    (resolver, calls)
}

async fn resolves(resolver: &GuardedResolver, name: &str) -> Result<Vec<SocketAddr>, BoxError> {
    resolver
        .resolve(name.parse().unwrap())
        .await
        .map(Iterator::collect)
}

#[tokio::test]
async fn declared_hosts_that_resolve_here_are_refused() {
    let refused = [
        "127.0.0.1",
        "10.0.0.5",
        "169.254.1.1",
        "100.64.0.1",
        "::1",
        "fd00::1",
        "::ffff:127.0.0.1",
        "198.18.0.1",
        "240.0.0.1",
        "192.0.0.8",
        "64:ff9b::a00:5",
        "2002:a00:5::1",
        "fec0::1",
        "::ffff:10.0.0.5",
    ];
    for answer in refused {
        let (resolver, calls) = stub(answer);
        let error = resolves(&resolver, "api.github.com").await.unwrap_err();
        assert!(error.is::<HostRefused>(), "{answer}");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(refused_address(answer.parse().unwrap()), "{answer}");
    }
    let (resolver, calls) = stub("140.82.112.6");
    let error = resolves(&resolver, "evil.com").await.unwrap_err();
    assert!(error.is::<HostRefused>());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "looked up an undeclared host"
    );
    let addresses = resolves(&resolver, "api.github.com").await.unwrap();
    assert_eq!(addresses, vec!["140.82.112.6:443".parse().unwrap()]);
    let addresses = resolves(&resolver, "acme.atlassian.net").await.unwrap();
    assert_eq!(addresses.len(), 1);
    assert!(!refused_address("140.82.112.6".parse().unwrap()));

    let own = crate::local_tools::interface_addresses_for_tests();
    assert!(!own.is_empty());
    assert!(own.iter().all(|ip| refused_address(*ip)));
}

#[tokio::test]
async fn a_guarded_client_never_connects_to_a_refused_answer() {
    let (resolver, calls) = stub("127.0.0.1");
    let http = AppHttp::guarded(resolver).unwrap();
    let error = http
        .raw(
            app("github"),
            &params(&[]),
            &secret(&canary().0),
            "GET",
            "/user",
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AppsError::Refused(m) if m == APP_HOST_REFUSED));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

static EVIL: AppConnector = AppConnector {
    host: AppHost::Fixed("evil.com"),
    ..CONNECTORS[0]
};

#[tokio::test]
async fn the_system_client_refuses_http_and_other_hosts() {
    for url in [
        "http://api.github.com/user",
        "https://evil.com/user",
        "https://api.github.com.evil.com/user",
        "https://api.github.com:8443/user",
        "https://name:pass@api.github.com/user",
        "https://name@api.github.com/user",
    ] {
        let parsed = Url::parse(url).unwrap();
        assert!(
            matches!(checked_url(parsed, "api.github.com"), Err(AppsError::Refused(m)) if m == APP_HOST_REFUSED),
            "{url}"
        );
    }
    let good = checked_url(
        Url::parse("https://api.github.com/user").unwrap(),
        "api.github.com",
    );
    assert_eq!(good.unwrap().as_str(), "https://api.github.com/user");

    let http = AppHttp::system().unwrap();
    let token = secret(&canary().0);
    let undeclared = http
        .raw(&EVIL, &params(&[]), &token, "GET", "/user", None)
        .await;
    assert!(matches!(undeclared, Err(AppsError::Refused(m)) if m == APP_HOST_REFUSED));
    for site in ["evil.com/x", "evil.com", "a@b", ""] {
        let account = params(&[("site", site), ("email", "me@acme.test")]);
        let built = app_url(
            app("jira"),
            &account,
            "/rest/api/3/myself",
            &BTreeMap::new(),
        );
        assert!(
            matches!(built, Err(AppsError::Refused(m)) if m == APP_HOST_REFUSED),
            "{site}"
        );
    }
    let built = app_url(
        app("jira"),
        &params(&[("site", "Acme")]),
        "/rest/api/3/myself",
        &BTreeMap::from([("jql".to_owned(), json!("a b&c=d"))]),
    )
    .unwrap();
    assert_eq!(
        built.as_str(),
        "https://acme.atlassian.net/rest/api/3/myself?jql=a+b%26c%3Dd"
    );
}

#[test]
fn secret_strings_cover_every_form_of_the_token() {
    let (token, _) = canary();
    let account = params(&[("site", "acme"), ("email", "me@acme.test")]);
    let strings = secret_strings(app("jira"), &account, &secret(&token));
    let strings: Vec<&str> = strings.iter().map(|text| text.as_str()).collect();
    let basic = base64::engine::general_purpose::STANDARD.encode(format!("me@acme.test:{token}"));
    let encoded: String = url::form_urlencoded::byte_serialize(token.as_bytes()).collect();
    for form in [
        token.clone(),
        basic.clone(),
        format!("Basic {basic}"),
        token.replace('/', "\\/"),
        encoded,
        token
            .replace('/', "%2F")
            .replace('+', "%2B")
            .replace('=', "%3D"),
        token
            .replace('/', "%2f")
            .replace('+', "%2b")
            .replace('=', "%3d"),
    ] {
        assert!(strings.contains(&form.as_str()), "missing {form}");
    }
    assert!(strings.iter().all(|text| text.len() >= 8));

    let mut trello = secret(&token);
    trello.key = Some(Zeroizing::new("trellokey1234".to_owned()));
    let strings = secret_strings(app("trello"), &params(&[]), &trello);
    assert!(strings.iter().any(|text| text.as_str() == "trellokey1234"));
    // Too short to scrub safely, so never listed (the catalog refuses them).
    let strings = secret_strings(app("github"), &params(&[]), &secret("short"));
    assert!(!strings.iter().any(|text| text.as_str() == "short"));
}

#[test]
fn scrub_replaces_the_longest_form_first() {
    assert_eq!(
        scrub(
            "Bearer abcdefgh and abcdefgh",
            &["abcdefgh", "Bearer abcdefgh"]
        ),
        "[redacted] and [redacted]"
    );
    assert_eq!(scrub("nothing here", &["", "abcdefgh"]), "nothing here");
}

#[test]
fn the_test_upstream_is_loopback_http_in_debug_builds_only() {
    for (value, addr) in [
        ("http://127.0.0.1:4010", "127.0.0.1:4010"),
        ("http://127.0.0.1:4010/", "127.0.0.1:4010"),
        ("http://[::1]:4010", "[::1]:4010"),
    ] {
        assert_eq!(
            upstream_from(Some(value), true),
            Some(addr.parse().unwrap())
        );
        assert_eq!(upstream_from(Some(value), false), None, "{value}");
    }
    for value in [
        "https://127.0.0.1:4010",
        "http://127.0.0.1",
        "http://localhost:4010",
        "http://10.0.0.5:4010",
        "http://127.0.0.2:4010",
        "http://user@127.0.0.1:4010",
        "http://127.0.0.1:4010/x",
        "http://127.0.0.1:4010/?a=1",
        "http://127.0.0.1:0",
        "",
    ] {
        assert_eq!(upstream_from(Some(value), true), None, "{value}");
    }
    assert_eq!(upstream_from(None, true), None);
}
