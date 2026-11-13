//! HTTP + streaming for both wire formats.

use std::collections::VecDeque;
use std::time::Duration;

use futures::{Stream, StreamExt, stream};
use serde_json::{Value, json};

use crate::sse::{SseEvent, SseParser};
use crate::{AiError, AuthStyle, ChatRequest, Format, Profile, StreamEvent};

const ANTHROPIC_VERSION: &str = "2023-06-01";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Non-streaming calls (model list) only; streams have no total timeout.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Cap on error bodies we keep for messages.
const MAX_ERROR_BODY: usize = 2000;

/// `{base}/v1/<path>`, tolerating a base that already ends in `/v1`.
pub fn endpoint(profile: &Profile, path: &str) -> String {
    let base = profile.base_url.trim_end_matches('/');
    let base = base.strip_suffix("/v1").unwrap_or(base);
    format!("{base}/v1/{path}")
}

fn chat_path(format: Format) -> &'static str {
    match format {
        Format::Anthropic => "messages",
        Format::OpenAi => "chat/completions",
    }
}

/// A custom path joined to the base URL, or a full URL as is.
fn custom(profile: &Profile, path: &str) -> String {
    let p = path.trim();
    if p.starts_with("http://") || p.starts_with("https://") {
        p.to_string()
    } else {
        format!("{}/{}", profile.base_url.trim().trim_end_matches('/'), p.trim_start_matches('/'))
    }
}

/// Where chat requests go (the profile's own path when set).
pub fn chat_url(profile: &Profile) -> String {
    match profile.advanced.chat_path.trim() {
        "" => endpoint(profile, chat_path(profile.format)),
        p => custom(profile, p),
    }
}

/// Where the model list is read.
pub fn models_url(profile: &Profile) -> String {
    match profile.advanced.models_path.trim() {
        "" => endpoint(profile, "models"),
        p => custom(profile, p),
    }
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .user_agent(concat!("CoShell/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest client");
        Self { http }
    }

    fn request(&self, method: reqwest::Method, profile: &Profile, key: Option<&str>, url: String) -> reqwest::RequestBuilder {
        let adv = &profile.advanced;
        let mut rb = self.http.request(method, url);
        if profile.format == Format::Anthropic {
            let v = adv.anthropic_version.trim();
            rb = rb.header("anthropic-version", if v.is_empty() { ANTHROPIC_VERSION } else { v });
        }
        if let Some(k) = key {
            rb = match (adv.auth, profile.format) {
                (AuthStyle::Auto, Format::Anthropic) => rb.header("x-api-key", k),
                (AuthStyle::Auto, Format::OpenAi) | (AuthStyle::Bearer, _) => rb.bearer_auth(k),
                (AuthStyle::Header, _) => rb.header(adv.auth_header.trim(), k),
                (AuthStyle::None, _) => rb,
            };
        }
        for h in &adv.headers {
            rb = rb.header(h.name.as_str(), h.value.as_str());
        }
        rb
    }

    /// Request body; the profile's parameters and extra JSON go on top.
    pub(crate) fn body(profile: &Profile, req: &ChatRequest) -> Result<Value, AiError> {
        let adv = &profile.advanced;
        let msgs: Vec<Value> = req.messages.iter().map(|m| json!({ "role": m.role.as_str(), "content": m.content })).collect();
        let mut b = match profile.format {
            Format::Anthropic => {
                let max = adv.max_tokens.unwrap_or(req.max_tokens);
                let mut b = json!({ "model": profile.model, "max_tokens": max, "messages": msgs, "stream": true });
                if !req.system.is_empty() {
                    b["system"] = json!(req.system);
                }
                b
            }
            Format::OpenAi => {
                let mut all = Vec::with_capacity(msgs.len() + 1);
                if !req.system.is_empty() {
                    all.push(json!({ "role": "system", "content": req.system }));
                }
                all.extend(msgs);
                let mut b = json!({ "model": profile.model, "messages": all, "stream": true });
                if let Some(m) = adv.max_tokens {
                    b["max_tokens"] = json!(m);
                }
                b
            }
        };
        if let Some(t) = adv.temperature {
            b["temperature"] = json!(t);
        }
        if !adv.extra_body.trim().is_empty() {
            let extra: Value = serde_json::from_str(&adv.extra_body).map_err(|e| AiError::Protocol(e.to_string()))?;
            let Value::Object(extra) = extra else {
                return Err(AiError::Protocol(if crate::english() { "extra JSON is not an object" } else { "추가 JSON이 객체가 아닙니다" }.into()));
            };
            for (k, v) in extra {
                b[k] = v;
            }
        }
        Ok(b)
    }

    /// Stream a chat completion. Errors are yielded as items; the stream ends
    /// after `Done` or the first error.
    pub fn stream(
        &self,
        profile: &Profile,
        key: Option<String>,
        req: &ChatRequest,
    ) -> impl Stream<Item = Result<StreamEvent, AiError>> + Send + 'static {
        let body = Self::body(profile, req);
        let rb = self
            .request(reqwest::Method::POST, profile, key.as_deref(), chat_url(profile))
            .header(reqwest::header::ACCEPT, "text/event-stream");
        let format = profile.format;
        let start = Box::pin(async move {
            let resp = rb.json(&body?).send().await.map_err(|e| AiError::Network(e.without_url().to_string()))?;
            Ok::<_, AiError>(Box::pin(check(resp).await?.bytes_stream()))
        });

        enum St<F, S> {
            Start(F),
            Reading { bytes: Option<S>, parser: SseParser, queue: Queue, stop: Option<String> },
            Done,
        }

        stream::unfold(St::Start(start), move |mut st| async move {
            loop {
                st = match st {
                    St::Done => return None,
                    St::Start(fut) => match fut.await {
                        Ok(bytes) => {
                            St::Reading { bytes: Some(bytes), parser: SseParser::default(), queue: Queue::new(), stop: None }
                        }
                        Err(e) => return Some((Err(e), St::Done)),
                    },
                    St::Reading { bytes, mut parser, mut queue, mut stop } => {
                        if let Some(item) = queue.pop_front() {
                            // Anything but a text delta (Done / error) ends the stream.
                            let next = if matches!(item, Ok(StreamEvent::Delta(_))) {
                                St::Reading { bytes, parser, queue, stop }
                            } else {
                                St::Done
                            };
                            return Some((item, next));
                        }
                        let Some(mut b) = bytes else {
                            // Server closed without an explicit end marker.
                            return Some((Ok(StreamEvent::Done { stop_reason: stop }), St::Done));
                        };
                        match b.next().await {
                            Some(Ok(chunk)) => {
                                for ev in parser.push(&chunk) {
                                    interpret(format, &ev, &mut stop, &mut queue);
                                }
                                St::Reading { bytes: Some(b), parser, queue, stop }
                            }
                            Some(Err(e)) => return Some((Err(AiError::Network(e.without_url().to_string())), St::Done)),
                            None => {
                                if let Some(ev) = parser.finish() {
                                    interpret(format, &ev, &mut stop, &mut queue);
                                }
                                St::Reading { bytes: None, parser, queue, stop }
                            }
                        }
                    }
                };
            }
        })
    }

    /// `GET /v1/models` → model ids.
    pub async fn list_models(&self, profile: &Profile, key: Option<String>) -> Result<Vec<String>, AiError> {
        let resp = self
            .request(reqwest::Method::GET, profile, key.as_deref(), models_url(profile))
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(|e| AiError::Network(e.without_url().to_string()))?;
        let v: Value = check(resp).await?.json().await.map_err(|e| AiError::Protocol(e.to_string()))?;
        let ids = v["data"]
            .as_array()
            .ok_or_else(|| AiError::Protocol(if crate::english() { "no data array" } else { "data 배열이 없습니다" }.into()))?
            .iter()
            .filter_map(|m| m["id"].as_str().map(String::from))
            .collect();
        Ok(ids)
    }
}

