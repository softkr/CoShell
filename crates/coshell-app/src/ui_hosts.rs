//! Host editor and the production attach confirmation.

use crate::i18n::{tf, tr};
use iced::widget::{Space, button, column, container, pick_list, row, scrollable, text, text_input};
use iced::{Alignment, Background, Border, Color, Font, Length};

use crate::ai::Attachment;
use crate::app::{CoShell, HostEditor, HostField, Message};
use crate::hosts::Env;
use crate::theme::{self, alpha};
use crate::ui::{Btn, El, btn, btn_style, card_style, input_style, mono, muted};

const EDITOR_W: f32 = 620.0;
const PREVIEW_H: f32 = 300.0;
const NO_JUMP: &str = "사용 안 함";

pub fn env_color(env: Env) -> Option<Color> {
    match env {
        Env::Prod => Some(theme::ENV_PROD),
        Env::Stage => Some(theme::ENV_STAGE),
        Env::Dev => Some(theme::ENV_DEV),
        Env::None => None,
    }
}

/// Small coloured "운영" / "스테이징" / "개발" tag.
pub fn env_tag<'a>(env: Env) -> El<'a> {
    let Some(c) = env_color(env) else { return Space::new().into() };
    container(text(env.label()).size(10).color(c))
        .padding([1, 5])
        .style(move |_| container::Style {
            background: Some(Background::Color(alpha(c, 0.15))),
            border: Border { radius: 4.0.into(), ..Default::default() },
            ..Default::default()
        })
        .into()
}

fn env_segment(current: Env) -> El<'static> {
    let buttons = Env::ALL.map(|e| {
        let on = e == current;
        let c = env_color(e).unwrap_or(theme::TEXT_2);
        button(text(e.label()).size(12).color(if on { c } else { theme::TEXT_2 }))
            .padding([5, 12])
            .on_press(Message::HostEnv(e))
            .style(move |_, _| iced::widget::button::Style {
                background: Some(Background::Color(if on { theme::BG_3 } else { Color::TRANSPARENT })),
                text_color: theme::TEXT,
                border: Border { color: if on { alpha(c, 0.6) } else { theme::LINE_2 }, width: 1.0, radius: 6.0.into() },
                ..Default::default()
            })
            .into()
    });
    row(buttons).spacing(6).into()
}

fn field<'a>(label: &'a str, placeholder: &'a str, value: &'a str, f: HostField) -> El<'a> {
    column![
        text(label).size(11.5).color(theme::TEXT_2),
        text_input(placeholder, value)
            .on_input(move |v| Message::HostField(f, v))
            .on_submit(Message::HostSave(false))
            .font(Font::MONOSPACE)
            .size(12)
            .padding([6, 10])
            .style(input_style),
    ]
    .spacing(4)
    .width(Length::Fill)
    .into()
}

/// Saved in the OS keychain only; the value is never shown again.
fn password_field(ed: &HostEditor) -> El<'_> {
    let saved = ed.saved_password.is_some() && !ed.forget_password;
    let placeholder = if saved { tr("저장됨 · 바꾸려면 새 비밀번호 입력") } else { tr("비워 두면 접속할 때 물어봅니다") };
    let input = text_input(placeholder, ed.password.as_str())
        .on_input(|v| Message::HostField(HostField::Password, v))
        .on_submit(Message::HostSave(false))
        .secure(true)
        .font(Font::MONOSPACE)
        .size(12)
        .padding([6, 10])
        .style(input_style);
    let mut line = row![input].spacing(8).align_y(Alignment::Center);
    if saved {
        line = line.push(btn(tr("저장된 비밀번호 삭제"), Btn::Ghost, Some(Message::HostForgetPassword)));
    }
    let note = if ed.forget_password && ed.saved_password.is_some() {
        tr("저장하면 키체인에서 비밀번호를 지웁니다.")
    } else {
        tr("키체인에만 저장되고 설정 파일에는 남지 않습니다.")
    };
    column![text(tr("비밀번호")).size(11.5).color(theme::TEXT_2), line, muted(note)].spacing(4).into()
}

