use super::ServerClient;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn check_shell(response: &'static [u8]) -> Result<(), super::ServerError> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 4096];
        let read = socket.read(&mut bytes).await.unwrap();
        assert!(String::from_utf8_lossy(&bytes[..read])
            .starts_with("GET /api/agent/rich-response HTTP/1.1\r\n"));
        socket.write_all(response).await.unwrap();
    });
    let client = ServerClient::new(base, "http://127.0.0.1:9").unwrap();
    let result = client.check_rich_response_host().await;
    server.await.unwrap();
    result
}

#[tokio::test]
async fn rich_response_shell_requires_the_expected_host_version() {
    check_shell(b"HTTP/1.1 200 OK\r\nx-bruhs-rich-response: 1\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await.unwrap();
    let error = check_shell(b"HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await.unwrap_err();
    assert!(error
        .to_string()
        .contains("Rebuild and restart bruhs-server"));
}

#[tokio::test]
async fn missing_rich_response_shell_is_a_visible_status_error() {
    let error =
        check_shell(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
            .await
            .unwrap_err();
    assert!(error.is_not_found());
}
