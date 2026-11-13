//! App-level tests: AI request assembly, stream handling, and headless UI
//! simulation of the AI panel (no desktop / user files involved).

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use coshell_ai::{AiError, Format, Profile, ProfileStore, Role, SecretStore, StreamEvent};
use crate::hosts::{Env, HostDef, HostStore};
use iced::widget::text_editor;
use iced_test::simulator;
use zeroize::Zeroizing;

use crate::ai::{Ai, Attachment, Chat, Turn};
use crate::app::{CoShell, HostField, Message, Phase};
use crate::ui;

#[derive(Default)]
struct MemSecrets(Mutex<HashMap<String, String>>);

impl SecretStore for MemSecrets {
    fn get(&self, id: &str) -> Option<Zeroizing<String>> {
        self.0.lock().unwrap().get(id).cloned().map(Zeroizing::new)
    }
    fn set(&self, id: &str, secret: &str) -> Result<(), String> {
        self.0.lock().unwrap().insert(id.into(), secret.into());
        Ok(())
    }
    fn delete(&self, id: &str) -> Result<(), String> {
        self.0.lock().unwrap().remove(id);
        Ok(())
    }
}

fn store(active: &str) -> ProfileStore {
    ProfileStore { active: active.into(), ..ProfileStore::default() }
}

fn ai_with(active: &str, keys: &[(&str, &str)]) -> Ai {
    let secrets = MemSecrets::default();
    for (id, k) in keys {
        secrets.set(id, k).unwrap();
    }
    Ai::new(store(active), None, Arc::new(secrets))
}

fn turn(role: Role, text: &str) -> Turn {
    Turn { role, text: text.into(), sent: text.into(), attachment: None, model: "m".into(), stopped: false, exec: None, hidden: false }
}

#[test]
fn send_frames_masked_attachment_with_history() {
    let ai = ai_with("claude", &[("claude", "sk-ant-test")]);
    let mut chat = Chat::default();
    chat.turns.push(turn(Role::User, "이전 질문"));
    chat.turns.push(turn(Role::Assistant, "이전 답변"));
    let (masked, n) = coshell_ai::redact::mask("DB_PASSWORD=hunter2\nnginx: bind() failed");
    chat.attachment = Some(Attachment { label: "최근 출력 2줄".into(), masked, secrets: n });
    chat.draft = text_editor::Content::with_text("왜 실패했지?");

    let (profile, key) = ai.begin(&mut chat, "SSH ubuntu@web (via ec2-user@bastion)").expect("sends");
    assert_eq!((profile.id.as_str(), key.as_deref()), ("claude", Some("sk-ant-test")));
    assert!(chat.attachment.is_none() && chat.draft.text().trim().is_empty(), "consumed");

    let req = Ai::request(&chat, "SSH ubuntu@web (via ec2-user@bastion)", false);
    assert!(req.system.contains("SSH ubuntu@web (via ec2-user@bastion)"));
    assert!(req.system.contains("untrusted"));
    let roles: Vec<Role> = req.messages.iter().map(|m| m.role).collect();
    assert_eq!(roles, [Role::User, Role::Assistant, Role::User]);
    let last = &req.messages[2].content;
    assert!(last.contains("<terminal_output source=\"SSH ubuntu@web"), "{last}");
    assert!(last.contains("bind() failed") && last.ends_with("왜 실패했지?"));
    assert!(!last.contains("hunter2"), "secret must be masked: {last}");
    assert_eq!(chat.turns.last().unwrap().text, "왜 실패했지?", "UI shows the question, not the framing");
}

#[test]
fn missing_key_blocks_send_but_local_profile_does_not() {
    let ai = ai_with("claude", &[]);
    let mut chat = Chat { draft: text_editor::Content::with_text("hi"), ..Chat::default() };
    // Only meaningful when the developer has no ANTHROPIC_API_KEY exported.
    if std::env::var_os("ANTHROPIC_API_KEY").is_none() {
        assert!(ai.begin(&mut chat, "local").is_none());
        assert!(chat.error.as_deref().is_some_and(|e| e.contains("API 키")));
        assert!(chat.turns.is_empty());
    }

    let ai = ai_with("ollama", &[]);
    let mut chat = Chat { draft: text_editor::Content::with_text("hi"), ..Chat::default() };
    let (p, key) = ai.begin(&mut chat, "local").expect("local needs no key");
    assert_eq!((p.format, key), (Format::OpenAi, None));
}

fn app_with(user_cfg: &str, store: HostStore, hosts_path: Option<std::path::PathBuf>, ai: Ai) -> CoShell {
    app_with_secrets(user_cfg, store, hosts_path, ai, Arc::default())
}

fn app_with_secrets(
    user_cfg: &str,
    store: HostStore,
    hosts_path: Option<std::path::PathBuf>,
    ai: Ai,
    passwords: Arc<MemSecrets>,
) -> CoShell {
    CoShell::with_parts("/nonexistent".into(), user_cfg.into(), store, hosts_path, ai, "me".into(), None, passwords)
}

fn app() -> CoShell {
    app_with("Host bastion\n HostName 10.0.0.1\n", HostStore::default(), None, ai_with("ollama", &[]))
}

#[test]
fn stream_events_update_chat_and_errors_clean_up() {
    let mut app = app();
    let id = app.open_local();
    let tab = app.tabs.iter_mut().find(|t| t.id == id).unwrap();
    tab.chat.turns.push(turn(Role::User, "q"));
    tab.chat.turns.push(Turn { text: String::new(), ..turn(Role::Assistant, "") });

    let _ = app.update(Message::AiEvent(id, Ok(StreamEvent::Delta("df ".into()))));
    let _ = app.update(Message::AiEvent(id, Ok(StreamEvent::Delta("-h".into()))));
    let _ = app.update(Message::AiEvent(id, Ok(StreamEvent::Done { stop_reason: Some("end_turn".into()) })));
    let chat = &app.tabs[0].chat;
    assert_eq!(chat.turns.last().unwrap().text, "df -h");
    assert!(chat.error.is_none());

    let tab = &mut app.tabs[0];
    tab.chat.turns.push(Turn { text: String::new(), ..turn(Role::Assistant, "") });
    let _ = app.update(Message::AiEvent(id, Err(AiError::Http { status: 401, message: "invalid x-api-key".into() })));
    let chat = &app.tabs[0].chat;
    assert_eq!(chat.turns.len(), 2, "empty assistant turn removed");
    assert_eq!(chat.error.as_deref(), Some("HTTP 401: invalid x-api-key"));
}

const ANSWER: &str = "80번 포트를 다른 프로세스가 쓰고 있습니다. `ss`로 확인하세요.\n\n```bash\nsudo ss -tulpn | grep :80\n```\n\n정리하려면:\n\n```bash\nsudo rm -rf /var/cache/nginx\n```\n";

#[test]
fn ui_run_button_sends_command_and_risky_ones_ask_twice() {
    let mut app = app();
    let id = app.open_local();
    let tab = app.tabs.iter_mut().find(|t| t.id == id).unwrap();
    assert!(matches!(tab.phase, Phase::Live(_)));
    tab.chat.turns.push(Turn { attachment: Some("최근 출력 12줄".into()), ..turn(Role::User, "nginx가 왜 안 뜨지?") });
    tab.chat.turns.push(turn(Role::Assistant, ANSWER));

    let mut ui = simulator(ui::view(&app));
    if let Ok(dir) = std::env::var("COSHELL_SNAPSHOT_DIR") {
        let snap = ui.snapshot(&iced::Theme::Dark).expect("snapshot");
        let _ = snap.matches_image(Path::new(&dir).join("ai-panel"));
    }
    ui.click(iced::widget::Id::new(crate::ui_ai::RUN_BTN)).expect("run button visible");
    let messages: Vec<Message> = ui.into_messages().collect();
    assert!(
        messages.iter().any(|m| matches!(m, Message::AiRun(t, c) if *t == id && c == "sudo ss -tulpn | grep :80")),
        "{messages:?}"
    );

    // A destructive command does not run on the first click; the button asks again.
    let risky = "sudo rm -rf /var/cache/nginx".to_string();
    let _ = app.update(Message::AiRun(id, risky.clone()));
    let tab = app.tabs.iter().find(|t| t.id == id).unwrap();
    assert_eq!(tab.chat.run_confirm.as_deref(), Some(risky.as_str()));
    simulator(ui::view(&app)).find("한 번 더 눌러 실행").expect("confirm state shown");
}

#[test]
fn risk_badges_and_labels_render() {
    let mut app = app();
    let id = app.open_local();
    let tab = app.tabs.iter_mut().find(|t| t.id == id).unwrap();
    tab.chat.turns.push(turn(Role::User, "q"));
    tab.chat.turns.push(turn(Role::Assistant, ANSWER));
    let mut ui = simulator(ui::view(&app));
    ui.find("조회").expect("read-only badge");
    ui.find("위험").expect("destructive badge");
}

#[test]
fn settings_dialog_never_shows_stored_key() {
    let mut app = app_with("", HostStore::default(), None, ai_with("claude", &[("claude", "sk-ant-SECRET-VALUE")]));
    let _ = app.update(Message::AiSettingsOpen);
    let mut ui = simulator(ui::view(&app));
    ui.find("키체인에 저장됨").expect("status shown");
    assert!(ui.find("sk-ant-SECRET-VALUE").is_err(), "key must never be rendered");
    let _ = Profile::clone; // keep import used in all cfgs
}

#[test]
fn tunnel_popover_lists_states_and_controls() {
    use crate::app::{TunnelEntry, TunnelState};
    use coshell_ssh::ForwardSpec;

    let mut app = app();
    let id = app.open_fake_ssh("prod-web", &["ec2-user@bastion", "ubuntu@10.0.1.42"], &[12, 20]);
    let tab = app.tabs.iter_mut().find(|t| t.id == id).unwrap();
    let entry = |id, spec: &str, kind, state| TunnelEntry { id, spec: ForwardSpec::parse(kind, spec).unwrap(), auto_port: false, state };
    tab.tunnels.push(entry(1, "3306:localhost:3306", 'L', TunnelState::Starting));
    tab.tunnels.push(entry(2, "0.0.0.0:9000:127.0.0.1:3000", 'R', TunnelState::Failed("서버가 거부했습니다".into())));
    tab.tunnels.push(entry(3, "1080", 'D', TunnelState::Stopped));
    app.tunnel_panel = true;

    let mut ui = simulator(ui::view(&app));
    if let Ok(dir) = std::env::var("COSHELL_SNAPSHOT_DIR") {
        let _ = ui.snapshot(&iced::Theme::Dark).unwrap().matches_image(Path::new(&dir).join("tunnel-popover"));
    }
    for t in ["시작 중…", "서버가 거부했습니다", "외부 노출", "중지됨", "서버 → 내 PC", "내 PC → 서버", "SOCKS 프록시", "경유: ec2-user@bastion"] {
        ui.find(t).unwrap_or_else(|_| panic!("{t} missing"));
    }
    ui.click("중지").expect("stop button");
    ui.click("시작").expect("start button");
    let msgs: Vec<Message> = ui.into_messages().collect();
    assert!(msgs.iter().any(|m| matches!(m, Message::TunnelStop(t, 1) if *t == id)), "{msgs:?}");
    assert!(msgs.iter().any(|m| matches!(m, Message::TunnelStart(t, 2) if *t == id)), "{msgs:?}");
}

