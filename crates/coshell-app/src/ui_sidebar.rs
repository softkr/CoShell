//! Left sidebar: servers grouped like a tree, built-in snippets, the active
//! session card and a footer (server count, version).

use crate::i18n::{tf, tr};
use iced::widget::{Space, button, column, container, row, scrollable, svg, text, text_input};
use iced::{Alignment, Background, Border, Color, Length, Padding};

use crate::app::{CoShell, Message, Phase, SIDEBAR_SEARCH_INPUT, SidebarTab};
use crate::hosts::Env;
use crate::icons::{CHEVRON_DOWN, PLUS, SEARCH, SERVER};
use crate::theme::{self, alpha};
use crate::ui::{BAR_H, Btn, El, R_CARD, R_CONTROL, SEMIBOLD, btn_style, dot, fill, input_style, kbd, mono, muted, rule};

/// A ready-made read-only command: typed into the terminal, optionally with Enter.
pub struct Snippet {
    pub title: &'static str,
    pub category: &'static str,
    pub cmd: &'static str,
}

pub const SNIPPETS: &[Snippet] = &[
    Snippet { title: "디스크 사용량", category: "진단", cmd: "df -h" },
    Snippet { title: "로그 디렉터리 용량", category: "진단", cmd: "du -h -d 1 /var/log 2>/dev/null | sort -rh | head -n 10" },
    Snippet { title: "CPU 점유 상위 프로세스", category: "성능", cmd: "ps aux --sort=-%cpu | head -n 6" },
    Snippet { title: "메모리 & 스왑 상태", category: "성능", cmd: "free -h" },
    Snippet { title: "가동 시간 & 부하", category: "성능", cmd: "uptime" },
    Snippet { title: "최근 시스템 에러 로그", category: "장애", cmd: "journalctl -p err -n 20 --no-pager" },
    Snippet { title: "열린 포트", category: "네트워크", cmd: "ss -tlnp" },
];

pub fn sidebar(app: &CoShell) -> El<'_> {
    let content: El<'_> = match app.sidebar_tab {
        SidebarTab::Servers => servers(app),
        SidebarTab::Snippets => snippets(app),
    };
    let mut col = column![head(app), rule(true)];
    if app.sidebar_search {
        col = col.push(
            container(
                text_input(tr("호스트명, IP 검색..."), &app.host_filter)
                    .id(SIDEBAR_SEARCH_INPUT)
                    .on_input(Message::HostFilter)
                    .size(12)
                    .padding([6, 10])
                    .style(|t, st| {
                        let mut s = input_style(t, st);
                        s.background = Background::Color(theme::BG_2);
                        s
                    }),
            )
            .padding([8, 12])
            .style(fill(alpha(theme::BG_0, 0.5))),
        );
        col = col.push(rule(true));
    }
    col = col.push(scrollable(container(content).padding(10)).height(Length::Fill));
    if let Some(card) = resource_card(app) {
        col = col.push(container(card).padding(Padding { top: 0.0, right: 10.0, bottom: 10.0, left: 10.0 }));
    }
    col = col.push(rule(true)).push(footer(app));
    container(col).width(app.settings.sidebar_width).height(Length::Fill).style(fill(theme::BG_1)).into()
}

