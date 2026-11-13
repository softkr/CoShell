//! Views: tab bar, terminal / connect / failure bodies, status bar,
//! ⌘K palette, host-key and interactive-auth dialogs.

use crate::i18n::{tf, tr};
use coshell_ssh::{HostKeyDecision, HostKeyKind, HostKeyQuery, InteractiveQuery};
use iced::widget::{Space, button, center, column, container, mouse_area, opaque, row, scrollable, stack, text, text_input};
use iced::{Alignment, Background, Border, Color, Element, Font, Length, Padding, Theme, border};
use zeroize::Zeroizing;

use crate::app::{CONFIRM_INPUT, CoShell, PALETTE_INPUT, Pending, Phase, RENAME_INPUT, SEARCH_INPUT, Search, Step, Tab, prompt_input_id};
use crate::app::{Message, PaletteItem};
use crate::terminal::TerminalView;
use crate::hosts::Env;
use crate::theme::{self, alpha};
use crate::ui_hosts::{env_color, env_tag};

/// macOS title-bar row (traffic lights). Taller than the native 28px to leave a gap above the tabs.
const TITLE_ROW_H: f32 = 36.0;
/// Search box size; centred on the traffic lights (measured centre ≈ 16pt from the top).
const SEARCH_W: f32 = 300.0;
const SEARCH_H: f32 = 24.0;
const SEARCH_TOP: f32 = 11.0;
/// Compact tab strip below it.
const TAB_ROW_H: f32 = 28.0;
const STATUSBAR_H: f32 = 24.0;
/// macOS draws the traffic lights in our title row (unified title bar).
const UNIFIED_TITLE: bool = cfg!(target_os = "macos");
const MODAL_W: f32 = 520.0;
const PALETTE_W: f32 = 620.0;
const PALETTE_LIST_H: f32 = 360.0;
const SEARCH_BAR_INPUT_W: f32 = 240.0;
const SEARCH_BAR_TOP: f32 = 8.0;
/// Clears the PRODUCTION / STAGING corner banner.
const SEARCH_BAR_TOP_BELOW_CORNER: f32 = 30.0;
const EXITED_BANNER_BOTTOM: f32 = 18.0;
const TAB_RENAME_W: f32 = 140.0;
/// Draggable strip between the terminal and the AI panel.
const AI_DIVIDER_W: f32 = 4.0;
const TOAST_MAX_W: f32 = 560.0;
/// Above the status bar and the "session ended" banner.
const TOAST_BOTTOM: f32 = 92.0;

pub(crate) type El<'a> = Element<'a, Message>;

// ------------------------------------------------------------------ styles

#[derive(Clone, Copy)]
pub(crate) enum Btn {
    Primary,
    Plain,
    Ghost,
    Danger,
}

pub(crate) fn btn_style(kind: Btn) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hover = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let (bg, fg, line) = match kind {
            Btn::Primary => (theme::ACCENT, theme::ON_ACCENT, Color::TRANSPARENT),
            Btn::Danger => (theme::ERR, Color::WHITE, Color::TRANSPARENT),
            Btn::Plain => (if hover { theme::BG_4 } else { theme::BG_3 }, theme::TEXT, theme::LINE_2),
            Btn::Ghost => (if hover { theme::BG_3 } else { Color::TRANSPARENT }, theme::TEXT_2, Color::TRANSPARENT),
        };
        let disabled = matches!(status, button::Status::Disabled);
        button::Style {
            background: Some(Background::Color(if disabled { alpha(bg, 0.4) } else { bg })),
            text_color: if disabled { alpha(fg, 0.5) } else { fg },
            border: Border { color: line, width: 1.0, radius: 6.0.into() },
            ..Default::default()
        }
    }
}

pub(crate) fn btn<'a>(label: impl Into<String>, kind: Btn, msg: Option<Message>) -> El<'a> {
    button(text(label.into()).size(12)).padding([6, 12]).style(btn_style(kind)).on_press_maybe(msg).into()
}

pub(crate) fn card_style(border_color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(theme::BG_1)),
        border: Border { color: border_color, width: 1.0, radius: 12.0.into() },
        text_color: Some(theme::TEXT),
        shadow: iced::Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.5),
            offset: iced::Vector::new(0.0, 16.0),
            blur_radius: 40.0,
        },
        ..Default::default()
    }
}

pub(crate) fn input_style(_: &Theme, status: text_input::Status) -> text_input::Style {
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: Background::Color(theme::BG_0),
        border: Border { color: if focused { theme::ACCENT } else { theme::LINE_2 }, width: 1.0, radius: 6.0.into() },
        icon: theme::TEXT_3,
        placeholder: theme::TEXT_3,
        value: theme::TEXT,
        selection: alpha(theme::ACCENT, 0.35),
    }
}

fn bar_style(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(theme::BG_1)),
        text_color: Some(theme::TEXT_2),
        ..Default::default()
    }
}

