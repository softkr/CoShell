//! Tunnel popover (opened from the status bar): purpose-first forwarding UI.

use crate::i18n::{tf, tr};
use std::sync::atomic::Ordering;

use coshell_ssh::ForwardSpec;
use iced::widget::{Space, button, checkbox, column, container, row, scrollable, text, text_input};
use iced::{Alignment, Background, Border, Color, Font, Length, Padding};

use crate::app::{CoShell, Message, Tab, TunnelEntry, TunnelField, TunnelForm, TunnelState, form_spec};
use crate::theme::{self, alpha};
use crate::ui::{Btn, El, btn, btn_style, card_style, input_style, mono, muted};

const POPOVER_W: f32 = 520.0;
const POPOVER_MAX_H: f32 = 620.0;
/// Distance from the window bottom (status bar + gap).
const POPOVER_BOTTOM: f32 = 28.0;

/// Presets for "server port → this PC".
const SERVER_PRESETS: [(&str, u16); 7] =
    [("MySQL", 3306), ("PostgreSQL", 5432), ("Redis", 6379), ("MongoDB", 27017), ("HTTP", 80), ("웹 8080", 8080), ("Elasticsearch", 9200)];
/// Presets for "this PC → server".
const LOCAL_PRESETS: [(&str, u16); 4] = [("개발 서버 3000", 3000), ("웹 8080", 8080), ("Vite 5173", 5173), ("HTTP 80", 80)];

pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 { format!("{n} B") } else { format!("{v:.1} {}", UNITS[u]) }
}

/// Ports worth an "open in browser" button; `Some(true)` = https.
pub fn web_port(p: u16) -> Option<bool> {
    match p {
        443 | 8443 => Some(true),
        80 | 3000 | 3001 | 4200 | 5000 | 5173 | 8000 | 8080 | 8081 | 8888 | 9000 | 9090 | 9200 => Some(false),
        _ => None,
    }
}

/// Status-bar button: label and number of tunnels with problems.
pub fn summary(tab: &Tab) -> (String, usize) {
    let failed = tab
        .tunnels
        .iter()
        .filter(|e| match &e.state {
            TunnelState::Failed(_) => true,
            TunnelState::Running(t) => t.stats.last_error.lock().is_ok_and(|l| l.is_some()),
            TunnelState::Starting | TunnelState::Stopped => false,
        })
        .count();
    let running = tab.tunnels.iter().filter(|e| matches!(e.state, TunnelState::Running(_))).count();
    let label = if tab.tunnels.is_empty() { tr("터널 열기").to_string() } else { tf("터널 {0}/{1}", &[&running, &tab.tunnels.len()]) };
    (label, failed)
}

fn bound_port(e: &TunnelEntry) -> Option<u16> {
    match &e.state {
        TunnelState::Running(t) => Some(t.bound_port),
        _ => None,
    }
}

/// Plain-language description of a forward.
fn describe(e: &TunnelEntry) -> (&'static str, String) {
    describe_spec(&e.spec, bound_port(e))
}

/// Kind label and route; `b` is the port actually bound, when running.
pub fn describe_spec(spec: &ForwardSpec, b: Option<u16>) -> (&'static str, String) {
    match spec {
        ForwardSpec::Local { port, host, host_port, .. } => {
            (tr("서버 → 내 PC"), tf("localhost:{0}  →  서버 {1}:{2}", &[&(b.unwrap_or(*port)), &host, &host_port]))
        }
        ForwardSpec::Remote { port, host_port, .. } => {
            (tr("내 PC → 서버"), tf("서버 localhost:{0}  →  내 PC localhost:{1}", &[&(b.unwrap_or(*port)), &host_port]))
        }
        ForwardSpec::Dynamic { port, .. } => (tr("SOCKS 프록시"), format!("localhost:{} (SOCKS5)", b.unwrap_or(*port))),
    }
}

