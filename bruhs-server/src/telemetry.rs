//! Opt-in, bounded OTLP/HTTP reporting for completed local API requests.
//! Only the matched route template, method, status and duration leave the host.
use crate::error::AppError;
use axum::{
    extract::{MatchedPath, Request, State},
    middleware::Next,
    response::Response,
    routing::{get, post},
    Json, Router,
};
use bruhs_models::telemetry::{Endpoint, ExportSettings, SetEndpoint, Signal};
use reqwest::Url;
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{mpsc, Mutex};

#[derive(Clone)]
pub struct Telemetry(Arc<Host>);
struct Host {
    path: PathBuf,
    stored: Mutex<[String; 3]>,
    from_env: [Option<(String, String)>; 3],
    enabled: AtomicBool,
    generation: AtomicU64,
    events: mpsc::Sender<Record>,
    client: reqwest::Client,
}
struct Record {
    generation: u64,
    start: u64,
    end: u64,
    method: String,
    route: String,
    status: u16,
}

fn validate(raw: &str) -> Result<String, AppError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(String::new());
    }
    if raw.len() <= 4096 {
        if let Ok(url) = Url::parse(raw) {
            if matches!(url.scheme(), "http" | "https")
                && url.host().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.fragment().is_none()
            {
                return Ok(url.to_string());
            }
        }
    }
    Err(AppError::BadRequest(
        "Enter an HTTP or HTTPS receiver URL without embedded credentials or a fragment.".into(),
    ))
}

fn env_endpoint(signal: Signal, get: impl Fn(&str) -> Option<String>) -> Option<(String, String)> {
    if let Some(raw) = get(signal.variable()).filter(|v| !v.is_empty()) {
        return Some((signal.variable().into(), raw));
    }
    let key = "OTEL_EXPORTER_OTLP_ENDPOINT";
    let base = get(key).filter(|v| !v.is_empty())?;
    let mut url = match Url::parse(&base) {
        Ok(u) => u,
        Err(_) => return Some((key.into(), base)),
    };
    let path = format!("{}/{}", url.path().trim_end_matches('/'), signal.path());
    url.set_path(&path);
    Some((key.into(), url.to_string()))
}
impl Telemetry {
    pub fn open(path: PathBuf) -> std::io::Result<Self> {
        let values = match std::fs::read(&path) {
            Ok(data) => serde_json::from_slice(&data).map_err(std::io::Error::other)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Default::default(),
            Err(e) => return Err(e),
        };
        Self::new(
            path,
            values,
            Signal::ALL.map(|s| env_endpoint(s, |k| std::env::var(k).ok())),
        )
    }
    fn new(
        path: PathBuf,
        values: [String; 3],
        from_env: [Option<(String, String)>; 3],
    ) -> std::io::Result<Self> {
        let enabled = values.iter().any(|s| !s.is_empty()) || from_env.iter().any(Option::is_some);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(4))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(std::io::Error::other)?;
        let (events, rx) = mpsc::channel(256);
        let owner = Self(Arc::new(Host {
            path,
            stored: Mutex::new(values),
            from_env,
            enabled: AtomicBool::new(enabled),
            generation: AtomicU64::new(0),
            events,
            client,
        }));
        tokio::spawn(worker(Arc::downgrade(&owner.0), rx));
        Ok(owner)
    }
    async fn settings(&self) -> ExportSettings {
        let stored = self.0.stored.lock().await;
        effective(&stored, &self.0.from_env)
    }
}
fn effective(stored: &[String; 3], from_env: &[Option<(String, String)>; 3]) -> ExportSettings {
    ExportSettings {
        signals: Signal::ALL.map(|s| {
            let i = s.index();
            match &from_env[i] {
                Some((var, url)) => Endpoint {
                    url: url.clone(),
                    environment: Some(var.clone()),
                },
                None => Endpoint {
                    url: stored[i].clone(),
                    environment: None,
                },
            }
        }),
    }
}
fn write(path: &Path, values: &[String; 3]) -> std::io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("Missing settings directory"))?;
    std::fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer(temp.as_file_mut(), values)?;
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    Ok(())
}
pub fn attach(app: Router, policy: crate::local_access::Policy) -> Router {
    let opened = bruhs_core::agent_extensions::directory()
        .and_then(|p| Telemetry::open(p.join("telemetry.json")));
    let (app, api) = match opened {
        Ok(host) => (
            app.layer(axum::middleware::from_fn_with_state(host.clone(), observe)),
            routes(host),
        ),
        Err(error) => {
            tracing::warn!(%error,"Cannot open diagnostic export settings");
            async fn unavailable() -> AppError {
                AppError::Unavailable("Diagnostic export settings could not be opened. Check the host's settings file and restart the server.".into())
            }
            (
                app,
                Router::new()
                    .route(
                        "/api/diagnostics/export",
                        get(unavailable).post(unavailable),
                    )
                    .route("/api/diagnostics/export/check", post(unavailable)),
            )
        }
    };
    app.merge(
        api.layer(policy.cors())
            .layer(axum::middleware::from_fn_with_state(
                policy,
                crate::local_access::guard,
            )),
    )
}

