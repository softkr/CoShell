//! Application state and update logic (tabs, palette, connection prompts).

use crate::i18n::{tf, tr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use coshell_ssh::config::{HostEntry, SshConfig};
use coshell_ssh::{
    ConnectOptions, Connection, ForwardSpec, HostKeyDecision, HostKeyPolicy, HostKeyQuery, HostSpec, InteractiveQuery, Progress,
    Tunnel,
};
use coshell_term::{GridSize, Session, TermEvent};
use futures::channel::oneshot;
use iced::keyboard::{self, key::Named};
use iced::{Event, Subscription, Task, event, window};
use zeroize::Zeroizing;

use crate::ai::{AI_INPUT, Ai, Attachment, Chat};
use crate::settings::{self, CursorKind, Scheme, Settings};
use crate::assist;
use crate::conn::{self, Answers, ConnEvent, Once};
use crate::hosts::{Env, HostDef, HostStore, parse_ssh_command, slugify};
use coshell_ai::{AiError, Format, Keychain, Role, SecretStore, StreamEvent};
use iced::widget::text_editor;
use crate::launch::Launch;

pub const PALETTE_INPUT: &str = "palette-input";
/// Tunnel statistics refresh while the panel is open.
const STATS_REFRESH: Duration = Duration::from_secs(1);
pub const CONFIRM_INPUT: &str = "hostkey-confirm";
pub const SEARCH_INPUT: &str = "term-search";
pub const RENAME_INPUT: &str = "tab-rename";
/// Two clicks on the active tab within this time start a rename.
const TAB_DOUBLE_CLICK: Duration = Duration::from_millis(400);
const TAB_NAME_MAX_CHARS: usize = 40;
/// Notices dismiss themselves after this long.
const NOTICE_TTL: Duration = Duration::from_secs(10);
/// Terminal width kept when the AI panel is dragged wider.
const TERMINAL_MIN_W: f32 = 320.0;
/// Remembered state (window size, zoom, panel) is written this long after it last changed.
const PERSIST_DELAY: Duration = Duration::from_secs(2);
pub fn prompt_input_id(i: usize) -> String {
    format!("prompt-{i}")
}

pub type TabId = u64;

/// Keychain service for saved server passwords (AI keys use "CoShell AI").
pub const PASSWORD_SERVICE: &str = "CoShell SSH";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Wait,
    Connecting,
    Authenticating,
    Done(Duration),
}

pub enum Phase {
    Connecting,
    Live(Session),
    Failed(String),
}

pub enum Pending {
    HostKey { query: HostKeyQuery, reply: oneshot::Sender<HostKeyDecision>, confirm: String },
    Interactive {
        query: InteractiveQuery,
        reply: oneshot::Sender<Answers>,
        answers: Vec<Zeroizing<String>>,
        /// "Save this password" (login password prompts only).
        save: bool,
    },
}

pub enum TunnelState {
    Starting,
    Running(Tunnel),
    /// Stopped by the user; kept in the list so it can be started again.
    Stopped,
    Failed(String),
}

pub struct TunnelEntry {
    pub id: u64,
    pub spec: ForwardSpec,
    /// Local port was left blank: fall back to a free port if it is taken.
    pub auto_port: bool,
    pub state: TunnelState,
}

/// "Add tunnel" form, phrased by purpose rather than -L/-R/-D.
pub struct TunnelForm {
    /// 'L' server port → this PC, 'R' this PC → server, 'D' SOCKS proxy.
    pub kind: char,
    pub remote_host: String,
    pub remote_port: String,
    pub local_port: String,
    /// Also save to the host so it opens on every connect.
    pub persist: bool,
    pub error: Option<String>,
}