pub(crate) fn mono<'a>(s: impl Into<String>, size: f32, color: Color) -> El<'a> {
    text(s.into()).size(size).font(Font::MONOSPACE).color(color).into()
}

pub(crate) fn muted<'a>(s: impl Into<String>) -> El<'a> {
    text(s.into()).size(11).color(theme::TEXT_3).into()
}

// ------------------------------------------------------------------ root

pub fn view(app: &CoShell) -> El<'_> {
    let center: El<'_> = if app.ai.open {
        let divider = mouse_area(container(Space::new()).width(AI_DIVIDER_W).height(Length::Fill).style(|_| container::Style {
            background: Some(Background::Color(theme::LINE)),
            ..Default::default()
        }))
        .on_press(Message::AiResizeStart)
        .interaction(iced::mouse::Interaction::ResizingHorizontally);
        row![body(app), divider, crate::ui_ai::panel(app)].height(Length::Fill).into()
    } else {
        body(app)
    };
    let main: El<'_> = column![tab_bar(app), center, status_bar(app)].into();
    let mut layers = vec![main];
    if let Some(n) = &app.notice {
        layers.push(toast(n));
    }
    if app.tunnel_panel {
        layers.push(crate::ui_tunnel::popover(app));
    }
    if let Some(s) = &app.ai.settings {
        layers.push(modal(crate::ui_ai::settings_dialog(s)));
    }
    if let Some(ed) = &app.editor {
        layers.push(modal(crate::ui_hosts::editor(app, ed)));
    }
    if let Some((id, a)) = &app.attach_confirm {
        let name = app.tabs.iter().find(|t| t.id == *id).map_or("", |t| t.display_name());
        layers.push(modal(crate::ui_hosts::attach_confirm(app, name, a)));
    }
    if let Some(tab) = app.active_tab()
        && let Some(p) = &tab.prompt
    {
        layers.push(modal(match p {
            Pending::HostKey { query, confirm, .. } => hostkey_dialog(tab, query, confirm),
            Pending::Interactive { query, answers, save, .. } => interactive_dialog(tab, query, answers, *save),
        }));
    }
    if let Some(d) = &app.settings_draft {
        layers.push(modal(crate::ui_settings::dialog(d)));
    }
    if let Some(tab) = app.close_confirm.and_then(|id| app.tabs.iter().find(|t| t.id == id)) {
        layers.push(modal(close_dialog(tab)));
    }
    if app.palette.is_some() {
        layers.push(palette(app));
    }
    if app.about {
        layers.push(modal(about_dialog()));
    }
    stack(layers).into()
}

