//! Left sidebar: servers grouped like a tree, snippets and the active
//! session card.

use crate::i18n::tr;
use iced::widget::{Space, button, column, container, mouse_area, row, svg, text, text_input};
use iced::{Alignment, Background, Border, Color, Length, Padding};

use crate::app::{CoShell, Message, Phase, SIDEBAR_SEARCH_INPUT, SidebarTab, SnippetEditor, SnippetField};
use crate::context_menu::ContextMenu;
use crate::hosts::Env;
use crate::icons::{CHEVRON_DOWN, PENCIL, PLUG, PLUS, SEARCH, UNPLUG};
use crate::settings::Snippet;
use crate::theme::{self, alpha};
use crate::ui::{Btn, El, R_CARD, R_CONTROL, SEMIBOLD, btn, btn_style, card_style, dot, fill, input_style, mono, muted};

/// Ready-made read-only commands (title, category, command), shown until the list is edited.
const BUILTIN_SNIPPETS: &[(&str, &str, &str)] = &[
    ("디스크 사용량", "진단", "df -h"),
    ("로그 디렉터리 용량", "진단", "du -h -d 1 /var/log 2>/dev/null | sort -rh | head -n 10"),
    ("CPU 점유 상위 프로세스", "성능", "ps aux --sort=-%cpu | head -n 6"),
    ("메모리 & 스왑 상태", "성능", "free -h"),
    ("가동 시간 & 부하", "성능", "uptime"),
    ("최근 시스템 에러 로그", "장애", "journalctl -p err -n 20 --no-pager"),
    ("열린 포트", "네트워크", "ss -tlnp"),
];

/// The built-in snippets in the current language.
pub fn builtin_snippets() -> Vec<Snippet> {
    BUILTIN_SNIPPETS.iter().map(|(title, category, cmd)| Snippet { title: tr(title).into(), category: tr(category).into(), cmd: (*cmd).into() }).collect()
}

/// The pencil on a snippet card.
pub const SNIPPET_EDIT: &str = "sidebar-snippet-edit";
const SNIPPET_EDITOR_W: f32 = 460.0;

/// Width of a host's right-click menu.
const HOST_MENU_W: f32 = 140.0;
/// The connect / disconnect icon on the hovered host row.
pub const HOST_ACTION: &str = "sidebar-host-action";

pub fn sidebar(app: &CoShell) -> El<'_> {
    let content: El<'_> = match app.sidebar_tab {
        SidebarTab::Servers => servers(app),
        SidebarTab::Snippets => snippets(app),
        // The file panel has its own path bar instead of the search field.
        SidebarTab::Files => {
            let col = column![head(app), container(crate::ui_files::panel(app)).height(Length::Fill)];
            return container(col).width(app.settings.sidebar_width).height(Length::Fill).style(fill(theme::SIDEBAR)).into();
        }
    };
    // The search field is always there, like Finder's.
    let field = text_input(tr("호스트명, IP 검색..."), &app.host_filter)
        .id(SIDEBAR_SEARCH_INPUT)
        .on_input(Message::HostFilter)
        .size(12)
        .padding(0)
        .style(crate::ui::bare_input_style);
    let search = container(row![crate::icons::tinted(&SEARCH, 13.0, theme::TEXT_3), field].spacing(7).align_y(Alignment::Center))
        .padding([7, 10])
        .style(|_| container::Style {
            background: Some(Background::Color(alpha(Color::WHITE, 0.06))),
            border: Border { radius: crate::ui::R_ROW.into(), ..Default::default() },
            ..Default::default()
        });
    let mut col = column![head(app), container(search).padding(Padding { top: 0.0, right: 12.0, bottom: 10.0, left: 12.0 })];
    col = col.push(crate::ui::scroll(container(content).padding(Padding { top: 0.0, right: 10.0, bottom: 8.0, left: 10.0 })).height(Length::Fill));
    if let Some(card) = resource_card(app) {
        col = col.push(container(card).padding(Padding { top: 0.0, right: 12.0, bottom: 12.0, left: 12.0 }));
    }
    container(col).width(app.settings.sidebar_width).height(Length::Fill).style(fill(theme::SIDEBAR)).into()
}

