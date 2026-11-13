//! Server list page: hosts grouped like a card board.

use crate::i18n::{tf, tr};
use coshell_ssh::DEFAULT_PORT;
use iced::widget::{Space, button, column, container, row, scrollable, text, text_input};
use iced::{Alignment, Background, Border, Font, Length, Padding};

use crate::app::{CoShell, Message};
use crate::hosts::Env;
use crate::theme::{self, alpha};
use crate::ui::{Btn, El, btn, btn_style, input_style, mono, muted};
use crate::ui_hosts::env_tag;

const CARD_W: f32 = 270.0;
const CARD_H: f32 = 104.0;
/// Group for hosts that live only in ~/.ssh/config.
const SSH_CONFIG_GROUP: &str = "~/.ssh/config";
const NO_GROUP: &str = "그룹 없음";

struct Card {
    alias: String,
    title: String,
    command: String,
    via: Option<String>,
    env: Env,
}

fn cards(app: &CoShell) -> Vec<(String, Vec<Card>)> {
    let q = app.host_filter.trim().to_lowercase();
    let mut groups: Vec<(String, Vec<Card>)> = app.groups().into_iter().map(|g| (g, Vec::new())).collect();
    for e in &app.hosts {
        let def = app.host_store.get(&e.alias);
        let own = def.is_some_and(|d| d.is_definition());
        let group = def
            .and_then(|d| d.group.clone())
            .unwrap_or_else(|| if own { NO_GROUP.into() } else { SSH_CONFIG_GROUP.into() });
        // Same shape as a real ssh command: no user part when none is configured.
        let user = e.user.as_deref().map(|u| format!("{u}@")).unwrap_or_default();
        let port = e.port.filter(|p| *p != DEFAULT_PORT).map(|p| format!(":{p}")).unwrap_or_default();
        let card = Card {
            alias: e.alias.clone(),
            title: app.display_of(&e.alias),
            command: format!("ssh {user}{}{port}", e.host()),
            via: e.proxy_jump.as_ref().map(|j| j.split(',').map(|a| app.display_of(a.trim())).collect::<Vec<_>>().join(" › ")),
            env: app.env_of(&e.alias),
        };
        let hay = format!("{} {} {} {group}", card.title, card.alias, card.command).to_lowercase();
        if !q.is_empty() && !hay.contains(&q) {
            continue;
        }
        match groups.iter_mut().find(|(g, _)| *g == group) {
            Some((_, v)) => v.push(card),
            None => groups.push((group, vec![card])),
        }
    }
    // Named groups first (store order), then ungrouped, then ~/.ssh/config.
    let rank = |g: &str| match g {
        NO_GROUP => 1,
        SSH_CONFIG_GROUP => 2,
        _ => 0,
    };
    groups.sort_by_key(|(g, _)| rank(g));
    groups.retain(|(_, v)| !v.is_empty());
    groups
}

fn card(c: Card) -> El<'static> {
    let accent = crate::ui_hosts::env_color(c.env);
    let head = row![
        text(c.title).size(14).color(theme::TEXT),
        env_tag(c.env),
        Space::new().width(Length::Fill),
        button(text("⋯").size(14).color(theme::TEXT_3))
            .padding([0, 6])
            .style(btn_style(Btn::Ghost))
            .on_press(Message::HostEdit(c.alias.clone())),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    let rule = container(Space::new()).height(1).width(Length::Fill).style(|_| container::Style {
        background: Some(Background::Color(theme::LINE_2)),
        ..Default::default()
    });
    let mut body = column![head, rule, mono(c.command, 12.0, theme::TEXT_2)].spacing(10);
    if let Some(v) = c.via {
        body = body.push(text(format!("via {v}")).size(11).color(theme::TEXT_3));
    }
    button(body)
        .width(CARD_W)
        .height(CARD_H)
        .padding([12, 14])
        .on_press(Message::HostConnect(c.alias))
        .style(move |_, status| {
            let hover = matches!(status, button::Status::Hovered | button::Status::Pressed);
            iced::widget::button::Style {
                background: Some(Background::Color(if hover { theme::BG_3 } else { theme::BG_2 })),
                text_color: theme::TEXT,
                border: Border {
                    color: match (accent, hover) {
                        (Some(a), _) => alpha(a, if hover { 0.8 } else { 0.45 }),
                        (None, true) => alpha(theme::ACCENT, 0.6),
                        (None, false) => theme::LINE_2,
                    },
                    width: 1.0,
                    radius: 12.0.into(),
                },
                ..Default::default()
            }
        })
        .into()
}

pub fn page(app: &CoShell) -> El<'_> {
    let total = app.hosts.len();
    let header = row![
        text(tr("서버")).size(18),
        muted(tf("{0}개", &[&total])),
        Space::new().width(Length::Fill),
        text_input(tr("이름, 주소, 그룹 검색"), &app.host_filter)
            .on_input(Message::HostFilter)
            .size(12.5)
            .padding([6, 10])
            .width(260)
            .style(input_style),
        btn(tr("+ 새 서버"), Btn::Primary, Some(Message::HostNewIn(None))),
    ]
    .spacing(12)
    .align_y(Alignment::Center);

    let mut col = column![header].spacing(22);
    let groups = cards(app);
    if groups.is_empty() {
        col = col.push(
            container(
                column![
                    text(if total == 0 { tr("저장된 서버가 없습니다") } else { tr("검색 결과가 없습니다") }).size(14),
                    muted(tr("‘+ 새 서버’를 눌러 추가하세요. ssh 명령을 붙여넣으면 접속 정보가 자동으로 채워집니다.")),
                ]
                .spacing(6)
                .align_x(Alignment::Center),
            )
            .center_x(Length::Fill)
            .padding(40),
        );
    }
    for (group, list) in groups {
        let addable = group != SSH_CONFIG_GROUP;
        let target = (group != NO_GROUP).then(|| group.clone());
        let mut head = row![
            text(tr(&group).to_string()).size(14).font(Font { weight: iced::font::Weight::Semibold, ..Font::DEFAULT }),
            muted(format!("{}", list.len())),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        if addable {
            head = head.push(Space::new().width(Length::Fill)).push(btn(tr("+ 추가"), Btn::Ghost, Some(Message::HostNewIn(target))));
        }
        let grid = row(list.into_iter().map(card).collect::<Vec<_>>()).spacing(14).wrap().vertical_spacing(14);
        col = col.push(column![head, grid].spacing(10));
    }
    container(scrollable(container(col).padding(Padding { top: 20.0, right: 24.0, bottom: 28.0, left: 24.0 })))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_| container::Style { background: Some(Background::Color(theme::BG_0)), ..Default::default() })
        .into()
}