/// Dismissible notice above the status bar (also goes away by itself).
fn toast(message: &str) -> El<'_> {
    let card = container(
        row![
            text("●").size(9).color(theme::WARN),
            text(message).size(12).width(Length::Shrink),
            button(text("×").size(13).color(theme::TEXT_3)).padding([0, 6]).style(btn_style(Btn::Ghost)).on_press(Message::NoticeDismiss),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .padding([8, 12])
    .max_width(TOAST_MAX_W)
    .style(card_style(alpha(theme::WARN, 0.45)));
    container(opaque(card)).center_x(Length::Fill).align_bottom(Length::Fill).padding(Padding::ZERO.bottom(TOAST_BOTTOM)).into()
}

pub(crate) fn modal(content: El<'_>) -> El<'_> {
    opaque(container(center(content)).width(Length::Fill).height(Length::Fill).style(|_| container::Style {
        background: Some(Background::Color(theme::SCRIM)),
        ..Default::default()
    }))
}

// ------------------------------------------------------------------ chrome

fn status_dot(tab: &Tab) -> Color {
    match (&tab.phase, tab.exited) {
        (Phase::Connecting, _) => theme::WARN,
        (Phase::Failed(_), _) | (Phase::Live(_), true) => theme::ERR,
        (Phase::Live(_), false) if tab.is_local() => theme::TEXT_3,
        (Phase::Live(_), false) => theme::OK,
    }
}

fn tab_row(content: El<'_>) -> container::Container<'_, Message> {
    container(content)
        .padding(Padding { top: 0.0, right: 8.0, bottom: 0.0, left: 8.0 })
        .height(TAB_ROW_H)
        .width(Length::Fill)
        .align_y(Alignment::End)
}

fn tab_bar(app: &CoShell) -> El<'_> {
    let tabs = app.tabs.iter().map(|t| {
        let active = Some(t.id) == app.active;
        let attention = t.prompt.is_some() && !active;
        let label = row![
            text("●").size(9).color(status_dot(t)),
            match &app.rename {
                Some((id, name)) if *id == t.id => El::from(
                    text_input(tr("탭 이름"), name)
                        .id(RENAME_INPUT)
                        .on_input(Message::TabRename)
                        .on_submit(Message::TabRenameCommit)
                        .size(12)
                        .padding([1, 5])
                        .width(TAB_RENAME_W)
                        .style(input_style)
                ),
                _ => text(t.display_name().to_string()).size(12).color(if active { theme::TEXT } else { theme::TEXT_2 }).into(),
            },
            if attention { text("!").size(11).color(theme::WARN) } else { text("") },
            button(text("×").size(12).color(theme::TEXT_3))
                .padding([0, 4])
                .style(btn_style(Btn::Ghost))
                .on_press(Message::Close(t.id)),
        ]
        .spacing(7)
        .align_y(Alignment::Center);
        let bar_color = env_color(t.env).map(|c| if active { c } else { alpha(c, 0.55) });
        // The env bar is a stack layer, so it takes the label's width instead of filling the row.
        let label = stack![
            container(label).padding(Padding { top: 4.0, right: 4.0, bottom: 4.0, left: 10.0 }),
            container(Space::new()).height(2).width(Length::Fill).style(move |_| container::Style {
                background: bar_color.map(Background::Color),
                ..Default::default()
            }),
        ];
        button(label)
            .padding(0)
            .on_press(Message::Activate(t.id))
            .style(move |_, status| button::Style {
                background: Some(Background::Color(if active {
                    theme::BG_0
                } else if matches!(status, button::Status::Hovered) {
                    theme::BG_2
                } else {
                    Color::TRANSPARENT
                })),
                text_color: theme::TEXT,
                border: Border {
                    color: if active { theme::LINE } else { Color::TRANSPARENT },
                    width: 1.0,
                    radius: border::Radius { top_left: 6.0, top_right: 6.0, bottom_right: 0.0, bottom_left: 0.0 },
                },
                ..Default::default()
            })
            .into()
    });
    let mut r = row(tabs.collect::<Vec<El<'_>>>()).spacing(2).align_y(Alignment::End);
    r = r.push(
        button(text("+").size(13).color(theme::TEXT_2))
            .padding([1, 8])
            .style(btn_style(Btn::Ghost))
            .on_press(Message::OpenPalette),
    );
    let on = app.host_view;
    let servers = button(
        row![text(tr("서버")).size(12.5).color(if on { theme::TEXT } else { theme::TEXT_2 }), mono("⌘O", 10.5, theme::TEXT_3)]
            .spacing(8)
            .align_y(Alignment::Center)
            .height(Length::Fill),
    )
    .height(SEARCH_H)
    .padding([0, 10])
    .on_press(Message::HostView)
    .style(move |_, status| button::Style {
        background: Some(Background::Color(if on || matches!(status, button::Status::Hovered) { theme::BG_3 } else { theme::BG_2 })),
        text_color: theme::TEXT,
        border: Border { color: if on { alpha(theme::ACCENT, 0.6) } else { theme::LINE }, width: 1.0, radius: 6.0.into() },
        ..Default::default()
    });
    let search = button(
        row![text(tr("서버 검색")).size(12.5).color(theme::TEXT_3), Space::new().width(Length::Fill), mono("⌘K", 10.5, theme::TEXT_2)]
            .align_y(Alignment::Center)
            .height(Length::Fill),
    )
    .width(SEARCH_W)
    .height(SEARCH_H)
    .padding([0, 10])
    .on_press(Message::OpenPalette)
    .style(|_, _| button::Style {
        background: Some(Background::Color(theme::BG_2)),
        text_color: theme::TEXT_3,
        border: Border { color: theme::LINE, width: 1.0, radius: 6.0.into() },
        ..Default::default()
    });
    let bar: El<'_> = if UNIFIED_TITLE {
        // Row 1: traffic lights (drawn by macOS) + search. Row 2: compact tabs.
        let title = container(row![Space::new().width(Length::Fill), servers, search].spacing(8).align_y(Alignment::Center))
            .padding(Padding { top: SEARCH_TOP, right: 8.0, bottom: 0.0, left: 0.0 })
            .height(TITLE_ROW_H)
            .width(Length::Fill)
            .align_y(Alignment::Start);
        container(column![title, tab_row(r.into())]).width(Length::Fill).style(bar_style).into()
    } else {
        container(tab_row(row![r, Space::new().width(Length::Fill), servers, search].spacing(8).align_y(Alignment::Center).into()))
            .width(Length::Fill)
            .style(bar_style)
            .into()
    };
    // Tabs and buttons capture their clicks; only empty space reaches this.
    mouse_area(bar).on_press(Message::WindowDrag).on_double_click(Message::WindowZoom).into()
}

fn status_bar(app: &CoShell) -> El<'_> {
    let small = |s: String, c: Color| text(s).size(11).color(c);
    let mut items: Vec<El<'_>> = Vec::new();
    match app.active_tab() {
        Some(t) => {
            let state = match (&t.phase, t.exited) {
                (Phase::Connecting, _) => tr("연결 중"),
                (Phase::Failed(_), _) => tr("실패"),
                (Phase::Live(_), true) => tr("종료됨"),
                (Phase::Live(_), false) if t.is_local() => tr("로컬"),
                (Phase::Live(_), false) => tr("연결됨"),
            };
            items.push(small("●".into(), status_dot(t)).into());
            items.push(env_tag(t.env));
            items.push(small(state.into(), theme::TEXT_2).into());
            if t.is_local() {
                items.push(mono("zsh", 11.0, theme::TEXT_3));
            } else {
                let total: std::time::Duration =
                    t.steps.iter().filter_map(|s| if let Step::Done(d) = s { Some(*d) } else { None }).sum();
                items.push(mono(t.route.last().cloned().unwrap_or_default(), 11.0, theme::TEXT_3));
                if matches!(t.phase, Phase::Live(_)) {
                    items.push(small(format!("{}ms", total.as_millis()), theme::TEXT_3).into());
                }
                // Jump hosts are a detail: a small hint, not part of the headline.
                if t.route.len() > 1 {
                    items.push(small(tf("경유 {0}", &[&(t.route.len() - 1)]), theme::TEXT_3).into());
                }
                if t.conn.is_some() || !t.tunnels.is_empty() {
                    let (label, failed) = crate::ui_tunnel::summary(t);
                    let mut content = row![crate::icons::icon(&crate::icons::PORT_FORWARDING, 13.0), text(label).size(11).color(theme::TEXT_2)]
                        .spacing(5)
                        .align_y(Alignment::Center);
                    if failed > 0 {
                        content = content.push(text(tf("오류 {0}", &[&failed])).size(11).color(theme::ERR));
                    }
                    items.push(button(content).padding([1, 6]).style(btn_style(Btn::Ghost)).on_press(Message::TunnelPanel).into());
                }
            }
        }
        None => items.push(small(tr("세션 없음").into(), theme::TEXT_3).into()),
    }
    items.push(Space::new().width(Length::Fill).into());
    if let Some(Tab { phase: Phase::Live(s), .. }) = app.active_tab() {
        let size = s.size();
        items.push(small(format!("{}×{}", size.cols, size.rows), theme::TEXT_3).into());
    }
    items.push(small(tf("서버 {0}", &[&app.hosts.len()]), theme::TEXT_3).into());
    items.push(
        button(text(tr("설정")).size(11).color(theme::TEXT_3)).padding([1, 6]).style(btn_style(Btn::Ghost)).on_press(Message::SettingsOpen).into(),
    );
    items.push(
        button(text(format!("v{}", crate::VERSION)).size(11).color(theme::TEXT_3))
            .padding([1, 6])
            .style(btn_style(Btn::Ghost))
            .on_press(Message::About)
            .into(),
    );
    container(row(items).spacing(10).align_y(Alignment::Center))
        .padding([0, 10])
        .height(STATUSBAR_H)
        .center_y(STATUSBAR_H)
        .width(Length::Fill)
        .style(bar_style)
        .into()
}

// ------------------------------------------------------------------ body

fn body(app: &CoShell) -> El<'_> {
    if app.host_view || app.tabs.is_empty() {
        return crate::ui_servers::page(app);
    }
    let Some(tab) = app.active_tab() else { return empty_state() };
    match &tab.phase {
        Phase::Live(s) => {
            let id = tab.id;
            let term: El<'_> =
                TerminalView::new(s.term(), s.size(), theme::term_palette(app.settings.scheme), move |b| Message::Input(id, b), move |g| Message::Resize(id, g))
                    .font(app.term_font, app.settings.font_size)
                    .option_as_meta(app.settings.option_as_meta)
                    .on_link(Message::OpenLink)
                    .active(app.terminal_active(tab))
                    .focus_request(app.term_focus.0, app.term_focus.1)
                    .into();
            let mut layers = vec![env_frame(tab.env, term)];
            if tab.exited {
                layers.push(exited_banner(tab));
            }
            if let Some(s) = &app.search {
                layers.push(search_bar(s, tab.env.corner().is_some()));
            }
            stack(layers).into()
        }
        Phase::Connecting => dark(center(connect_card(tab))),
        Phase::Failed(e) => dark(center(failed_card(tab, e))),
    }
}