/// Turn a non-2xx response into `AiError::Http` with the provider's message.
async fn check(resp: reqwest::Response) -> Result<reqwest::Response, AiError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let text = resp.text().await.unwrap_or_default();
    let message = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(String::from))
        .unwrap_or_else(|| text.chars().take(MAX_ERROR_BODY).collect());
    Err(AiError::Http { status: status.as_u16(), message })
}

type Queue = VecDeque<Result<StreamEvent, AiError>>;

fn interpret(format: Format, ev: &SseEvent, stop: &mut Option<String>, q: &mut Queue) {
    match format {
        Format::Anthropic => anthropic_event(ev, stop, q),
        Format::OpenAi => openai_event(ev, stop, q),
    }
}

fn anthropic_event(ev: &SseEvent, stop: &mut Option<String>, q: &mut Queue) {
    let Ok(v) = serde_json::from_str::<Value>(&ev.data) else { return };
    match v["type"].as_str().or(ev.event.as_deref()) {
        Some("content_block_delta") if v["delta"]["type"] == "text_delta" => {
            if let Some(t) = v["delta"]["text"].as_str() {
                q.push_back(Ok(StreamEvent::Delta(t.to_string())));
            }
        }
        Some("message_delta") => {
            if let Some(r) = v["delta"]["stop_reason"].as_str() {
                *stop = Some(r.to_string());
            }
        }
        Some("message_stop") => q.push_back(Ok(StreamEvent::Done { stop_reason: stop.take() })),
        Some("error") => {
            let msg = v["error"]["message"].as_str().unwrap_or("stream error").to_string();
            q.push_back(Err(AiError::Protocol(msg)));
        }
        _ => {}
    }
}

fn openai_event(ev: &SseEvent, stop: &mut Option<String>, q: &mut Queue) {
    if ev.data.trim() == "[DONE]" {
        q.push_back(Ok(StreamEvent::Done { stop_reason: stop.take() }));
        return;
    }
    let Ok(v) = serde_json::from_str::<Value>(&ev.data) else { return };
    if let Some(msg) = v["error"]["message"].as_str() {
        q.push_back(Err(AiError::Protocol(msg.to_string())));
        return;
    }
    let choice = &v["choices"][0];
    if let Some(t) = choice["delta"]["content"].as_str()
        && !t.is_empty()
    {
        q.push_back(Ok(StreamEvent::Delta(t.to_string())));
    }
    if let Some(r) = choice["finish_reason"].as_str() {
        *stop = Some(r.to_string());
    }
}