#[test]
fn tunnel_form_by_purpose() {
    use crate::app::{TunnelForm, form_spec};
    use coshell_ssh::ForwardSpec;

    let f = |kind, rh: &str, rp: &str, lp: &str| TunnelForm {
        kind,
        remote_host: rh.into(),
        remote_port: rp.into(),
        local_port: lp.into(),
        ..TunnelForm::default()
    };
    // Server port → this PC: blank local port = same port, auto-fallback allowed.
    assert_eq!(
        form_spec(&f('L', "", "3306", "")),
        Ok((ForwardSpec::Local { bind: "127.0.0.1".into(), port: 3306, host: "localhost".into(), host_port: 3306 }, true))
    );
    assert!(!form_spec(&f('L', "db.internal", "5432", "15432")).unwrap().1, "explicit local port: no auto fallback");
    // This PC → server: blank server port = same port.
    assert_eq!(
        form_spec(&f('R', "", "", "3000")),
        Ok((ForwardSpec::Remote { bind: "127.0.0.1".into(), port: 3000, host: "127.0.0.1".into(), host_port: 3000 }, false))
    );
    // SOCKS default.
    assert_eq!(form_spec(&f('D', "", "", "")), Ok((ForwardSpec::Dynamic { bind: "127.0.0.1".into(), port: 1080 }, true)));
    for bad in [f('L', "", "", ""), f('L', "", "99999", ""), f('R', "", "", "x"), f('L', "a b", "80", "")] {
        assert!(form_spec(&bad).is_err());
    }

    // Presets fill the field that matches the purpose.
    let mut app = app();
    let _ = app.update(Message::TunnelKind('L'));
    let _ = app.update(Message::TunnelPreset(3306));
    assert_eq!(app.tunnel_form.remote_port, "3306");
    let _ = app.update(Message::TunnelKind('R'));
    let _ = app.update(Message::TunnelPreset(3000));
    assert_eq!((app.tunnel_form.local_port.as_str(), app.tunnel_form.remote_port.as_str()), ("3000", ""));
}

#[test]
fn plain_ssh_does_not_talk_about_jumps() {
    let mut direct = app();
    direct.open_fake_ssh("web", &["ubuntu@203.0.113.9"], &[]);
    let mut ui = simulator(ui::view(&direct));
    ui.find("web에 연결 중").expect("connect card");
    ui.find("ubuntu@203.0.113.9").expect("target shown");
    assert!(ui.find("경유 서버 1곳을 거쳐 연결합니다").is_err());
    assert!(ui.find("경유 1").is_err());

    let mut jumped = app();
    jumped.open_fake_ssh("db", &["ec2-user@bastion", "postgres@10.0.0.5"], &[]);
    let mut ui = simulator(ui::view(&jumped));
    ui.find("경유 서버 1곳을 거쳐 연결합니다").expect("jumps explained only when present");
    ui.find("경유 1").expect("status bar hint");
}

#[test]
fn prod_output_needs_confirmation_before_ai() {
    let mut app = app();
    let id = app.open_local();
    let tab = app.tabs.iter_mut().find(|t| t.id == id).unwrap();
    if let Phase::Live(sess) = &tab.phase {
        // Give the grid some content without depending on the shell prompt.
        sess.write(b"");
    }
    tab.env = Env::Prod;
    // Wait for the shell to print something attachable.
    for _ in 0..50 {
        if let Phase::Live(sess) = &app.tabs[0].phase
            && !sess.recent_text(5).trim().is_empty()
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let _ = app.update(Message::AiAttach(id));
    assert!(app.attach_confirm.is_some(), "prod attach must wait for confirmation");
    assert!(app.tabs[0].chat.attachment.is_none());
    assert!(!app.terminal_active(&app.tabs[0]), "dialog owns the keyboard");

    let mut ui = simulator(ui::view(&app));
    ui.find("AI로 보낼 내용 확인").expect("dialog shown");
    ui.click("첨부").expect("confirm");
    for m in ui.into_messages() {
        let _ = app.update(m);
    }
    assert!(app.attach_confirm.is_none());
    assert!(app.tabs[0].chat.attachment.is_some());

    // Non-prod tabs attach directly.
    app.tabs[0].env = Env::Dev;
    app.tabs[0].chat.attachment = None;
    let _ = app.update(Message::AiAttach(id));
    assert!(app.attach_confirm.is_none() && app.tabs[0].chat.attachment.is_some());
}

#[test]
fn host_editor_saves_definition_and_tags() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hosts.toml");
    let mut app = app_with("Host bastion\n HostName 13.124.88.12\n User ec2-user\n", HostStore::default(), Some(path.clone()), ai_with("ollama", &[]));

    // New CoShell host jumping through an ~/.ssh/config host.
    let _ = app.update(Message::PalettePick(crate::app::PaletteItem::NewHost));
    for (f, v) in [
        (HostField::Alias, "prod-web"),
        (HostField::HostName, "10.0.1.42"),
        (HostField::User, "ubuntu"),
        (HostField::ProxyJump, "bastion"),
    ] {
        let _ = app.update(Message::HostField(f, v.into()));
    }
    let _ = app.update(Message::HostEnv(Env::Prod));
    // Built from blocks: purpose, then ports.
    let _ = app.update(Message::TunnelKind('L'));
    let _ = app.update(Message::TunnelField(crate::app::TunnelField::RemoteHost, "127.0.0.1".into()));
    let _ = app.update(Message::TunnelField(crate::app::TunnelField::RemotePort, "80".into()));
    let _ = app.update(Message::TunnelField(crate::app::TunnelField::LocalPort, "8080".into()));
    let _ = app.update(Message::TunnelAdd);
    if let Ok(dir) = std::env::var("COSHELL_SNAPSHOT_DIR") {
        let mut ui = simulator(ui::view(&app));
        let _ = ui.snapshot(&iced::Theme::Dark).unwrap().matches_image(Path::new(&dir).join("host-editor"));
    }
    let _ = app.update(Message::HostSave(false));
    assert!(app.editor.is_none(), "saved: {:?}", app.editor.as_ref().and_then(|e| e.error.clone()));

    let saved = HostStore::load(&path).unwrap();
    let def = saved.get("prod-web").unwrap();
    assert_eq!((def.env, def.hostname.as_deref(), def.proxy_jump.as_deref()), (Env::Prod, Some("10.0.1.42"), Some("bastion")));
    assert_eq!(def.forwards, ["L:127.0.0.1:8080:127.0.0.1:80"]);
    assert!(app.hosts.iter().any(|h| h.alias == "prod-web"), "listed in palette");
    assert_eq!(app.env_of("prod-web"), Env::Prod);

    // Tag-only overlay for the ~/.ssh/config host: connection fields are not stored.
    let _ = app.update(Message::HostEdit("bastion".into()));
    assert!(app.editor.as_ref().unwrap().external.is_some());
    let _ = app.update(Message::HostField(HostField::HostName, "evil.example".into()));
    let _ = app.update(Message::HostEnv(Env::Stage));
    let _ = app.update(Message::HostSave(false));
    let saved = HostStore::load(&path).unwrap();
    assert_eq!(saved.get("bastion"), Some(&HostDef { alias: "bastion".into(), env: Env::Stage, ..HostDef::default() }));
}

#[test]
fn host_editor_rejects_injection_and_alias_clash() {
    let mut app = app_with("Host bastion\n HostName 1.2.3.4\n", HostStore::default(), None, ai_with("ollama", &[]));
    let _ = app.update(Message::PalettePick(crate::app::PaletteItem::NewHost));
    let _ = app.update(Message::HostField(HostField::Alias, "x".into()));
    let _ = app.update(Message::HostField(HostField::HostName, "1.1.1.1 ProxyCommand=evil".into()));
    let _ = app.update(Message::HostSave(false));
    assert!(app.editor.as_ref().and_then(|e| e.error.clone()).is_some_and(|e| e.contains("주소")));

    let _ = app.update(Message::HostField(HostField::HostName, "1.1.1.1".into()));
    let _ = app.update(Message::HostField(HostField::Alias, "bastion".into()));
    let _ = app.update(Message::HostSave(false));
    assert!(app.editor.as_ref().and_then(|e| e.error.clone()).is_some_and(|e| e.contains("~/.ssh/config")));
}

#[test]
fn prod_tab_is_marked_without_framing_the_terminal() {
    let mut app = app();
    let id = app.open_local();
    app.tabs.iter_mut().find(|t| t.id == id).unwrap().env = Env::Prod;
    let mut ui = simulator(ui::view(&app));
    assert!(ui.find("PRODUCTION").is_err(), "no banner over the terminal");
    ui.find("운영").expect("env tag in the status bar");
    if let Ok(dir) = std::env::var("COSHELL_SNAPSHOT_DIR") {
        let _ = ui.snapshot(&iced::Theme::Dark).unwrap().matches_image(Path::new(&dir).join("prod-tab"));
    }
}

/// `target` is `[user@]host[:port]`; `jump` an alias.
fn add_server(app: &mut CoShell, group: Option<&str>, name: &str, target: &str, jump: Option<&str>) {
    let _ = app.update(Message::HostNewIn(group.map(String::from)));
    let (user, rest) = target.split_once('@').map_or((None, target), |(u, r)| (Some(u), r));
    let (host, port) = rest.split_once(':').map_or((rest, None), |(h, p)| (h, Some(p)));
    let _ = app.update(Message::HostField(HostField::HostName, host.into()));
    if let Some(u) = user {
        let _ = app.update(Message::HostField(HostField::User, u.into()));
    }
    if let Some(p) = port {
        let _ = app.update(Message::HostField(HostField::Port, p.into()));
    }
    if let Some(j) = jump {
        let _ = app.update(Message::HostField(HostField::ProxyJump, j.into()));
    }
    let _ = app.update(Message::HostField(HostField::Name, name.into()));
    let _ = app.update(Message::HostSave(false));
    assert!(app.editor.is_none(), "saved {name}: {:?}", app.editor.as_ref().and_then(|e| e.error.clone()));
}

