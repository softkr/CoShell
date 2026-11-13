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
    /// Command the assistant ran itself (agent mode) and what came back.
    pub exec: Option<Exec>,
    /// A command result fed back to the model: sent, but not shown as a bubble.
    pub hidden: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecState {
    Running,
    /// Output already cleaned and masked.
    Done { output: String, exit: Option<u32>, secrets: usize },
    Failed(String),
}

/// A command the assistant ran in the background to read its result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exec {
    pub cmd: String,
    pub state: ExecState,
    /// The result went back to the model.
    pub sent: bool,
    /// Output expanded in the chat.
    pub open: bool,
}

pub struct Chat {
    pub turns: Vec<Turn>,
    pub draft: text_editor::Content,
    pub attachment: Option<Attachment>,
    pub streaming: Option<iced::task::Handle>,
    pub error: Option<String>,
    /// Risky command clicked once; a second click on Run runs it.
    pub run_confirm: Option<String>,
}

impl Default for Chat {
    fn default() -> Self {
        Self { turns: Vec::new(), draft: text_editor::Content::new(), attachment: None, streaming: None, error: None, run_confirm: None }
    }
}

impl Chat {
    pub fn is_streaming(&self) -> bool {
        self.streaming.is_some()
    }

    /// Stop streaming (or a running command) and mark the partial answer.
    pub fn stop(&mut self) {
        if let Some(h) = self.streaming.take() {
            h.abort();
            if let Some(t) = self.turns.last_mut().filter(|t| t.role == Role::Assistant) {
                t.stopped = true;
                if let Some(e) = t.exec.as_mut().filter(|e| e.state == ExecState::Running) {
                    e.state = ExecState::Failed(tr("중지했습니다").into());
                }
            }
        }
    }

    /// Commands the assistant ran since the user last spoke.
    pub fn agent_steps(&self) -> usize {
        self.turns.iter().rev().take_while(|t| t.role != Role::User || t.hidden).filter(|t| t.hidden).count()
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
    /// "Advanced" section expanded.
    pub advanced_open: bool,
    /// Raw text of the selected profile's advanced fields (parsed as they are typed).
    pub adv: AdvInputs,
}

/// Fields of the advanced section that are typed as text and parsed.
pub struct AdvInputs {
    pub max_tokens: String,
    pub temperature: String,
    /// `Name: value` per line.
    pub headers: text_editor::Content,
    pub extra: text_editor::Content,
    /// Why the current input is not usable (shown under the section, blocks saving).
    pub error: Option<String>,
}

impl AdvInputs {
    pub fn of(p: &Profile) -> Self {
        let a = &p.advanced;
        let headers: Vec<String> = a.headers.iter().map(|h| format!("{}: {}", h.name, h.value)).collect();
        Self {
            max_tokens: a.max_tokens.map(|m| m.to_string()).unwrap_or_default(),
            temperature: a.temperature.map(|t| t.to_string()).unwrap_or_default(),
            headers: text_editor::Content::with_text(&headers.join("\n")),
            extra: text_editor::Content::with_text(&a.extra_body),
            error: None,
        }
    }
}

impl Settings {
    pub fn profile(&self) -> &Profile {
        &self.draft.profiles[self.selected]
    }
    pub fn profile_mut(&mut self) -> &mut Profile {
        &mut self.draft.profiles[self.selected]
    }

    /// After switching profiles.
    pub fn reload_adv(&mut self) {
        self.adv = AdvInputs::of(self.profile());
    }

    /// Parse the typed fields into the profile and re-check it.
    pub fn apply_adv(&mut self) {
        let parsed = (|| -> Result<(Option<u32>, Option<f32>, Vec<coshell_ai::Header>), String> {
            let num = |v: &str, what: &str| -> Result<_, String> {
                Ok(match v.trim() {
                    "" => None,
                    t => Some(t.parse::<f64>().map_err(|_| tf("{0}: 숫자로 입력하세요", &[&what]))?),
                })
            };
            let max = num(&self.adv.max_tokens, tr("최대 토큰"))?;
            if max.is_some_and(|m| m.fract() != 0.0 || m < 0.0 || m > f64::from(u32::MAX)) {
                return Err(tr("최대 토큰은 정수로 입력하세요").into());
            }
            let temp = num(&self.adv.temperature, "temperature")?;
            let mut headers = Vec::new();
            for line in self.adv.headers.text().lines().map(str::trim).filter(|l| !l.is_empty()) {
                let (n, v) = line.split_once(':').ok_or_else(|| tf("헤더는 ‘이름: 값’ 형식으로 한 줄에 하나씩: {0}", &[&line]))?;
                headers.push(coshell_ai::Header { name: n.trim().into(), value: v.trim().into() });
            }
            Ok((max.map(|m| m as u32), temp.map(|t| t as f32), headers))
        })();
        let extra = self.adv.extra.text().trim().to_string();
        match parsed {
            Ok((max, temp, headers)) => {
                let a = &mut self.profile_mut().advanced;
                a.max_tokens = max;
                a.temperature = temp;
                a.headers = headers;
                a.extra_body = extra;
                self.adv.error = self.profile().advanced.check().err();
            }
            Err(e) => self.adv.error = Some(e),
        }
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
    pub fn send(&self, tab: TabId, chat: &mut Chat, target: &str, agent: bool) -> Task<Message> {
        match self.begin(chat, target) {
            Some((profile, key)) => self.stream(tab, chat, target, agent, profile, key),
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
            exec: None,
            hidden: false,
        });
        Some((profile, key))
    }

    /// Conversation so far as a provider request.
    pub fn request(chat: &Chat, target: &str, agent: bool) -> ChatRequest {
        ChatRequest {
            system: assist::system_prompt(target, agent),
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
    pub fn stream(&self, tab: TabId, chat: &mut Chat, target: &str, agent: bool, profile: Profile, key: Option<String>) -> Task<Message> {
        let req = Self::request(chat, target, agent);
        chat.turns.push(Turn {
            role: Role::Assistant,
            text: String::new(),
            sent: String::new(),
            attachment: None,
            model: profile.model.clone(),
            stopped: false,
            exec: None,
            hidden: false,
        });
        let (task, handle) = Task::run(self.client.stream(&profile, key, &req), move |item| Message::AiEvent(tab, item)).abortable();
        chat.streaming = Some(handle);
        task
    }

    pub fn retry(&self, tab: TabId, chat: &mut Chat, target: &str, agent: bool) -> Task<Message> {
        if chat.is_streaming() || chat.turns.last().is_none_or(|t| t.role != Role::User) {
            return Task::none();
        }
        let profile = self.active().clone();
        let key = self.key(&profile);
        chat.error = None;
        self.stream(tab, chat, target, agent, profile, key)
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
            advanced_open: false,
            adv: AdvInputs::of(&self.store.profiles[selected]),
        });
    }

    /// Whether the AI page can be saved as typed.
    pub fn check_settings(&self) -> Result<(), String> {
        let Some(s) = &self.settings else { return Ok(()) };
        if let Some(e) = &s.adv.error {
            return Err(tf("{0}: {1}", &[&s.profile().name, e]));
        }
        for p in &s.draft.profiles {
            p.advanced.check().map_err(|e| tf("{0}: {1}", &[&p.name, &e]))?;
        }
        Ok(())
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
            advanced: Default::default(),
        });
        s.key_status.push(KeySource::None);
        s.selected = s.draft.profiles.len() - 1;
        s.models.clear();
        s.status = None;
        s.reload_adv();
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
        s.reload_adv();
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
