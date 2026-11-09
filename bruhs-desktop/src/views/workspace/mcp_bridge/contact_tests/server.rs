use bruhs_models::PersonProfile;
use serde_json::Value;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(super) struct Server {
    base: String,
    online: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<Value>>>,
    worker: tokio::task::JoinHandle<()>,
}

impl Server {
    pub(super) fn new(runtime: &tokio::runtime::Runtime) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("owned listener");
        listener.set_nonblocking(true).expect("nonblocking");
        let base = format!("http://{}", listener.local_addr().expect("address"));
        let online = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (is_online, captured) = (online.clone(), requests.clone());
        let worker = runtime.spawn(async move {
            let listener = tokio::net::TcpListener::from_std(listener).expect("async listener");
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut bytes = Vec::new();
                let (body_at, length) = loop {
                    let mut chunk = [0; 4096];
                    let size = stream.read(&mut chunk).await.expect("request read");
                    assert!(size > 0 && bytes.len() < 65536);
                    bytes.extend_from_slice(&chunk[..size]);
                    if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                        assert!(headers.starts_with("post /api/people ") || headers.starts_with("get /api/people ") || headers.starts_with("post /api/people/resolve-merges "));
                        let length: usize = headers.lines().find_map(|line| line.strip_prefix("content-length:")).unwrap_or("0").trim().parse().expect("integer");
                        break (end + 4, length);
                    }
                };
                while bytes.len() < body_at + length {
                    let mut chunk = [0; 4096];
                    let size = stream.read(&mut chunk).await.expect("body read");
                    assert!(size > 0 && bytes.len() < 65536);
                    bytes.extend_from_slice(&chunk[..size]);
                }
                let headers = String::from_utf8_lossy(&bytes[..body_at]).to_lowercase();
                let create = headers.starts_with("post /api/people ");
                let request: Value = if length == 0 { Value::Null } else { serde_json::from_slice(&bytes[body_at..body_at + length]).expect("JSON") };
                if create { captured.lock().expect("requests").push(request.clone()); }
                let (status, body) = if !create && is_online.load(Ordering::SeqCst) {
                    ("200 OK", "[]".into())
                } else if create && is_online.load(Ordering::SeqCst) {
                    let mut person = PersonProfile::new(request["display_name"].as_str().expect("name"));
                    person.id = request["id"].as_str().expect("id").into();
                    person.created_at = request["created_at"].as_i64().expect("created");
                    person.updated_at = request["updated_at"].as_i64().expect("updated");
                    ("200 OK", serde_json::to_string(&person).expect("response"))
                } else { ("503 Service Unavailable", "{}".into()) };
                let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                stream.write_all(response.as_bytes()).await.expect("response write");
            }
        });
        Self {
            base,
            online,
            requests,
            worker,
        }
    }
    pub(super) fn client(&self) -> crate::server_client::ServerClient {
        crate::server_client::ServerClient::new(&self.base, &self.base).expect("test client")
    }
    pub(super) fn go_online(&self) {
        self.online.store(true, Ordering::SeqCst);
    }
    pub(super) fn requests(&self) -> Vec<Value> {
        self.requests.lock().expect("requests").clone()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.worker.abort();
    }
}