#[test]
fn servers_added_with_groups_and_jump() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hosts.toml");
    let mut app = app_with("", HostStore::default(), Some(path.clone()), ai_with("ollama", &[]));

    add_server(&mut app, Some("hanwha"), "jumphost 서버", "root@203.0.113.83:41", None);
    add_server(&mut app, Some("hanwha"), "테스트 서버", "root@10.240.0.7:41", Some("jumphost"));
    add_server(&mut app, Some("Local"), "jumphost 서버", "pi@192.0.2.36", None);

    let store = HostStore::load(&path).unwrap();
    let aliases: Vec<_> = store.hosts.iter().map(|h| (h.alias.as_str(), h.group.as_deref(), h.display())).collect();
    assert_eq!(
        aliases,
        [
            ("jumphost", Some("hanwha"), "jumphost 서버"),
            ("host", Some("hanwha"), "테스트 서버"),
            ("jumphost-2", Some("Local"), "jumphost 서버"),
        ]
    );
    let j = store.get("jumphost").unwrap();
    assert_eq!((j.user.as_deref(), j.hostname.as_deref(), j.port), (Some("root"), Some("203.0.113.83"), Some(41)));
    assert_eq!(app.groups(), ["hanwha", "Local"]);
    assert_eq!(app.display_of("host"), "테스트 서버");

    // The jump resolves to a two-hop chain.
    let mut ui = simulator(ui::view(&app));
    ui.find("via jumphost 서버").expect("jump shown on the card");
    drop(ui);
    let _ = app.update(Message::HostView);
    assert!(app.host_view);
}

#[test]
fn server_page_groups_cards_and_actions() {
    let mut store = HostStore::default();
    for (alias, name, group, host, port, env) in [
        ("pi", "라즈베리파이", "Devora", "198.51.100.10", Some(3022), Env::None),
        ("mac", "mac-server", "Devora", "198.51.100.10", Some(4022), Env::Dev),
        ("srv", "Server", "Softkr", "203.0.113.25", None, Env::Prod),
    ] {
        store.upsert(None, HostDef {
            alias: alias.into(),
            name: Some(name.into()),
            group: Some(group.into()),
            env,
            hostname: Some(host.into()),
            user: Some("ubuntu".into()),
            port,
            ..HostDef::default()
        });
    }
    let mut app = app_with("Host legacy\n HostName 192.0.2.5\n User admin\n", store, None, ai_with("ollama", &[]));
    // No tabs → the server page is the start screen.
    let mut ui = simulator(ui::view(&app));
    for t in ["Devora", "Softkr", "~/.ssh/config", "라즈베리파이", "ssh ubuntu@198.51.100.10:3022", "ssh ubuntu@203.0.113.25", "ssh admin@192.0.2.5"] {
        ui.find(t).unwrap_or_else(|_| panic!("{t} missing"));
    }
    if let Ok(dir) = std::env::var("COSHELL_SNAPSHOT_DIR") {
        let _ = ui.snapshot(&iced::Theme::Dark).unwrap().matches_image(Path::new(&dir).join("servers"));
    }
    ui.click("mac-server").expect("card clickable");
    let msgs: Vec<Message> = ui.into_messages().collect();
    assert!(msgs.iter().any(|m| matches!(m, Message::HostConnect(a) if a == "mac")), "{msgs:?}");

    // Filter narrows the board.
    let _ = app.update(Message::HostFilter("softkr".into()));
    let mut ui = simulator(ui::view(&app));
    ui.find("Server").expect("filtered card");
    assert!(ui.find("라즈베리파이").is_err());
}

#[test]
fn new_server_dialog_copy() {
    let mut app = app();
    let _ = app.update(Message::HostNewIn(None));
    let _ = app.update(Message::HostAdvanced);
    let mut ui = simulator(ui::view(&app));
    if let Ok(dir) = std::env::var("COSHELL_SNAPSHOT_DIR") {
        let _ = ui.snapshot(&iced::Theme::Dark).unwrap().matches_image(Path::new(&dir).join("new-server"));
    }
    for t in ["새 서버", "주소", "고급 설정", "경유 서버", "SSH 키", "자동 터널", "저장하고 연결"] {
        ui.find(t).unwrap_or_else(|_| panic!("{t} missing"));
    }
    for t in ["서버 포트 열기", "내 PC 포트 공유", "SOCKS 프록시", "MySQL", "+ 터널 추가"] {
        ui.find(t).unwrap_or_else(|_| panic!("tunnel builder: {t} missing"));
    }
    for gone in ["jumphost 서버", "ProxyJump", "IdentityFile", "새 호스트", "ssh 명령으로 채우기", "예: L:3306:localhost:3306"] {
        assert!(ui.find(gone).is_err(), "{gone} should not appear");
    }
}

#[test]
fn version_is_shown_in_status_bar_and_about() {
    let v = env!("CARGO_PKG_VERSION");
    assert_eq!(crate::VERSION, v);
    let mut app = app();
    let mut ui = simulator(ui::view(&app));
    ui.click(format!("v{v}")).expect("status bar version");
    for m in ui.into_messages() {
        let _ = app.update(m);
    }
    assert!(app.about);
    let mut ui = simulator(ui::view(&app));
    ui.find(format!("버전 {v}")).expect("about dialog");
    if let Ok(dir) = std::env::var("COSHELL_SNAPSHOT_DIR") {
        let _ = ui.snapshot(&iced::Theme::Dark).unwrap().matches_image(Path::new(&dir).join("about"));
    }
    ui.click("닫기").expect("close");
}

fn saved(secrets: &MemSecrets) -> Vec<(String, String)> {
    let mut v: Vec<_> = secrets.0.lock().unwrap().iter().map(|(k, p)| (k.clone(), p.clone())).collect();
    v.sort();
    v
}

#[test]
fn editor_password_lives_in_keychain_only_and_follows_the_server() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hosts.toml");
    let secrets = Arc::new(MemSecrets::default());
    let mut app = app_with_secrets("", HostStore::default(), Some(path.clone()), ai_with("ollama", &[]), secrets.clone());

    let _ = app.update(Message::HostNewIn(None));
    let _ = app.update(Message::HostField(HostField::HostName, "203.0.113.7".into()));
    let _ = app.update(Message::HostField(HostField::User, "deploy".into()));
    let _ = app.update(Message::HostField(HostField::Port, "2222".into()));
    let _ = app.update(Message::HostField(HostField::Password, "s3cret-pw".into()));
    {
        let ui = simulator(ui::view(&app));
        drop(ui); // secure input renders without the value
    }
    let _ = app.update(Message::HostSave(false));
    assert!(app.editor.is_none());
    assert_eq!(saved(&secrets), [("deploy@203.0.113.7:2222".to_string(), "s3cret-pw".to_string())]);
    assert!(!std::fs::read_to_string(&path).unwrap().contains("s3cret"), "never written to hosts.toml");

    // Reopen: shown as saved, never echoed back.
    let alias = app.host_store.hosts[0].alias.clone();
    let _ = app.update(Message::HostEdit(alias.clone()));
    let ed = app.editor.as_ref().unwrap();
    assert_eq!(ed.saved_password.as_deref(), Some("deploy@203.0.113.7:2222"));
    assert!(ed.password.is_empty());
    let mut ui = simulator(ui::view(&app));
    ui.find("저장된 비밀번호 삭제").expect("delete button");
    assert!(ui.find("s3cret-pw").is_err());
    drop(ui);

    // Port change moves the password to the new address.
    let _ = app.update(Message::HostField(HostField::Port, "2200".into()));
    let _ = app.update(Message::HostSave(false));
    assert_eq!(saved(&secrets), [("deploy@203.0.113.7:2200".to_string(), "s3cret-pw".to_string())]);

    // Forget.
    let _ = app.update(Message::HostEdit(alias.clone()));
    let _ = app.update(Message::HostForgetPassword);
    let _ = app.update(Message::HostSave(false));
    assert!(saved(&secrets).is_empty());

    // Deleting a server drops its password.
    let _ = app.update(Message::HostEdit(alias.clone()));
    let _ = app.update(Message::HostField(HostField::Password, "again".into()));
    let _ = app.update(Message::HostSave(false));
    assert_eq!(saved(&secrets).len(), 1);
    let _ = app.update(Message::HostEdit(alias));
    let _ = app.update(Message::HostDelete);
    assert!(saved(&secrets).is_empty());
}

#[test]
fn shared_password_survives_deleting_one_alias() {
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemSecrets::default());
    let mut app =
        app_with_secrets("", HostStore::default(), Some(dir.path().join("hosts.toml")), ai_with("ollama", &[]), secrets.clone());
    add_server(&mut app, None, "a", "root@198.51.100.4", None);
    add_server(&mut app, None, "b", "root@198.51.100.4", None);
    secrets.set("root@198.51.100.4:22", "pw").unwrap();
    let alias = app.host_store.hosts[0].alias.clone();
    let _ = app.update(Message::HostEdit(alias));
    let _ = app.update(Message::HostDelete);
    assert_eq!(saved(&secrets).len(), 1, "the other server still uses it");
}

fn password_prompt(app: &mut CoShell, id: crate::app::TabId, password: bool) -> futures::channel::oneshot::Receiver<crate::conn::Answers> {
    let (tx, rx) = futures::channel::oneshot::channel();
    let query = coshell_ssh::InteractiveQuery {
        hop: 0,
        label: "u@web".into(),
        name: String::new(),
        instructions: String::new(),
        prompts: vec![coshell_ssh::InteractivePrompt { text: "u@web 비밀번호".into(), echo: false }],
        password,
    };
    let _ = app.update(Message::Conn(id, crate::conn::ConnEvent::Interactive(query, crate::conn::Once::new(tx))));
    rx
}