/// Segmented 서버 | 스니펫 switch, then add and search.
fn head(app: &CoShell) -> El<'_> {
    let seg = |label: &'static str, t: SidebarTab| {
        let on = app.sidebar_tab == t;
        button(text(tr(label)).size(11).font(if on { SEMIBOLD } else { crate::ui::MEDIUM }))
            .padding([4, 10])
            .on_press(Message::SidebarTab(t))
            .style(move |_, st| button::Style {
                background: on.then_some(Background::Color(theme::BG_2)),
                text_color: if on { theme::SLATE_100 } else if matches!(st, button::Status::Hovered) { theme::TEXT } else { theme::TEXT_2 },
                border: Border { radius: 6.0.into(), ..Default::default() },
                shadow: if on {
                    iced::Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.25), offset: iced::Vector::new(0.0, 1.0), blur_radius: 2.0 }
                } else {
                    iced::Shadow::default()
                },
                ..Default::default()
            })
    };
    let switch = container(row![seg("서버", SidebarTab::Servers), seg("스니펫", SidebarTab::Snippets)]).padding(2).style(|_| container::Style {
        background: Some(Background::Color(theme::BG_0)),
        border: Border { color: theme::LINE, width: 1.0, radius: R_CONTROL.into() },
        ..Default::default()
    });
    let icon = |h: &'static std::sync::LazyLock<iced::widget::svg::Handle>, size: f32, label: &'static str, msg: Message| -> El<'static> {
        let b = button(crate::icons::tinted(h, size, theme::TEXT_2)).padding(6).on_press(msg).style(|_, st| button::Style {
            background: matches!(st, button::Status::Hovered).then_some(Background::Color(theme::BG_3)),
            border: Border { radius: 6.0.into(), ..Default::default() },
            ..Default::default()
        });
        crate::ui::tip(b, label, iced::widget::tooltip::Position::Bottom)
    };
    container(
        row![
            switch,
            Space::new().width(Length::Fill),
            icon(&PLUS, 16.0, tr("새 호스트 추가"), Message::HostNewIn(None)),
            icon(&SEARCH, 14.0, tr("호스트 검색"), Message::SidebarSearch),
        ]
        .spacing(2)
        .align_y(Alignment::Center),
    )
    .padding([0, 10])
    .height(BAR_H - 1.0)
    .center_y(BAR_H - 1.0)
    .into()
}

fn chevron<'a>(open: bool, color: Color) -> El<'a> {
    let mut s = svg(std::sync::LazyLock::force(&CHEVRON_DOWN).clone()).width(14).height(14).style(move |_, _| svg::Style { color: Some(color) });
    if !open {
        s = s.rotation(iced::Radians(-std::f32::consts::FRAC_PI_2));
    }
    s.into()
}