/// Coloured frame + corner banner for production / staging sessions.
fn env_frame(env: Env, content: El<'_>) -> El<'_> {
    let (Some(c), Some(label)) = (env_color(env), env.corner()) else { return content };
    let framed = container(content).padding(2).width(Length::Fill).height(Length::Fill).style(move |_| container::Style {
        background: Some(Background::Color(c)),
        ..Default::default()
    });
    let corner = container(
        container(text(label).size(9.5).color(Color::WHITE).font(Font { weight: iced::font::Weight::Bold, ..Font::DEFAULT }))
            .padding([3, 9])
            .style(move |_| container::Style {
                background: Some(Background::Color(c)),
                border: Border {
                    radius: border::Radius { top_left: 0.0, top_right: 0.0, bottom_right: 0.0, bottom_left: 7.0 },
                    ..Default::default()
                },
                ..Default::default()
            }),
    )
    .width(Length::Fill)
    .align_right(Length::Fill);
    stack![framed, corner].into()
}

/// Shown over a session that ended: the output stays readable and selectable.
fn exited_banner(tab: &Tab) -> El<'_> {
    let (label, again) = if tab.is_local() { (tr("셸이 종료되었습니다"), tr("새 셸 시작")) } else { (tr("연결이 끊어졌습니다"), tr("다시 연결")) };
    let card = container(
        row![
            text("●").size(9).color(theme::ERR),
            text(label).size(12.5),
            Space::new().width(12),
            btn(tr("탭 닫기"), Btn::Ghost, Some(Message::Close(tab.id))),
            btn(again, Btn::Primary, Some(Message::Retry(tab.id))),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .padding([8, 12])
    .style(card_style(alpha(theme::ERR, 0.45)));
    container(card).center_x(Length::Fill).align_bottom(Length::Fill).padding(EXITED_BANNER_BOTTOM).into()
}

/// ⌘F bar at the top right of the terminal. `below_corner`: leave room for the environment banner.
fn search_bar(s: &Search, below_corner: bool) -> El<'_> {
    let status: El<'_> = match s.found {
        Some(false) => text(tr("결과 없음")).size(11).color(theme::ERR_TX).into(),
        _ => Space::new().into(),
    };
    let has_query = !s.query.is_empty();
    let card = container(
        row![
            text_input(tr("터미널 출력 검색"), &s.query)
                .id(SEARCH_INPUT)
                .on_input(Message::SearchQuery)
                .on_submit(Message::SearchNext(false))
                .size(12.5)
                .padding([5, 9])
                .width(SEARCH_BAR_INPUT_W)
                .style(input_style),
            status,
            btn("↑", Btn::Ghost, has_query.then_some(Message::SearchNext(false))),
            btn("↓", Btn::Ghost, has_query.then_some(Message::SearchNext(true))),
            btn("×", Btn::Ghost, Some(Message::SearchClose)),
        ]
        .spacing(4)
        .align_y(Alignment::Center),
    )
    .padding(6)
    .style(card_style(theme::LINE_2));
    let top = if below_corner { SEARCH_BAR_TOP_BELOW_CORNER } else { SEARCH_BAR_TOP };
    container(opaque(card)).width(Length::Fill).align_right(Length::Fill).padding(Padding { top, right: 16.0, bottom: 0.0, left: 0.0 }).into()
}