pub fn routes(host: Telemetry) -> Router {
    Router::new()
        .route("/api/diagnostics/export", get(read).post(set))
        .route("/api/diagnostics/export/check", post(check))
        .with_state(host)
}
async fn read(State(host): State<Telemetry>) -> Json<ExportSettings> {
    Json(host.settings().await)
}
async fn set(
    State(host): State<Telemetry>,
    Json(change): Json<SetEndpoint>,
) -> Result<Json<ExportSettings>, AppError> {
    let i = change.signal.index();
    if host.0.from_env[i].is_some() {
        return Err(AppError::Conflict(
            "This receiver is set by the host environment.".into(),
        ));
    }
    let url = validate(&change.url)?;
    // The owned writer finishes its transaction even when the caller disconnects.
    tokio::spawn(async move {
        let mut stored = host.0.stored.lock().await;
        let mut next = stored.clone();
        next[i] = url;
        let path = host.0.path.clone();
        let next = tokio::task::spawn_blocking(move || {
            write(&path, &next)?;
            Ok::<_, std::io::Error>(next)
        })
        .await
        .map_err(AppError::internal)?
        .map_err(AppError::internal)?;
        let settings = effective(&next, &host.0.from_env);
        if *stored != next {
            host.0.generation.fetch_add(1, Ordering::Release);
        }
        *stored = next;
        drop(stored);
        host.0.enabled.store(
            settings.signals.iter().any(|e| !e.url.is_empty()),
            Ordering::Relaxed,
        );
        Ok(Json(settings))
    })
    .await
    .map_err(AppError::internal)?
}
async fn check(
    State(host): State<Telemetry>,
    Json(request): Json<SetEndpoint>,
) -> Result<Json<Value>, AppError> {
    let url = validate(&request.url)?;
    if url.is_empty() {
        return Err(AppError::BadRequest("Enter a receiver URL first.".into()));
    }
    let payload = match request.signal {
        Signal::Traces => json!({"resourceSpans":[]}),
        Signal::Metrics => json!({"resourceMetrics":[]}),
        Signal::Logs => json!({"resourceLogs":[]}),
    };
    send(&host.0.client, &url, &payload)
        .await
        .map_err(AppError::Unavailable)?;
    Ok(Json(json!({"accepted":true})))
}
pub async fn observe(State(host): State<Telemetry>, request: Request, next: Next) -> Response {
    let template = request
        .extensions()
        .get::<MatchedPath>()
        .map_or("", |p| p.as_str());
    if !host.0.enabled.load(Ordering::Relaxed)
        || !template.starts_with("/api/")
        || template.starts_with("/api/diagnostics/export")
    {
        return next.run(request).await;
    }
    let generation = host.0.generation.load(Ordering::Acquire);
    let route = template.to_owned();
    let method = request.method().as_str().chars().take(20).collect();
    let start = u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    )
    .unwrap_or(u64::MAX);
    let clock = Instant::now();
    let response = next.run(request).await;
    let end = start.saturating_add(u64::try_from(clock.elapsed().as_nanos()).unwrap_or(u64::MAX));
    let _ = host.0.events.try_send(Record {
        generation,
        start,
        end,
        method,
        route,
        status: response.status().as_u16(),
    });
    response
}