#[test]
fn prompt_offers_save_and_saves_only_after_success() {
    let secrets = Arc::new(MemSecrets::default());
    let mut app = app_with_secrets("", HostStore::default(), None, ai_with("ollama", &[]), secrets.clone());
    let id = app.open_fake_ssh("web", &["u@web"], &[]);

    // OTP-style prompt: no save option.
    let _rx = password_prompt(&mut app, id, false);
    let mut ui = simulator(ui::view(&app));
    assert!(ui.find("이 서버 비밀번호 저장 (접속에 성공하면 키체인에 저장)").is_err());
    drop(ui);

    let mut rx = password_prompt(&mut app, id, true);
    let mut ui = simulator(ui::view(&app));
    ui.find("이 서버 비밀번호 저장 (접속에 성공하면 키체인에 저장)").expect("save checkbox");
    if let Ok(dir) = std::env::var("COSHELL_SNAPSHOT_DIR") {
        let _ = ui.snapshot(&iced::Theme::Dark).unwrap().matches_image(Path::new(&dir).join("password-prompt"));
    }
    drop(ui);
    let _ = app.update(Message::PromptInput(id, 0, "typed-pw".into()));
    let _ = app.update(Message::PromptSave(id, true));
    let _ = app.update(Message::PromptSubmit(id));
    let answers = rx.try_recv().unwrap().unwrap().unwrap();
    assert_eq!(answers[0].as_str(), "typed-pw");

    let tab = app.tabs.iter().find(|t| t.id == id).unwrap();
    assert_eq!(tab.save_passwords.len(), 1);
    assert_eq!(tab.save_passwords[0].0, "u@web:22");
    assert!(saved(&secrets).is_empty(), "not before the connection succeeds");

    // Failed connection: nothing is stored.
    let _ = app.update(Message::Conn(id, crate::conn::ConnEvent::Done(crate::conn::Once::new(Err("auth failed".into())))));
    assert!(saved(&secrets).is_empty());
    assert!(app.tabs.iter().find(|t| t.id == id).unwrap().save_passwords.is_empty());
    assert!(app.tabs[0].prompt.is_none());
}

#[test]
fn saved_passwords_fill_only_auto_hops() {
    use coshell_ssh::{Auth, HostSpec};
    let secrets = MemSecrets::default();
    secrets.set("ops@10.0.0.1:22", "jump-pw").unwrap();
    secrets.set("app@10.0.0.2:2222", "target-pw").unwrap();
    let auto = || Auth::Auto { identity: None, password: None, passphrase: None };
    let mut chain = vec![
        HostSpec { host: "10.0.0.1".into(), port: 22, user: "ops".into(), auth: auto() },
        HostSpec { host: "10.0.0.2".into(), port: 2222, user: "app".into(), auth: Auth::Agent },
        HostSpec { host: "10.0.0.3".into(), port: 22, user: "app".into(), auth: auto() },
    ];
    crate::conn::fill_saved_passwords(&mut chain, &secrets);
    let pw = |h: &HostSpec| match &h.auth {
        Auth::Auto { password, .. } => password.as_ref().map(|p| p.as_str().to_string()),
        _ => None,
    };
    assert_eq!(pw(&chain[0]).as_deref(), Some("jump-pw"));
    assert!(matches!(chain[1].auth, Auth::Agent), "explicit auth is left alone");
    assert_eq!(pw(&chain[2]), None);
    assert!(!format!("{:?}", chain[0]).contains("jump-pw"), "Debug never prints the password");
}

/// Feed bytes straight into a tab's emulator (no shell round trip).
fn feed(app: &CoShell, id: crate::app::TabId, bytes: &[u8]) {
    use coshell_term::alacritty_terminal::vte::ansi::Processor;
    let tab = app.tabs.iter().find(|t| t.id == id).unwrap();
    let Phase::Live(s) = &tab.phase else { panic!("live tab") };
    let mut parser: Processor = Processor::new();
    parser.advance(&mut *s.term().lock(), bytes);
}

fn selection(app: &CoShell, id: crate::app::TabId) -> Option<String> {
    match &app.tabs.iter().find(|t| t.id == id).unwrap().phase {
        Phase::Live(s) => s.selection_text(),
        _ => None,
    }
}

#[test]
fn search_bar_finds_literal_text_and_reports_misses() {
    use crate::app::Shortcut;
    let mut app = app();
    let id = app.open_local();
    feed(&app, id, b"\r\nerror: a.b failed (code 3)\r\n");
    let _ = app.update(Message::Shortcut(Shortcut::Find));
    assert!(app.search.is_some());
    // The bar owns the keyboard while it is open.
    assert!(!app.terminal_active(app.active_tab().unwrap()));
    simulator(ui::view(&app)).find("터미널 출력 검색").expect("search input");

    // Regex metacharacters are taken literally.
    let _ = app.update(Message::SearchQuery("a.b failed (code".into()));
    assert_eq!(app.search.as_ref().unwrap().found, Some(true));
    assert_eq!(selection(&app, id).as_deref(), Some("a.b failed (code"));
    if let Ok(dir) = std::env::var("COSHELL_SNAPSHOT_DIR") {
        let mut ui = simulator(ui::view(&app));
        let _ = ui.snapshot(&iced::Theme::Dark).unwrap().matches_image(Path::new(&dir).join("search"));
    }

    let _ = app.update(Message::SearchQuery("no-such-text-zz".into()));
    assert_eq!(app.search.as_ref().unwrap().found, Some(false));
    simulator(ui::view(&app)).find("결과 없음").expect("miss is shown");

    let _ = app.update(Message::Shortcut(Shortcut::Escape));
    assert!(app.search.is_none());
    assert!(app.terminal_active(app.active_tab().unwrap()));
}

#[test]
fn zoom_shortcuts_change_clamp_and_persist_font_size() {
    use crate::app::Shortcut;
    use crate::settings::{FONT_SIZE_DEFAULT, FONT_SIZE_MIN, Settings};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.toml");
    let mut app = app();
    app.settings_path = Some(path.clone());
    let _ = app.update(Message::Shortcut(Shortcut::Zoom(1)));
    assert_eq!(app.settings.font_size, FONT_SIZE_DEFAULT + 1.0);
    let _ = app.update(Message::PersistState);
    assert_eq!(Settings::load(&path).unwrap().font_size, FONT_SIZE_DEFAULT + 1.0);
    for _ in 0..100 {
        let _ = app.update(Message::Shortcut(Shortcut::Zoom(-1)));
    }
    assert_eq!(app.settings.font_size, FONT_SIZE_MIN);
    let _ = app.update(Message::Shortcut(Shortcut::Zoom(0)));
    assert_eq!(app.settings.font_size, FONT_SIZE_DEFAULT);
}

#[test]
fn settings_dialog_validates_applies_and_saves() {
    use crate::app::Shortcut;
    use crate::settings::{CursorKind, Scheme, Settings};
    use coshell_term::alacritty_terminal::vte::ansi::CursorShape;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.toml");
    let mut app = app();
    app.settings_path = Some(path.clone());
    let id = app.open_local();

    let _ = app.update(Message::Shortcut(Shortcut::Settings));
    assert!(app.settings_draft.is_some());
    assert!(!app.terminal_active(app.active_tab().unwrap()));
    {
        let mut ui = simulator(ui::view(&app));
        ui.find("터미널 모양").expect("dialog");
        ui.find("터미널 출력 검색").expect("shortcut reference");
        if let Ok(dir) = std::env::var("COSHELL_SNAPSHOT_DIR") {
            let _ = ui.snapshot(&iced::Theme::Dark).unwrap().matches_image(Path::new(&dir).join("settings"));
        }
    }
    let _ = app.update(Message::SettingsScheme(Scheme::Light));
    let _ = app.update(Message::SettingsCursor(CursorKind::Beam));
    let _ = app.update(Message::SettingsOptionMeta(false));
    let _ = app.update(Message::SettingsFontSize(2));

    // Bad numbers keep the dialog open and change nothing.
    let _ = app.update(Message::SettingsScrollback("lots".into()));
    let _ = app.update(Message::SettingsSave);
    assert!(app.settings_draft.as_ref().is_some_and(|d| d.error.is_some()));
    assert_eq!(app.settings, Settings::default());
    assert!(!path.exists());

    let _ = app.update(Message::SettingsScrollback("500".into()));
    let _ = app.update(Message::SettingsAttachLines("120".into()));
    let _ = app.update(Message::SettingsSave);
    assert!(app.settings_draft.is_none());
    let want = Settings {
        scheme: Scheme::Light,
        cursor: CursorKind::Beam,
        option_as_meta: false,
        font_size: 15.0,
        scrollback: 500,
        ai_attach_lines: 120,
        ..Settings::default()
    };
    assert_eq!(app.settings, want);
    assert_eq!(Settings::load(&path).unwrap(), want);
    // Open sessions pick the change up at once.
    let Phase::Live(s) = &app.tabs.iter().find(|t| t.id == id).unwrap().phase else { panic!("live") };
    assert_eq!(s.term().lock().renderable_content().cursor.shape, CursorShape::Beam);
    if let Ok(dir) = std::env::var("COSHELL_SNAPSHOT_DIR") {
        feed(&app, id, b"\r\n\x1b[32mok\x1b[0m light scheme \x1b[31merror\x1b[0m\r\n");
        let mut ui = simulator(ui::view(&app));
        let _ = ui.snapshot(&iced::Theme::Dark).unwrap().matches_image(Path::new(&dir).join("light-scheme"));
    }

    // Cancel discards edits.
    let _ = app.update(Message::SettingsOpen);
    let _ = app.update(Message::SettingsScheme(Scheme::Dark));
    let _ = app.update(Message::Shortcut(Shortcut::Escape));
    assert_eq!(app.settings.scheme, Scheme::Light);
}

#[test]
fn window_size_and_ai_panel_are_remembered() {
    use crate::settings::Settings;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.toml");
    let mut app = app();
    app.settings_path = Some(path.clone());
    let _ = app.update(Message::WindowResized(iced::Size::new(900.0, 620.0)));
    let _ = app.update(Message::AiToggle);
    assert!(!path.exists(), "written after a delay, not on every event");
    let _ = app.update(Message::PersistState);
    let saved = Settings::load(&path).unwrap();
    assert_eq!((saved.window.width, saved.window.height), (900.0, 620.0));
    assert_eq!(saved.ai_panel_open, app.ai.open);
    assert!(!saved.ai_panel_open);
}

#[test]
fn exited_session_offers_restart() {
    let mut app = app();
    let id = app.open_local();
    simulator(ui::view(&app)).find("셸이 종료되었습니다").expect_err("no banner while running");
    let _ = app.update(Message::Term(id, coshell_term::TermEvent::Exited));
    let mut ui = simulator(ui::view(&app));
    ui.find("셸이 종료되었습니다").expect("banner");
    ui.click("새 셸 시작").expect("restart button");
    let messages: Vec<Message> = ui.into_messages().collect();
    assert!(messages.iter().any(|m| matches!(m, Message::Retry(t) if *t == id)), "{messages:?}");

    let _ = app.update(Message::Retry(id));
    let tab = app.tabs.iter().find(|t| t.id == id).unwrap();
    assert!(!tab.exited && matches!(tab.phase, Phase::Live(_)));

    // A dropped SSH session is worded as a connection.
    app.mark_remote(id);
    let _ = app.update(Message::Term(id, coshell_term::TermEvent::Exited));
    simulator(ui::view(&app)).find("다시 연결").expect("reconnect button");
}

