//! Chat providers for CoShell: Anthropic Messages and OpenAI Chat Completions
//! (and any server speaking one of them: Ollama, vLLM, LiteLLM, OpenRouter …).

pub mod profile;
mod provider;
pub mod redact;
mod sse;

pub use profile::{Format, KeySource, Keychain, Profile, ProfileStore, SecretStore, resolve_key};
pub use provider::{Client, endpoint};

/// Default response budget (Anthropic requires `max_tokens`).
pub const DEFAULT_MAX_TOKENS: u32 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

impl Role {
    fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

#[derive(Clone, Debug)]
pub struct ChatRequest {
    pub system: String,
    pub messages: Vec<ChatMessage>,
    pub max_tokens: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamEvent {
    Delta(String),
    Done { stop_reason: Option<String> },
}

static ENGLISH: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Word user-facing messages (errors, prompts) in English instead of Korean.
pub fn set_english(on: bool) {
    ENGLISH.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn english() -> bool {
    ENGLISH.load(std::sync::atomic::Ordering::Relaxed)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiError {
    Http { status: u16, message: String },
    Network(String),
    Protocol(String),
    MissingKey,
}

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self, english()) {
            (Self::Http { status, message }, _) => write!(f, "HTTP {status}: {message}"),
            (Self::Network(e), true) => write!(f, "Network error: {e}"),
            (Self::Network(e), false) => write!(f, "네트워크 오류: {e}"),
            (Self::Protocol(e), true) => write!(f, "Unexpected response: {e}"),
            (Self::Protocol(e), false) => write!(f, "응답 형식 오류: {e}"),
            (Self::MissingKey, true) => write!(f, "No API key"),
            (Self::MissingKey, false) => write!(f, "API 키가 없습니다"),
        }
    }
}

impl std::error::Error for AiError {}

#[cfg(test)]
mod language_tests {
    use super::*;

    #[test]
    fn errors_are_worded_in_the_selected_language() {
        assert_eq!(AiError::MissingKey.to_string(), "API 키가 없습니다");
        set_english(true);
        let english = (AiError::MissingKey.to_string(), AiError::Network("timeout".into()).to_string());
        set_english(false);
        assert_eq!(english.0, "No API key");
        assert_eq!(english.1, "Network error: timeout");
    }
}