fn close_dialog(tab: &Tab) -> El<'_> {
    let running = tab.tunnels.len();
    let mut col = column![
        text(tf("{0} 연결을 닫을까요?", &[&tab.display_name()])).size(14),
        muted(tr("서버에서 실행 중이던 작업이 중단될 수 있습니다.")),
    ]
    .spacing(6);
    if running > 0 {
        col = col.push(muted(tf("열어 둔 터널 {0}개도 함께 닫힙니다.", &[&running])));
    }
    col = col.push(Space::new().height(8)).push(
        row![
            Space::new().width(Length::Fill),
            btn(tr("취소"), Btn::Ghost, Some(Message::CloseCancel)),
            btn(tr("연결 닫기"), Btn::Danger, Some(Message::CloseConfirmed(tab.id))),
        ]
        .spacing(8),
    );
    container(col).padding(18).width(MODAL_W - 80.0).style(card_style(env_color(tab.env).map_or(theme::LINE_2, |c| alpha(c, 0.6)))).into()
}

pub(crate) fn dark<'a>(content: impl Into<El<'a>>) -> El<'a> {
    container(content).width(Length::Fill).height(Length::Fill).style(|_| container::Style {
        background: Some(Background::Color(theme::BG_0)),
        ..Default::default()
    })
    .into()
}

fn step_status(st: Step) -> (&'static str, Color, String) {
    match st {
        Step::Wait => ("○", theme::TEXT_3, tr("대기").to_string()),
        Step::Connecting => ("◌", theme::ACCENT, tr("연결 중…").into()),
        Step::Authenticating => ("◌", theme::ACCENT, tr("인증 중…").into()),
        Step::Done(d) => ("✓", theme::OK, tf("완료 · {0}ms", &[&d.as_millis()])),
    }
}