/// Environments in the order the sidebar lists them, with their header colour.
fn env_groups() -> [(Env, &'static str, Color); 4] {
    [
        (Env::Prod, tr("운영"), theme::WARN),
        (Env::Stage, tr("스테이징"), theme::INFO),
        (Env::Dev, tr("개발"), theme::ACCENT_TX),
        (Env::None, tr("기타"), theme::TEXT_2),
    ]
}

/// Hosts grouped by environment; a row opens the server (or switches to its open tab).
fn servers(app: &CoShell) -> El<'_> {
    let q = app.host_filter.trim().to_lowercase();
    let active_alias = app.active_tab().filter(|_| !app.host_view).and_then(|t| t.alias.clone());
    let mut col = column![].spacing(12);
    let mut any = false;
    for (env, label, color) in env_groups() {
        let items: Vec<&coshell_ssh::config::HostEntry> = app
            .hosts
            .iter()
            .filter(|e| app.env_of(&e.alias) == env)
            .filter(|e| q.is_empty() || format!("{} {} {}", app.display_of(&e.alias), e.alias, e.host()).to_lowercase().contains(&q))
            .collect();
        if items.is_empty() {
            continue;
        }
        any = true;
        let key = label.to_string();
        let collapsed = app.collapsed_groups.contains(&key);
        let header = button(
            row![
                chevron(!collapsed, color),
                text(label).size(12).font(SEMIBOLD).color(color),
                Space::new().width(Length::Fill),
                mono(items.len().to_string(), 10.0, theme::TEXT_3),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .padding([4, 6])
        .on_press(Message::GroupToggle(key))
        .style(|_, st| button::Style {
            background: matches!(st, button::Status::Hovered).then_some(Background::Color(alpha(theme::BG_3, 0.6))),
            border: Border { radius: 6.0.into(), ..Default::default() },
            ..Default::default()
        });
        let mut list = column![header].spacing(4);
        if !collapsed {
            for e in items {
                let alias = e.alias.clone();
                let open_tab = app.tabs.iter().find(|t| t.alias.as_deref() == Some(alias.as_str()));
                let active = active_alias.as_deref() == Some(alias.as_str());
                let msg = match open_tab {
                    Some(t) => Message::Activate(t.id),
                    None => Message::HostConnect(alias.clone()),
                };
                let disk = open_tab.and_then(|t| t.stats.as_ref()).map(|s| s.disk).filter(|d| *d >= 85);
                let mut line = row![
                    crate::icons::tinted(&SERVER, 14.0, if active { theme::ACCENT_TX } else { theme::TEXT_3 }),
                    container(mono(app.display_of(&alias), 12.0, if active { theme::EMERALD_300 } else { theme::SLATE_300 })).width(Length::Fill).clip(true),
                ]
                .spacing(8)
                .align_y(Alignment::Center);
                if let Some(d) = disk {
                    line = line.push(container(mono(format!("{d}%"), 10.0, theme::WARN)).padding([0, 4]).style(|_| container::Style {
                        background: Some(Background::Color(alpha(theme::WARN_500, 0.1))),
                        border: Border { radius: 3.0.into(), ..Default::default() },
                        ..Default::default()
                    }));
                }
                if active {
                    line = line.push(container(dot(theme::ACCENT_TX, 8.0)).style(|_| container::Style {
                        shadow: iced::Shadow { color: alpha(theme::ACCENT_TX, 0.6), offset: iced::Vector::ZERO, blur_radius: 4.0 },
                        ..Default::default()
                    }));
                }
                list = list.push(
                    button(line).width(Length::Fill).padding([8, 10]).on_press(msg).style(move |_, st| {
                        let hover = matches!(st, button::Status::Hovered | button::Status::Pressed);
                        button::Style {
                            background: Some(Background::Color(if active {
                                alpha(theme::ACCENT, 0.1)
                            } else if hover {
                                theme::BG_3
                            } else {
                                Color::TRANSPARENT
                            })),
                            text_color: theme::TEXT,
                            border: Border {
                                color: if active { alpha(theme::ACCENT, 0.3) } else { Color::TRANSPARENT },
                                width: 1.0,
                                radius: R_CONTROL.into(),
                            },
                            ..Default::default()
                        }
                    }),
                );
            }
        }
        col = col.push(list);
    }
    if !any {
        return container(text(tr("일치하는 호스트가 없습니다.")).size(12).color(theme::TEXT_3)).center_x(Length::Fill).padding([32, 0]).into();
    }
    col.into()
}

fn snippets(app: &CoShell) -> El<'_> {
    let q = app.host_filter.trim().to_lowercase();
    let live = app.live_tab().is_some() && !app.host_view;
    let mut col = column![].spacing(8);
    let mut any = false;
    for (i, sn) in SNIPPETS.iter().enumerate() {
        let (title, category) = (tr(sn.title), tr(sn.category));
        if !q.is_empty() && ![title, category, sn.cmd].iter().any(|s| s.to_lowercase().contains(&q)) {
            continue;
        }
        any = true;
        let tag = container(text(category).size(10).color(theme::TEXT_2)).padding([1, 6]).style(|_| container::Style {
            background: Some(Background::Color(theme::BG_0)),
            border: Border { color: theme::LINE, width: 1.0, radius: 4.0.into() },
            ..Default::default()
        });
        let small = |label: &'static str, primary: bool, msg: Option<Message>| {
            button(text(tr(label)).size(10.5)).padding([2, 9]).on_press_maybe(msg).style(crate::ui::tint_style(
                if primary { theme::ACCENT_TX } else { theme::TEXT_2 },
                primary,
            ))
        };
        let card = column![
            row![text(title).size(12).font(SEMIBOLD).color(theme::TEXT).width(Length::Fill), tag].spacing(8).align_y(Alignment::Center),
            container(mono(format!(">_ {}", sn.cmd), 11.0, alpha(theme::ACCENT_TX, 0.9))).width(Length::Fill).clip(true),
            row![
                Space::new().width(Length::Fill),
                small("입력", false, live.then_some(Message::Snippet(i, false))),
                small("실행", true, live.then_some(Message::Snippet(i, true))),
            ]
            .spacing(6),
        ]
        .spacing(6);
        col = col.push(container(card).padding(10).width(Length::Fill).style(|_| container::Style {
            background: Some(Background::Color(alpha(theme::BG_2, 0.5))),
            border: Border { color: theme::LINE, width: 1.0, radius: R_CARD.into() },
            ..Default::default()
        }));
    }
    if !any {
        return container(muted(tr("일치하는 스니펫이 없습니다"))).center_x(Length::Fill).padding(28).into();
    }
    if !live {
        col = col.push(container(muted(tr("세션을 열면 터미널에 바로 입력·실행됩니다"))).padding([4, 2]));
    }
    col.into()
}