pub fn editor<'a>(app: &'a CoShell, ed: &'a HostEditor) -> El<'a> {
    let d = &ed.def;
    let title = match &ed.original {
        Some(a) => tf("서버 편집 · {0}", &[&app.display_of(a)]),
        None => tr("새 서버").into(),
    };
    let mut col = column![text(title).size(14)].spacing(10);

    if ed.external.is_none() {
        col = col.push(
            column![
                text(tr("ssh 명령으로 채우기")).size(11.5).color(theme::TEXT_2),
                row![
                    text_input(tr("예: ssh -p 2222 user@192.168.0.10"), &ed.quick)
                        .on_input(|v| Message::HostField(HostField::Quick, v))
                        .on_submit(Message::HostQuickApply)
                        .font(Font::MONOSPACE)
                        .size(12)
                        .padding([6, 10])
                        .style(input_style),
                    btn(tr("채우기"), Btn::Plain, (!ed.quick.trim().is_empty()).then_some(Message::HostQuickApply)),
                ]
                .spacing(8),
            ]
            .spacing(4),
        );
    }
    col = col.push(
        row![
            field(tr("이름"), tr("예: 웹 서버"), d.name.as_deref().unwrap_or(""), HostField::Name),
            field(tr("그룹"), tr("예: 회사"), d.group.as_deref().unwrap_or(""), HostField::Group),
        ]
        .spacing(10),
    );
    let groups = app.groups();
    if !groups.is_empty() {
        let chips = groups.into_iter().map(|g| {
            let on = d.group.as_deref() == Some(g.as_str());
            button(text(g.clone()).size(11).color(if on { theme::TEXT } else { theme::TEXT_2 }))
                .padding([2, 8])
                .on_press(Message::HostGroupPick(g))
                .style(move |_, _| iced::widget::button::Style {
                    background: Some(Background::Color(if on { theme::BG_3 } else { theme::BG_2 })),
                    text_color: theme::TEXT,
                    border: Border { color: theme::LINE_2, width: 1.0, radius: 10.0.into() },
                    ..Default::default()
                })
                .into()
        });
        col = col.push(row![muted(tr("기존 그룹"))].extend(chips).spacing(6).align_y(Alignment::Center).wrap());
    }

    col = col.push(text(tr("환경")).size(11.5).color(theme::TEXT_2)).push(env_segment(d.env));
    if d.env == Env::Prod {
        col = col.push(muted(tr("운영 서버는 빨간색으로 표시되고, AI에 터미널 출력을 보내기 전에 한 번 더 확인합니다.")));
    }

    match &ed.external {
        Some(e) => {
            let user = e.user.as_deref().map(|u| format!("{u}@")).unwrap_or_default();
            let port = e.port.map(|p| format!(":{p}")).unwrap_or_default();
            let via = e.proxy_jump.as_deref().map(|j| format!("  ·  via {j}")).unwrap_or_default();
            col = col
                .push(
                    container(
                        column![
                            mono(format!("{user}{}{port}{via}", e.host()), 12.0, theme::TEXT),
                            muted(tr("~/.ssh/config에 있는 서버입니다. 접속 정보는 그 파일에서 바꾸고, 여기서는 이름·그룹·환경만 정합니다.")),
                        ]
                        .spacing(4),
                    )
                    .padding(10)
                    .width(Length::Fill)
                    .style(|_| container::Style {
                        background: Some(Background::Color(theme::BG_2)),
                        border: Border { color: theme::LINE_2, width: 1.0, radius: 8.0.into() },
                        ..Default::default()
                    }),
                )
                .push(password_field(ed));
        }
        None => {
            col = col.push(
                row![
                    field(tr("주소"), tr("IP 또는 도메인"), d.hostname.as_deref().unwrap_or(""), HostField::HostName),
                    field(tr("사용자"), tr("예: ubuntu"), d.user.as_deref().unwrap_or(""), HostField::User),
                    container(field(tr("포트"), "22", &ed.port, HostField::Port)).width(100),
                ]
                .spacing(10),
            );
            col = col.push(password_field(ed));
            let n_extra = usize::from(d.proxy_jump.is_some()) + usize::from(d.identity_file.is_some()) + d.forwards.len();
            let summary = if n_extra > 0 { tf("고급 설정 ({0})", &[&n_extra]) } else { tr("고급 설정").to_string() };
            col = col.push(
                button(row![text(if ed.advanced { "▾" } else { "▸" }).size(11), text(summary).size(12)].spacing(6))
                    .padding([3, 4])
                    .style(btn_style(Btn::Ghost))
                    .on_press(Message::HostAdvanced),
            );
            if ed.advanced {
                let mut jumps: Vec<String> = vec![tr(NO_JUMP).into()];
                jumps.extend(app.hosts.iter().map(|h| h.alias.clone()).filter(|a| *a != d.alias));
                let jump_sel = Some(d.proxy_jump.clone().unwrap_or_else(|| tr(NO_JUMP).into()));
                col = col
                    .push(
                        row![
                            field(tr("ssh 별칭"), tr("비워 두면 자동으로 만듭니다"), &d.alias, HostField::Alias),
                            column![
                                text(tr("경유 서버")).size(11.5).color(theme::TEXT_2),
                                pick_list(jumps, jump_sel, |j: String| {
                                    Message::HostField(HostField::ProxyJump, if j == tr(NO_JUMP) { String::new() } else { j })
                                })
                                .text_size(12)
                                .padding([6, 10])
                                .width(Length::Fill),
                            ]
                            .spacing(4)
                            .width(Length::Fill),
                        ]
                        .spacing(10),
                    )
                    .push(field(tr("SSH 키 파일"), tr("비워 두면 ssh-agent와 기본 키를 사용합니다"), d.identity_file.as_deref().unwrap_or(""), HostField::Identity));

                let mut fw = column![
                    text(tr("자동 터널")).size(11.5).color(theme::TEXT_2),
                    muted(tr("접속하면 바로 열립니다. 접속 중에 상태바의 ‘터널’에서 추가할 수도 있습니다.")),
                ]
                .spacing(4);
                for (i, f) in d.forwards.iter().enumerate() {
                    fw = fw.push(
                        row![
                            mono(f.clone(), 12.0, theme::TEXT),
                            Space::new().width(Length::Fill),
                            button(text("×").size(12).color(theme::TEXT_3)).padding([0, 6]).style(btn_style(Btn::Ghost)).on_press(Message::HostForwardRemove(i)),
                        ]
                        .align_y(Alignment::Center),
                    );
                }
                fw = fw.push(
                    row![
                        text_input(tr("예: L:3306:localhost:3306"), &ed.forward_input)
                            .on_input(Message::HostForwardInput)
                            .on_submit(Message::HostForwardAdd)
                            .font(Font::MONOSPACE)
                            .size(12)
                            .padding([6, 10])
                            .style(input_style),
                        btn(tr("추가"), Btn::Plain, Some(Message::HostForwardAdd)),
                    ]
                    .spacing(8),
                );
                col = col.push(fw);
            }
        }
    }

    if let Some(e) = &ed.error {
        col = col.push(text(e.clone()).size(11.5).color(theme::ERR_TX));
    }
    let delete_label = if ed.external.is_some() { tr("표시 설정 지우기") } else { tr("서버 삭제") };
    let can_delete = ed.original.as_deref().is_some_and(|a| app.host_store.get(a).is_some());
    col = col.push(Space::new().height(4)).push(
        row![
            if ed.confirm_delete {
                btn(tr("정말 삭제"), Btn::Danger, can_delete.then_some(Message::HostDelete))
            } else {
                btn(delete_label, Btn::Ghost, can_delete.then_some(Message::HostDeleteAsk))
            },
            Space::new().width(Length::Fill),
            btn(tr("취소"), Btn::Ghost, Some(Message::HostEditorClose)),
            btn(tr("저장"), Btn::Plain, Some(Message::HostSave(false))),
            btn(tr("저장하고 연결"), Btn::Primary, Some(Message::HostSave(true))),
        ]
        .spacing(8),
    );
    let border = env_color(d.env).map_or(theme::LINE_2, |c| alpha(c, 0.6));
    container(col).padding(18).width(EDITOR_W).style(card_style(border)).into()
}