/// Plain SSH is the common case: one line. Hops are listed only when there are jump hosts.
fn connect_card(tab: &Tab) -> El<'_> {
    let hops = tab.route.len().saturating_sub(1);
    let mut col = column![text(tf("{0}에 연결 중", &[&tab.name])).size(14)].spacing(6);
    if hops == 0 {
        let (mark, color, status) = step_status(tab.steps.first().copied().unwrap_or(Step::Wait));
        let target = tab.route.first().cloned().unwrap_or_default();
        col = col.push(Space::new().height(4)).push(step_row(mark, color, target, status, color));
    } else {
        col = col.push(muted(tf("경유 서버 {0}곳을 거쳐 연결합니다", &[&hops])));
        col = col.push(Space::new().height(4));
        for (i, label) in tab.route.iter().enumerate() {
            let (mark, color, status) = step_status(tab.steps.get(i).copied().unwrap_or(Step::Wait));
            let name = if i + 1 == tab.route.len() { label.clone() } else { tf("{0}  (경유)", &[&label]) };
            col = col.push(step_row(mark, color, name, status, color));
        }
    }
    col = col.push(Space::new().height(8)).push(row![Space::new().width(Length::Fill), btn(tr("취소"), Btn::Ghost, Some(Message::Close(tab.id)))]);
    container(col).padding(18).width(440).style(card_style(theme::LINE_2)).into()
}

fn step_row<'a>(mark: &'a str, mark_color: Color, name: String, status: String, status_color: Color) -> El<'a> {
    row![
        container(text(mark).size(12).color(mark_color)).width(18),
        mono(name, 12.0, theme::TEXT),
        Space::new().width(Length::Fill),
        text(status).size(11).color(status_color),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

fn failed_card<'a>(tab: &'a Tab, err: &'a str) -> El<'a> {
    container(
        column![
            text(tf("{0}에 연결하지 못했습니다", &[&tab.name])).size(14).color(theme::ERR_TX),
            text(err.to_string()).size(12).color(theme::TEXT_2),
            Space::new().height(8),
            row![
                Space::new().width(Length::Fill),
                btn(tr("탭 닫기"), Btn::Ghost, Some(Message::Close(tab.id))),
                btn(tr("다시 시도"), Btn::Primary, Some(Message::Retry(tab.id))),
            ]
            .spacing(8),
        ]
        .spacing(6),
    )
    .padding(18)
    .width(MODAL_W)
    .style(card_style(alpha(theme::ERR, 0.45)))
    .into()
}

fn about_dialog<'a>() -> El<'a> {
    container(
        column![
            crate::icons::icon(&crate::icons::APP, 80.0),
            text("CoShell").size(20),
            text(tf("버전 {0}", &[&crate::VERSION])).size(12.5).color(theme::TEXT_2),
            muted(tr("SSH 접속, 터널, AI 도우미를 한곳에서 쓰는 터미널")),
            Space::new().height(6),
            btn(tr("닫기"), Btn::Plain, Some(Message::About)),
        ]
        .spacing(6)
        .align_x(Alignment::Center),
    )
    .padding(24)
    .width(320)
    .style(card_style(theme::LINE_2))
    .into()
}

fn empty_state<'a>() -> El<'a> {
    dark(center(
        column![
            crate::icons::icon(&crate::icons::APP, 72.0),
            text("CoShell").size(20),
            muted(tr("서버에 접속하거나 로컬 셸을 여세요")),
            Space::new().height(12),
            row![btn(tr("서버 열기  ⌘K"), Btn::Plain, Some(Message::OpenPalette)), btn(tr("새 로컬 셸  ⌘T"), Btn::Plain, Some(Message::NewLocal))]
                .spacing(8),
        ]
        .spacing(6)
        .align_x(Alignment::Center),
    ))
}

// ------------------------------------------------------------------ palette

fn palette(app: &CoShell) -> El<'_> {
    let Some(p) = &app.palette else { return Space::new().into() };
    let items = app.palette_items();
    let rows: Vec<El<'_>> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let (title, sub) = match item {
                PaletteItem::Host(h) => {
                    let e = &app.hosts[*h];
                    let user = e.user.as_deref().map(|u| format!("{u}@")).unwrap_or_default();
                    let port = e.port.filter(|p| *p != coshell_ssh::DEFAULT_PORT).map(|p| format!(":{p}")).unwrap_or_default();
                    let via = e.proxy_jump.as_deref().map(|j| format!("  ·  via {}", j.replace(',', " › "))).unwrap_or_default();
                    let title = app.display_of(&e.alias);
                    (title, format!("{user}{}{port}{via}", e.host()))
                }
                PaletteItem::Direct(q) => (tf("직접 접속: {0}", &[&q]), tr("~/.ssh/config 규칙 적용").into()),
                PaletteItem::NewLocal => (tr("새 로컬 셸").into(), "⌘T".into()),
                PaletteItem::NewHost => (tr("새 서버 추가").into(), tr("CoShell에 저장").into()),
                PaletteItem::Servers => (tr("서버 관리").into(), "⌘O".into()),
                PaletteItem::Settings => (tr("설정").into(), "⌘,".into()),
                PaletteItem::About => (tr("CoShell 정보").into(), tf("버전 {0}", &[&crate::VERSION])),
            };
            let selected = i == p.selected;
            let (env, edit) = match item {
                PaletteItem::Host(h) => {
                    let alias = app.hosts[*h].alias.clone();
                    (app.env_of(&alias), Some(alias))
                }
                _ => (Env::None, None),
            };
            let bar = env_color(env);
            let main = button(
                row![
                    container(Space::new()).width(3).height(28).style(move |_| container::Style {
                        background: bar.map(Background::Color),
                        border: Border { radius: 2.0.into(), ..Default::default() },
                        ..Default::default()
                    }),
                    column![row![text(title).size(13), env_tag(env)].spacing(8).align_y(Alignment::Center), mono(sub, 11.0, theme::TEXT_3)]
                        .spacing(2),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            )
            .width(Length::Fill)
            .padding([7, 10])
            .on_press(Message::PalettePick(item.clone()))
            .style(move |_, status| button::Style {
                background: Some(Background::Color(
                    if selected || matches!(status, button::Status::Hovered) { theme::BG_3 } else { Color::TRANSPARENT },
                )),
                text_color: theme::TEXT,
                border: Border { radius: 7.0.into(), ..Default::default() },
                ..Default::default()
            });
            match edit {
                Some(alias) => row![
                    main,
                    button(text(tr("편집")).size(11).color(theme::TEXT_3)).padding([4, 8]).style(btn_style(Btn::Ghost)).on_press(Message::HostEdit(alias))
                ]
                .align_y(Alignment::Center)
                .into(),
                None => main.into(),
            }
        })
        .collect();
    let list: El<'_> = if rows.is_empty() {
        container(muted(tr("일치하는 서버가 없습니다"))).padding(16).into()
    } else {
        scrollable(column(rows).spacing(2).padding(6)).height(Length::Shrink).into()
    };
    let card = container(
        column![
            container(
                text_input(tr("서버 이름, 주소 또는 user@host:port"), &p.query)
                    .id(PALETTE_INPUT)
                    .on_input(Message::PaletteQuery)
                    .on_submit(Message::PaletteSubmit)
                    .size(14)
                    .padding([10, 12])
                    .style(input_style)
            )
            .padding(10),
            container(list).max_height(PALETTE_LIST_H),
            container(
                row![muted(tr("↑↓ 이동")), muted(tr("⏎ 연결")), muted(tr("esc 닫기")), Space::new().width(Length::Fill), muted(tf("~/.ssh/config · {0}개", &[&app.hosts.len()]))]
                    .spacing(16)
            )
            .padding([8, 14]),
        ],
    )
    .width(PALETTE_W)
    .style(card_style(theme::LINE_2));
    let scrim = mouse_area(container(Space::new()).width(Length::Fill).height(Length::Fill).style(|_| container::Style {
        background: Some(Background::Color(theme::SCRIM)),
        ..Default::default()
    }))
    .on_press(Message::ClosePalette);
    stack![scrim, container(card).center_x(Length::Fill).padding(Padding::ZERO.top(90))].into()
}

