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
    Turn { role, text: text.into(), sent: text.into(), attachment: None, model: "m".into(), stopped: false }
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

    let req = Ai::request(&chat, "SSH ubuntu@web (via ec2-user@bastion)");
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
fn ui_insert_button_types_command_without_running_it() {
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
    ui.click("터미널에 입력").expect("insert button visible");
    let messages: Vec<Message> = ui.into_messages().collect();
    assert!(
        messages.iter().any(|m| matches!(m, Message::AiInsert(t, c) if *t == id && c == "sudo ss -tulpn | grep :80")),
        "{messages:?}"
    );
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
    let _ = app.update(Message::HostForwardInput("L:8080:127.0.0.1:80".into()));
    let _ = app.update(Message::HostForwardAdd);
    if let Ok(dir) = std::env::var("COSHELL_SNAPSHOT_DIR") {
        let mut ui = simulator(ui::view(&app));
        let _ = ui.snapshot(&iced::Theme::Dark).unwrap().matches_image(Path::new(&dir).join("host-editor"));
    }
    let _ = app.update(Message::HostSave(false));
    assert!(app.editor.is_none(), "saved: {:?}", app.editor.as_ref().and_then(|e| e.error.clone()));

    let saved = HostStore::load(&path).unwrap();
    let def = saved.get("prod-web").unwrap();
    assert_eq!((def.env, def.hostname.as_deref(), def.proxy_jump.as_deref()), (Env::Prod, Some("10.0.1.42"), Some("bastion")));
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
fn prod_tab_gets_frame_and_banner() {
    let mut app = app();
    let id = app.open_local();
    app.tabs.iter_mut().find(|t| t.id == id).unwrap().env = Env::Prod;
    let mut ui = simulator(ui::view(&app));
    ui.find("PRODUCTION").expect("corner banner");
    ui.find("운영").expect("env tag");
    if let Ok(dir) = std::env::var("COSHELL_SNAPSHOT_DIR") {
        let _ = ui.snapshot(&iced::Theme::Dark).unwrap().matches_image(Path::new(&dir).join("prod-tab"));
    }
}

fn add_via_paste(app: &mut CoShell, group: Option<&str>, name: &str, ssh: &str) {
    let _ = app.update(Message::HostNewIn(group.map(String::from)));
    let _ = app.update(Message::HostField(HostField::Quick, ssh.into()));
    let _ = app.update(Message::HostQuickApply);
    let _ = app.update(Message::HostField(HostField::Name, name.into()));
    let _ = app.update(Message::HostSave(false));
    assert!(app.editor.is_none(), "saved {name}: {:?}", app.editor.as_ref().and_then(|e| e.error.clone()));
}

#[test]
fn servers_added_by_pasting_ssh_commands_with_groups_and_jump() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hosts.toml");
    let mut app = app_with("", HostStore::default(), Some(path.clone()), ai_with("ollama", &[]));

    add_via_paste(&mut app, Some("hanwha"), "jumphost 서버", "ssh -p 41 root@203.0.113.83");
    add_via_paste(&mut app, Some("hanwha"), "테스트 서버", "ssh -J jumphost root@10.240.0.7:41");
    add_via_paste(&mut app, Some("Local"), "jumphost 서버", "pi@192.0.2.36");

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

    // The pasted -J resolves to a two-hop chain.
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
    for t in ["새 서버", "ssh 명령으로 채우기", "주소", "고급 설정", "경유 서버", "SSH 키 파일", "자동 터널", "저장하고 연결"] {
        ui.find(t).unwrap_or_else(|_| panic!("{t} missing"));
    }
    for gone in ["jumphost 서버", "ProxyJump", "IdentityFile", "새 호스트"] {
        assert!(ui.find(gone).is_err(), "{gone} should not appear");
    }
}

#[test]
fn version_is_shown_in_status_bar_and_about() {
    assert_eq!(crate::VERSION, "0.2.0");
    let mut app = app();
    let mut ui = simulator(ui::view(&app));
    ui.click("v0.2.0").expect("status bar version");
    for m in ui.into_messages() {
        let _ = app.update(m);
    }
    assert!(app.about);
    let mut ui = simulator(ui::view(&app));
    ui.find("버전 0.2.0").expect("about dialog");
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
    let _ = app.update(Message::HostField(HostField::Quick, "ssh -p 2222 deploy@203.0.113.7".into()));
    let _ = app.update(Message::HostQuickApply);
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
    add_via_paste(&mut app, None, "a", "root@198.51.100.4");
    add_via_paste(&mut app, None, "b", "root@198.51.100.4");
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
    let auto = || Auth::Auto { identity: None, password: None };
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