/// The active session's CPU, memory and disk as bars (polled live).
fn resource_card(app: &CoShell) -> Option<El<'_>> {
    let t = app.active_tab().filter(|_| !app.host_view)?;
    let st = t.stats.as_ref();
    let bar = |label: String, v: Option<u8>| -> El<'_> {
        let val = v.unwrap_or(0);
        let color = if val >= 85 {
            theme::WARN
        } else if val >= 70 {
            theme::INFO
        } else {
            theme::ACCENT_TX
        };
        let value = v.map_or("—".to_string(), |x| format!("{x}%"));
        let fill_w = f32::from(val) / 100.0;
        column![
            row![
                mono(label, 10.0, theme::TEXT_2),
                Space::new().width(Length::Fill),
                text(value).size(10).font(if val >= 85 { crate::fonts::weight(crate::fonts::MONO, iced::font::Weight::Semibold) } else { crate::fonts::MONO }).color(if val >= 85 { theme::WARN } else { theme::SLATE_300 }),
            ],
            container(
                row![
                    container(Space::new()).width(Length::FillPortion((fill_w * 1000.0) as u16)).height(6).style(move |_| container::Style {
                        background: (val > 0).then_some(Background::Color(color)),
                        border: Border { radius: 3.0.into(), ..Default::default() },
                        ..Default::default()
                    }),
                    Space::new().width(Length::FillPortion(((1.0 - fill_w) * 1000.0) as u16)),
                ]
            )
            .height(6)
            .width(Length::Fill)
            .clip(true)
            .style(|_| container::Style {
                background: Some(Background::Color(theme::BG_0)),
                border: Border { radius: 3.0.into(), ..Default::default() },
                ..Default::default()
            }),
        ]
        .spacing(4)
        .into()
    };
    let live = matches!(t.phase, Phase::Live(_)) && !t.exited;
    let mount = st.map(|s| s.disk_mount.clone()).filter(|m| !m.is_empty()).unwrap_or_else(|| "/".into());
    let col = column![
        row![
            container(mono(tf("{0} 리소스", &[&t.display_name()]), 11.0, theme::SLATE_300)).width(Length::Fill).clip(true),
            mono(if live { "LIVE" } else { "OFF" }, 10.0, if live { theme::ACCENT_TX } else { theme::TEXT_3 }),
        ]
        .align_y(Alignment::Center),
        bar("CPU".into(), st.map(|s| s.cpu)),
        bar("MEM".into(), st.map(|s| s.mem)),
        bar(format!("DISK {mount}"), st.map(|s| s.disk)),
    ]
    .spacing(8);
    Some(
        container(col)
            .padding(12)
            .width(Length::Fill)
            .style(|_| container::Style {
                background: Some(Background::Color(alpha(theme::BG_2, 0.6))),
                border: Border { color: theme::LINE, width: 1.0, radius: R_CARD.into() },
                ..Default::default()
            })
            .into(),
    )
}

fn footer(app: &CoShell) -> El<'_> {
    container(
        row![
            text(tf("{0}대 호스트 등록됨", &[&app.hosts.len()])).size(11).color(theme::TEXT_2),
            Space::new().width(Length::Fill),
            button(kbd(format!("v{}", crate::VERSION))).padding(0).style(btn_style(Btn::Ghost)).on_press(Message::About),
        ]
        .align_y(Alignment::Center),
    )
    .padding([0, 14])
    .height(40)
    .center_y(40)
    .into()
}