// ------------------------------------------------------------------ dialogs

fn hostkey_dialog<'a>(tab: &'a Tab, q: &'a HostKeyQuery, confirm: &'a str) -> El<'a> {
    let id = tab.id;
    let target = format!("{}:{}", q.host, q.port);
    let hop = tab.route.get(q.hop).cloned().unwrap_or_else(|| target.clone());
    match &q.kind {
        HostKeyKind::Unknown => container(
            column![
                row![crate::icons::icon(&crate::icons::SAFE_GUARD, 18.0), text(tr("처음 접속하는 서버입니다")).size(14)].spacing(8).align_y(Alignment::Center),
                mono(hop, 12.0, theme::TEXT_2),
                muted(tr("서버가 보낸 호스트 키 지문입니다. 서버 관리자에게 받은 지문과 같은지 확인하세요.")),
                fingerprint_box(format!("{}  {}", q.algorithm, q.fingerprint), theme::LINE_2, theme::TEXT),
                muted(tr("저장하면 known_hosts에 추가되고 다음부터는 묻지 않습니다.")),
                Space::new().height(6),
                row![
                    btn(tr("취소"), Btn::Ghost, Some(Message::HostKeyAnswer(id, HostKeyDecision::Reject))),
                    Space::new().width(Length::Fill),
                    btn(tr("이번만 접속"), Btn::Plain, Some(Message::HostKeyAnswer(id, HostKeyDecision::AcceptOnce))),
                    btn(tr("신뢰하고 저장"), Btn::Primary, Some(Message::HostKeyAnswer(id, HostKeyDecision::AcceptAndSave))),
                ]
                .spacing(8),
            ]
            .spacing(8),
        )
        .padding(18)
        .width(MODAL_W)
        .style(card_style(theme::LINE_2))
        .into(),
        HostKeyKind::Changed { line } => {
            let matches = confirm == q.host;
            container(
                column![
                    row![crate::icons::icon(&crate::icons::SAFE_GUARD, 18.0), text(tr("서버 호스트 키가 변경되었습니다")).size(14).color(theme::ERR_TX)]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    container(
                        text(tf("{0} 의 호스트 키가 known_hosts {1}행과 다릅니다. 서버를 재설치했을 수도 있지만, 누군가 중간에서 통신을 가로채고 있을 수도 있습니다.", &[&hop, &line]))
                        .size(12)
                        .color(theme::ERR_TX)
                    )
                    .padding(10)
                    .style(|_| container::Style {
                        background: Some(Background::Color(alpha(theme::ERR, 0.08))),
                        border: Border { color: alpha(theme::ERR, 0.35), width: 1.0, radius: 7.0.into() },
                        ..Default::default()
                    }),
                    text(tr("지금 받은 키")).size(11).color(theme::TEXT_2),
                    fingerprint_box(format!("{}  {}", q.algorithm, q.fingerprint), alpha(theme::ERR, 0.5), theme::ERR_TX),
                    muted(tr("서버 관리자에게 키 변경 여부를 확인하기 전에는 접속하지 마세요.")),
                    text(tf("교체하려면 서버 주소 \"{0}\"을(를) 입력하세요 (기존 파일은 known_hosts.old로 백업)", &[&q.host]))
                        .size(11)
                        .color(theme::TEXT_2),
                    text_input(tr("서버 주소 입력"), confirm)
                        .id(CONFIRM_INPUT)
                        .on_input(move |s| Message::HostKeyConfirm(id, s))
                        .font(Font::MONOSPACE)
                        .size(12)
                        .padding([7, 10])
                        .style(input_style),
                    Space::new().height(6),
                    row![
                        btn(tr("키 교체 후 접속"), Btn::Danger, matches.then_some(Message::HostKeyAnswer(id, HostKeyDecision::ReplaceAndSave))),
                        Space::new().width(Length::Fill),
                        btn(tr("연결 중단"), Btn::Primary, Some(Message::HostKeyAnswer(id, HostKeyDecision::Reject))),
                    ]
                    .spacing(8),
                ]
                .spacing(8),
            )
            .padding(18)
            .width(MODAL_W + 40.0)
            .style(card_style(alpha(theme::ERR, 0.6)))
            .into()
        }
    }
}