impl Default for TunnelForm {
    fn default() -> Self {
        Self { kind: 'L', remote_host: String::new(), remote_port: String::new(), local_port: String::new(), persist: false, error: None }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum TunnelField {
    RemoteHost,
    RemotePort,
    LocalPort,
}

/// Default SOCKS port when the field is left blank.
const SOCKS_DEFAULT_PORT: u16 = 1080;
const LOOPBACK: &str = "127.0.0.1";

fn port_of(field: &str, v: &str) -> Result<u16, String> {
    v.trim().parse::<u16>().ok().filter(|p| *p != 0).ok_or_else(|| tf("{0}: 1–65535 사이의 숫자를 입력하세요", &[&field]))
}

/// Turn the form into a forward. Returns (spec, auto_port).
pub fn form_spec(f: &TunnelForm) -> Result<(ForwardSpec, bool), String> {
    let blank = |v: &str| v.trim().is_empty();
    match f.kind {
        'L' => {
            let remote = port_of(tr("서버 포트"), &f.remote_port)?;
            let host = if blank(&f.remote_host) { "localhost".to_string() } else { f.remote_host.trim().to_string() };
            if host.contains(char::is_whitespace) {
                return Err(tr("서버 쪽 주소에 공백을 쓸 수 없습니다").into());
            }
            let (local, auto) = if blank(&f.local_port) { (remote, true) } else { (port_of(tr("내 PC 포트"), &f.local_port)?, false) };
            Ok((ForwardSpec::Local { bind: LOOPBACK.into(), port: local, host, host_port: remote }, auto))
        }
        'R' => {
            let local = port_of(tr("내 PC 포트"), &f.local_port)?;
            let remote = if blank(&f.remote_port) { local } else { port_of(tr("서버 포트"), &f.remote_port)? };
            Ok((ForwardSpec::Remote { bind: LOOPBACK.into(), port: remote, host: LOOPBACK.into(), host_port: local }, false))
        }
        _ => {
            let (port, auto) =
                if blank(&f.local_port) { (SOCKS_DEFAULT_PORT, true) } else { (port_of(tr("프록시 포트"), &f.local_port)?, false) };
            Ok((ForwardSpec::Dynamic { bind: LOOPBACK.into(), port }, auto))
        }
    }
}

/// `L:port:host:hostport` form stored in hosts.toml.
fn forward_string(spec: &ForwardSpec) -> String {
    match spec {
        ForwardSpec::Local { bind, port, host, host_port } => format!("L:{bind}:{port}:{host}:{host_port}"),
        ForwardSpec::Remote { bind, port, host, host_port } => format!("R:{bind}:{port}:{host}:{host_port}"),
        ForwardSpec::Dynamic { bind, port } => format!("D:{bind}:{port}"),
    }
}

pub struct Tab {
    pub id: TabId,
    pub name: String,
    pub env: Env,
    pub phase: Phase,
    /// Hop labels (`user@host[:port]`); empty for local shells.
    pub route: Vec<String>,
    pub steps: Vec<Step>,
    pub title: String,
    /// Name typed by the user; wins over the program's title.
    pub custom_name: Option<String>,
    pub exited: bool,
    pub prompt: Option<Pending>,
    pub chat: Chat,
    pub conn: Option<Connection>,
    pub tunnels: Vec<TunnelEntry>,
    /// Host alias this tab was opened from (saved tunnels go there).
    pub alias: Option<String>,
    /// Started after every successful connect (from ~/.ssh/config).
    auto_forwards: Vec<ForwardSpec>,
    /// What to reconnect to (SSH only).
    spec: Option<(Vec<HostSpec>, ConnectOptions)>,
    /// Passwords the user asked to save: written to the keychain only once
    /// the connection succeeds. (keychain account, password)
    pub(crate) save_passwords: Vec<(String, Zeroizing<String>)>,
}

impl Tab {
    pub fn is_local(&self) -> bool {
        self.spec.is_none()
    }
    pub fn display_name(&self) -> &str {
        match &self.custom_name {
            Some(n) => n,
            None if self.title.is_empty() => &self.name,
            None => &self.title,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaletteItem {
    Host(usize),
    Direct(String),
    NewLocal,
    NewHost,
    Servers,
    Settings,
    About,
}

pub struct Palette {
    pub query: String,
    pub selected: usize,
}

/// Settings dialog (⌘,): an edited copy, applied on save.
pub struct SettingsDraft {
    pub s: Settings,
    pub scrollback: String,
    pub attach_lines: String,
    pub error: Option<String>,
}

fn cursor_shape(kind: CursorKind) -> coshell_term::alacritty_terminal::vte::ansi::CursorShape {
    use coshell_term::alacritty_terminal::vte::ansi::CursorShape;
    match kind {
        CursorKind::Block => CursorShape::Block,
        CursorKind::Beam => CursorShape::Beam,
        CursorKind::Underline => CursorShape::Underline,
    }
}

/// A whole number within `min..=max`.
fn number_in(label: &str, v: &str, min: usize, max: usize) -> Result<usize, String> {
    v.trim().parse::<usize>().ok().filter(|n| (min..=max).contains(n)).ok_or_else(|| tf("{0}: {1}–{2} 사이의 숫자를 입력하세요", &[&label, &min, &max]))
}

/// Scrollback search bar (⌘F) for the active tab.
#[derive(Default)]
pub struct Search {
    pub query: String,
    /// Result of the last lookup; `None` while the query is empty.
    pub found: Option<bool>,
}

pub struct CoShell {
    pub tabs: Vec<Tab>,
    pub active: Option<TabId>,
    next_id: TabId,
    pub hosts: Vec<HostEntry>,
    config: SshConfig,
    user: String,
    known_hosts: Option<PathBuf>,
    pub palette: Option<Palette>,
    pub notice: Option<String>,
    pub ai: Ai,
    /// Terminal focus requests: (generation, focused).
    pub term_focus: (u64, bool),
    pub tunnel_panel: bool,
    pub tunnel_form: TunnelForm,
    tunnel_seq: u64,
    pub host_store: HostStore,
    hosts_path: Option<PathBuf>,
    /// Saved server passwords (OS keychain, service [`PASSWORD_SERVICE`]).
    passwords: Arc<dyn SecretStore>,
    /// Raw ~/.ssh/config text (CoShell definitions are prepended to it).
    user_config: String,
    home: PathBuf,
    pub editor: Option<HostEditor>,
    /// Server list page shown instead of the terminal.
    pub host_view: bool,
    /// "About CoShell" dialog.
    pub about: bool,
    pub host_filter: String,
    /// Prod output waiting for the user's OK before it can go to the AI.
    pub attach_confirm: Option<(TabId, Attachment)>,
    pub settings: Settings,
    pub(crate) settings_path: Option<PathBuf>,
    pub settings_draft: Option<SettingsDraft>,
    /// Terminal font for `settings.font_family` (and the name it was built from).
    pub term_font: iced::Font,
    term_font_name: String,
    /// Remembered state changed since the last write.
    state_dirty: bool,
    pub search: Option<Search>,
    /// Connected SSH tab waiting for "close it?" confirmation.
    pub close_confirm: Option<TabId>,
    /// Tab being renamed and the text typed so far.
    pub rename: Option<(TabId, String)>,
    last_tab_click: Option<(TabId, std::time::Instant)>,
    /// The AI panel divider is being dragged.
    ai_resizing: bool,
}

pub struct HostEditor {
    pub original: Option<String>,
    pub def: HostDef,
    /// Settings shown read-only when the host comes from ~/.ssh/config.
    pub external: Option<HostEntry>,
    pub port: String,
    pub forward_input: String,
    /// "ssh -p 41 root@host" pasted by the user.
    pub quick: String,
    /// Jump host / key file / tunnels section expanded.
    pub advanced: bool,
    pub error: Option<String>,
    /// New password typed by the user (empty = keep as is).
    pub password: Zeroizing<String>,
    /// Keychain account of the saved password when the editor opened.
    pub saved_password: Option<String>,
    /// "Delete saved password" pressed.
    pub forget_password: bool,
    /// Delete was pressed once; the next press deletes.
    pub confirm_delete: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum HostField {
    Name,
    Group,
    Quick,
    Alias,
    HostName,
    User,
    Port,
    ProxyJump,
    Identity,
    Password,
}

#[derive(Debug, Clone)]
pub enum Message {
    Conn(TabId, ConnEvent),
    Term(TabId, TermEvent),
    Input(TabId, Vec<u8>),
    Resize(TabId, GridSize),
    Activate(TabId),
    Close(TabId),
    CloseConfirmed(TabId),
    CloseCancel,
    Retry(TabId),
    SearchQuery(String),
    /// Next match; `true` = towards newer output.
    SearchNext(bool),
    SearchClose,
    /// The app has launched: put up the native menu bar (macOS).
    InstallMenu,
    /// Menu: copy the terminal selection.
    TermCopy,
    /// Menu: paste the clipboard into the terminal.
    TermPaste,
    TermPasted(Option<String>),
    /// ⌘+click on a web link in the terminal.
    OpenLink(String),
    TabRename(String),
    TabRenameCommit,
    TabRenameCancel,
    NoticeDismiss,
    AiResizeStart,
    /// Cursor x while dragging the divider.
    AiResizeTo(f32),
    AiResizeEnd,
    NewLocal,
    OpenPalette,
    ClosePalette,
    PaletteQuery(String),
    PaletteMove(i32),
    PaletteSubmit,
    PalettePick(PaletteItem),
    HostKeyAnswer(TabId, HostKeyDecision),
    HostKeyConfirm(TabId, String),
    PromptInput(TabId, usize, String),
    PromptSave(TabId, bool),
    PromptSubmit(TabId),
    PromptCancel(TabId),
    Shortcut(Shortcut),
    SettingsOpen,
    SettingsClose,
    SettingsSave,
    SettingsLanguage(settings::Language),
    SettingsFontSize(i8),
    SettingsFontFamily(String),
    SettingsScheme(Scheme),
    SettingsCursor(CursorKind),
    SettingsScrollback(String),
    SettingsAttachLines(String),
    SettingsOptionMeta(bool),
    WindowResized(iced::Size),
    PersistState,
    /// Drag the window from an empty spot of the tab bar.
    WindowDrag,
    /// Double-click on the tab bar: zoom like a native title bar.
    WindowZoom,
    TunnelPanel,
    TunnelKind(char),
    TunnelField(TunnelField, String),
    TunnelPreset(u16),
    TunnelPersist(bool),
    TunnelAdd,
    TunnelStarted(TabId, u64, Once<Result<Tunnel, String>>),
    TunnelStop(TabId, u64),
    TunnelStart(TabId, u64),
    TunnelRemove(TabId, u64),
    TunnelCopy(String),
    TunnelOpen(u16, bool),
    Tick,
    AttachConfirm,
    AttachCancel,
    HostEdit(String),
    HostView,
    HostConnect(String),
    HostNewIn(Option<String>),
    HostQuickApply,
    HostAdvanced,
    HostGroupPick(String),
    HostFilter(String),
    HostEditorClose,
    HostField(HostField, String),
    HostEnv(Env),
    HostForwardInput(String),
    HostForwardAdd,
    HostForwardRemove(usize),
    HostSave(bool),
    HostDelete,
    HostDeleteAsk,
    HostForgetPassword,
    AiToggle,
    About,
    AiProfile(String),
    AiDraft(TabId, text_editor::Action),
    AiSend(TabId),
    AiStop(TabId),
    AiEvent(TabId, Result<StreamEvent, AiError>),
    AiAttach(TabId),
    AiDetach(TabId),
    AiNewChat(TabId),
    AiRetry(TabId),
    AiSuggest(TabId, String, bool),
    AiCopy(String),
    AiInsert(TabId, String),
    AiSettingsOpen,
    AiSettingsClose,
    AiSettingsSelect(usize),
    AiSettingsName(String),
    AiSettingsFormat(Format),
    AiSettingsBase(String),
    AiSettingsModel(String),
    AiSettingsKey(String),
    AiSettingsSaveKey,
    AiSettingsDeleteKey,
    AiSettingsFetch(bool),
    AiModels(bool, Result<Vec<String>, String>),
    AiSettingsAdd,
    AiSettingsRemove,
    AiSettingsUse,
    AiSettingsSave,
}

#[derive(Debug, Clone, Copy)]
pub enum Shortcut {
    Palette,
    NewLocal,
    CloseTab,
    Nth(usize),
    AiPanel,
    Servers,
    Settings,
    /// Tab / Shift+Tab inside a dialog; `true` = backwards.
    FocusStep(bool),
    /// Next (+1) / previous (−1) tab.
    TabStep(i32),
    /// Move the active tab right (+1) / left (−1).
    TabMove(i32),
    Find,
    /// `true` = towards newer output.
    FindNext(bool),
    /// Font size step; 0 resets.
    Zoom(i8),
    Escape,
    Up,
    Down,
}

fn app_opts(known_hosts: PathBuf) -> ConnectOptions {
    ConnectOptions { policy: HostKeyPolicy::Ask, ..ConnectOptions::new(known_hosts) }
}

impl CoShell {
    pub fn boot(launch: Launch, prefs: Settings, settings_path: Option<PathBuf>, prefs_warning: Option<String>) -> (Self, Task<Message>) {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let mut warnings: Vec<String> = prefs_warning.into_iter().collect();
        let user_config = match std::fs::read_to_string(home.join(".ssh").join("config")) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => {
                warnings.push(tf("~/.ssh/config 읽기 실패: {0}", &[&e]));
                String::new()
            }
        };
        let hosts_path = HostStore::default_path();
        let store = match hosts_path.as_deref().map(HostStore::load) {
            Some(Ok(s)) => s,
            Some(Err(e)) => {
                warnings.push(tf("hosts.toml 읽기 실패: {0}", &[&e]));
                HostStore::default()
            }
            None => HostStore::default(),
        };
        let (ai, ai_warn) = Ai::load();
        warnings.extend(ai_warn);
        let mut app = Self::with_parts(
            home,
            user_config,
            store,
            hosts_path,
            ai,
            std::env::var("USER").unwrap_or_else(|_| "root".into()),
            ConnectOptions::default_known_hosts(),
            Arc::new(Keychain(PASSWORD_SERVICE)),
        );
        app.notice = (!warnings.is_empty()).then(|| warnings.join(" · "));
        app.ai.open = prefs.ai_panel_open;
        app.settings = prefs;
        app.settings_path = settings_path;
        app.apply_settings();
        let task = match launch {
            Launch::Local => app.new_local(),
            Launch::Alias(name) => app.connect_alias(&name),
            Launch::Ssh { chain, opts } => {
                let name = chain.last().map(HostSpec::label).unwrap_or_default();
                app.open_ssh(name, chain, opts, Vec::new(), Env::None)
            }
        };
        (app, Task::batch([task, Task::done(Message::InstallMenu)]))
    }

    /// State without touching user files (also used by tests).
    #[allow(clippy::too_many_arguments)]
    pub fn with_parts(
        home: PathBuf,
        user_config: String,
        host_store: HostStore,
        hosts_path: Option<PathBuf>,
        ai: Ai,
        user: String,
        known_hosts: Option<PathBuf>,
        passwords: Arc<dyn SecretStore>,
    ) -> Self {
        let config = SshConfig::parse("", &home);
        let mut app = Self {
            tabs: Vec::new(),
            active: None,
            next_id: 1,
            hosts: Vec::new(),
            config,
            user,
            known_hosts,
            palette: None,
            notice: None,
            ai,
            term_focus: (0, true),
            tunnel_panel: false,
            tunnel_form: TunnelForm::default(),
            tunnel_seq: 0,
            host_store,
            hosts_path,
            passwords,
            user_config,
            home,
            editor: None,
            host_view: false,
            about: false,
            host_filter: String::new(),
            attach_confirm: None,
            settings: Settings::default(),
            settings_path: None,
            settings_draft: None,
            term_font: iced::Font::MONOSPACE,
            term_font_name: String::new(),
            state_dirty: false,
            search: None,
            close_confirm: None,
            rename: None,
            last_tab_click: None,
            ai_resizing: false,
        };
        app.rebuild_config();
        app
    }

    /// CoShell definitions first (first match wins), then ~/.ssh/config.
    fn rebuild_config(&mut self) {
        let text = format!("{}\n{}", self.host_store.to_ssh_config(), self.user_config);
        self.config = SshConfig::parse(&text, &self.home);
        self.hosts = self.config.entries();
    }

    fn user_config_entry(&self, alias: &str) -> Option<HostEntry> {
        let user = SshConfig::parse(&self.user_config, &self.home);
        user.aliases().iter().any(|a| a == alias).then(|| user.entry(alias))
    }

    pub fn env_of(&self, alias: &str) -> Env {
        self.host_store.env_of(alias)
    }

    /// Display name for a host alias (CoShell name, else the alias).
    pub fn display_of(&self, alias: &str) -> String {
        self.host_store.get(alias).map_or(alias, HostDef::display).to_string()
    }

    fn connect_alias(&mut self, alias: &str) -> Task<Message> {
        self.host_view = false;
        let display = self.display_of(alias);
        self.open_target(alias, display)
    }

    /// Existing groups (store order), for suggestions.
    pub fn groups(&self) -> Vec<String> {
        let mut g: Vec<String> = Vec::new();
        for h in &self.host_store.hosts {
            if let Some(x) = &h.group
                && !g.contains(x)
            {
                g.push(x.clone());
            }
        }
        g
    }

    /// An SSH tab stuck before connecting, with a route (tests).
    #[cfg(test)]
    pub fn open_fake_ssh(&mut self, name: &str, route: &[&str], latencies_ms: &[u64]) -> TabId {
        let id = self.alloc_id();
        let host = HostSpec { host: name.into(), port: 22, user: "u".into(), auth: coshell_ssh::Auth::Interactive };
        self.tabs.push(Tab {
            id,
            name: name.into(),
            env: Env::Prod,
            phase: Phase::Connecting,
            route: route.iter().map(|s| s.to_string()).collect(),
            steps: latencies_ms.iter().map(|ms| Step::Done(Duration::from_millis(*ms))).collect(),
            title: String::new(),
            custom_name: None,
            exited: false,
            prompt: None,
            chat: Chat::default(),
            conn: None,
            tunnels: Vec::new(),
            alias: None,
            auto_forwards: Vec::new(),
            save_passwords: Vec::new(),
            spec: Some((vec![host], app_opts(PathBuf::from("/nonexistent")))),
        });
        self.active = Some(id);
        id
    }

    /// Make a tab count as a connected SSH session (tests).
    #[cfg(test)]
    pub fn mark_remote(&mut self, id: TabId) {
        let host = HostSpec { host: "h".into(), port: 22, user: "u".into(), auth: coshell_ssh::Auth::Interactive };
        if let Some(t) = self.tab_mut(id) {
            t.spec = Some((vec![host], app_opts(PathBuf::from("/nonexistent"))));
        }
    }

    /// Open a local shell tab (tests).
    #[cfg(test)]
    pub fn open_local(&mut self) -> TabId {
        let _ = self.new_local();
        self.active.expect("tab just opened")
    }

    pub fn title(&self) -> String {
        match self.active_tab() {
            Some(t) => format!("CoShell — {}", t.display_name()),
            None => "CoShell".into(),
        }
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.iter().find(|t| Some(t.id) == self.active)
    }

    fn tab_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    fn alloc_id(&mut self) -> TabId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Terminal accepts keys only with no overlay on top.
    pub fn terminal_active(&self, tab: &Tab) -> bool {
        self.palette.is_none()
            && tab.prompt.is_none()
            && self.ai.settings.is_none()
            && !self.tunnel_panel
            && self.editor.is_none()
            && self.attach_confirm.is_none()
            && !self.host_view
            && !self.about
            && self.search.is_none()
            && self.close_confirm.is_none()
            && self.settings_draft.is_none()
            && self.rename.is_none()
    }

    /// A dialog with several fields is on screen.
    pub fn form_open(&self) -> bool {
        self.editor.is_some()
            || self.settings_draft.is_some()
            || self.ai.settings.is_some()
            || self.tunnel_panel
            || self.active_tab().is_some_and(|t| t.prompt.is_some())
    }

    fn active_index(&self) -> Option<usize> {
        self.tabs.iter().position(|t| Some(t.id) == self.active)
    }

    /// Push the current settings into the font and every open session.
    fn apply_settings(&mut self) {
        // The language is process-wide; tests run in parallel and stay on the source language.
        #[cfg(not(test))]
        crate::i18n::set(crate::i18n::resolve(self.settings.language));
        if self.settings.font_family != self.term_font_name {
            self.term_font_name = self.settings.font_family.clone();
            self.term_font = if self.term_font_name.is_empty() {
                iced::Font::MONOSPACE
            } else {
                // iced wants a 'static family name; this leaks one short string per font change.
                iced::Font::with_name(Box::leak(self.term_font_name.clone().into_boxed_str()))
            };
        }
        for tab in &self.tabs {
            if let Phase::Live(s) = &tab.phase {
                self.configure_session(s);
            }
        }
    }

    fn configure_session(&self, s: &Session) {
        s.configure(self.settings.scrollback, cursor_shape(self.settings.cursor));
    }

    fn persist_settings(&mut self) {
        self.state_dirty = false;
        if let Some(p) = &self.settings_path
            && let Err(e) = self.settings.save(p)
        {
            self.notice = Some(tf("설정 저장 실패: {0}", &[&e]));
        }
    }

    fn open_settings(&mut self) {
        self.palette = None;
        self.settings_draft = Some(SettingsDraft {
            s: self.settings.clone(),
            scrollback: self.settings.scrollback.to_string(),
            attach_lines: self.settings.ai_attach_lines.to_string(),
            error: None,
        });
    }

    fn save_settings(&mut self) {
        let Some(d) = &mut self.settings_draft else { return };
        let parsed = number_in(tr("스크롤백 줄 수"), &d.scrollback, settings::SCROLLBACK_MIN, settings::SCROLLBACK_MAX).and_then(|sb| {
            number_in(tr("AI 첨부 줄 수"), &d.attach_lines, settings::ATTACH_LINES_MIN, settings::ATTACH_LINES_MAX).map(|al| (sb, al))
        });
        let (scrollback, attach_lines) = match parsed {
            Ok(v) => v,
            Err(e) => {
                d.error = Some(e);
                return;
            }
        };
        // Remembered state is not edited in the dialog; keep what is current.
        let next = Settings {
            scrollback,
            ai_attach_lines: attach_lines,
            window: self.settings.window,
            ai_panel_open: self.settings.ai_panel_open,
            ..d.s.clone()
        };
        self.settings = next.sanitize();
        self.settings_draft = None;
        self.apply_settings();
        self.persist_settings();
    }

    fn update_settings(&mut self, m: Message) {
        let Some(d) = &mut self.settings_draft else { return };
        d.error = None;
        match m {
            Message::SettingsFontSize(step) => {
                d.s.font_size = (d.s.font_size + f32::from(step)).clamp(settings::FONT_SIZE_MIN, settings::FONT_SIZE_MAX)
            }
            Message::SettingsLanguage(v) => d.s.language = v,
            Message::SettingsFontFamily(v) => d.s.font_family = v,
            Message::SettingsScheme(v) => d.s.scheme = v,
            Message::SettingsCursor(v) => d.s.cursor = v,
            Message::SettingsScrollback(v) => d.scrollback = v,
            Message::SettingsAttachLines(v) => d.attach_lines = v,
            Message::SettingsOptionMeta(v) => d.s.option_as_meta = v,
            _ => {}
        }
    }

    fn live_session(&self) -> Option<&Session> {
        match self.active_tab() {
            Some(Tab { phase: Phase::Live(s), .. }) => Some(s),
            _ => None,
        }
    }

    /// Look up the search query in the active tab's scrollback.
    fn run_search(&mut self, forward: bool) {
        let Some(query) = self.search.as_ref().map(|s| s.query.clone()) else { return };
        // Literal search: the query is not a regex for the user.
        let found = match self.live_session() {
            Some(s) if !query.is_empty() => Some(s.search(&regex::escape(&query), forward)),
            _ => None,
        };
        if let Some(s) = &mut self.search {
            s.found = found;
        }
    }

    /// A connected SSH session: closing it kills remote work and tunnels.
    fn needs_close_confirm(&self, id: TabId) -> bool {
        self.tabs.iter().any(|t| t.id == id && !t.is_local() && !t.exited && matches!(t.phase, Phase::Live(_)))
    }

    /// Start a fresh shell in an exited local tab.
    fn restart_local(&mut self, id: TabId) -> Task<Message> {
        let (phase, task) = match Session::spawn_local(GridSize::DEFAULT) {
            Ok(s) => {
                self.configure_session(&s);
                live(id, s)
            }
            Err(e) => (Phase::Failed(tf("셸을 시작하지 못했습니다: {0}", &[&e])), Task::none()),
        };
        if let Some(tab) = self.tab_mut(id) {
            tab.phase = phase;
            tab.exited = false;
            tab.title.clear();
        }
        task
    }

    fn focus_terminal(&mut self) -> Task<Message> {
        self.term_focus = (self.term_focus.0 + 1, true);
        iced::advanced::widget::operate(iced::advanced::widget::operation::focusable::unfocus())
    }

    fn focus_ai(&mut self) -> Task<Message> {
        self.term_focus = (self.term_focus.0 + 1, false);
        iced::widget::operation::focus(AI_INPUT)
    }

    /// Attach the active tab's recent output (masked) to its chat.
    fn attach_output(&mut self, id: TabId) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else { return };
        let prod = tab.env == Env::Prod;
        let Phase::Live(s) = &tab.phase else { return };
        let raw = s.recent_text(self.settings.ai_attach_lines);
        if raw.trim().is_empty() {
            self.notice = Some(tr("첨부할 터미널 출력이 없습니다").into());
            return;
        }
        let (masked, secrets) = coshell_ai::redact::mask(&raw);
        let lines = masked.lines().count();
        let a = Attachment { label: tf("최근 출력 {0}줄", &[&lines]), masked, secrets };
        if prod {
            // Production output needs an explicit look before it can leave the machine.
            self.attach_confirm = Some((id, a));
        } else {
            tab.chat.attachment = Some(a);
        }
    }

    fn ai_send(&mut self, id: TabId) -> Task<Message> {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else { return Task::none() };
        let target = crate::ui_ai::target_label(tab);
        self.ai.send(id, &mut tab.chat, &target)
    }

    fn new_local(&mut self) -> Task<Message> {
        self.host_view = false;
        let id = self.alloc_id();
        let (phase, task) = match Session::spawn_local(GridSize::DEFAULT) {
            Ok(s) => {
                self.configure_session(&s);
                live(id, s)
            }
            Err(e) => (Phase::Failed(tf("셸을 시작하지 못했습니다: {0}", &[&e])), Task::none()),
        };
        self.tabs.push(Tab {
            id,
            name: "zsh".into(),
            env: Env::None,
            phase,
            route: vec![],
            steps: vec![],
            title: String::new(),
            custom_name: None,
            exited: false,
            prompt: None,
            chat: Chat::default(),
            conn: None,
            tunnels: Vec::new(),
            alias: None,
            auto_forwards: Vec::new(),
            save_passwords: Vec::new(),
            spec: None,
        });
        self.active = Some(id);
        task
    }

    fn open_ssh(
        &mut self,
        name: String,
        chain: Vec<HostSpec>,
        opts: ConnectOptions,
        forwards: Vec<ForwardSpec>,
        alias_env: Env,
    ) -> Task<Message> {
        let id = self.alloc_id();
        let env = alias_env;
        self.tabs.push(Tab {
            id,
            name,
            env,
            phase: Phase::Connecting,
            route: vec![],
            steps: vec![],
            title: String::new(),
            custom_name: None,
            exited: false,
            prompt: None,
            chat: Chat::default(),
            conn: None,
            tunnels: Vec::new(),
            alias: None,
            auto_forwards: forwards,
            save_passwords: Vec::new(),
            spec: Some((chain, opts)),
        });
        self.active = Some(id);
        self.start_connect(id)
    }

    fn start_connect(&mut self, id: TabId) -> Task<Message> {
        let Some(tab) = self.tab_mut(id) else { return Task::none() };
        let Some((chain, opts)) = tab.spec.clone() else { return Task::none() };
        tab.phase = Phase::Connecting;
        tab.route = chain.iter().map(HostSpec::label).collect();
        tab.steps = vec![Step::Wait; chain.len()];
        tab.exited = false;
        tab.prompt = None;
        tab.conn = None;
        tab.tunnels.clear();
        tab.save_passwords.clear();
        let (events, driver) = conn::connect(chain, opts, self.passwords.clone());
        Task::batch([Task::run(events, move |e| Message::Conn(id, e)), Task::future(driver).discard()])
    }

    fn open_target(&mut self, name: &str, display: String) -> Task<Message> {
        let Some(kh) = self.known_hosts.clone() else {
            self.notice = Some(tr("HOME을 알 수 없어 known_hosts를 사용할 수 없습니다").into());
            return Task::none();
        };
        match self.config.resolve(name, &self.user) {
            Ok(chain) => {
                let forwards = self.config.entry(name).forwards;
                let env = self.host_store.env_of(name);
                let task = self.open_ssh(display, chain, app_opts(kh), forwards, env);
                if let Some(t) = self.tabs.last_mut() {
                    t.alias = Some(name.to_string());
                }
                task
            }
            Err(e) => {
                self.notice = Some(e);
                Task::none()
            }
        }
    }

    // ------------------------------------------------------------ palette

    pub fn palette_items(&self) -> Vec<PaletteItem> {
        let q = self.palette.as_ref().map(|p| p.query.trim().to_lowercase()).unwrap_or_default();
        let mut items: Vec<PaletteItem> = self
            .hosts
            .iter()
            .enumerate()
            .filter(|(_, h)| {
                q.is_empty()
                    || h.alias.to_lowercase().contains(&q)
                    || self.display_of(&h.alias).to_lowercase().contains(&q)
                    || h.host().to_lowercase().contains(&q)
                    || h.user.as_deref().is_some_and(|u| u.to_lowercase().contains(&q))
            })
            .map(|(i, _)| PaletteItem::Host(i))
            .collect();
        let exact = self.hosts.iter().any(|h| h.alias.eq_ignore_ascii_case(&q));
        if !q.is_empty() && !exact && !q.contains(char::is_whitespace) {
            items.push(PaletteItem::Direct(q.clone()));
        }
        if q.is_empty() || "새 로컬 셸 new local shell".contains(&q) {
            items.push(PaletteItem::NewLocal);
        }
        if q.is_empty() || "새 서버 추가 new server host".contains(&q) {
            items.push(PaletteItem::NewHost);
        }
        if q.is_empty() || "서버 관리 servers".contains(&q) {
            items.push(PaletteItem::Servers);
        }
        if q.is_empty() || "설정 환경설정 settings preferences".contains(&q) {
            items.push(PaletteItem::Settings);
        }
        if q.is_empty() || "coshell 정보 버전 about version".contains(&q) {
            items.push(PaletteItem::About);
        }
        items
    }

    fn pick(&mut self, item: PaletteItem) -> Task<Message> {
        self.palette = None;
        match item {
            PaletteItem::NewLocal => self.new_local(),
            PaletteItem::NewHost => self.open_editor(None),
            PaletteItem::Servers => {
                self.host_view = true;
                Task::none()
            }
            PaletteItem::Settings => {
                self.open_settings();
                Task::none()
            }
            PaletteItem::About => {
                self.about = true;
                Task::none()
            }
            PaletteItem::Host(i) => {
                let Some(alias) = self.hosts.get(i).map(|h| h.alias.clone()) else { return Task::none() };
                self.connect_alias(&alias)
            }
            PaletteItem::Direct(q) => self.open_target(&q, q.clone()),
        }
    }

    // ------------------------------------------------------------ update

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Conn(id, ev) => self.on_conn(id, ev),
            Message::Term(id, ev) => {
                if let Some(tab) = self.tab_mut(id) {
                    match ev {
                        TermEvent::Wakeup => {
                            if let Phase::Live(s) = &tab.phase {
                                s.clear_dirty();
                            }
                        }
                        TermEvent::Title(t) => tab.title = t,
                        TermEvent::Bell => {}
                        TermEvent::Exited => {
                            tab.exited = true;
                            // The connection is gone with the shell; so are its forwards.
                            tab.tunnels.clear();
                            tab.conn = None;
                        }
                    }
                }
                Task::none()
            }
            Message::Input(id, bytes) => {
                if let Some(Tab { phase: Phase::Live(s), exited: false, .. }) = self.tab_mut(id) {
                    s.write(&bytes);
                }
                Task::none()
            }
            Message::Resize(id, size) => {
                if let Some(Tab { phase: Phase::Live(s), .. }) = self.tab_mut(id) {
                    s.resize(size);
                }
                Task::none()
            }
            Message::Activate(id) => {
                // Clicks inside the rename field also press the tab: keep editing.
                if self.rename.as_ref().is_some_and(|(t, _)| *t == id) {
                    return Task::none();
                }
                let now = std::time::Instant::now();
                let again = self.active == Some(id)
                    && self.last_tab_click.is_some_and(|(t, at)| t == id && now.duration_since(at) < TAB_DOUBLE_CLICK);
                self.last_tab_click = Some((id, now));
                self.active = Some(id);
                if again && let Some(name) = self.active_tab().map(|t| t.display_name().to_string()) {
                    self.rename = Some((id, name));
                    return Task::batch([iced::widget::operation::focus(RENAME_INPUT), iced::widget::operation::select_all(RENAME_INPUT)]);
                }
                self.rename = None;
                self.focus_prompt()
            }
            Message::TabRename(text) => {
                if let Some((_, name)) = &mut self.rename {
                    *name = text;
                }
                Task::none()
            }
            Message::TabRenameCommit => {
                if let Some((id, name)) = self.rename.take() {
                    let name: String = name.trim().chars().filter(|c| !c.is_control()).take(TAB_NAME_MAX_CHARS).collect();
                    if let Some(tab) = self.tab_mut(id) {
                        // An empty name goes back to the automatic one.
                        tab.custom_name = (!name.is_empty()).then_some(name);
                    }
                }
                self.focus_terminal()
            }
            Message::TabRenameCancel => {
                self.rename = None;
                self.focus_terminal()
            }
            Message::InstallMenu => {
                // Not in `boot`: winit installs its default menu when launching finishes, after boot.
                #[cfg(target_os = "macos")]
                if let Err(e) = crate::menu::install() {
                    self.notice = Some(tf("메뉴 막대를 만들지 못했습니다: {0}", &[&e]));
                }
                Task::none()
            }
            Message::TermCopy => match self.live_session().and_then(Session::selection_text) {
                Some(text) => iced::clipboard::write(text),
                None => Task::none(),
            },
            Message::TermPaste => iced::clipboard::read().map(Message::TermPasted),
            Message::TermPasted(text) => {
                // Only when the terminal owns the keyboard: a dialog's field must not leak into the shell.
                if let (Some(text), Some(tab)) = (text, self.active_tab())
                    && self.terminal_active(tab)
                    && !tab.exited
                    && let Phase::Live(s) = &tab.phase
                {
                    let bracketed = s.term().lock().mode().contains(coshell_term::alacritty_terminal::term::TermMode::BRACKETED_PASTE);
                    s.write(&crate::terminal::paste_bytes(&text, bracketed));
                }
                Task::none()
            }
            Message::OpenLink(url) => {
                // Terminal output is untrusted: web links only, nothing a launcher could misread.
                let web = url.starts_with("http://") || url.starts_with("https://");
                if !web || url.chars().any(|c| c.is_control() || c.is_whitespace()) {
                    self.notice = Some(tr("열 수 없는 링크입니다").into());
                } else if let Err(e) = open_url(&url) {
                    self.notice = Some(tf("브라우저를 열 수 없습니다: {0}", &[&e]));
                }
                Task::none()
            }
            Message::NoticeDismiss => {
                self.notice = None;
                Task::none()
            }
            Message::AiResizeStart => {
                self.ai_resizing = true;
                Task::none()
            }
            Message::AiResizeTo(x) => {
                let room = (self.settings.window.width - TERMINAL_MIN_W).max(settings::AI_PANEL_W_MIN);
                let max = settings::AI_PANEL_W_MAX.min(room);
                self.settings.ai_panel_width = (self.settings.window.width - x).clamp(settings::AI_PANEL_W_MIN, max);
                self.state_dirty = true;
                Task::none()
            }
            Message::AiResizeEnd => {
                self.ai_resizing = false;
                Task::none()
            }
            Message::Close(id) => {
                if self.needs_close_confirm(id) {
                    self.close_confirm = Some(id);
                } else {
                    self.close(id);
                }
                Task::none()
            }
            Message::CloseConfirmed(id) => {
                self.close_confirm = None;
                self.close(id);
                Task::none()
            }
            Message::CloseCancel => {
                self.close_confirm = None;
                Task::none()
            }
            Message::Retry(id) => {
                if self.tabs.iter().any(|t| t.id == id && t.is_local()) {
                    self.restart_local(id)
                } else {
                    self.start_connect(id)
                }
            }
            Message::SearchQuery(q) => {
                if let Some(s) = &mut self.search {
                    s.query = q;
                }
                // A new query starts over from the bottom of the view.
                if let Some(s) = self.live_session() {
                    s.clear_selection();
                }
                self.run_search(false);
                Task::none()
            }
            Message::SearchNext(forward) => {
                self.run_search(forward);
                Task::none()
            }
            Message::SearchClose => {
                self.search = None;
                self.focus_terminal()
            }
            Message::NewLocal => self.new_local(),
            Message::OpenPalette => {
                self.palette = Some(Palette { query: String::new(), selected: 0 });
                iced::widget::operation::focus(PALETTE_INPUT)
            }
            Message::ClosePalette => {
                self.palette = None;
                self.focus_prompt()
            }
            Message::PaletteQuery(q) => {
                if let Some(p) = &mut self.palette {
                    p.query = q;
                    p.selected = 0;
                }
                Task::none()
            }
            Message::PaletteMove(d) => {
                let n = self.palette_items().len();
                if let Some(p) = &mut self.palette
                    && n > 0
                {
                    p.selected = (p.selected as i32 + d).rem_euclid(n as i32) as usize;
                }
                Task::none()
            }
            Message::PaletteSubmit => {
                let sel = self.palette.as_ref().map_or(0, |p| p.selected);
                match self.palette_items().get(sel).cloned() {
                    Some(item) => self.pick(item),
                    None => Task::none(),
                }
            }
            Message::PalettePick(item) => self.pick(item),
            Message::HostKeyAnswer(id, d) => {
                if let Some(tab) = self.tab_mut(id)
                    && let Some(Pending::HostKey { reply, query, confirm }) = tab.prompt.take()
                {
                    // Replacing a changed key requires typing the host name.
                    let allowed = d != HostKeyDecision::ReplaceAndSave || confirm == query.host;
                    let _ = reply.send(if allowed { d } else { HostKeyDecision::Reject });
                }
                Task::none()
            }
            Message::HostKeyConfirm(id, text) => {
                if let Some(Tab { prompt: Some(Pending::HostKey { confirm, .. }), .. }) = self.tab_mut(id) {
                    *confirm = text;
                }
                Task::none()
            }
            Message::PromptInput(id, i, text) => {
                if let Some(Tab { prompt: Some(Pending::Interactive { answers, .. }), .. }) = self.tab_mut(id)
                    && let Some(a) = answers.get_mut(i)
                {
                    *a = Zeroizing::new(text);
                }
                Task::none()
            }
            Message::PromptSave(id, on) => {
                if let Some(Tab { prompt: Some(Pending::Interactive { save, .. }), .. }) = self.tab_mut(id) {
                    *save = on;
                }
                Task::none()
            }
            Message::PromptSubmit(id) => {
                if let Some(tab) = self.tab_mut(id)
                    && let Some(Pending::Interactive { query, reply, answers, save }) = tab.prompt.take()
                {
                    let hop = tab.spec.as_ref().and_then(|(chain, _)| chain.get(query.hop));
                    if let (true, true, Some(h), Some(pw)) = (save, query.password, hop, answers.first()) {
                        let key = conn::password_key(h);
                        tab.save_passwords.retain(|(k, _)| *k != key);
                        tab.save_passwords.push((key, pw.clone()));
                    }
                    let _ = reply.send(Some(answers));
                }
                Task::none()
            }
            Message::PromptCancel(id) => {
                if let Some(tab) = self.tab_mut(id) {
                    match tab.prompt.take() {
                        Some(Pending::Interactive { reply, .. }) => {
                            let _ = reply.send(None);
                        }
                        Some(Pending::HostKey { reply, .. }) => {
                            let _ = reply.send(HostKeyDecision::Reject);
                        }
                        None => {}
                    }
                }
                Task::none()
            }
            Message::Shortcut(s) => self.on_shortcut(s),
            Message::SettingsOpen => {
                self.open_settings();
                Task::none()
            }
            Message::SettingsClose => {
                self.settings_draft = None;
                self.focus_terminal()
            }
            Message::SettingsSave => {
                self.save_settings();
                if self.settings_draft.is_none() { self.focus_terminal() } else { Task::none() }
            }
            m @ (Message::SettingsLanguage(_)
            | Message::SettingsFontSize(_)
            | Message::SettingsFontFamily(_)
            | Message::SettingsScheme(_)
            | Message::SettingsCursor(_)
            | Message::SettingsScrollback(_)
            | Message::SettingsAttachLines(_)
            | Message::SettingsOptionMeta(_)) => {
                self.update_settings(m);
                Task::none()
            }
            Message::WindowResized(size) => {
                if (size.width, size.height) != (self.settings.window.width, self.settings.window.height) {
                    self.settings.window = settings::WindowSize { width: size.width, height: size.height };
                    self.state_dirty = true;
                }
                Task::none()
            }
            Message::PersistState => {
                self.persist_settings();
                Task::none()
            }
            Message::WindowDrag => iced::window::latest().and_then(iced::window::drag),
            Message::WindowZoom => iced::window::latest().and_then(iced::window::toggle_maximize),
            Message::TunnelPanel => {
                self.tunnel_panel = !self.tunnel_panel;
                self.tunnel_form.error = None;
                if self.tunnel_panel { Task::none() } else { self.focus_terminal() }
            }
            Message::TunnelKind(k) => {
                self.tunnel_form = TunnelForm { kind: k, persist: self.tunnel_form.persist, ..TunnelForm::default() };
                Task::none()
            }
            Message::TunnelField(f, v) => {
                let form = &mut self.tunnel_form;
                match f {
                    TunnelField::RemoteHost => form.remote_host = v,
                    TunnelField::RemotePort => form.remote_port = v,
                    TunnelField::LocalPort => form.local_port = v,
                }
                form.error = None;
                Task::none()
            }
            Message::TunnelPreset(p) => {
                let form = &mut self.tunnel_form;
                if form.kind == 'R' {
                    form.local_port = p.to_string();
                } else {
                    form.remote_port = p.to_string();
                }
                form.error = None;
                Task::none()
            }
            Message::TunnelPersist(on) => {
                self.tunnel_form.persist = on;
                Task::none()
            }
            Message::TunnelAdd => self.add_tunnel(),
            Message::TunnelStarted(id, entry, result) => {
                if let (Some(tab), Some(r)) = (self.tabs.iter_mut().find(|t| t.id == id), result.take())
                    && let Some(e) = tab.tunnels.iter_mut().find(|e| e.id == entry && matches!(e.state, TunnelState::Starting))
                {
                    e.state = match r {
                        Ok(t) => TunnelState::Running(t),
                        Err(err) => TunnelState::Failed(err),
                    };
                }
                Task::none()
            }
            Message::TunnelStop(id, entry) => {
                let Some(e) = self.tunnel_mut(id, entry) else { return Task::none() };
                match std::mem::replace(&mut e.state, TunnelState::Stopped) {
                    TunnelState::Running(t) => Task::future(t.stop()).discard(),
                    _ => Task::none(),
                }
            }
            Message::TunnelStart(id, entry) => self.launch_tunnel(id, entry),
            Message::TunnelRemove(id, entry) => {
                let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else { return Task::none() };
                let Some(i) = tab.tunnels.iter().position(|e| e.id == entry) else { return Task::none() };
                match tab.tunnels.remove(i).state {
                    TunnelState::Running(t) => Task::future(t.stop()).discard(),
                    _ => Task::none(),
                }
            }
            Message::TunnelCopy(text) => {
                self.notice = Some(tf("{0} 복사됨", &[&text]));
                iced::clipboard::write(text)
            }
            Message::TunnelOpen(port, https) => {
                // URL is built here from a port number only; nothing user-typed reaches the shell.
                let url = format!("{}://localhost:{port}", if https { "https" } else { "http" });
                if let Err(e) = open_url(&url) {
                    self.notice = Some(tf("브라우저를 열 수 없습니다: {0}", &[&e]));
                }
                Task::none()
            }
            Message::Tick => Task::none(),
            Message::About => {
                self.about = !self.about;
                Task::none()
            }
            Message::HostEdit(a) => self.open_editor(Some(a)),
            Message::HostView => {
                self.host_view = !self.host_view;
                if self.host_view { Task::none() } else { self.focus_terminal() }
            }
            Message::HostConnect(a) => self.connect_alias(&a),
            Message::HostNewIn(group) => {
                let t = self.open_editor(None);
                if let Some(ed) = &mut self.editor {
                    ed.def.group = group;
                }
                t
            }
            Message::HostFilter(f) => {
                self.host_filter = f;
                Task::none()
            }
            m @ (Message::HostEditorClose
            | Message::HostField(..)
            | Message::HostEnv(_)
            | Message::HostForwardInput(_)
            | Message::HostForwardAdd
            | Message::HostForwardRemove(_)
            | Message::HostSave(_)
            | Message::HostQuickApply
            | Message::HostAdvanced
            | Message::HostGroupPick(_)
            | Message::HostForgetPassword
            | Message::HostDeleteAsk
            | Message::HostDelete) => self.update_editor(m),
            m => self.update_ai(m),
        }
    }

    fn open_editor(&mut self, alias: Option<String>) -> Task<Message> {
        self.palette = None;
        let external = alias.as_deref().and_then(|a| self.user_config_entry(a));
        let def = match &alias {
            Some(a) => self.host_store.get(a).cloned().unwrap_or(HostDef { alias: a.clone(), ..HostDef::default() }),
            None => HostDef::default(),
        };
        let port = def.port.map(|p| p.to_string()).unwrap_or_default();
        let advanced = def.proxy_jump.is_some() || def.identity_file.is_some() || !def.forwards.is_empty();
        let saved_password =
            alias.as_deref().and_then(|a| self.password_key_of(a)).filter(|k| self.passwords.get(k).is_some());
        self.editor = Some(HostEditor {
            original: alias,
            def,
            external,
            port,
            forward_input: String::new(),
            quick: String::new(),
            advanced,
            error: None,
            password: Zeroizing::default(),
            saved_password,
            forget_password: false,
            confirm_delete: false,
        });
        Task::none()
    }

    fn update_editor(&mut self, m: Message) -> Task<Message> {
        let Some(ed) = &mut self.editor else { return Task::none() };
        let opt = |v: String| (!v.trim().is_empty()).then(|| v.trim().to_string());
        match m {
            Message::HostEditorClose => self.editor = None,
            Message::HostEnv(e) => ed.def.env = e,
            Message::HostField(f, v) => match f {
                HostField::Name => ed.def.name = (!v.trim().is_empty()).then_some(v),
                HostField::Group => ed.def.group = (!v.trim().is_empty()).then_some(v),
                HostField::Quick => ed.quick = v,
                HostField::Alias => ed.def.alias = v.trim().to_string(),
                HostField::HostName => ed.def.hostname = opt(v),
                HostField::User => ed.def.user = opt(v),
                HostField::Port => ed.port = v,
                HostField::ProxyJump => ed.def.proxy_jump = opt(v),
                HostField::Identity => ed.def.identity_file = opt(v),
                HostField::Password => ed.password = Zeroizing::new(v),
            },
            Message::HostForgetPassword => {
                ed.forget_password = true;
                ed.password = Zeroizing::default();
            }
            Message::HostGroupPick(g) => ed.def.group = Some(g),
            Message::HostAdvanced => ed.advanced = !ed.advanced,
            Message::HostQuickApply => match parse_ssh_command(&ed.quick) {
                Ok(c) => {
                    if ed.def.name.is_none() && ed.original.is_none() {
                        ed.def.name = Some(c.host.clone());
                    }
                    ed.def.hostname = Some(c.host);
                    ed.def.user = c.user.or(ed.def.user.take());
                    ed.port = c.port.map(|p| p.to_string()).unwrap_or_default();
                    if c.identity_file.is_some() {
                        ed.def.identity_file = c.identity_file;
                    }
                    if c.proxy_jump.is_some() {
                        ed.def.proxy_jump = c.proxy_jump;
                    }
                    ed.quick.clear();
                    ed.error = None;
                }
                Err(e) => ed.error = Some(e),
            },
            Message::HostForwardInput(v) => ed.forward_input = v,
            Message::HostForwardAdd => match crate::hosts::parse_forward(ed.forward_input.trim()) {
                Ok(_) => {
                    ed.def.forwards.push(ed.forward_input.trim().to_string());
                    ed.forward_input.clear();
                    ed.error = None;
                }
                Err(e) => ed.error = Some(e),
            },
            Message::HostForwardRemove(i) if i < ed.def.forwards.len() => {
                ed.def.forwards.remove(i);
            }
            Message::HostDeleteAsk => ed.confirm_delete = true,
            Message::HostDelete => {
                if let Some(a) = ed.original.clone() {
                    let key = self.password_key_of(&a);
                    self.host_store.remove(&a);
                    self.editor = None;
                    let task = self.persist_hosts(None);
                    if let Some(k) = key {
                        self.drop_password_if_unused(&k);
                    }
                    return task;
                }
            }
            Message::HostSave(connect) => return self.save_editor(connect),
            _ => {}
        }
        Task::none()
    }

    fn save_editor(&mut self, connect: bool) -> Task<Message> {
        let Some(mut ed) = self.editor.take() else { return Task::none() };
        match self.validated_def(&mut ed) {
            Ok(def) => {
                let alias = def.alias.clone();
                self.host_store.upsert(ed.original.as_deref(), def);
                // Before persist_hosts: "save and connect" must see the new password.
                self.rebuild_config();
                self.store_password(&ed, &alias);
                // Retag open tabs of this host.
                let env = self.host_store.env_of(&alias);
                let display = self.display_of(&alias);
                for t in self.tabs.iter_mut().filter(|t| t.name == alias || t.name == display) {
                    t.env = env;
                }
                self.persist_hosts(connect.then_some(alias))
            }
            Err(e) => {
                ed.error = Some(e);
                self.editor = Some(ed);
                Task::none()
            }
        }
    }

    /// Keychain account of the password for `alias` (its target server).
    fn password_key_of(&self, alias: &str) -> Option<String> {
        self.config.resolve(alias, &self.user).ok()?.last().map(conn::password_key)
    }

    /// Apply the editor's password changes for the (already saved) `alias`.
    /// A changed address/user/port moves the saved password along.
    fn store_password(&mut self, ed: &HostEditor, alias: &str) {
        let new_key = self.password_key_of(alias);
        let old_key = ed.saved_password.as_deref();
        let result = match (&new_key, old_key) {
            _ if ed.forget_password && ed.password.is_empty() => old_key.map_or(Ok(()), |k| self.passwords.delete(k)),
            (Some(new), _) if !ed.password.is_empty() => self.passwords.set(new, &ed.password),
            (Some(new), Some(old)) if new != old => match self.passwords.get(old) {
                Some(pw) => self.passwords.set(new, &pw),
                None => Ok(()),
            },
            (None, _) if !ed.password.is_empty() => Err(tr("서버 주소를 확인할 수 없습니다").into()),
            _ => Ok(()),
        };
        match result {
            Ok(()) => {
                if let Some(old) = old_key
                    && Some(old) != new_key.as_deref()
                {
                    self.drop_password_if_unused(old);
                }
            }
            Err(e) => self.notice = Some(tf("비밀번호를 키체인에 저장하지 못했습니다: {0}", &[&e])),
        }
    }

    /// Delete a saved password no remaining server points to.
    fn drop_password_if_unused(&mut self, key: &str) {
        let in_use = self.hosts.iter().any(|h| self.password_key_of(&h.alias).as_deref() == Some(key));
        if !in_use && let Err(e) = self.passwords.delete(key) {
            self.notice = Some(tf("키체인에서 비밀번호를 지우지 못했습니다: {0}", &[&e]));
        }
    }

    fn validated_def(&self, ed: &mut HostEditor) -> Result<HostDef, String> {
        let mut def = ed.def.clone();
        def.name = def.name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
        def.group = def.group.map(|g| g.trim().to_string()).filter(|g| !g.is_empty());
        if ed.external.is_some() {
            // ~/.ssh/config host: only the tag, name and group are ours.
            def = HostDef { alias: def.alias, env: def.env, name: def.name, group: def.group, ..HostDef::default() };
        } else {
            if def.alias.is_empty() {
                def.alias = self.unique_alias(def.name.as_deref().or(def.hostname.as_deref()).unwrap_or(""), ed.original.as_deref());
            }
            def.port = match ed.port.trim() {
                "" => None,
                p => Some(p.parse::<u16>().map_err(|_| tr("포트는 1–65535 사이의 숫자로 입력하세요").to_string())?),
            };
            if def.hostname.is_none() {
                return Err(tr("주소를 입력하세요").into());
            }
            if self.user_config_entry(&def.alias).is_some() {
                return Err(tr("~/.ssh/config에 같은 별칭이 있습니다. 고급 설정에서 다른 ssh 별칭을 입력하세요.").into());
            }
        }
        def.validate()?;
        Ok(def)
    }

    /// Slug of `base`, made unique among known aliases (`name`, `name-2`, …).
    fn unique_alias(&self, base: &str, original: Option<&str>) -> String {
        let slug = slugify(base);
        let slug = if slug.is_empty() { "host".to_string() } else { slug };
        let taken = |a: &str| Some(a) != original && (self.config.aliases().iter().any(|x| x == a) || self.host_store.get(a).is_some());
        (1..).map(|n| if n == 1 { slug.clone() } else { format!("{slug}-{n}") }).find(|a| !taken(a)).expect("unbounded")
    }

    fn persist_hosts(&mut self, connect: Option<String>) -> Task<Message> {
        match &self.hosts_path {
            Some(p) => {
                if let Err(e) = self.host_store.save(p) {
                    self.notice = Some(tf("hosts.toml 저장 실패: {0}", &[&e]));
                }
            }
            None => self.notice = Some(tr("HOME 폴더를 찾을 수 없어 서버를 저장하지 못했습니다").into()),
        }
        self.rebuild_config();
        match connect {
            Some(a) => self.connect_alias(&a),
            None => Task::none(),
        }
    }

    fn tunnel_mut(&mut self, tab: TabId, entry: u64) -> Option<&mut TunnelEntry> {
        self.tabs.iter_mut().find(|t| t.id == tab)?.tunnels.iter_mut().find(|e| e.id == entry)
    }

    /// Add a tunnel entry and start it.
    fn start_tunnel(&mut self, id: TabId, spec: ForwardSpec, auto_port: bool) -> Task<Message> {
        self.tunnel_seq += 1;
        let entry = self.tunnel_seq;
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else { return Task::none() };
        tab.tunnels.push(TunnelEntry { id: entry, spec, auto_port, state: TunnelState::Stopped });
        self.launch_tunnel(id, entry)
    }

    /// (Re)start an existing entry; a busy auto port falls back to a free one.
    fn launch_tunnel(&mut self, id: TabId, entry: u64) -> Task<Message> {
        let Some(conn) = self.tabs.iter().find(|t| t.id == id).and_then(|t| t.conn.clone()) else { return Task::none() };
        let Some(e) = self.tunnel_mut(id, entry) else { return Task::none() };
        if matches!(e.state, TunnelState::Running(_) | TunnelState::Starting) {
            return Task::none();
        }
        e.state = TunnelState::Starting;
        let (spec, auto) = (e.spec.clone(), e.auto_port);
        Task::perform(
            async move {
                let r = if auto { conn.start_forward_auto(spec).await } else { conn.start_forward(spec).await };
                r.map_err(|e| e.to_string())
            },
            move |r| Message::TunnelStarted(id, entry, Once::new(r)),
        )
    }

    /// Whether the active tab's host can store tunnels (CoShell-defined hosts only).
    pub fn can_persist_tunnels(&self) -> bool {
        self.active_tab()
            .and_then(|t| t.alias.as_deref())
            .and_then(|a| self.host_store.get(a))
            .is_some_and(HostDef::is_definition)
    }

    fn add_tunnel(&mut self) -> Task<Message> {
        let Some(id) = self.active.filter(|_| self.active_tab().is_some_and(|t| t.conn.is_some())) else {
            self.tunnel_form.error = Some(tr("연결된 SSH 세션이 없습니다").into());
            return Task::none();
        };
        let (spec, auto) = match form_spec(&self.tunnel_form) {
            Ok(x) => x,
            Err(e) => {
                self.tunnel_form.error = Some(e);
                return Task::none();
            }
        };
        let persist = self.tunnel_form.persist && self.can_persist_tunnels();
        self.tunnel_form = TunnelForm { kind: self.tunnel_form.kind, persist: self.tunnel_form.persist, ..TunnelForm::default() };
        let mut tasks = vec![self.start_tunnel(id, spec.clone(), auto)];
        if persist
            && let Some(alias) = self.active_tab().and_then(|t| t.alias.clone())
            && let Some(mut def) = self.host_store.get(&alias).cloned()
        {
            let f = forward_string(&spec);
            if !def.forwards.contains(&f) {
                def.forwards.push(f);
                self.host_store.upsert(Some(&alias), def);
                tasks.push(self.persist_hosts(None));
            }
        }
        Task::batch(tasks)
    }

    fn on_shortcut(&mut self, s: Shortcut) -> Task<Message> {
        match s {
            Shortcut::Palette => self.update(Message::OpenPalette),
            Shortcut::NewLocal => self.new_local(),
            Shortcut::CloseTab => match self.active {
                Some(id) => self.update(Message::Close(id)),
                None => Task::none(),
            },
            Shortcut::FocusStep(back) => {
                // Only in forms: elsewhere Tab belongs to the shell or the text being typed.
                if !self.form_open() {
                    return Task::none();
                }
                if back { iced::widget::operation::focus_previous() } else { iced::widget::operation::focus_next() }
            }
            Shortcut::TabStep(d) => {
                let n = self.tabs.len() as i32;
                match self.active_index() {
                    Some(i) if n > 1 => {
                        let id = self.tabs[(i as i32 + d).rem_euclid(n) as usize].id;
                        self.active = Some(id);
                        self.rename = None;
                        self.focus_prompt()
                    }
                    _ => Task::none(),
                }
            }
            Shortcut::TabMove(d) => {
                if let Some(i) = self.active_index() {
                    let j = i as i32 + d;
                    if (0..self.tabs.len() as i32).contains(&j) {
                        self.tabs.swap(i, j as usize);
                    }
                }
                Task::none()
            }
            Shortcut::Escape if self.rename.is_some() => self.update(Message::TabRenameCancel),
            Shortcut::Find => {
                if self.live_session().is_none() || self.host_view {
                    return Task::none();
                }
                self.search.get_or_insert_default();
                self.term_focus = (self.term_focus.0 + 1, false);
                iced::widget::operation::focus(SEARCH_INPUT)
            }
            Shortcut::FindNext(forward) => self.update(Message::SearchNext(forward)),
            Shortcut::Zoom(step) => {
                self.settings.font_size = match step {
                    0 => settings::FONT_SIZE_DEFAULT,
                    n => (self.settings.font_size + f32::from(n)).clamp(settings::FONT_SIZE_MIN, settings::FONT_SIZE_MAX),
                };
                self.state_dirty = true;
                Task::none()
            }
            Shortcut::Settings => self.update(if self.settings_draft.is_some() { Message::SettingsClose } else { Message::SettingsOpen }),
            Shortcut::Escape if self.settings_draft.is_some() => self.update(Message::SettingsClose),
            Shortcut::Escape if self.close_confirm.is_some() => self.update(Message::CloseCancel),
            Shortcut::Nth(n) => match self.tabs.get(n).map(|t| t.id) {
                Some(id) => self.update(Message::Activate(id)),
                None => Task::none(),
            },
            Shortcut::AiPanel => self.update(Message::AiToggle),
            Shortcut::Servers => self.update(Message::HostView),
            Shortcut::Escape if self.host_view && !self.tabs.is_empty() && self.editor.is_none() && self.palette.is_none() => {
                self.update(Message::HostView)
            }
            Shortcut::Escape if self.about => self.update(Message::About),
            Shortcut::Escape if self.palette.is_some() => self.update(Message::ClosePalette),
            Shortcut::Escape if self.tunnel_panel => self.update(Message::TunnelPanel),
            Shortcut::Escape if self.editor.is_some() => self.update(Message::HostEditorClose),
            Shortcut::Escape if self.attach_confirm.is_some() => self.update(Message::AttachCancel),
            Shortcut::Escape if self.ai.settings.is_some() => self.update(Message::AiSettingsClose),
            Shortcut::Escape if self.search.is_some() && self.active_tab().is_none_or(|t| t.prompt.is_none()) => {
                self.update(Message::SearchClose)
            }
            Shortcut::Escape => match self.active {
                Some(id) if self.active_tab().is_some_and(|t| t.prompt.is_some()) => self.update(Message::PromptCancel(id)),
                _ => Task::none(),
            },
            Shortcut::Up if self.palette.is_some() => self.update(Message::PaletteMove(-1)),
            Shortcut::Down if self.palette.is_some() => self.update(Message::PaletteMove(1)),
            Shortcut::Up | Shortcut::Down => Task::none(),
        }
    }

    fn close(&mut self, id: TabId) {
        let Some(i) = self.tabs.iter().position(|t| t.id == id) else { return };
        // Dropping the tab drops pending replies (→ reject) and the session (→ disconnect).
        self.tabs[i].chat.stop();
        self.tabs.remove(i);
        if self.active == Some(id) {
            self.active = self.tabs.get(i.min(self.tabs.len().saturating_sub(1))).map(|t| t.id);
            self.search = None;
        }
        if self.rename.as_ref().is_some_and(|(t, _)| *t == id) {
            self.rename = None;
        }
    }

    /// Put keyboard focus into the active tab's dialog, if any.
    fn focus_prompt(&self) -> Task<Message> {
        match self.active_tab().and_then(|t| t.prompt.as_ref()) {
            Some(Pending::Interactive { .. }) => iced::widget::operation::focus(prompt_input_id(0)),
            Some(Pending::HostKey { query, .. }) if query.kind != coshell_ssh::HostKeyKind::Unknown => {
                iced::widget::operation::focus(CONFIRM_INPUT)
            }
            _ => Task::none(),
        }
    }

    fn on_conn(&mut self, id: TabId, ev: ConnEvent) -> Task<Message> {
        let is_active = self.active == Some(id);
        let (scrollback, cursor) = (self.settings.scrollback, cursor_shape(self.settings.cursor));
        let Some(tab) = self.tab_mut(id) else { return Task::none() };
        match ev {
            ConnEvent::Progress(p) => {
                let set = |steps: &mut Vec<Step>, hop: usize, s: Step| {
                    if let Some(x) = steps.get_mut(hop) {
                        *x = s;
                    }
                };
                match p {
                    Progress::Connecting { hop } => set(&mut tab.steps, hop, Step::Connecting),
                    Progress::Authenticating { hop } => set(&mut tab.steps, hop, Step::Authenticating),
                    Progress::Authenticated { hop, latency } => set(&mut tab.steps, hop, Step::Done(latency)),
                    Progress::OpeningShell => {}
                }
                Task::none()
            }
            ConnEvent::HostKey(query, reply) => {
                if let Some(reply) = reply.take() {
                    tab.prompt = Some(Pending::HostKey { query, reply, confirm: String::new() });
                }
                if is_active { self.focus_prompt() } else { Task::none() }
            }
            ConnEvent::Interactive(query, reply) => {
                if let Some(reply) = reply.take() {
                    let answers = query.prompts.iter().map(|_| Zeroizing::new(String::new())).collect();
                    tab.prompt = Some(Pending::Interactive { query, reply, answers, save: false });
                }
                if is_active { self.focus_prompt() } else { Task::none() }
            }
            ConnEvent::Done(result) => {
                tab.prompt = None;
                let save = std::mem::take(&mut tab.save_passwords);
                match result.take() {
                    Some(Ok((transport, reader, conn))) => {
                        tab.route = conn.labels().to_vec();
                        tab.conn = Some(conn);
                        let session = Session::new(Arc::new(transport), reader, GridSize::DEFAULT);
                        session.configure(scrollback, cursor);
                        let (phase, task) = live(id, session);
                        tab.phase = phase;
                        let forwards = tab.auto_forwards.clone();
                        // Only passwords that just worked are kept.
                        if let Some(e) = save.iter().find_map(|(key, pw)| self.passwords.set(key, pw).err()) {
                            self.notice = Some(tf("비밀번호를 키체인에 저장하지 못했습니다: {0}", &[&e]));
                        }
                        Task::batch(std::iter::once(task).chain(forwards.into_iter().map(|f| self.start_tunnel(id, f, false))))
                    }
                    Some(Err(e)) => {
                        tab.phase = Phase::Failed(e);
                        Task::none()
                    }
                    None => Task::none(),
                }
            }
        }
    }

    fn chat_mut(&mut self, id: TabId) -> Option<&mut Chat> {
        self.tabs.iter_mut().find(|t| t.id == id).map(|t| &mut t.chat)
    }

    fn update_ai(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::AiToggle => {
                self.ai.open = !self.ai.open;
                self.settings.ai_panel_open = self.ai.open;
                self.state_dirty = true;
                if self.ai.open { self.focus_ai() } else { self.focus_terminal() }
            }
            Message::AiProfile(pid) => {
                if let Some(e) = self.ai.set_active(&pid) {
                    self.notice = Some(e);
                }
                Task::none()
            }
            Message::AiDraft(id, action) => {
                if let Some(c) = self.chat_mut(id) {
                    c.draft.perform(action);
                }
                Task::none()
            }
            Message::AiSend(id) => self.ai_send(id),
            Message::AiStop(id) => {
                if let Some(c) = self.chat_mut(id) {
                    c.stop();
                }
                Task::none()
            }
            Message::AiEvent(id, item) => {
                if let Some(c) = self.chat_mut(id) {
                    match item {
                        Ok(StreamEvent::Delta(t)) => {
                            if let Some(turn) = c.turns.last_mut().filter(|t| t.role == Role::Assistant) {
                                turn.text.push_str(&t);
                            }
                        }
                        Ok(StreamEvent::Done { stop_reason }) => {
                            c.streaming = None;
                            if matches!(stop_reason.as_deref(), Some("max_tokens" | "length")) {
                                c.error = Some(tr("응답이 길이 제한에서 잘렸습니다").into());
                            }
                        }
                        Err(e) => {
                            c.streaming = None;
                            if c.turns.last().is_some_and(|t| t.role == Role::Assistant && t.text.is_empty()) {
                                c.turns.pop();
                            }
                            c.error = Some(e.to_string());
                        }
                    }
                }
                Task::none()
            }
            Message::AiAttach(id) => {
                self.attach_output(id);
                self.focus_ai()
            }
            Message::AiDetach(id) => {
                if let Some(c) = self.chat_mut(id) {
                    c.attachment = None;
                }
                Task::none()
            }
            Message::AiNewChat(id) => {
                if let Some(c) = self.chat_mut(id) {
                    c.stop();
                    *c = Chat::default();
                }
                self.focus_ai()
            }
            Message::AiRetry(id) => {
                let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else { return Task::none() };
                let target = crate::ui_ai::target_label(tab);
                self.ai.retry(id, &mut tab.chat, &target)
            }
            Message::AiSuggest(id, text, attach) => {
                if attach {
                    self.attach_output(id);
                }
                if let Some(c) = self.chat_mut(id) {
                    c.draft = text_editor::Content::with_text(&text);
                }
                if self.attach_confirm.is_some() { Task::none() } else { self.ai_send(id) }
            }
            Message::AttachConfirm => {
                if let Some((id, a)) = self.attach_confirm.take()
                    && let Some(c) = self.chat_mut(id)
                {
                    c.attachment = Some(a);
                }
                self.focus_ai()
            }
            Message::AttachCancel => {
                self.attach_confirm = None;
                Task::none()
            }
            Message::AiCopy(text) => iced::clipboard::write(text),
            Message::AiInsert(id, cmd) => {
                if let Some(Tab { phase: Phase::Live(s), exited: false, .. }) = self.tabs.iter().find(|t| t.id == id) {
                    let bracketed = s.term().lock().mode().contains(coshell_term::alacritty_terminal::term::TermMode::BRACKETED_PASTE);
                    // Insert only; the user presses Enter to run it.
                    if let Some(bytes) = assist::insert_bytes(&cmd, bracketed) {
                        s.write(&bytes);
                    }
                }
                self.focus_terminal()
            }
            Message::AiSettingsOpen => {
                self.ai.open_settings();
                Task::none()
            }
            Message::AiSettingsClose => {
                self.ai.settings = None;
                Task::none()
            }
            Message::AiSettingsSave => {
                if let Err(e) = self.ai.save_settings() {
                    self.notice = Some(tf("AI 설정 저장 실패: {0}", &[&e]));
                }
                Task::none()
            }
            Message::AiSettingsSaveKey => {
                self.ai.save_key();
                Task::none()
            }
            Message::AiSettingsDeleteKey => {
                self.ai.delete_key();
                Task::none()
            }
            Message::AiSettingsAdd => {
                self.ai.add_profile();
                Task::none()
            }
            Message::AiSettingsRemove => {
                self.ai.remove_profile();
                Task::none()
            }
            Message::AiSettingsFetch(test_only) => self.ai.fetch_models(test_only),
            m => {
                if let Some(s) = &mut self.ai.settings {
                    match m {
                        Message::AiSettingsSelect(i) if i < s.draft.profiles.len() => {
                            s.selected = i;
                            s.models.clear();
                            s.status = None;
                            s.key_input = zeroize::Zeroizing::new(String::new());
                        }
                        Message::AiSettingsName(v) => s.profile_mut().name = v,
                        Message::AiSettingsBase(v) => s.profile_mut().base_url = v,
                        Message::AiSettingsModel(v) => s.profile_mut().model = v,
                        Message::AiSettingsKey(v) => s.key_input = zeroize::Zeroizing::new(v),
                        Message::AiSettingsUse => s.draft.active = s.profile().id.clone(),
                        Message::AiSettingsFormat(f) => {
                            let p = s.profile_mut();
                            if p.base_url.trim().is_empty() || p.base_url == p.format.default_base() {
                                p.base_url = f.default_base().into();
                            }
                            p.format = f;
                            s.models.clear();
                        }
                        Message::AiModels(test_only, r) => {
                            s.busy = false;
                            s.status = Some(match r {
                                Ok(ids) if test_only => Ok(tf("연결됨 · 모델 {0}개", &[&ids.len()])),
                                Ok(ids) => {
                                    let n = ids.len();
                                    s.models = ids;
                                    Ok(tf("모델 {0}개를 불러왔습니다", &[&n]))
                                }
                                Err(e) => Err(e),
                            });
                        }
                        _ => {}
                    }
                }
                Task::none()
            }
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let live_stats = self.tunnel_panel
            && self.active_tab().is_some_and(|t| t.tunnels.iter().any(|e| matches!(e.state, TunnelState::Running(_))));
        let tick = if live_stats {
            iced::time::every(STATS_REFRESH).map(|_| Message::Tick)
        } else {
            Subscription::none()
        };
        let persist = if self.state_dirty {
            iced::time::every(PERSIST_DELAY).map(|_| Message::PersistState)
        } else {
            Subscription::none()
        };
        let notice = if self.notice.is_some() {
            iced::time::every(NOTICE_TTL).map(|_| Message::NoticeDismiss)
        } else {
            Subscription::none()
        };
        let resize = if self.ai_resizing { event::listen_with(on_divider_drag) } else { Subscription::none() };
        #[cfg(target_os = "macos")]
        let menu = Subscription::run(crate::menu::events);
        #[cfg(not(target_os = "macos"))]
        let menu = Subscription::none();
        Subscription::batch([event::listen_with(on_event), tick, persist, notice, resize, menu])
    }
}

fn live(id: TabId, mut session: Session) -> (Phase, Task<Message>) {
    let events = session.take_events().expect("fresh session has events");
    (Phase::Live(session), Task::run(events, move |e| Message::Term(id, e)))
}

/// Mouse events while the AI panel divider is held.
fn on_divider_drag(ev: Event, _status: event::Status, _window: window::Id) -> Option<Message> {
    match ev {
        Event::Mouse(iced::mouse::Event::CursorMoved { position }) => Some(Message::AiResizeTo(position.x)),
        Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left)) => Some(Message::AiResizeEnd),
        _ => None,
    }
}