async fn worker(host: std::sync::Weak<Host>, mut rx: mpsc::Receiver<Record>) {
    while let Some(record) = rx.recv().await {
        let mut batch = vec![record];
        let until = tokio::time::Instant::now() + Duration::from_secs(2);
        while batch.len() < 64 {
            match tokio::time::timeout_at(until, rx.recv()).await {
                Ok(Some(record)) => batch.push(record),
                _ => break,
            }
        }
        let Some(host) = host.upgrade() else { break };
        let (urls, generation) = {
            let stored = host.stored.lock().await;
            (
                effective(&stored, &host.from_env),
                host.generation.load(Ordering::Acquire),
            )
        };
        batch.retain(|r| r.generation == generation);
        if batch.is_empty() {
            continue;
        }
        let export = |s: Signal| {
            let host = &host;
            let url = &urls.signals[s.index()].url;
            let batch = &batch;
            async move {
                if url.is_empty() || validate(url).is_err() {
                    return;
                }
                let body = envelope(s, batch);
                if let Err(reason) = send(&host.client, url, &body).await {
                    tracing::debug!(signal=s.label(),%reason,"Diagnostic export failed");
                }
            }
        };
        tokio::join!(
            export(Signal::Traces),
            export(Signal::Metrics),
            export(Signal::Logs)
        );
    }
}
fn envelope(signal: Signal, batch: &[Record]) -> Value {
    let resource =
        json!({"attributes":[{"key":"service.name","value":{"stringValue":"bruhs-server"}}]});
    let scope = json!({"name":"bruhs.http"});
    let attributes = |r: &Record| json!([{"key":"http.request.method","value":{"stringValue":r.method}},{"key":"http.route","value":{"stringValue":r.route}},{"key":"http.response.status_code","value":{"intValue":r.status.to_string()}}]);
    match signal {
        Signal::Traces => {
            let spans:Vec<_>=batch.iter().map(|r|{
                let trace=uuid::Uuid::new_v4().simple().to_string();
                let span=uuid::Uuid::new_v4().simple().to_string();
                json!({"traceId":trace,"spanId":&span[..16],"name":format!("{} {}",r.method,r.route),"kind":2,"startTimeUnixNano":r.start.to_string(),"endTimeUnixNano":r.end.to_string(),"attributes":attributes(r),"status":{"code":if r.status>=500{2}else{0}}})
            }).collect();
            json!({"resourceSpans":[{"resource":resource,"scopeSpans":[{"scope":scope,"spans":spans}]}]})
        }
        Signal::Logs => {
            let records:Vec<_>=batch.iter().map(|r|json!({"timeUnixNano":r.end.to_string(),"severityNumber":if r.status>=500{17}else{9},"body":{"stringValue":format!("{} {} {}",r.method,r.route,r.status)},"attributes":attributes(r)})).collect();
            json!({"resourceLogs":[{"resource":resource,"scopeLogs":[{"scope":scope,"logRecords":records}]}]})
        }
        Signal::Metrics => {
            let start = batch.iter().map(|r| r.start).min().unwrap_or(0);
            let end = batch.iter().map(|r| r.end).max().unwrap_or(0);
            let duration = batch
                .iter()
                .map(|r| Duration::from_nanos(r.end.saturating_sub(r.start)).as_secs_f64())
                .sum::<f64>();
            let point = json!({"startTimeUnixNano":start.to_string(),"timeUnixNano":end.to_string(),"count":batch.len().to_string(),"sum":duration,"bucketCounts":[batch.len().to_string()]});
            json!({"resourceMetrics":[{"resource":resource,"scopeMetrics":[{"scope":scope,"metrics":[{"name":"http.server.request.duration","unit":"s","histogram":{"aggregationTemporality":1,"dataPoints":[point]}}]}]}]})
        }
    }
}
async fn send(client: &reqwest::Client, url: &str, body: &Value) -> Result<(), String> {
    let response = client
        .post(url)
        .json(body)
        .send()
        .await
        .map_err(|_| "Could not reach the receiver.".to_owned())?;
    if response.status().as_u16() != 200 {
        return Err(format!(
            "Receiver returned HTTP {}.",
            response.status().as_u16()
        ));
    }
    use futures_util::StreamExt;
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "The receiver response was interrupted.")?;
        if bytes.len() + chunk.len() > 65536 {
            return Err("The receiver response is too large.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let reply: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "The receiver did not return an OTLP JSON response.")?;
    if !reply.is_object() {
        return Err("The receiver did not return an OTLP JSON response.".into());
    }
    if let Some(partial) = reply.get("partialSuccess") {
        for name in ["rejectedSpans", "rejectedLogRecords", "rejectedDataPoints"] {
            if partial.get(name).is_some_and(|v| {
                v.as_str()
                    .map_or_else(|| v.as_u64() != Some(0), |n| n != "0")
            }) {
                return Err("The receiver rejected some telemetry records.".into());
            }
        }
    }
    Ok(())
}