fn chip<'a>(label: &'a str, color: Color) -> El<'a> {
    container(text(label).size(10.5).color(color))
        .padding([1, 6])
        .style(move |_| container::Style {
            background: Some(Background::Color(alpha(color, 0.12))),
            border: Border { radius: 4.0.into(), ..Default::default() },
            ..Default::default()
        })
        .into()
}

fn tunnel_row(tab_id: u64, e: &TunnelEntry) -> El<'_> {
    let (kind, desc) = describe(e);
    let requested = match &e.spec {
        ForwardSpec::Local { port, .. } | ForwardSpec::Dynamic { port, .. } | ForwardSpec::Remote { port, .. } => *port,
    };
    let (dot, status, status_color): (Color, String, Color) = match &e.state {
        TunnelState::Starting => (theme::WARN, tr("시작 중…").into(), theme::TEXT_3),
        TunnelState::Stopped => (theme::TEXT_3, tr("중지됨").into(), theme::TEXT_3),
        TunnelState::Failed(err) => (theme::ERR, err.clone(), theme::ERR_TX),
        TunnelState::Running(t) => {
            let s = &t.stats;
            let mut line = tf("↑ {0}  ↓ {1}  · 연결 {2}", &[&human_bytes(s.up.load(Ordering::Relaxed)), &human_bytes(s.down.load(Ordering::Relaxed)), &s.open.load(Ordering::Relaxed)]);
            if e.auto_port && t.bound_port != requested {
                line = tf("{0}번이 사용 중이라 {1}번으로 열었습니다 · {2}", &[&requested, &t.bound_port, &line]);
            }
            match s.last_error.lock().ok().and_then(|l| l.clone()) {
                Some(err) => (theme::WARN, tf("{0}  · 최근 오류: {1}", &[&line, &err]), theme::WARN),
                None => (theme::OK, line, theme::TEXT_3),
            }
        }
    };
    let id = e.id;
    let mut actions = row![].spacing(4).align_y(Alignment::Center);
    match (&e.state, &e.spec, bound_port(e)) {
        (TunnelState::Running(_), ForwardSpec::Local { host_port, .. }, Some(b)) => {
            actions = actions.push(btn(tr("복사"), Btn::Ghost, Some(Message::TunnelCopy(format!("localhost:{b}")))));
            if let Some(https) = web_port(*host_port) {
                actions = actions.push(btn(tr("열기"), Btn::Ghost, Some(Message::TunnelOpen(b, https))));
            }
        }
        (TunnelState::Running(_), ForwardSpec::Dynamic { .. }, Some(b)) => {
            actions = actions.push(btn(tr("복사"), Btn::Ghost, Some(Message::TunnelCopy(format!("socks5://localhost:{b}")))));
        }
        _ => {}
    }
    actions = match &e.state {
        TunnelState::Running(_) | TunnelState::Starting => actions.push(btn(tr("중지"), Btn::Plain, Some(Message::TunnelStop(tab_id, id)))),
        TunnelState::Stopped | TunnelState::Failed(_) => actions.push(btn(tr("시작"), Btn::Primary, Some(Message::TunnelStart(tab_id, id)))),
    };
    actions = actions.push(
        button(text("×").size(13).color(theme::TEXT_3)).padding([0, 6]).style(btn_style(Btn::Ghost)).on_press(Message::TunnelRemove(tab_id, id)),
    );
    let exposed: El<'_> = if e.spec.is_exposed() { chip(tr("외부 노출"), theme::WARN) } else { Space::new().into() };
    container(
        row![
            text("●").size(9).color(dot),
            column![
                row![chip(kind, theme::ACCENT), exposed, mono(desc, 11.5, theme::TEXT)].spacing(6).align_y(Alignment::Center),
                text(status).size(10.5).color(status_color),
            ]
            .spacing(3)
            .width(Length::Fill),
            actions,
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .padding([7, 2])
    .into()
}

fn purpose(kind: char, current: char, title: &'static str, desc: &'static str) -> El<'static> {
    let on = kind == current;
    button(column![text(title).size(12.5).color(if on { theme::TEXT } else { theme::TEXT_2 }), text(desc).size(10.5).color(theme::TEXT_3)].spacing(2))
        .width(Length::Fill)
        .padding([7, 10])
        .on_press(Message::TunnelKind(kind))
        .style(move |_, status| iced::widget::button::Style {
            background: Some(Background::Color(if on || matches!(status, iced::widget::button::Status::Hovered) {
                theme::BG_3
            } else {
                Color::TRANSPARENT
            })),
            text_color: theme::TEXT,
            border: Border { color: if on { alpha(theme::ACCENT, 0.7) } else { theme::LINE_2 }, width: 1.0, radius: 8.0.into() },
            ..Default::default()
        })
        .into()
}