#[test]
fn closing_a_connected_ssh_tab_asks_first() {
    use crate::app::Shortcut;
    let mut app = app();
    let local = app.open_local();
    let _ = app.update(Message::Close(local));
    assert!(app.tabs.is_empty(), "local shells close at once");

    let id = app.open_local();
    app.mark_remote(id);
    let _ = app.update(Message::Shortcut(Shortcut::CloseTab));
    assert_eq!((app.tabs.len(), app.close_confirm), (1, Some(id)));
    simulator(ui::view(&app)).find("연결 닫기").expect("confirm dialog");
    let _ = app.update(Message::Shortcut(Shortcut::Escape));
    assert_eq!((app.tabs.len(), app.close_confirm), (1, None));

    let _ = app.update(Message::Close(id));
    let _ = app.update(Message::CloseConfirmed(id));
    assert!(app.tabs.is_empty() && app.close_confirm.is_none());
}

#[test]
fn server_delete_needs_a_second_press() {
    let mut store = HostStore::default();
    store.upsert(None, HostDef { alias: "web".into(), hostname: Some("10.0.0.1".into()), ..HostDef::default() });
    let mut app = app_with("", store, None, ai_with("ollama", &[]));
    let _ = app.update(Message::HostEdit("web".into()));
    let mut ui = simulator(ui::view(&app));
    ui.find("정말 삭제").expect_err("not armed yet");
    ui.click("서버 삭제").expect("delete button");
    let messages: Vec<Message> = ui.into_messages().collect();
    assert!(messages.iter().any(|m| matches!(m, Message::HostDeleteAsk)), "{messages:?}");
    assert!(app.host_store.get("web").is_some());

    let _ = app.update(Message::HostDeleteAsk);
    simulator(ui::view(&app)).find("정말 삭제").expect("armed");
    let _ = app.update(Message::HostDelete);
    assert!(app.host_store.get("web").is_none());
}

fn mouse(e: iced::mouse::Event) -> iced::Event {
    iced::Event::Mouse(e)
}

/// A point inside the terminal grid on every platform's chrome layout.
const IN_TERMINAL_Y: f32 = 200.0;

#[test]
fn dragging_selects_text_and_typing_clears_it() {
    use iced::mouse::{Button, Event as M};
    let mut app = app();
    let id = app.open_local();
    feed(&app, id, "hello world\r\n".repeat(20).as_bytes());
    {
        let mut ui = simulator(ui::view(&app));
        ui.point_at((5.0, IN_TERMINAL_Y));
        let _ = ui.simulate([mouse(M::ButtonPressed(Button::Left))]);
        ui.point_at((600.0, IN_TERMINAL_Y));
        let _ = ui.simulate([mouse(M::CursorMoved { position: (600.0, IN_TERMINAL_Y).into() }), mouse(M::ButtonReleased(Button::Left))]);
    }
    assert_eq!(selection(&app, id).as_deref(), Some("hello world"));
    {
        // A plain click (no drag) leaves nothing selected.
        let mut ui = simulator(ui::view(&app));
        ui.point_at((50.0, IN_TERMINAL_Y));
        let _ = ui.simulate([mouse(M::ButtonPressed(Button::Left)), mouse(M::ButtonReleased(Button::Left))]);
    }
    assert_eq!(selection(&app, id), None);
}

#[test]
fn mouse_aware_programs_get_clicks_and_wheel() {
    use iced::mouse::{Button, Event as M, ScrollDelta};
    let mut app = app();
    let id = app.open_local();
    // DECSET 1000 (report clicks) + 1006 (SGR encoding), as vim/tmux/htop do.
    feed(&app, id, b"\x1b[?1000h\x1b[?1006h");
    let mut ui = simulator(ui::view(&app));
    ui.point_at((50.0, IN_TERMINAL_Y));
    let _ = ui.simulate([
        mouse(M::ButtonPressed(Button::Left)),
        mouse(M::ButtonReleased(Button::Left)),
        mouse(M::WheelScrolled { delta: ScrollDelta::Lines { x: 0.0, y: -1.0 } }),
    ]);
    let sent: Vec<String> = ui
        .into_messages()
        .filter_map(|m| match m {
            Message::Input(t, bytes) if t == id => Some(String::from_utf8_lossy(&bytes).into_owned()),
            _ => None,
        })
        .collect();
    assert_eq!(sent.len(), 3, "{sent:?}");
    assert!(sent[0].starts_with("\x1b[<0;") && sent[0].ends_with('M'), "{sent:?}");
    assert!(sent[1].starts_with("\x1b[<0;") && sent[1].ends_with('m'), "{sent:?}");
    assert!(sent[2].starts_with("\x1b[<65;") && sent[2].ends_with('M'), "{sent:?}");
    // The click went to the program, not into a local selection.
    assert_eq!(selection(&app, id), None);
}

#[test]
fn tabs_rename_step_and_reorder() {
    use crate::app::Shortcut;
    let mut app = app();
    let a = app.open_local();
    let b = app.open_local();
    let order = |app: &CoShell| app.tabs.iter().map(|t| t.id).collect::<Vec<_>>();

    let _ = app.update(Message::Shortcut(Shortcut::TabStep(1)));
    assert_eq!(app.active, Some(a), "wraps around");
    let _ = app.update(Message::Shortcut(Shortcut::TabMove(1)));
    assert_eq!(order(&app), [b, a]);
    let _ = app.update(Message::Shortcut(Shortcut::TabMove(1)));
    assert_eq!(order(&app), [b, a], "already last");

    // Second click on the active tab starts a rename.
    let _ = app.update(Message::Activate(a));
    assert!(app.rename.is_none());
    let _ = app.update(Message::Activate(a));
    assert!(app.rename.as_ref().is_some_and(|(id, _)| *id == a));
    assert!(!app.terminal_active(app.active_tab().unwrap()));
    let _ = app.update(Message::TabRename("  배포 로그\u{7}  ".into()));
    let _ = app.update(Message::Activate(a));
    assert!(app.rename.is_some(), "clicking inside the field keeps editing");
    let _ = app.update(Message::TabRenameCommit);
    assert_eq!(app.active_tab().unwrap().display_name(), "배포 로그");
    simulator(ui::view(&app)).find("배포 로그").expect("tab shows the new name");

    // Escape cancels; an empty name restores the automatic one.
    app.rename = Some((a, "x".into()));
    let _ = app.update(Message::Shortcut(Shortcut::Escape));
    assert_eq!(app.active_tab().unwrap().display_name(), "배포 로그");
    app.rename = Some((a, "  ".into()));
    let _ = app.update(Message::TabRenameCommit);
    assert!(app.active_tab().unwrap().custom_name.is_none());
}

#[test]
fn notices_are_toasts_and_unsafe_links_are_refused() {
    let mut app = app();
    assert!(app.notice.is_none());
    for bad in ["file:///etc/passwd", "javascript:alert(1)", "https://a.test/ --flag", "https://a.test/\u{1b}[0m"] {
        app.notice = None;
        let _ = app.update(Message::OpenLink(bad.into()));
        assert_eq!(app.notice.as_deref(), Some("열 수 없는 링크입니다"), "{bad}");
    }
    let mut ui = simulator(ui::view(&app));
    ui.find("열 수 없는 링크입니다").expect("toast");
    if let Ok(dir) = std::env::var("COSHELL_SNAPSHOT_DIR") {
        let _ = ui.snapshot(&iced::Theme::Dark).unwrap().matches_image(Path::new(&dir).join("toast"));
    }
    drop(ui);
    let _ = app.update(Message::NoticeDismiss);
    assert!(app.notice.is_none());
}

#[test]
fn ai_panel_width_follows_the_divider_within_limits() {
    use crate::settings::{AI_PANEL_W_MAX, AI_PANEL_W_MIN};
    let mut app = app();
    let _ = app.update(Message::WindowResized(iced::Size::new(1200.0, 760.0)));
    let _ = app.update(Message::AiResizeStart);
    let _ = app.update(Message::AiResizeTo(700.0));
    assert_eq!(app.settings.ai_panel_width, 500.0);
    let _ = app.update(Message::AiResizeTo(1190.0));
    assert_eq!(app.settings.ai_panel_width, AI_PANEL_W_MIN);
    let _ = app.update(Message::AiResizeTo(0.0));
    assert_eq!(app.settings.ai_panel_width, AI_PANEL_W_MAX);
    // A narrow window keeps room for the terminal.
    let _ = app.update(Message::WindowResized(iced::Size::new(800.0, 600.0)));
    let _ = app.update(Message::AiResizeTo(0.0));
    assert_eq!(app.settings.ai_panel_width, 480.0);
    let _ = app.update(Message::AiResizeEnd);
}

#[test]
fn tab_key_moves_focus_only_inside_forms() {
    let mut app = app();
    app.open_local();
    assert!(!app.form_open(), "in the terminal, Tab goes to the shell");
    let _ = app.update(Message::SettingsOpen);
    assert!(app.form_open());
    let _ = app.update(Message::SettingsClose);
    let _ = app.update(Message::PalettePick(crate::app::PaletteItem::NewHost));
    assert!(app.form_open());
    let _ = app.update(Message::HostEditorClose);
    assert!(!app.form_open());
}

#[test]
fn menu_paste_reaches_the_terminal_only_when_it_has_the_keyboard() {
    use crate::terminal::paste_bytes;
    assert_eq!(paste_bytes("a\r\nb\n", false), b"a\rb\r");
    assert_eq!(paste_bytes("x\x1b[201~y", true), b"\x1b[200~xy\x1b[201~");

    let mut app = app();
    let id = app.open_local();
    feed(&app, id, b"copy me\r\n");
    let _ = app.update(Message::Shortcut(crate::app::Shortcut::Find));
    let _ = app.update(Message::SearchQuery("copy me".into()));
    let _ = app.update(Message::SearchClose);
    assert_eq!(selection(&app, id).as_deref(), Some("copy me"), "what the Copy menu item copies");

    // Pasting while a dialog is open must not type into the shell (it would also clear the selection there).
    let _ = app.update(Message::SettingsOpen);
    let _ = app.update(Message::TermPasted(Some("rm -rf /\n".into())));
    assert!(!app.terminal_active(app.active_tab().unwrap()));
}

