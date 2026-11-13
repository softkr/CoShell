//! Wire-level tests against a tiny in-process HTTP server.

use std::time::Duration;

use coshell_ai::{AiError, ChatMessage, ChatRequest, Client, Format, Profile, Role, StreamEvent, endpoint};
use futures::StreamExt;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

struct Captured {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl Captured {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap()
    }
}

/// Serve one request: reply with `status`, `content_type` and `chunks`
/// (written separately with small pauses to exercise chunk boundaries).
async fn serve_once(status: u16, content_type: &'static str, chunks: Vec<Vec<u8>>) -> (String, oneshot::Receiver<Captured>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = oneshot::channel();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        let header_end = loop {
            let n = sock.read(&mut tmp).await.unwrap();
            buf.extend_from_slice(&tmp[..n]);
            if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break i + 4;
            }
        };
        let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
        let mut lines = head.split("\r\n");
        let mut first = lines.next().unwrap().split(' ');
        let (method, path) = (first.next().unwrap().to_string(), first.next().unwrap().to_string());
        let headers: Vec<(String, String)> = lines
            .filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
            .collect();
        let len: usize = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).map_or(0, |(_, v)| v.parse().unwrap());
        while buf.len() < header_end + len {
            let n = sock.read(&mut tmp).await.unwrap();
            buf.extend_from_slice(&tmp[..n]);
        }
        let body = String::from_utf8_lossy(&buf[header_end..header_end + len]).to_string();
        let _ = tx.send(Captured { method, path, headers, body });

        let head = format!("HTTP/1.1 {status} X\r\nContent-Type: {content_type}\r\nConnection: close\r\n\r\n");
        sock.write_all(head.as_bytes()).await.unwrap();
        for c in chunks {
            sock.write_all(&c).await.unwrap();
            sock.flush().await.unwrap();
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    });
    (base, rx)
}

fn profile(format: Format, base: &str) -> Profile {
    Profile { id: "t".into(), name: "t".into(), format, base_url: base.into(), model: "m-1".into() }
}

fn req() -> ChatRequest {
    ChatRequest {
        system: "be brief".into(),
        messages: vec![ChatMessage { role: Role::User, content: "디스크 확인".into() }],
        max_tokens: 256,
    }
}

async fn collect(client: &Client, p: &Profile, key: Option<&str>) -> Vec<Result<StreamEvent, AiError>> {
    client.stream(p, key.map(String::from), &req()).collect().await
}

fn text_of(items: &[Result<StreamEvent, AiError>]) -> String {
    items.iter().filter_map(|i| match i {
        Ok(StreamEvent::Delta(t)) => Some(t.as_str()),
        _ => None,
    }).collect()
}

/// Split a UTF-8 string into byte chunks of `n` (may cut characters).
fn byte_chunks(s: &str, n: usize) -> Vec<Vec<u8>> {
    s.as_bytes().chunks(n).map(<[u8]>::to_vec).collect()
}

#[tokio::test]
async fn anthropic_stream_request_and_deltas() {
    let sse = concat!(
        "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{}}\n\n",
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0}\n\n",
        "event: ping\ndata: {\"type\":\"ping\"}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"디스크는 \"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"`df -h`로 확인합니다.\"}}\n\n",
        "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    );
    let (base, cap) = serve_once(200, "text/event-stream", byte_chunks(sse, 7)).await;
    let items = collect(&Client::new(), &profile(Format::Anthropic, &base), Some("sk-ant-test")).await;

    assert_eq!(text_of(&items), "디스크는 `df -h`로 확인합니다.");
    assert_eq!(items.last(), Some(&Ok(StreamEvent::Done { stop_reason: Some("end_turn".into()) })));
    let c = cap.await.unwrap();
    assert_eq!((c.method.as_str(), c.path.as_str()), ("POST", "/v1/messages"));
    assert_eq!(c.header("x-api-key"), Some("sk-ant-test"));
    assert_eq!(c.header("anthropic-version"), Some("2023-06-01"));
    assert_eq!(c.header("authorization"), None);
    let b = c.json();
    assert_eq!(b["model"], "m-1");
    assert_eq!(b["max_tokens"], 256);
    assert_eq!(b["system"], "be brief");
    assert_eq!(b["stream"], true);
    assert_eq!(b["messages"][0]["role"], "user");
    assert_eq!(b["messages"][0]["content"], "디스크 확인");
}