fn port_input<'a>(label: &'a str, placeholder: &'a str, value: &'a str, f: TunnelField) -> El<'a> {
    column![
        text(label).size(11).color(theme::TEXT_2),
        text_input(placeholder, value)
            .on_input(move |v| Message::TunnelField(f, v))
            .on_submit(Message::TunnelAdd)
            .font(Font::MONOSPACE)
            .size(12)
            .padding([5, 8])
            .style(input_style),
    ]
    .spacing(3)
    .width(Length::Fill)
    .into()
}

/// Purpose → common port → ports, with a sentence saying what will happen.
/// Shared by the popover (opens now) and the server editor (opens on every connect).
pub fn builder(f: &TunnelForm) -> El<'_> {
    let presets: &[(&str, u16)] = match f.kind {
        'L' => &SERVER_PRESETS,
        'R' => &LOCAL_PRESETS,
        _ => &[],
    };
    let chips = presets.iter().map(|(name, port)| {
        button(text(tr(name)).size(11).color(theme::TEXT_2))
            .padding([2, 8])
            .on_press(Message::TunnelPreset(*port))
            .style(|_, _| iced::widget::button::Style {
                background: Some(Background::Color(theme::BG_2)),
                text_color: theme::TEXT,
                border: Border { color: theme::LINE_2, width: 1.0, radius: crate::ui::R_CARD.into() },
                ..Default::default()
            })
            .into()
    });
    let inputs: El<'_> = match f.kind {
        'L' => row![
            port_input(tr("서버 쪽 주소"), "localhost", &f.remote_host, TunnelField::RemoteHost),
            port_input(tr("서버 포트"), tr("예: 3306"), &f.remote_port, TunnelField::RemotePort),
            port_input(tr("내 PC 포트"), tr("같은 포트"), &f.local_port, TunnelField::LocalPort),
        ]
        .spacing(8)
        .into(),
        'R' => row![
            port_input(tr("내 PC 포트"), tr("예: 3000"), &f.local_port, TunnelField::LocalPort),
            port_input(tr("서버 포트"), tr("같은 포트"), &f.remote_port, TunnelField::RemotePort),
        ]
        .spacing(8)
        .into(),
        _ => row![port_input(tr("프록시 포트"), "1080", &f.local_port, TunnelField::LocalPort), Space::new().width(Length::FillPortion(2))]
            .spacing(8)
            .into(),
    };
    let preview: El<'_> = match form_spec(f) {
        Ok((ForwardSpec::Local { port, host, host_port, .. }, auto)) => muted(tf("이 PC에서 localhost:{0} 로 접속하면 서버의 {1}:{2} 로 연결됩니다{3}", &[&port, &host, &host_port, &(if auto { " (사용 중이면 빈 포트로 자동 변경)" } else { "" })])),
        Ok((ForwardSpec::Remote { port, host_port, .. }, _)) => {
            muted(tf("서버에서 localhost:{0} 로 접속하면 이 PC의 localhost:{1} 로 연결됩니다", &[&port, &host_port]))
        }
        Ok((ForwardSpec::Dynamic { port, .. }, _)) => muted(tf("브라우저·앱의 프록시를 SOCKS5 localhost:{0} 로 지정하세요", &[&port])),
        Err(_) => muted(tr("포트를 입력하거나 위에서 고르세요")),
    };
    let mut col = column![
        row![
            purpose('L', f.kind, tr("서버 포트 열기"), tr("서버의 DB·웹을 내 PC에서 접속")),
            purpose('R', f.kind, tr("내 PC 포트 공유"), tr("내 PC 서비스를 서버에 공개")),
            purpose('D', f.kind, tr("SOCKS 프록시"), tr("브라우저 트래픽을 서버로")),
        ]
        .spacing(6),
    ]
    .spacing(8);
    if !presets.is_empty() {
        col = col.push(row(chips.collect::<Vec<El<'_>>>()).spacing(6).wrap());
    }
    col = col.push(inputs).push(preview);
    if let Some(e) = &f.error {
        col = col.push(text(e.clone()).size(11).color(theme::ERR_TX));
    }
    col.into()
}

fn add_form(app: &CoShell) -> El<'_> {
    let f = &app.tunnel_form;
    let persist: El<'_> = if app.can_persist_tunnels() {
        checkbox(f.persist).label(tr("다음 접속 때도 자동으로 열기")).on_toggle(Message::TunnelPersist).size(14).text_size(11.5).into()
    } else {
        muted(tr("서버 목록에 저장된 서버라면 다음 접속 때 자동으로 열도록 저장할 수 있습니다"))
    };
    let mut col = column![text(tr("새 터널")).size(12.5), builder(f)].spacing(8);
    let connected = app.active_tab().is_some_and(|t| t.conn.is_some());
    let open_btn = btn(tr("터널 열기"), Btn::Primary, connected.then_some(Message::TunnelAdd));
    col = col.push(row![persist, Space::new().width(Length::Fill), open_btn].align_y(Alignment::Center));
    if !connected {
        col = col.push(muted(tr("연결이 끝나면 열 수 있습니다")));
    }
    container(col)
        .padding(12)
        .style(|_| container::Style {
            background: Some(Background::Color(theme::BG_2)),
            border: Border { color: theme::LINE_2, width: 1.0, radius: crate::ui::R_CARD.into() },
            ..Default::default()
        })
        .into()
}