#[test]
fn settings_dialog_holds_servers_and_ai_pages() {
    use crate::app::{SettingsTab, Shortcut};
    let mut app = app();
    app.open_local();

    // The AI panel's settings button opens the AI page of the one dialog.
    let _ = app.update(Message::AiSettingsOpen);
    assert_eq!(app.settings_draft.as_ref().map(|d| d.tab), Some(SettingsTab::Ai));
    assert!(app.ai.settings.is_some());
    let _ = app.update(Message::AiSettingsName("Renamed".into()));

    // Servers page lists hosts; editing one opens the editor above the dialog, and Esc closes the editor first.
    let _ = app.update(Message::SettingsTab(SettingsTab::Servers));
    {
        let mut ui = simulator(ui::view(&app));
        ui.find("bastion").expect("server listed");
        if let Ok(dir) = std::env::var("COSHELL_SNAPSHOT_DIR") {
            let _ = ui.snapshot(&iced::Theme::Dark).unwrap().matches_image(Path::new(&dir).join("settings-servers"));
        }
    }
    let _ = app.update(Message::HostEdit("bastion".into()));
    let _ = app.update(Message::Shortcut(Shortcut::Escape));
    assert!(app.editor.is_none());
    assert!(app.settings_draft.is_some());

    // One Save stores the AI draft too.
    let _ = app.update(Message::SettingsSave);
    assert!(app.settings_draft.is_none() && app.ai.settings.is_none());
    assert_eq!(app.ai.active().name, "Renamed");

    // Cancel drops an AI draft.
    let _ = app.update(Message::AiSettingsOpen);
    let _ = app.update(Message::AiSettingsName("Dropped".into()));
    let _ = app.update(Message::Shortcut(Shortcut::Escape));
    assert!(app.ai.settings.is_none());
    assert_eq!(app.ai.active().name, "Renamed");
}

#[test]
fn ai_panel_toggles_from_the_title_bar_and_tabs_stay_compact() {
    let mut app = app();
    app.open_local();
    app.open_local();
    app.ai.open = true;
    let _ = app.update(Message::AiToggle);
    assert!(!app.ai.open);
    let mut ui = simulator(ui::view(&app));
    if let Ok(dir) = std::env::var("COSHELL_SNAPSHOT_DIR") {
        let _ = ui.snapshot(&iced::Theme::Dark).unwrap().matches_image(Path::new(&dir).join("ai-closed"));
    }
    assert!(ui.find("터미널 출력은 직접 첨부할 때만 전송됩니다").is_err(), "panel hidden");
    drop(ui);
    let _ = app.update(Message::AiToggle);
    simulator(ui::view(&app)).find("터미널 출력은 직접 첨부할 때만 전송됩니다").expect("panel shown");
}

/// Throwaway keys generated for these tests only.
const TEST_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACCupRzXAGYmowrqEFkAqjdtyvtD0QxKHmqOXVtSzFTGiQAAAIgfyi2uH8ot
rgAAAAtzc2gtZWQyNTUxOQAAACCupRzXAGYmowrqEFkAqjdtyvtD0QxKHmqOXVtSzFTGiQ
AAAEBanwDBbhObOrD2XHhiR/WzwthNSeXIqywbhUIIrD7yb66lHNcAZiajCuoQWQCqN23K
+0PRDEoeao5dW1LMVMaJAAAABHRlc3QB
-----END OPENSSH PRIVATE KEY-----
";
/// Passphrase `pw`.
const TEST_KEY_ENCRYPTED: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jdHIAAAAGYmNyeXB0AAAAGAAAABBVctzBIt
AnwQEs/5bNwgZ1AAAAGAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAIHEK6QZ//+nnRihM
7IJZ23aStYHnQ36HAvW7IeF5EaRAAAAAkNN+ucdNAcm3q4Sao1GJUCG/Ifr2QCorNxgLw1
7wx8sVRniKc+tgQITrG6Z6ERr/ANgTGV3ZpCLuKsUg8U85T4vxfzdbhqEYaxAny/YCod7n
J50yCVMeApycgdYG6T+PEQHaQvFzU3YpIFOGAaWLcSp6aSlSEP+0V/uwsoxtwi4Iia4FRo
5e/uW6mCMpqV44Ow==
-----END OPENSSH PRIVATE KEY-----
";

#[test]
fn ssh_keys_are_listed_imported_picked_and_deleted() {
    use crate::app::SettingsTab;
    let home = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();
    std::fs::create_dir(home.path().join(".ssh")).unwrap();
    std::fs::write(home.path().join(".ssh/id_ed25519"), TEST_KEY).unwrap();
    std::fs::write(home.path().join(".ssh/config"), "Host x\n").unwrap();
    std::fs::write(home.path().join(".ssh/known_hosts"), "").unwrap();
    let secrets = Arc::new(MemSecrets::default());
    let mut app = CoShell::with_parts(
        home.path().into(),
        String::new(),
        HostStore::default(),
        Some(cfg.path().join("hosts.toml")),
        ai_with("ollama", &[]),
        "me".into(),
        None,
        secrets.clone(),
    );

    // ~/.ssh keys show up; config / known_hosts do not.
    let _ = app.update(Message::SettingsOpen);
    let _ = app.update(Message::SettingsTab(SettingsTab::Keys));
    let names: Vec<_> = app.keys.iter().map(|k| (k.name.as_str(), k.info.algorithm.as_str(), k.managed)).collect();
    assert_eq!(names, [("id_ed25519", "ED25519", false)]);

    // Import from a file: name taken from the file, saved owner-only with its public half.
    let _ = app.update(Message::KeyImportOpen);
    let _ = app.update(Message::KeyImportLoaded(Some(("work.pem".into(), TEST_KEY.into()))));
    let _ = app.update(Message::KeyImportNote("배포용".into()));
    {
        let mut ui = simulator(ui::view(&app));
        ui.find("✓ ED25519 키").expect("key recognised");
        if let Ok(dir) = std::env::var("COSHELL_SNAPSHOT_DIR") {
            let _ = ui.snapshot(&iced::Theme::Dark).unwrap().matches_image(Path::new(&dir).join("key-import"));
        }
    }
    let _ = app.update(Message::KeyImportSave);
    assert!(app.key_import.is_none(), "{:?}", app.key_import.as_ref().and_then(|k| k.error.clone()));
    let saved_key = cfg.path().join("keys/work.pem");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&saved_key).unwrap().permissions().mode() & 0o777, 0o600);
    }
    assert!(std::fs::read_to_string(cfg.path().join("keys/work.pem.pub")).unwrap().starts_with("ssh-ed25519 "));
    let work = app.keys.iter().find(|k| k.name == "work.pem").expect("listed");
    assert!(work.managed);
    assert_eq!(work.note.as_deref(), Some("배포용"));

    // Encrypted key: a wrong passphrase is refused; the right one goes to the keychain only.
    let _ = app.update(Message::KeyImportOpen);
    let _ = app.update(Message::KeyImportName("locked".into()));
    let _ = app.update(Message::KeyImportLoaded(Some(("e".into(), TEST_KEY_ENCRYPTED.into()))));
    let _ = app.update(Message::KeyImportPassphrase("wrong".into()));
    let _ = app.update(Message::KeyImportSave);
    assert!(app.key_import.as_ref().is_some_and(|k| k.error.is_some()));
    let _ = app.update(Message::KeyImportPassphrase("pw".into()));
    let _ = app.update(Message::KeyImportSave);
    assert!(app.key_import.is_none());
    let locked = cfg.path().join("keys/locked");
    assert_eq!(saved(&secrets), [(format!("key:{}", locked.display()), "pw".to_string())]);
    {
        let mut ui = simulator(ui::view(&app));
        if let Ok(dir) = std::env::var("COSHELL_SNAPSHOT_DIR") {
            let _ = ui.snapshot(&iced::Theme::Dark).unwrap().matches_image(Path::new(&dir).join("keys-page"));
        }
        ui.find("locked").expect("card");
    }

    // Picked in the server editor, stored as a ~ path.
    let _ = app.update(Message::HostNewIn(None));
    let _ = app.update(Message::HostAdvanced);
    let _ = app.update(Message::HostField(HostField::Identity, "~/.ssh/id_ed25519".into()));
    let chosen = app.editor.as_ref().unwrap().def.identity_file.clone();
    assert_eq!(chosen.as_deref(), Some("~/.ssh/id_ed25519"));
    assert_eq!(crate::ui_keys::chosen_key_label(&app, chosen.as_deref()), "id_ed25519 · ED25519");
    assert_eq!(crate::ui_keys::chosen_key_label(&app, None), "자동 (ssh-agent · 기본 키)");
    assert_eq!(crate::ui_keys::chosen_key_label(&app, Some("/elsewhere/k")), "직접 지정: /elsewhere/k");
    if let Ok(dir) = std::env::var("COSHELL_SNAPSHOT_DIR") {
        let mut ui = simulator(ui::view(&app));
        let _ = ui.snapshot(&iced::Theme::Dark).unwrap().matches_image(Path::new(&dir).join("key-picker"));
    }
    let _ = app.update(Message::HostEditorClose);

    // Deleting needs a second click and removes the file, side files and passphrase.
    let _ = app.update(Message::KeyDeleteAsk(locked.clone()));
    assert!(locked.exists());
    let _ = app.update(Message::KeyDelete(locked.clone()));
    assert!(!locked.exists() && !cfg.path().join("keys/locked.pub").exists());
    assert!(saved(&secrets).is_empty());
    // ~/.ssh keys are never deleted from the app.
    let _ = app.update(Message::KeyDelete(home.path().join(".ssh/id_ed25519")));
    assert!(home.path().join(".ssh/id_ed25519").exists());
}

#[test]
fn saved_key_passphrase_is_handed_to_the_connection() {
    use coshell_ssh::{Auth, HostSpec};
    let secrets = MemSecrets::default();
    secrets.set("key:/k/work", "pw").unwrap();
    let mut chain = vec![HostSpec {
        host: "h".into(),
        port: 22,
        user: "u".into(),
        auth: Auth::Auto { identity: Some("/k/work".into()), password: None, passphrase: None },
    }];
    crate::conn::fill_saved_passwords(&mut chain, &secrets);
    assert!(matches!(&chain[0].auth, Auth::Auto { passphrase: Some(p), .. } if p.as_str() == "pw"));
}

/// A tab whose terminal has no shell behind it: only what a test feeds shows up,
/// and whatever the app types goes nowhere (no timing races with a real shell).
fn quiet_tab(app: &mut CoShell) -> crate::app::TabId {
    use coshell_term::{GridSize, Session, Transport};
    struct Sink;
    impl Transport for Sink {
        fn write(&self, _: &[u8]) {}
        fn resize(&self, _: GridSize) {}
    }
    let id = app.open_local();
    let tab = app.tabs.iter_mut().find(|t| t.id == id).unwrap();
    tab.phase = Phase::Live(Session::new(Arc::new(Sink), Box::new(std::io::Cursor::new(Vec::new())), GridSize::DEFAULT));
    id
}

