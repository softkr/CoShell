//! AI panel state: profiles, per-tab chats, settings dialog.

use crate::i18n::{tf, tr};
use std::path::PathBuf;
use std::sync::Arc;

use coshell_ai::profile::KeySource;
use coshell_ai::{
    ChatMessage, ChatRequest, Client, DEFAULT_MAX_TOKENS, Format, Keychain, Profile, ProfileStore, Role, SecretStore,
    resolve_key,
};
use iced::Task;
use iced::widget::text_editor;
use zeroize::Zeroizing;

use crate::app::{Message, TabId};
use crate::assist;

pub const AI_INPUT: &str = "ai-input";

pub struct Attachment {
    pub label: String,
    pub masked: String,
    pub secrets: usize,
}

pub struct Turn {
    pub role: Role,
    /// What the UI shows.
    pub text: String,
    /// What was sent to the model (question + framed attachment).
    pub sent: String,
    pub attachment: Option<String>,
    pub model: String,
    pub stopped: bool,
}

pub struct Chat {
    pub turns: Vec<Turn>,
    pub draft: text_editor::Content,
    pub attachment: Option<Attachment>,
    pub streaming: Option<iced::task::Handle>,
    pub error: Option<String>,
}

impl Default for Chat {
    fn default() -> Self {
        Self { turns: Vec::new(), draft: text_editor::Content::new(), attachment: None, streaming: None, error: None }
    }
}

impl Chat {
    pub fn is_streaming(&self) -> bool {
        self.streaming.is_some()
    }

    /// Stop streaming and mark the partial answer.
    pub fn stop(&mut self) {
        if let Some(h) = self.streaming.take() {
            h.abort();
            if let Some(t) = self.turns.last_mut().filter(|t| t.role == Role::Assistant) {
                t.stopped = true;
            }
        }
    }
}

pub struct Settings {
    pub draft: ProfileStore,
    pub selected: usize,
    pub key_input: Zeroizing<String>,
    /// Cached per profile (reading the keychain on every frame would prompt the user).
    pub key_status: Vec<KeySource>,
    pub models: Vec<String>,
    pub status: Option<Result<String, String>>,
    pub busy: bool,
    /// Profiles removed in this dialog; their keys are deleted on save.
    removed: Vec<String>,
}

impl Settings {
    pub fn profile(&self) -> &Profile {
        &self.draft.profiles[self.selected]
    }
    pub fn profile_mut(&mut self) -> &mut Profile {
        &mut self.draft.profiles[self.selected]
    }
}

pub struct Ai {
    pub open: bool,
    pub store: ProfileStore,
    path: Option<PathBuf>,
    pub client: Client,
    secrets: Arc<dyn SecretStore>,
    pub settings: Option<Settings>,
}

impl Ai {
    /// Load profiles; returns a warning when the file is unreadable.
    pub fn load() -> (Self, Option<String>) {
        let path = ProfileStore::default_path();
        let (store, warn) = match path.as_deref().map(ProfileStore::load) {
            Some(Ok(s)) => (s, None),
            Some(Err(e)) => (ProfileStore::default(), Some(tf("AI 설정 읽기 실패: {0}", &[&e]))),
            None => (ProfileStore::default(), None),
        };
        (Self::new(store, path, Arc::new(Keychain::AI)), warn)
    }

    pub fn new(store: ProfileStore, path: Option<PathBuf>, secrets: Arc<dyn SecretStore>) -> Self {
        Self { open: true, store, path, client: Client::new(), secrets, settings: None }
    }

    pub fn active(&self) -> &Profile {
        self.store.active()
    }

    fn key(&self, p: &Profile) -> Option<String> {
        resolve_key(p, self.secrets.as_ref()).0.map(|k| k.as_str().to_string())
    }

    fn save_store(&self) -> Result<(), String> {
        match &self.path {
            Some(p) => self.store.save(p),
            None => Err(tr("HOME을 알 수 없어 설정을 저장할 수 없습니다").into()),
        }
    }

    pub fn set_active(&mut self, id: &str) -> Option<String> {
        if self.store.profiles.iter().any(|p| p.id == id) {
            self.store.active = id.to_string();
            return self.save_store().err();
        }
        None
    }

    /// Send the draft (+ attachment) and start streaming the answer.
    pub fn send(&self, tab: TabId, chat: &mut Chat, target: &str) -> Task<Message> {
        match self.begin(chat, target) {
            Some((profile, key)) => self.stream(tab, chat, target, profile, key),
            None => Task::none(),
        }
    }

    /// Validate and record the user's turn. `None` = nothing to send (or an
    /// error was put on the chat).
    pub fn begin(&self, chat: &mut Chat, target: &str) -> Option<(Profile, Option<String>)> {
        if chat.is_streaming() {
            return None;
        }
        let question = chat.draft.text().trim().to_string();
        if question.is_empty() && chat.attachment.is_none() {
            return None;
        }
        let profile = self.active().clone();
        let key = self.key(&profile);
        if key.is_none() && !profile.is_local() {
            chat.error = Some(tf("{0} 프로필에 API 키가 없습니다. AI 설정에서 키를 저장하세요.", &[&profile.name]));
            return None;
        }
        let question = if question.is_empty() { tr("첨부한 터미널 출력을 분석해줘").to_string() } else { question };
        let attachment = chat.attachment.take();
        let sent = match &attachment {
            Some(a) => assist::with_attachment(&question, target, &a.masked),
            None => question.clone(),
        };
        chat.error = None;
        chat.draft = text_editor::Content::new();
        chat.turns.push(Turn {
            role: Role::User,
            text: question,
            sent,
            attachment: attachment.map(|a| a.label),
            model: String::new(),
            stopped: false,
        });
        Some((profile, key))
    }