pub fn attach_confirm<'a>(app: &'a CoShell, tab_name: &'a str, a: &'a Attachment) -> El<'a> {
    let p = app.ai.active();
    let host = p.base_url.split("://").nth(1).unwrap_or(&p.base_url).split('/').next().unwrap_or_default().to_string();
    container(
        column![
            row![crate::icons::icon(&crate::icons::SAFE_GUARD, 18.0), text(tr("AI로 보낼 내용 확인")).size(14), Space::new().width(Length::Fill), env_tag(Env::Prod)]
                .spacing(8)
                .align_y(Alignment::Center),
            container(
                text(tf("운영 서버({0})의 터미널 출력입니다. 아래 내용이 {1} ({2})로 전송됩니다.", &[&tab_name, &p.name, &host]))
                    .size(12)
                    .color(theme::WARN)
            )
            .padding(10)
            .width(Length::Fill)
            .style(|_| container::Style {
                background: Some(Background::Color(alpha(theme::WARN, 0.08))),
                border: Border { color: alpha(theme::WARN, 0.35), width: 1.0, radius: 7.0.into() },
                ..Default::default()
            }),
            muted(tf("{0} · 마스킹 {1}건", &[&a.label, &a.secrets])),
            container(scrollable(container(mono(a.masked.clone(), 11.5, theme::TEXT)).padding(10)).height(PREVIEW_H))
                .width(Length::Fill)
                .style(|_| container::Style {
                    background: Some(Background::Color(theme::BG_0)),
                    border: Border { color: theme::LINE_2, width: 1.0, radius: 8.0.into() },
                    ..Default::default()
                }),
            row![
                Space::new().width(Length::Fill),
                btn(tr("취소"), Btn::Ghost, Some(Message::AttachCancel)),
                btn(tr("첨부"), Btn::Primary, Some(Message::AttachConfirm)),
            ]
            .spacing(8),
        ]
        .spacing(10),
    )
    .padding(18)
    .width(EDITOR_W)
    .style(card_style(alpha(theme::ENV_PROD, 0.5)))
    .into()
}