/// Simulate a finished reply in tab `id` (the terminal sits at an idle prompt).
fn reply(app: &mut CoShell, id: crate::app::TabId, question: &str, answer: &str) {
    feed(app, id, b"\x1b[2J\x1b[3J\x1b[Hme@host:~ $ ");
    let tab = app.tabs.iter_mut().find(|t| t.id == id).unwrap();
    tab.chat.turns.push(turn(Role::User, question));
    tab.chat.turns.push(Turn { text: String::new(), ..turn(Role::Assistant, "") });
    let _ = app.update(Message::AiEvent(id, Ok(StreamEvent::Delta(answer.to_string()))));
    let _ = app.update(Message::AiEvent(id, Ok(StreamEvent::Done { stop_reason: Some("end_turn".into()) })));
}

fn chat_of(app: &CoShell, id: crate::app::TabId) -> &Chat {
    &app.tabs.iter().find(|t| t.id == id).unwrap().chat
}

fn ran(lines: &str, _exit: u32) -> Message {
    Message::AgentDone(0, Ok(crate::app::ScreenRun { text: lines.trim_end().to_string(), finished: true }))
}

#[test]
fn agent_runs_a_read_only_command_reads_it_and_carries_on() {
    use crate::ai::ExecState;
    let mut app = app();
    let id = quiet_tab(&mut app);
    assert!(app.settings.ai_agent, "on by default");

    reply(&mut app, id, "목록 보여줘", "홈 폴더부터 볼게요.\n```bash\nls -al\n```\n");
    let chat = chat_of(&app, id);
    let exec = chat.turns.last().unwrap().exec.as_ref().expect("command started");
    assert_eq!((exec.cmd.as_str(), &exec.state), ("ls -al", &ExecState::Running));
    assert!(chat.is_streaming(), "busy while it runs: send is blocked, stop cancels");

    // Result comes back: shown on the command, fed to the model as a hidden turn, next answer streams.
    let Message::AgentDone(_, r) = ran("devora-ai\ndevora-gateway\napi_key=sk-live-0123456789abcdef\n", 0) else { unreachable!() };
    let _ = app.update(Message::AgentDone(id, r));
    let chat = chat_of(&app, id);
    let n = chat.turns.len();
    assert_eq!(n, 4, "user, assistant(command), hidden result, new assistant");
    let exec = chat.turns[1].exec.as_ref().unwrap();
    assert!(exec.sent);
    let ExecState::Done { output, exit, secrets } = &exec.state else { panic!("{:?}", exec.state) };
    assert_eq!((*exit, *secrets), (None, 1), "read off the screen: no exit code");
    assert!(!output.contains("sk-live"), "secrets masked before anything leaves");
    let hidden = &chat.turns[2];
    assert!(hidden.hidden && hidden.role == Role::User);
    assert!(hidden.sent.starts_with("<command_result>\n$ ls -al\ndevora-ai"), "{}", hidden.sent);
    assert!(chat.is_streaming() && chat.turns[3].role == Role::Assistant);

    // The chat shows one compact line for the command, never the hidden turn; output opens on click.
    let mut ui = simulator(ui::view(&app));
    ui.find("실행함 · 3줄").expect("compact command line");
    assert!(ui.find("<command_result").is_err());
    drop(ui);
    let _ = app.update(Message::AgentToggle(id, 1));
    simulator(ui::view(&app)).find("devora-ai\ndevora-gateway\napi_key=••••••••").expect("output shown");
    let _ = app.update(Message::AgentToggle(id, 1));

    // The model answers from the result, in Markdown; no command, so nothing more runs.
    let summary = "홈 디렉터리 `/home/hwr` 목록이에요.\n\n**폴더**\n\n| 이름 | 수정일 |\n|---|---|\n| devora-ai/ | 11월 12일 |\n| devora-gateway/ | 11월 4일 |\n\n- 설정 파일에 키가 하나 보여서 가렸어요.\n\n안쪽 폴더도 볼까요?";
    let _ = app.update(Message::AiEvent(id, Ok(StreamEvent::Delta(summary.into()))));
    let _ = app.update(Message::AiEvent(id, Ok(StreamEvent::Done { stop_reason: None })));
    assert!(chat_of(&app, id).turns.last().unwrap().exec.is_none());
    // Rich text is not searchable by the simulator; the snapshot shows the table and bold text.
    let mut ui = simulator(ui::view(&app));
    if let Ok(dir) = std::env::var("COSHELL_SNAPSHOT_DIR") {
        let _ = ui.snapshot(&iced::Theme::Dark).unwrap().matches_image(Path::new(&dir).join("ai-agent"));
    }
}

#[test]
fn agent_holds_back_on_risk_production_and_long_chains() {
    let mut app = app();
    let id = quiet_tab(&mut app);
    let started = |app: &CoShell| chat_of(app, id).turns.last().unwrap().exec.is_some();

    // Changes are never run for the user; a placeholder can not run.
    for answer in ["```bash\nsudo systemctl restart nginx\n```", "```bash\nrm -rf /nonexistent-coshell\n```", "```bash\npodman logs <name>\n```", "그냥 설명만 할게요."] {
        let _ = app.update(Message::AiNewChat(id));
        reply(&mut app, id, "q", answer);
        assert!(!started(&app), "{answer}");
    }

    // Production: runs, but the result waits for a click before it goes to the model.
    let _ = app.update(Message::AiNewChat(id));
    app.tabs.iter_mut().find(|t| t.id == id).unwrap().env = Env::Prod;
    reply(&mut app, id, "상태", "```bash\nuptime\n```");
    let Message::AgentDone(_, r) = ran("up 8 days\n", 0) else { unreachable!() };
    let _ = app.update(Message::AgentDone(id, r));
    assert_eq!(chat_of(&app, id).turns.len(), 2, "nothing sent yet");
    simulator(ui::view(&app)).find("AI에 보내기").expect("asks first");
    let _ = app.update(Message::AgentSend(id));
    assert_eq!(chat_of(&app, id).turns.len(), 4);
    app.tabs.iter_mut().find(|t| t.id == id).unwrap().env = Env::None;

    // At most AGENT steps in a row, then it waits for the user.
    let _ = app.update(Message::AiNewChat(id));
    reply(&mut app, id, "q", "```bash\nuptime\n```");
    for _ in 0..10 {
        if !started(&app) {
            break;
        }
        let Message::AgentDone(_, r) = ran("x\n", 0) else { unreachable!() };
        let _ = app.update(Message::AgentDone(id, r));
        feed(&app, id, b"\r\nme@host:~ $ ");
        let _ = app.update(Message::AiEvent(id, Ok(StreamEvent::Delta("```bash\nuptime\n```".into()))));
        let _ = app.update(Message::AiEvent(id, Ok(StreamEvent::Done { stop_reason: None })));
    }
    assert_eq!(chat_of(&app, id).agent_steps(), 4);
    assert!(!started(&app));

    // Off: the assistant only suggests.
    let _ = app.update(Message::AiAutoRun);
    let _ = app.update(Message::AiNewChat(id));
    reply(&mut app, id, "q", "```bash\nuptime\n```");
    assert!(!started(&app));
}

#[test]
fn agent_reads_the_command_off_the_terminal_it_typed_into() {
    use coshell_term::{GridSize, Session, Transport};
    struct Sink;
    impl Transport for Sink {
        fn write(&self, _: &[u8]) {}
        fn resize(&self, _: GridSize) {}
    }
    // What the user's terminal shows once the command has run.
    let screen = b"Last login: today\r\nhwr@pi:~ $ ls -al\r\ndevora-ai\r\ndevora-gateway\r\nhwr@pi:~ $ ";
    let s = Session::new(std::sync::Arc::new(Sink), Box::new(std::io::Cursor::new(&screen[..])), GridSize::DEFAULT);
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let run = rt.block_on(crate::app::watch_screen(s.term().clone(), "hwr@pi:~ $".into(), "ls -al".into())).unwrap();
    assert_eq!(run, crate::app::ScreenRun { text: "devora-ai\ndevora-gateway".into(), finished: true });
}

#[test]
fn ai_is_told_where_commands_run() {
    let mut app = app();
    let id = app.open_local();
    let tab = app.tabs.iter_mut().find(|t| t.id == id).unwrap();
    assert_eq!(crate::ui_ai::target_label(tab), "local shell on this computer");
    // The title (usually user@host: dir) comes from the shell: kept plain and short.
    tab.title = "hwr@raspberrypi: ~/apps\x07\"x".into();
    assert_eq!(crate::ui_ai::target_label(tab), "local shell on this computer, terminal title \"hwr@raspberrypi: ~/appsx\"");
    tab.title = "y".repeat(500);
    assert!(crate::ui_ai::target_label(tab).len() < 150);
}