/// Global shortcuts. Sees captured events too (text inputs capture Escape).
fn on_event(ev: Event, _status: event::Status, _window: window::Id) -> Option<Message> {
    let (key, modifiers, physical_key) = match ev {
        Event::Window(window::Event::Resized(size)) => return Some(Message::WindowResized(size)),
        Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, physical_key, .. }) => (key, modifiers, physical_key),
        _ => return None,
    };
    let s = if modifiers.command() && modifiers.shift() && matches!(key, keyboard::Key::Named(Named::ArrowLeft | Named::ArrowRight)) {
        Shortcut::TabMove(if key == keyboard::Key::Named(Named::ArrowLeft) { -1 } else { 1 })
    } else if modifiers.command() {
        match key.to_latin(physical_key)? {
            '{' => Shortcut::TabStep(-1),
            '}' => Shortcut::TabStep(1),
            '[' if modifiers.shift() => Shortcut::TabStep(-1),
            ']' if modifiers.shift() => Shortcut::TabStep(1),
            'k' => Shortcut::Palette,
            't' => Shortcut::NewLocal,
            'w' => Shortcut::CloseTab,
            'l' => Shortcut::AiPanel,
            'o' => Shortcut::Servers,
            'f' => Shortcut::Find,
            ',' => Shortcut::Settings,
            'g' => Shortcut::FindNext(modifiers.shift()),
            '=' | '+' => Shortcut::Zoom(1),
            '-' => Shortcut::Zoom(-1),
            '0' => Shortcut::Zoom(0),
            c @ '1'..='9' => Shortcut::Nth(c as usize - '1' as usize),
            _ => return None,
        }
    } else {
        match key {
            keyboard::Key::Named(Named::Escape) => Shortcut::Escape,
            keyboard::Key::Named(Named::Tab) => Shortcut::FocusStep(modifiers.shift()),
            keyboard::Key::Named(Named::ArrowUp) => Shortcut::Up,
            keyboard::Key::Named(Named::ArrowDown) => Shortcut::Down,
            _ => return None,
        }
    };
    Some(Message::Shortcut(s))
}

/// Open a URL in the default browser.
fn open_url(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("open");
    // No shell on Windows: `cmd /C start` would interpret `&` in a URL.
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = std::process::Command::new("xdg-open");
    cmd.arg(url).spawn().map(|_| ())
}