/// Segmented 서버 | 스니펫 switch, then add and search.
fn head(app: &CoShell) -> El<'_> {
    let seg = |label: &'static str, t: SidebarTab| {
        let on = app.sidebar_tab == t;
        button(container(text(tr(label)).size(11.5).font(if on { SEMIBOLD } else { crate::ui::MEDIUM })).center_x(Length::Fill))
            .width(Length::Fill)
            .padding([4, 10])
            .on_press(Message::SidebarTab(t))
            .style(move |_, st| button::Style {
                background: on.then_some(Background::Color(alpha(Color::WHITE, 0.16))),
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
    let switch = container(row![seg("서버", SidebarTab::Servers), seg("스니펫", SidebarTab::Snippets), seg("파일", SidebarTab::Files)]).padding(2).width(Length::Fill).style(|_| container::Style {
        background: Some(Background::Color(alpha(Color::WHITE, 0.07))),
        border: Border { color: theme::LINE, width: 1.0, radius: crate::ui::R_ROW.into() },
        ..Default::default()
    });
    let icon = |h: &'static std::sync::LazyLock<iced::widget::svg::Handle>, size: f32, label: &'static str, msg: Message| -> El<'static> {
        let b = button(container(crate::icons::tinted(h, size, theme::TEXT_2)).center(28)).padding(0).on_press(msg).style(|_, st| button::Style {
            background: Some(Background::Color(if matches!(st, button::Status::Hovered) { theme::BG_3 } else { theme::BG_2 })),
            border: Border { color: theme::LINE, width: 1.0, radius: R_CONTROL.into() },
            ..Default::default()
        });
        crate::ui::tip(b, label, iced::widget::tooltip::Position::Bottom)
    };
    let add = match app.sidebar_tab {
        SidebarTab::Snippets => icon(&PLUS, 16.0, tr("스니펫 추가"), Message::SnippetNew),
        _ => icon(&PLUS, 16.0, tr("새 호스트 추가"), Message::HostNewIn(None)),
    };
    container(
        row![
            switch,
            add,
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .padding(Padding { top: 12.0, right: 12.0, bottom: 10.0, left: 12.0 })
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
        (Env::Dev, tr("개발"), theme::OK),
        (Env::None, tr("기타"), theme::TEXT_2),
    ]
}

/// Hosts grouped by environment; a row opens the server (or switches to its open tab).
fn servers(app: &CoShell) -> El<'_> {
    let q = app.host_filter.trim().to_lowercase();
    let active_alias = app.active_tab().filter(|_| !app.host_view).and_then(|t| t.alias.clone());
    let mut col = column![].spacing(14);
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
                text(label).size(11).font(SEMIBOLD).color(theme::TEXT_3),
                Space::new().width(Length::Fill),
                text(items.len().to_string()).size(11).color(theme::TEXT_3),
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
        let mut list = column![header].spacing(2);
        if !collapsed {
            for e in items {
                let alias = e.alias.clone();
                let open_tab = app.tabs.iter().find(|t| t.alias.as_deref() == Some(alias.as_str()));
                let active = active_alias.as_deref() == Some(alias.as_str());
                let menu_open = app.host_menu.as_deref() == Some(alias.as_str());
                let hovered = app.host_hover.as_deref() == Some(alias.as_str());
                // Connecting takes the plug icon, so a stray click on the row does nothing.
                let msg = open_tab.map(|t| Message::Activate(t.id));
                let disk = open_tab.and_then(|t| t.stats.as_ref()).map(|s| s.disk).filter(|d| *d >= 85);
                let on = |c: Color| if active { Color::WHITE } else { c };
                let mono_medium = crate::fonts::weight(crate::fonts::MONO, iced::font::Weight::Medium);
                let names = column![
                    text(app.display_of(&alias)).size(12.5).font(mono_medium).color(on(theme::TEXT)).wrapping(text::Wrapping::None),
                    text(e.host().to_string()).size(10).font(crate::fonts::MONO).color(if active { alpha(Color::WHITE, 0.75) } else { theme::TEXT_3 }).wrapping(text::Wrapping::None),
                ]
                .spacing(1);
                let mut line = row![dot(on(color), 8.0), container(names).width(Length::Fill).clip(true)].spacing(10).align_y(Alignment::Center);
                if let Some(d) = disk {
                    line = line.push(container(mono(format!("{d}%"), 10.0, on(theme::WARN))).padding([1, 6]).style(move |_| container::Style {
                        background: Some(Background::Color(if active { alpha(Color::WHITE, 0.2) } else { alpha(theme::WARN, 0.15) })),
                        border: Border { radius: 999.0.into(), ..Default::default() },
                        ..Default::default()
                    }));
                }
                line = line.push(link_icon(open_tab, &alias, active, hovered || menu_open));
                let row_btn = button(line).width(Length::Fill).padding([7, 10]).on_press_maybe(msg).style(move |_, st| {
                        let hover = hovered || menu_open || matches!(st, button::Status::Hovered | button::Status::Pressed);
                        button::Style {
                            background: Some(Background::Color(if active {
                                theme::ACCENT
                            } else if hover {
                                alpha(Color::WHITE, 0.075)
                            } else {
                                Color::TRANSPARENT
                            })),
                            text_color: theme::TEXT,
                            border: Border { radius: 10.0.into(), ..Default::default() },
                            ..Default::default()
                        }
                    });
                let row_btn = mouse_area(row_btn).on_enter(Message::HostHover(alias.clone(), true)).on_exit(Message::HostHover(alias.clone(), false));
                list = list.push(
                    ContextMenu::new(row_btn, Message::HostMenu(Some(alias.clone())), Message::HostMenu(None))
                        .menu(menu_open.then(|| host_menu(app, &alias))),
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

/// The host's connection at a glance: a plug in its socket while connected (coloured like its
/// tab's dot), pulled apart otherwise. Hovered, it turns into the action: unplug to disconnect
/// (which asks first), plug to connect or reconnect.
fn link_icon<'a>(open_tab: Option<&crate::app::Tab>, alias: &str, active: bool, hovered: bool) -> El<'a> {
    let live = open_tab.is_some_and(|t| matches!(t.phase, Phase::Live(_)) && !t.exited);
    let action = match open_tab {
        Some(t) if live => Some((&UNPLUG, tr("연결 해제"), Message::Close(t.id), theme::ERR_TX)),
        Some(t) if matches!(t.phase, Phase::Connecting) => None,
        Some(t) => Some((&PLUG, tr("다시 연결"), Message::Retry(t.id), theme::OK)),
        None => Some((&PLUG, tr("연결"), Message::HostConnect(alias.to_string()), theme::OK)),
    };
    let Some((icon, label, msg, color)) = action.filter(|_| hovered) else {
        let icon = if live { &PLUG } else { &UNPLUG };
        let color = if active { Color::WHITE } else { open_tab.map_or(theme::TEXT_3, crate::ui::tab_color) };
        return container(crate::icons::tinted(icon, 14.0, color)).center(24).into();
    };
    let fg = if active { Color::WHITE } else { color };
    let b = button(container(crate::icons::tinted(icon, 14.0, fg)).center(24)).padding(0).on_press(msg).style(move |_, st| button::Style {
        background: Some(Background::Color(alpha(if active { Color::WHITE } else { fg }, if matches!(st, button::Status::Hovered | button::Status::Pressed) { 0.25 } else { 0.12 }))),
        border: Border { radius: 6.0.into(), ..Default::default() },
        ..Default::default()
    });
    container(crate::ui::tip(b, label, iced::widget::tooltip::Position::Right)).id(HOST_ACTION).into()
}

/// 편집 / 삭제 for a sidebar host. Only servers saved here can be deleted
/// (not ones from ~/.ssh/config).
fn host_menu<'a>(app: &CoShell, alias: &str) -> El<'a> {
    let item = |label: &'a str, color: Color, msg: Option<Message>| {
        button(text(label).size(12.5))
            .width(Length::Fill)
            .padding([5, 10])
            .on_press_maybe(msg)
            .style(move |_, st| {
                let hover = matches!(st, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: hover.then_some(Background::Color(theme::ACCENT)),
                    text_color: match st {
                        button::Status::Disabled => theme::TEXT_3,
                        _ if hover => theme::ON_ACCENT,
                        _ => color,
                    },
                    border: Border { radius: 6.0.into(), ..Default::default() },
                    ..Default::default()
                }
            })
    };
    let deletable = app.host_store.get(alias).is_some() && app.user_config_entry(alias).is_none();
    let col = column![
        item(tr("편집"), theme::TEXT, Some(Message::HostEdit(alias.to_string()))),
        item(tr("삭제"), theme::ERR_TX, deletable.then(|| Message::HostMenuDelete(alias.to_string()))),
    ]
    .spacing(2);
    container(col)
        .padding(4)
        .width(HOST_MENU_W)
        .style(|_| container::Style {
            background: Some(Background::Color(theme::BG_2)),
            border: Border { color: theme::LINE_2, width: 1.0, radius: R_CONTROL.into() },
            shadow: iced::Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.45), offset: iced::Vector::new(0.0, 8.0), blur_radius: 24.0 },
            ..Default::default()
        })
        .into()
}

fn snippets(app: &CoShell) -> El<'_> {
    let q = app.host_filter.trim().to_lowercase();
    let live = app.live_tab().is_some() && !app.host_view;
    let mut col = column![].spacing(8);
    let mut any = false;
    for (i, sn) in app.snippets().into_iter().enumerate() {
        if !q.is_empty() && ![&sn.title, &sn.category, &sn.cmd].iter().any(|s| s.to_lowercase().contains(&q)) {
            continue;
        }
        any = true;
        let tag: El<'_> = if sn.category.is_empty() {
            Space::new().into()
        } else {
            container(text(sn.category).size(10).color(theme::TEXT_2))
                .padding([1, 6])
                .style(|_| container::Style {
                    background: Some(Background::Color(theme::CHIP)),
                    border: Border { radius: 6.0.into(), ..Default::default() },
                    ..Default::default()
                })
                .into()
        };
        let edit = button(crate::icons::tinted(&PENCIL, 11.0, theme::TEXT_3)).padding(4).style(btn_style(Btn::Ghost)).on_press(Message::SnippetEdit(i));
        let small = |label: &'static str, primary: bool, msg: Option<Message>| {
            button(text(tr(label)).size(10.5)).padding([2, 9]).on_press_maybe(msg).style(crate::ui::tint_style(
                if primary { theme::ACCENT_TX } else { theme::TEXT_2 },
                primary,
            ))
        };
        let card = column![
            row![text(sn.title).size(12).font(SEMIBOLD).color(theme::TEXT).width(Length::Fill), tag].spacing(8).align_y(Alignment::Center),
            container(mono(format!("$ {}", sn.cmd), 11.0, theme::ACCENT_TX)).width(Length::Fill).clip(true),
            row![
                container(crate::ui::tip(edit, tr("편집"), iced::widget::tooltip::Position::Top)).id(SNIPPET_EDIT),
                Space::new().width(Length::Fill),
                small("입력", false, live.then_some(Message::Snippet(i, false))),
                small("실행", true, live.then_some(Message::Snippet(i, true))),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        ]
        .spacing(6);
        col = col.push(container(card).padding(10).width(Length::Fill).style(|_| container::Style {
            background: Some(Background::Color(theme::CARD)),
            border: Border { color: theme::LINE, width: 1.0, radius: R_CARD.into() },
            ..Default::default()
        }));
    }
    if !any {
        let none = if q.is_empty() { tr("저장된 스니펫이 없습니다") } else { tr("일치하는 스니펫이 없습니다") };
        return container(muted(none)).center_x(Length::Fill).padding(28).into();
    }
    if !live {
        col = col.push(container(muted(tr("세션을 열면 터미널에 바로 입력·실행됩니다"))).padding([4, 2]));
    }
    col.into()
}