fn fingerprint_box<'a>(s: String, line: Color, color: Color) -> El<'a> {
    container(mono(s, 12.0, color))
        .padding([9, 12])
        .width(Length::Fill)
        .style(move |_| container::Style {
            background: Some(Background::Color(theme::BG_0)),
            border: Border { color: line, width: 1.0, radius: 7.0.into() },
            ..Default::default()
        })
        .into()
}

fn interactive_dialog<'a>(tab: &'a Tab, q: &'a InteractiveQuery, answers: &'a [Zeroizing<String>], save: bool) -> El<'a> {
    let id = tab.id;
    let title = if q.name.is_empty() { tr("인증 필요").to_string() } else { q.name.clone() };
    let mut col = column![text(title).size(14), mono(q.label.clone(), 12.0, theme::TEXT_2)].spacing(8);
    if !q.instructions.is_empty() {
        col = col.push(muted(q.instructions.clone()));
    }
    let last = q.prompts.len().saturating_sub(1);
    for (i, p) in q.prompts.iter().enumerate() {
        let value = answers.get(i).map(|z| z.as_str()).unwrap_or("");
        let mut input = text_input("", value)
            .id(prompt_input_id(i))
            .on_input(move |s| Message::PromptInput(id, i, s))
            .secure(!p.echo)
            .font(Font::MONOSPACE)
            .size(13)
            .padding([8, 10])
            .style(input_style);
        if i == last {
            input = input.on_submit(Message::PromptSubmit(id));
        }
        col = col.push(text(p.text.trim().to_string()).size(12).color(theme::TEXT_2)).push(input);
    }
    if q.password {
        col = col.push(
            iced::widget::checkbox(save)
                .label(tr("이 서버 비밀번호 저장 (접속에 성공하면 키체인에 저장)"))
                .on_toggle(move |on| Message::PromptSave(id, on))
                .size(14)
                .text_size(11.5),
        );
    }
    col = col.push(Space::new().height(6)).push(
        row![
            Space::new().width(Length::Fill),
            btn(tr("취소"), Btn::Ghost, Some(Message::PromptCancel(id))),
            btn(tr("확인"), Btn::Primary, Some(Message::PromptSubmit(id))),
        ]
        .spacing(8),
    );
    container(col).padding(18).width(MODAL_W - 80.0).style(card_style(alpha(theme::WARN, 0.45))).into()
}