    /// Conversation so far as a provider request.
    pub fn request(chat: &Chat, target: &str) -> ChatRequest {
        ChatRequest {
            system: assist::system_prompt(target),
            messages: chat
                .turns
                .iter()
                .filter(|t| !(t.role == Role::Assistant && t.text.is_empty()))
                .map(|t| ChatMessage { role: t.role, content: t.sent.clone() })
                .collect(),
            max_tokens: DEFAULT_MAX_TOKENS,
        }
    }

    /// (Re)stream an answer for the conversation so far.
    pub fn stream(&self, tab: TabId, chat: &mut Chat, target: &str, profile: Profile, key: Option<String>) -> Task<Message> {
        let req = Self::request(chat, target);
        chat.turns.push(Turn {
            role: Role::Assistant,
            text: String::new(),
            sent: String::new(),
            attachment: None,
            model: profile.model.clone(),
            stopped: false,
        });
        let (task, handle) = Task::run(self.client.stream(&profile, key, &req), move |item| Message::AiEvent(tab, item)).abortable();
        chat.streaming = Some(handle);
        task
    }

    pub fn retry(&self, tab: TabId, chat: &mut Chat, target: &str) -> Task<Message> {
        if chat.is_streaming() || chat.turns.last().is_none_or(|t| t.role != Role::User) {
            return Task::none();
        }
        let profile = self.active().clone();
        let key = self.key(&profile);
        chat.error = None;
        self.stream(tab, chat, target, profile, key)
    }

    // ------------------------------------------------------------ settings

    pub fn open_settings(&mut self) {
        let draft = self.store.clone();
        let selected = draft.profiles.iter().position(|p| p.id == draft.active).unwrap_or(0);
        let key_status = draft.profiles.iter().map(|p| resolve_key(p, self.secrets.as_ref()).1).collect();
        self.settings = Some(Settings {
            draft,
            selected,
            key_input: Zeroizing::new(String::new()),
            key_status,
            models: Vec::new(),
            status: None,
            busy: false,
            removed: Vec::new(),
        });
    }

    pub fn save_settings(&mut self) -> Result<(), String> {
        let Some(s) = self.settings.take() else { return Ok(()) };
        for id in &s.removed {
            let _ = self.secrets.delete(id);
        }
        self.store = s.draft;
        self.save_store()
    }

    pub fn save_key(&mut self) {
        let Some(s) = &mut self.settings else { return };
        let id = s.profile().id.clone();
        let key = std::mem::replace(&mut s.key_input, Zeroizing::new(String::new()));
        if key.trim().is_empty() {
            return;
        }
        s.status = Some(match self.secrets.set(&id, key.trim()) {
            Ok(()) => {
                s.key_status[s.selected] = KeySource::Keychain;
                Ok(tr("키체인에 저장했습니다").into())
            }
            Err(e) => Err(tf("키 저장 실패: {0}", &[&e])),
        });
    }

    pub fn delete_key(&mut self) {
        let Some(s) = &mut self.settings else { return };
        let id = s.profile().id.clone();
        s.status = Some(match self.secrets.delete(&id) {
            Ok(()) => {
                let p = s.profile().clone();
                s.key_status[s.selected] = resolve_key(&p, self.secrets.as_ref()).1;
                Ok(tr("키를 삭제했습니다").into())
            }
            Err(e) => Err(tf("키 삭제 실패: {0}", &[&e])),
        });
    }

    pub fn add_profile(&mut self) {
        let Some(s) = &mut self.settings else { return };
        let n = s.draft.profiles.len() + 1;
        s.draft.profiles.push(Profile {
            id: format!("profile-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(n as u128, |d| d.as_millis())),
            name: tf("새 프로필 {0}", &[&n]),
            format: Format::OpenAi,
            base_url: Format::OpenAi.default_base().into(),
            model: String::new(),
        });
        s.key_status.push(KeySource::None);
        s.selected = s.draft.profiles.len() - 1;
        s.models.clear();
        s.status = None;
    }

    pub fn remove_profile(&mut self) {
        let Some(s) = &mut self.settings else { return };
        if s.draft.profiles.len() <= 1 {
            return;
        }
        let removed = s.draft.profiles.remove(s.selected);
        s.key_status.remove(s.selected);
        s.removed.push(removed.id.clone());
        s.selected = s.selected.min(s.draft.profiles.len() - 1);
        if s.draft.active == removed.id {
            s.draft.active = s.draft.profiles[0].id.clone();
        }
    }

    /// `GET /v1/models` for the selected draft profile.
    pub fn fetch_models(&mut self, test_only: bool) -> Task<Message> {
        let Some(s) = &mut self.settings else { return Task::none() };
        s.busy = true;
        s.status = None;
        let profile = s.profile().clone();
        let key = resolve_key(&profile, self.secrets.as_ref()).0.map(|k| k.as_str().to_string());
        let client = self.client.clone();
        Task::perform(async move { client.list_models(&profile, key).await.map_err(|e| e.to_string()) }, move |r| {
            Message::AiModels(test_only, r)
        })
    }
}