/// Add or edit a snippet: name, category, command. Deleting asks twice.
pub fn snippet_editor(ed: &SnippetEditor) -> El<'_> {
    let label = |s: &'static str| text(tr(s)).size(11.5).color(theme::TEXT_2);
    let input = |placeholder: &'static str, value: &str, f: SnippetField| {
        text_input(tr(placeholder), value)
            .on_input(move |v| Message::SnippetField(f, v))
            .on_submit(Message::SnippetSave)
            .size(12.5)
            .padding([7, 10])
            .style(input_style)
    };
    let d = &ed.draft;
    let mut col = column![
        text(if ed.index.is_some() { tr("스니펫 편집") } else { tr("새 스니펫") }).size(14),
        Space::new().height(2),
        row![
            column![label("이름"), input("예: 디스크 사용량", &d.title, SnippetField::Title)].spacing(6).width(Length::FillPortion(2)),
            column![label("분류"), input("예: 진단", &d.category, SnippetField::Category)].spacing(6).width(Length::FillPortion(1)),
        ]
        .spacing(10),
        Space::new().height(2),
        label("명령"),
        input("예: df -h", &d.cmd, SnippetField::Cmd).font(crate::fonts::MONO),
    ]
    .spacing(6);
    if let Some(e) = &ed.error {
        col = col.push(text(e.clone()).size(11.5).color(theme::ERR_TX));
    }
    let delete: El<'_> = match (ed.index, ed.confirm_delete) {
        (None, _) => Space::new().into(),
        (Some(_), true) => btn(tr("정말 삭제"), Btn::Danger, Some(Message::SnippetDelete)),
        (Some(_), false) => btn(tr("스니펫 삭제"), Btn::Ghost, Some(Message::SnippetDeleteAsk)),
    };
    col = col.push(Space::new().height(6)).push(
        row![
            delete,
            Space::new().width(Length::Fill),
            btn(tr("취소"), Btn::Ghost, Some(Message::SnippetEditorClose)),
            btn(tr("저장"), Btn::Primary, Some(Message::SnippetSave)),
        ]
        .spacing(8),
    );
    container(col).padding(20).width(SNIPPET_EDITOR_W).style(card_style(theme::LINE_2)).into()
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
            theme::OK
        };
        let value = v.map_or("—".to_string(), |x| format!("{x}%"));
        let fill_w = f32::from(val) / 100.0;
        column![
            row![
                text(label).size(11).font(crate::ui::MEDIUM).color(theme::TEXT_2),
                Space::new().width(Length::Fill),
                text(value).size(11).font(if val >= 85 { crate::fonts::weight(crate::fonts::MONO, iced::font::Weight::Semibold) } else { crate::fonts::MONO }).color(if val >= 85 { theme::WARN } else { theme::TEXT }),
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
                background: Some(Background::Color(alpha(Color::WHITE, 0.09))),
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
            dot(if live { theme::OK } else { theme::TEXT_3 }, 8.0),
            container(text(t.display_name().to_string()).size(12).font(crate::fonts::weight(crate::fonts::MONO, iced::font::Weight::Semibold)).color(theme::TEXT).wrapping(text::Wrapping::None))
                .width(Length::Fill)
                .clip(true),
            container(text(if live { "LIVE" } else { "OFF" }).size(10).font(crate::ui::MEDIUM).color(theme::TEXT_3)).padding([1, 6]).style(|_| container::Style {
                background: Some(Background::Color(theme::CHIP)),
                border: Border { radius: 6.0.into(), ..Default::default() },
                ..Default::default()
            }),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
        bar("CPU".into(), st.map(|s| s.cpu)),
        bar("MEM".into(), st.map(|s| s.mem)),
        bar(format!("DISK {mount}"), st.map(|s| s.disk)),
    ]
    .spacing(10);
    Some(
        container(col)
            .padding(14)
            .width(Length::Fill)
            .style(|_| container::Style {
                background: Some(Background::Color(alpha(theme::BG_2, 0.65))),
                border: Border { color: theme::LINE, width: 1.0, radius: 16.0.into() },
                ..Default::default()
            })
            .into(),
    )
}