#[test]
fn ai_profiles_take_custom_paths_auth_headers_and_parameters() {
    use crate::app::{AdvField, SettingsTab};
    use coshell_ai::{AuthStyle, Header};
    use iced::widget::text_editor::{Action, Edit};
    let mut app = app_with("", HostStore::default(), None, ai_with("claude", &[]));
    let _ = app.update(Message::AiSettingsOpen);
    let _ = app.update(Message::AiSettingsSelect(0)); // Claude (Anthropic)
    let _ = app.update(Message::AiSettingsAdvanced);

    let _ = app.update(Message::AiSettingsAdv(AdvField::ChatPath, "/anthropic/v1/messages".into()));
    let _ = app.update(Message::AiSettingsAuth(AuthStyle::Bearer));
    let _ = app.update(Message::AiSettingsAdv(AdvField::MaxTokens, "8192".into()));
    let _ = app.update(Message::AiSettingsAdv(AdvField::Temperature, "0.2".into()));
    let type_into = |app: &mut CoShell, text: &str, on: fn(Action) -> Message| {
        for c in text.chars() {
            let _ = app.update(on(Action::Edit(if c == '\n' { Edit::Enter } else { Edit::Insert(c) })));
        }
    };
    // A bad header line blocks saving and says why.
    type_into(&mut app, "X-Team ops", Message::AiSettingsHeaders);
    assert!(app.ai.settings.as_ref().unwrap().adv.error.is_some());
    let _ = app.update(Message::SettingsSave);
    let d = app.settings_draft.as_ref().expect("still open");
    assert_eq!(d.tab, SettingsTab::Ai);
    assert!(d.error.as_deref().is_some_and(|e| e.contains("헤더")), "{:?}", d.error);

    let _ = app.update(Message::AiSettingsHeaders(Action::SelectAll));
    type_into(&mut app, "X-Team: ops", Message::AiSettingsHeaders);
    assert_eq!(app.settings_draft.as_ref().unwrap().error, None, "fixed input clears the old save error");
    type_into(&mut app, r#"{"thinking": {"type": "enabled", "budget_tokens": 2000}}"#, Message::AiSettingsExtra);
    assert_eq!(app.ai.settings.as_ref().unwrap().adv.error, None);
    if let Ok(dir) = std::env::var("COSHELL_SNAPSHOT_DIR") {
        let mut ui = simulator(ui::view(&app));
        let _ = ui.snapshot(&iced::Theme::Dark).unwrap().matches_image(Path::new(&dir).join("ai-advanced"));
    }
    let _ = app.update(Message::SettingsSave);
    assert!(app.settings_draft.is_none(), "saved");

    let a = &app.ai.store.profiles[0].advanced;
    assert_eq!(a.chat_path, "/anthropic/v1/messages");
    assert_eq!(a.auth, AuthStyle::Bearer);
    assert_eq!(a.headers, [Header { name: "X-Team".into(), value: "ops".into() }]);
    assert_eq!((a.max_tokens, a.temperature), (Some(8192), Some(0.2)));
    assert!(a.extra_body.contains("budget_tokens"));
    assert_eq!(coshell_ai::chat_url(&app.ai.store.profiles[0]), "https://api.anthropic.com/anthropic/v1/messages");

    // Reopened: the typed fields come back as they were saved.
    let _ = app.update(Message::AiSettingsOpen);
    let adv = &app.ai.settings.as_ref().unwrap().adv;
    assert_eq!((adv.max_tokens.as_str(), adv.temperature.as_str(), adv.headers.text().trim()), ("8192", "0.2", "X-Team: ops"));
}

/// Product screenshots for the README (demo data only).
/// `COSHELL_README_DIR=docs/images cargo test -p coshell-app readme_screenshots -- --ignored`
#[test]
#[ignore = "writes README images"]
fn readme_screenshots() {
    use crate::ai::{Exec, ExecState};
    use crate::app::SettingsTab;
    let Ok(dir) = std::env::var("COSHELL_README_DIR") else { return };
    let dir = Path::new(&dir).to_path_buf();
    let shot = |app: &CoShell, name: &str| {
        let mut ui = simulator(ui::view(app));
        let _ = ui.snapshot(&iced::Theme::Dark).unwrap().matches_image(dir.join(name));
    };
    let def = |alias: &str, name: &str, group: &str, env: Env, host: &str, user: &str, jump: Option<&str>| HostDef {
        alias: alias.into(),
        name: Some(name.into()),
        group: Some(group.into()),
        env,
        hostname: Some(host.into()),
        user: Some(user.into()),
        proxy_jump: jump.map(String::from),
        ..HostDef::default()
    };
    let mut store = HostStore::default();
    for d in [
        def("web-01", "웹 서버 1", "서비스", Env::Prod, "10.0.1.21", "deploy", Some("bastion")),
        def("web-02", "웹 서버 2", "서비스", Env::Prod, "10.0.1.22", "deploy", Some("bastion")),
        def("db-01", "주 DB", "서비스", Env::Prod, "10.0.2.10", "dba", Some("bastion")),
        def("stage", "스테이징", "서비스", Env::Stage, "10.0.9.5", "deploy", Some("bastion")),
        def("pi", "라즈베리파이", "홈랩", Env::Dev, "192.0.2.36", "pi", None),
        def("nas", "NAS", "홈랩", Env::None, "192.0.2.40", "admin", None),
    ] {
        store.upsert(None, d);
    }
    let cfg = "Host bastion\n HostName 203.0.113.10\n User ec2-user\n";

    // 1. Hero: a production session where the assistant checks for itself and explains.
    let mut app = app_with(cfg, store.clone(), None, ai_with("claude", &[("claude", "k")]));
    let _ = quiet_tab(&mut app);
    let id = app.open_fake_ssh("web-01", &["ec2-user@bastion", "deploy@web-01"], &[38, 41]);
    {
        use coshell_term::{GridSize, Session, Transport};
        struct Sink;
        impl Transport for Sink {
            fn write(&self, _: &[u8]) {}
            fn resize(&self, _: GridSize) {}
        }
        let tab = app.tabs.iter_mut().find(|t| t.id == id).unwrap();
        tab.phase = Phase::Live(Session::new(Arc::new(Sink), Box::new(std::io::Cursor::new(Vec::new())), GridSize::DEFAULT));
        tab.env = Env::Prod;
        tab.title = "deploy@web-01: ~".into();
        tab.route = vec!["ec2-user@bastion".into(), "deploy@web-01".into()];
    }
    feed(
        &app,
        id,
        concat!(
            "\x1b[2J\x1b[H\x1b[1;32mdeploy@web-01\x1b[0m:\x1b[1;34m~\x1b[0m$ sudo systemctl status nginx --no-pager | head -4\r\n",
            "\x1b[31m×\x1b[0m nginx.service - A high performance web server and a reverse proxy server\r\n",
            "     Loaded: loaded (/lib/systemd/system/nginx.service; enabled; preset: enabled)\r\n",
            "     Active: \x1b[1;31mfailed\x1b[0m (Result: exit-code) since Fri 2026-11-13 09:12:44 KST; 3min ago\r\n",
            "    Process: 4182 ExecStart=/usr/sbin/nginx -g daemon on; (code=exited, status=1/FAILURE)\r\n",
            "\x1b[1;32mdeploy@web-01\x1b[0m:\x1b[1;34m~\x1b[0m$ sudo ss -tlnp | grep :80\r\n",
            "LISTEN 0      511          0.0.0.0:80        0.0.0.0:*    users:((\"apache2\",pid=912,fd=4))\r\n",
            "\x1b[1;32mdeploy@web-01\x1b[0m:\x1b[1;34m~\x1b[0m$ "
        )
        .as_bytes(),
    );
    {
        let model = "claude-sonnet-5-5";
        let t = |role, text: &str| Turn { model: model.into(), ..turn(role, text) };
        let chat = &mut app.tabs.iter_mut().find(|t| t.id == id).unwrap().chat;
        chat.turns.push(t(Role::User, "nginx가 왜 안 떠?"));
        chat.turns.push(Turn {
            exec: Some(Exec {
                cmd: "sudo ss -tlnp | grep :80".into(),
                state: ExecState::Done {
                    output: "LISTEN 0 511 0.0.0.0:80 0.0.0.0:* users:((\"apache2\",pid=912,fd=4))".into(),
                    exit: None,
                    secrets: 0,
                },
                sent: true,
                open: false,
            }),
            ..t(Role::Assistant, "80번 포트를 누가 쓰고 있는지부터 볼게요.\n```bash\nsudo ss -tlnp | grep :80\n```")
        });
        chat.turns.push(Turn { hidden: true, ..t(Role::User, "") });
        chat.turns.push(t(
            Role::Assistant,
            "**apache2**가 80번 포트를 잡고 있어서 nginx가 시작을 못 하고 있어요.\n\n| 포트 | 프로세스 | PID |\n|---|---|---|\n| 80 | apache2 | 912 |\n\napache2를 멈추고 nginx를 다시 올리면 됩니다. 운영 서버라 실행은 직접 눌러 주세요.\n```bash\nsudo systemctl stop apache2 && sudo systemctl restart nginx\n```",
        ));
    }
    shot(&app, "hero");

    // 2. Server board: groups, environments, jump hosts.
    let mut app = app_with(cfg, store.clone(), None, ai_with("claude", &[]));
    app.ai.open = false;
    shot(&app, "servers");

    // 3. Server editor: block-built auto tunnel.
    let _ = app.update(Message::HostEdit("db-01".into()));
    if !app.editor.as_ref().unwrap().advanced {
        let _ = app.update(Message::HostAdvanced);
    }
    let _ = app.update(Message::TunnelPreset(3306));
    shot(&app, "server-editor");

    // 4. SSH keys.
    let home = tempfile::tempdir().unwrap();
    let conf = tempfile::tempdir().unwrap();
    std::fs::create_dir(home.path().join(".ssh")).unwrap();
    std::fs::write(home.path().join(".ssh/id_ed25519"), TEST_KEY).unwrap();
    let mut app = CoShell::with_parts(
        home.path().into(),
        cfg.into(),
        store.clone(),
        Some(conf.path().join("hosts.toml")),
        ai_with("claude", &[]),
        "me".into(),
        None,
        Arc::new(MemSecrets::default()),
    );
    app.ai.open = false;
    for (name, key, note) in [("work-deploy", TEST_KEY, "운영 배포용"), ("homelab", TEST_KEY_ENCRYPTED, "")] {
        let _ = app.update(Message::SettingsOpen);
        let _ = app.update(Message::KeyImportOpen);
        let _ = app.update(Message::KeyImportName(name.into()));
        let _ = app.update(Message::KeyImportLoaded(Some((name.into(), key.into()))));
        let _ = app.update(Message::KeyImportNote(note.into()));
        if key == TEST_KEY_ENCRYPTED {
            let _ = app.update(Message::KeyImportPassphrase("pw".into()));
        }
        let _ = app.update(Message::KeyImportSave);
    }
    let _ = app.update(Message::SettingsTab(SettingsTab::Keys));
    shot(&app, "ssh-keys");

    // 5. AI providers: Anthropic / OpenAI compatible, advanced options.
    let _ = app.update(Message::SettingsTab(SettingsTab::Ai));
    let _ = app.update(Message::AiSettingsSelect(1));
    let _ = app.update(Message::AiSettingsAdvanced);
    let _ = app.update(Message::AiSettingsAdv(crate::app::AdvField::MaxTokens, "8192".into()));
    let _ = app.update(Message::AiSettingsAdv(crate::app::AdvField::Temperature, "0.2".into()));
    shot(&app, "ai-settings");
}

#[test]
fn korean_being_composed_is_drawn_inline() {
    let mut app = app();
    let _ = quiet_tab(&mut app);
    fn compose(app: &CoShell) -> iced_test::Simulator<'_, Message> {
        use iced::advanced::input_method;
        let mut ui = simulator(ui::view(app));
        ui.click(iced::widget::Id::new(crate::ai::AI_INPUT)).expect("composer");
        let _ = ui.simulate([
            iced::Event::InputMethod(input_method::Event::Opened),
            iced::Event::InputMethod(input_method::Event::Preedit("한글입ㄹ".into(), None)),
            iced::Event::Window(iced::window::Event::RedrawRequested(std::time::Instant::now())),
        ]);
        ui
    }
    // The app hears that composing started…
    let msgs: Vec<Message> = compose(&app).into_messages().collect();
    assert!(msgs.iter().any(|m| matches!(m, Message::Composing(true))), "{msgs:?}");
    for m in msgs {
        let _ = app.update(m);
    }
    assert!(app.composing);
    // …so the editor's placeholder steps aside and the syllable shows inline, no box or underline.
    let mut ui = compose(&app);
    assert!(ui.find("이 세션에 대해 질문하거나 명령을 요청하세요").is_err());
    if let Ok(dir) = std::env::var("COSHELL_SNAPSHOT_DIR") {
        let _ = ui.snapshot(&iced::Theme::Dark).unwrap().matches_image(Path::new(&dir).join("ime-preedit"));
    }
}