#[tokio::test]
async fn openai_stream_request_and_done() {
    let sse = concat!(
        "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"한글 \"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"응답\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    let (base, cap) = serve_once(200, "text/event-stream", byte_chunks(sse, 5)).await;
    let items = collect(&Client::new(), &profile(Format::OpenAi, &format!("{base}/v1/")), Some("sk-test")).await;

    assert_eq!(text_of(&items), "한글 응답");
    assert_eq!(items.last(), Some(&Ok(StreamEvent::Done { stop_reason: Some("stop".into()) })));
    let c = cap.await.unwrap();
    assert_eq!(c.path, "/v1/chat/completions", "base ending in /v1/ must not double the prefix");
    assert_eq!(c.header("authorization"), Some("Bearer sk-test"));
    assert_eq!(c.header("x-api-key"), None);
    let b = c.json();
    assert_eq!(b["messages"][0]["role"], "system");
    assert_eq!(b["messages"][0]["content"], "be brief");
    assert_eq!(b["messages"][1]["content"], "디스크 확인");
    assert!(b.get("max_tokens").is_none());
}

#[tokio::test]
async fn keyless_local_request_has_no_auth_header() {
    let (base, cap) = serve_once(200, "text/event-stream", vec![b"data: [DONE]\n\n".to_vec()]).await;
    let items = collect(&Client::new(), &profile(Format::OpenAi, &base), None).await;
    assert_eq!(items, [Ok(StreamEvent::Done { stop_reason: None })]);
    assert_eq!(cap.await.unwrap().header("authorization"), None);
}

#[tokio::test]
async fn http_error_surfaces_provider_message() {
    let body = br#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#.to_vec();
    let (base, _cap) = serve_once(401, "application/json", vec![body]).await;
    let items = collect(&Client::new(), &profile(Format::Anthropic, &base), Some("bad")).await;
    assert_eq!(items, [Err(AiError::Http { status: 401, message: "invalid x-api-key".into() })]);
}

#[tokio::test]
async fn mid_stream_error_event_ends_stream() {
    let sse = concat!(
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"부분\"}}\n\n",
        "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"무시됨\"}}\n\n",
    );
    let (base, _cap) = serve_once(200, "text/event-stream", vec![sse.as_bytes().to_vec()]).await;
    let items = collect(&Client::new(), &profile(Format::Anthropic, &base), Some("k")).await;
    assert_eq!(items, [Ok(StreamEvent::Delta("부분".into())), Err(AiError::Protocol("Overloaded".into()))]);
}

#[tokio::test]
async fn closed_without_end_marker_still_finishes() {
    let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"끝\"}}]}\n\n";
    let (base, _cap) = serve_once(200, "text/event-stream", vec![sse.as_bytes().to_vec()]).await;
    let items = collect(&Client::new(), &profile(Format::OpenAi, &base), None).await;
    assert_eq!(items, [Ok(StreamEvent::Delta("끝".into())), Ok(StreamEvent::Done { stop_reason: None })]);
}

#[tokio::test]
async fn list_models_both_formats() {
    let body = br#"{"data":[{"id":"claude-opus-5-5","type":"model"},{"id":"claude-sonnet-5-5","type":"model"}]}"#.to_vec();
    let (base, cap) = serve_once(200, "application/json", vec![body]).await;
    let ids = Client::new().list_models(&profile(Format::Anthropic, &base), Some("k".into())).await.unwrap();
    assert_eq!(ids, ["claude-opus-5-5", "claude-sonnet-5-5"]);
    let c = cap.await.unwrap();
    assert_eq!((c.method.as_str(), c.path.as_str(), c.header("x-api-key")), ("GET", "/v1/models", Some("k")));

    let body = br#"{"object":"list","data":[{"id":"llama3.1"}]}"#.to_vec();
    let (base, _cap) = serve_once(200, "application/json", vec![body]).await;
    let ids = Client::new().list_models(&profile(Format::OpenAi, &base), None).await.unwrap();
    assert_eq!(ids, ["llama3.1"]);
}

#[test]
fn endpoint_joining() {
    let p = |b: &str| profile(Format::OpenAi, b);
    assert_eq!(endpoint(&p("https://api.openai.com"), "models"), "https://api.openai.com/v1/models");
    assert_eq!(endpoint(&p("https://openrouter.ai/api/v1"), "chat/completions"), "https://openrouter.ai/api/v1/chat/completions");
    assert_eq!(endpoint(&p("http://localhost:11434/"), "models"), "http://localhost:11434/v1/models");
}