pub fn popover(app: &CoShell) -> El<'_> {
    let Some(tab) = app.active_tab().filter(|t| !t.is_local()) else { return Space::new().into() };
    let mut col = column![
        row![
            crate::icons::icon(&crate::icons::PORT_FORWARDING, 16.0),
            text(tr("터널")).size(14),
            Space::new().width(Length::Fill),
            button(text("×").size(13).color(theme::TEXT_3)).padding([0, 6]).style(btn_style(Btn::Ghost)).on_press(Message::TunnelPanel),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        muted(tr("서버의 서비스를 내 PC에서 쓰거나, 내 PC 서비스를 서버에 공유합니다.")),
    ]
    .spacing(6);
    if tab.route.len() > 1 {
        let hops = tab.route[..tab.route.len() - 1].join(" › ");
        col = col.push(text(tf("경유: {0}", &[&hops])).size(10.5).color(theme::TEXT_3));
    }
    let mut list = column![].spacing(0);
    for e in &tab.tunnels {
        list = list.push(tunnel_row(tab.id, e));
    }
    if !tab.tunnels.is_empty() {
        col = col.push(list);
    }
    col = col.push(add_form(app));
    let card = container(scrollable(col).height(Length::Shrink))
        .padding(14)
        .width(POPOVER_W)
        .max_height(POPOVER_MAX_H)
        .style(card_style(theme::LINE_2));
    container(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_bottom(Length::Fill)
        .padding(Padding { top: 0.0, right: 0.0, bottom: POPOVER_BOTTOM, left: 8.0 })
        .into()
}

#[cfg(test)]
mod tests {
    use super::{human_bytes, web_port};

    #[test]
    fn bytes() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1536), "1.5 KB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn web_ports() {
        assert_eq!(web_port(443), Some(true));
        assert_eq!(web_port(8080), Some(false));
        assert_eq!(web_port(3306), None);
    }
}
