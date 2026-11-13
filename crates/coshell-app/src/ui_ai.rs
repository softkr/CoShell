//! AI panel and AI settings dialog.

use crate::i18n::{tf, tr};
use coshell_ai::profile::KeySource;
use coshell_ai::{Format, Role, endpoint};
use iced::keyboard::key::Named;
use iced::widget::text::Span;
use iced::widget::text_editor::{self, Binding, KeyPress};
use iced::widget::{Space, button, column, container, pick_list, rich_text, row, scrollable, span, text, text_input};
use iced::{Alignment, Background, Border, Color, Font, Length, Padding};

use crate::ai::{AI_INPUT, Chat, Settings, Turn};
use crate::app::{CoShell, Message, Phase, Tab, TabId};
use crate::assist::{self, Risk, Segment};
use crate::theme::{self, alpha};
use crate::ui::{Btn, El, btn, card_style, input_style, mono, muted};

const SETTINGS_W: f32 = 780.0;
const COMPOSER_H: f32 = 76.0;
/// Space left beside a user bubble.
const BUBBLE_MARGIN: f32 = 60.0;

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProfileChoice {
    id: String,
    name: String,
}
impl std::fmt::Display for ProfileChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FormatChoice(Format);
impl std::fmt::Display for FormatChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.label())
    }
}

fn panel_style(_: &iced::Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(theme::BG_1)),
        border: Border { color: theme::LINE, width: 1.0, radius: 0.0.into() },
        text_color: Some(theme::TEXT),
        ..Default::default()
    }
}

/// What the assistant is told it is looking at.
pub fn target_label(tab: &Tab) -> String {
    if tab.is_local() {
        "local shell on this computer".into()
    } else {
        let mut r = tab.route.clone();
        let target = r.pop().unwrap_or_default();
        if r.is_empty() { format!("SSH {target}") } else { format!("SSH {target} (via {})", r.join(" -> ")) }
    }
}

pub fn panel(app: &CoShell) -> El<'_> {
    let ai = &app.ai;
    let active = ai.active();
    let choices: Vec<ProfileChoice> =
        ai.store.profiles.iter().map(|p| ProfileChoice { id: p.id.clone(), name: p.name.clone() }).collect();
    let selected = choices.iter().find(|c| c.id == active.id).cloned();
    let tab = app.active_tab();

    let mut header = row![
        crate::icons::icon(&crate::icons::AI_SPARK, 15.0),
        text("AI").size(13),
        pick_list(choices, selected, |c: ProfileChoice| Message::AiProfile(c.id)).text_size(12).padding([3, 8]),
        Space::new().width(Length::Fill),
        btn(tr("설정"), Btn::Ghost, Some(Message::AiSettingsOpen)),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    if let Some(t) = tab {
        header = header.push(btn(tr("새 대화"), Btn::Ghost, Some(Message::AiNewChat(t.id))));
    }

    let context = row![
        muted(tr("대상")),
        text(tab.map_or(tr("없음").to_string(), |t| t.display_name().to_string())).size(11).color(theme::TEXT_2),
        Space::new().width(Length::Fill),
        mono(active.model.clone(), 10.5, theme::TEXT_3),
    ]
    .spacing(6)
    .align_y(Alignment::Center);

    let body: El<'_> = match tab {
        None => container(muted(tr("세션을 열면 해당 서버를 기준으로 답합니다"))).center(Length::Fill).into(),
        Some(t) => conversation(t, app.settings.ai_panel_width),
    };
    let composer: El<'_> = match tab {
        Some(t) => composer(t),
        None => Space::new().into(),
    };
    container(
        column![
            container(header).padding([6, 10]),
            container(context).padding([4, 12]),
            container(Space::new()).height(1).width(Length::Fill).style(|_| container::Style {
                background: Some(Background::Color(theme::LINE)),
                ..Default::default()
            }),
            body,
            composer,
        ]
        .height(Length::Fill),
    )
    .width(app.settings.ai_panel_width)
    .height(Length::Fill)
    .style(panel_style)
    .into()
}

fn conversation(tab: &Tab, panel_w: f32) -> El<'_> {
    let chat = &tab.chat;
    if chat.turns.is_empty() && chat.error.is_none() {
        return welcome(tab);
    }
    let last = chat.turns.len().saturating_sub(1);
    let mut col = column![].spacing(14).padding(Padding { top: 12.0, right: 14.0, bottom: 12.0, left: 12.0 });
    for (i, t) in chat.turns.iter().enumerate() {
        col = col.push(match t.role {
            Role::User => user_turn(t, panel_w),
            Role::Assistant => assistant_turn(tab, t, i == last && chat.is_streaming()),
        });
    }
    if let Some(e) = &chat.error {
        col = col.push(error_card(tab.id, e));
    }
    scrollable(col).anchor_bottom().height(Length::Fill).into()
}

fn welcome(tab: &Tab) -> El<'_> {
    let id = tab.id;
    let suggestions: &[(&str, bool)] = if tab.is_local() {
        &[(tr("마지막 출력에서 문제를 찾아줘"), true), (tr("이 폴더에서 용량 큰 파일 찾는 명령"), false)]
    } else {
        &[(tr("마지막 출력에서 문제를 찾아줘"), true), (tr("디스크를 많이 쓰는 디렉터리 찾는 명령"), false), (tr("최근 1시간 에러 로그 보는 명령"), false)]
    };
    let mut col = column![
        text(tf("{0}에 대해 물어보세요", &[&tab.display_name()])).size(13),
        muted(tr("터미널 출력은 직접 첨부할 때만 전송됩니다")),
        Space::new().height(10),
    ]
    .spacing(4)
    .align_x(Alignment::Center);
    for (s, attach) in suggestions {
        col = col.push(
            button(text(*s).size(12).color(theme::TEXT_2))
                .width(Length::Fill)
                .padding([8, 12])
                .on_press(Message::AiSuggest(id, (*s).to_string(), *attach))
                .style(|_, st| iced::widget::button::Style {
                    background: Some(Background::Color(theme::BG_2)),
                    text_color: theme::TEXT_2,
                    border: Border {
                        color: if matches!(st, iced::widget::button::Status::Hovered) { alpha(theme::ACCENT, 0.5) } else { theme::LINE },
                        width: 1.0,
                        radius: 8.0.into(),
                    },
                    ..Default::default()
                }),
        );
    }
    container(col).padding(16).center(Length::Fill).into()
}

fn chip<'a>(label: String) -> El<'a> {
    container(text(label).size(11).color(theme::TEXT_2))
        .padding([2, 8])
        .style(|_| container::Style {
            background: Some(Background::Color(theme::BG_2)),
            border: Border { color: theme::LINE_2, width: 1.0, radius: 5.0.into() },
            ..Default::default()
        })
        .into()
}

fn user_turn(t: &Turn, panel_w: f32) -> El<'_> {
    let mut col = column![].spacing(6);
    if let Some(a) = &t.attachment {
        col = col.push(chip(format!("📎 {a}")));
    }
    col = col.push(text(t.text.clone()).size(12.5));
    row![
        Space::new().width(Length::FillPortion(1)),
        container(col).padding([8, 12]).max_width(panel_w - BUBBLE_MARGIN).style(|_| container::Style {
            background: Some(Background::Color(theme::BG_3)),
            border: Border { radius: 12.0.into(), ..Default::default() },
            text_color: Some(theme::TEXT),
            ..Default::default()
        })
    ]
    .into()
}

/// Prose with `inline code` highlighted.
fn prose<'a>(s: &str) -> El<'a> {
    let spans: Vec<Span<'a, (), Font>> = s
        .split('`')
        .enumerate()
        .map(|(i, part)| {
            let sp = span(part.to_string());
            if i % 2 == 1 {
                sp.font(Font::MONOSPACE).color(theme::TEXT).background(theme::BG_3)
            } else {
                sp.color(theme::TEXT)
            }
        })
        .collect();
    rich_text(spans).size(12.5).into()
}

fn assistant_turn<'a>(tab: &'a Tab, t: &'a Turn, streaming: bool) -> El<'a> {
    let mut col = column![
        row![crate::icons::icon(&crate::icons::AI_SPARK, 12.0), mono(t.model.clone(), 10.5, theme::TEXT_3)].spacing(6).align_y(Alignment::Center)
    ]
    .spacing(8);
    for seg in assist::segments(&t.text) {
        col = col.push(match seg {
            Segment::Text(s) => prose(&s),
            Segment::Code { lang, code, closed } => command_card(tab, lang, code, closed),
        });
    }
    if streaming {
        col = col.push(text("▍").size(12).color(theme::ACCENT));
    }
    if t.stopped {
        col = col.push(muted(tr("응답을 중지했습니다")));
    }
    col.into()
}

fn risk_badge<'a>(r: Risk) -> El<'a> {
    let c = match r {
        Risk::Low => theme::OK,
        Risk::Mid => theme::WARN,
        Risk::High => theme::ERR,
    };
    container(text(r.label()).size(10.5).color(c))
        .padding([1, 6])
        .style(move |_| container::Style {
            background: Some(Background::Color(alpha(c, 0.12))),
            border: Border { radius: 4.0.into(), ..Default::default() },
            ..Default::default()
        })
        .into()
}

fn command_card<'a>(tab: &'a Tab, lang: String, code: String, closed: bool) -> El<'a> {
    let shell = assist::is_shell(&lang);
    let cmd = assist::command_text(&code);
    let r = assist::risk(&cmd);
    let bracketed = match &tab.phase {
        Phase::Live(s) => {
            s.term().lock().mode().contains(coshell_term::alacritty_terminal::term::TermMode::BRACKETED_PASTE)
        }
        _ => false,
    };
    let can_insert = shell && closed && !tab.exited && matches!(tab.phase, Phase::Live(_))
        && assist::insert_bytes(&cmd, bracketed).is_some();
    let header = row![
        mono(if lang.is_empty() { "bash".to_string() } else { lang }, 10.5, theme::TEXT_3),
        text(tab.display_name().to_string()).size(10.5).color(theme::TEXT_2),
        Space::new().width(Length::Fill),
        if shell { risk_badge(r) } else { Space::new().into() },
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    let warn: El<'a> = if shell && r != Risk::Low && !tab.is_local() {
        text(tr("⚠ 원격 서버 상태를 변경합니다")).size(10.5).color(theme::WARN).into()
    } else {
        Space::new().into()
    };
    let insert_hint = if shell && cmd.contains('\n') && !bracketed { tr("여러 줄은 복사해서 사용") } else { tr("터미널에 입력") };
    let footer = row![
        warn,
        Space::new().width(Length::Fill),
        btn(tr("복사"), Btn::Ghost, Some(Message::AiCopy(cmd.clone()))),
        btn(insert_hint, Btn::Plain, can_insert.then(|| Message::AiInsert(tab.id, cmd.clone()))),
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    let border = if r == Risk::High && shell { alpha(theme::ERR, 0.5) } else { theme::LINE_2 };
    container(
        column![
            container(header).padding([5, 10]),
            container(mono(code, 12.0, theme::TEXT)).padding([8, 10]).width(Length::Fill),
            container(footer).padding([5, 8]),
        ],
    )
    .style(move |_| container::Style {
        background: Some(Background::Color(theme::BG_0)),
        border: Border { color: border, width: 1.0, radius: 9.0.into() },
        ..Default::default()
    })
    .into()
}

fn error_card<'a>(id: TabId, e: &'a str) -> El<'a> {
    container(
        column![
            text(e.to_string()).size(12).color(theme::ERR_TX),
            row![btn(tr("다시 시도"), Btn::Plain, Some(Message::AiRetry(id))), btn(tr("AI 설정"), Btn::Ghost, Some(Message::AiSettingsOpen))]
                .spacing(6),
        ]
        .spacing(8),
    )
    .padding(12)
    .style(|_| container::Style {
        background: Some(Background::Color(alpha(theme::ERR, 0.06))),
        border: Border { color: alpha(theme::ERR, 0.45), width: 1.0, radius: 9.0.into() },
        ..Default::default()
    })
    .into()
}

fn composer(tab: &Tab) -> El<'_> {
    let id = tab.id;
    let chat: &Chat = &tab.chat;
    let mut col = column![].spacing(6);
    if let Some(a) = &chat.attachment {
        let masked = if a.secrets > 0 { tf(" · 마스킹 {0}", &[&a.secrets]) } else { String::new() };
        col = col.push(
            row![
                chip(format!("📎 {}{masked}", a.label)),
                button(text("×").size(12).color(theme::TEXT_3)).padding([0, 6]).style(crate::ui::btn_style(Btn::Ghost)).on_press(Message::AiDetach(id)),
            ]
            .align_y(Alignment::Center),
        );
    }
    let editor = iced::widget::text_editor(&chat.draft)
        .id(AI_INPUT)
        .placeholder(tr("이 세션에 대해 질문하거나 명령을 요청하세요"))
        .on_action(move |a| Message::AiDraft(id, a))
        .key_binding(move |kp: KeyPress| {
            let focused = matches!(kp.status, text_editor::Status::Focused { .. });
            if focused && kp.key == iced::keyboard::Key::Named(Named::Enter) && !kp.modifiers.shift() {
                Some(Binding::Custom(Message::AiSend(id)))
            } else {
                Binding::from_key_press(kp)
            }
        })
        .height(COMPOSER_H)
        .size(12.5)
        .padding(8)
        .style(|_, st| text_editor::Style {
            background: Background::Color(theme::BG_0),
            border: Border {
                color: if matches!(st, text_editor::Status::Focused { .. }) { alpha(theme::ACCENT, 0.6) } else { theme::LINE_2 },
                width: 1.0,
                radius: 10.0.into(),
            },
            placeholder: theme::TEXT_3,
            value: theme::TEXT,
            selection: alpha(theme::ACCENT, 0.35),
        });
    let can_attach = matches!(tab.phase, Phase::Live(_));
    let action = if chat.is_streaming() {
        btn(tr("중지"), Btn::Plain, Some(Message::AiStop(id)))
    } else {
        btn(tr("전송"), Btn::Primary, Some(Message::AiSend(id)))
    };
    col = col.push(editor).push(
        row![
            btn(tr("출력 첨부"), Btn::Ghost, can_attach.then_some(Message::AiAttach(id))),
            muted(tr("⏎ 전송 · ⇧⏎ 줄바꿈")),
            Space::new().width(Length::Fill),
            action,
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    );
    container(col).padding([10, 12]).into()
}

// ------------------------------------------------------------------ settings

fn key_status_text(s: &Settings) -> (String, Color) {
    match s.key_status.get(s.selected).copied().unwrap_or(KeySource::None) {
        KeySource::Keychain => (tr("키체인에 저장됨").into(), theme::OK),
        KeySource::Env => (tr("환경 변수에서 읽음 (읽기 전용)").into(), theme::TEXT_2),
        KeySource::None if s.profile().is_local() => (tr("키 없음 · 로컬 서버는 선택 사항").into(), theme::TEXT_3),
        KeySource::None => (tr("키 없음").into(), theme::WARN),
    }
}

pub fn settings_dialog(s: &Settings) -> El<'_> {
    let p = s.profile();
    let list = s.draft.profiles.iter().enumerate().fold(column![muted(tr("AI 프로필"))].spacing(4), |col, (i, prof)| {
        let sel = i == s.selected;
        let using = prof.id == s.draft.active;
        col.push(
            button(
                column![
                    row![text(prof.name.clone()).size(12.5), if using { text(tr("사용 중")).size(10).color(theme::ACCENT) } else { text("") }]
                        .spacing(6),
                    text(prof.format.label()).size(10.5).color(theme::TEXT_3),
                ]
                .spacing(1),
            )
            .width(Length::Fill)
            .padding([7, 10])
            .on_press(Message::AiSettingsSelect(i))
            .style(move |_, st| iced::widget::button::Style {
                background: Some(Background::Color(if sel || matches!(st, iced::widget::button::Status::Hovered) {
                    theme::BG_3
                } else {
                    Color::TRANSPARENT
                })),
                text_color: theme::TEXT,
                border: Border { radius: 7.0.into(), ..Default::default() },
                ..Default::default()
            }),
        )
    });
    let list = list.push(btn(tr("+ 프로필 추가"), Btn::Ghost, Some(Message::AiSettingsAdd)));

    let label = |t: &str| text(t.to_string()).size(11.5).color(theme::TEXT_2);
    let (key_text, key_color) = key_status_text(s);
    let formats = [FormatChoice(Format::Anthropic), FormatChoice(Format::OpenAi)];
    let mut model_row = row![
        text_input("model id", &p.model).on_input(Message::AiSettingsModel).font(Font::MONOSPACE).size(12).padding([6, 10]).style(input_style),
        btn(if s.busy { tr("불러오는 중…") } else { tr("목록 불러오기") }, Btn::Plain, (!s.busy).then_some(Message::AiSettingsFetch(false))),
    ]
    .spacing(8);
    if !s.models.is_empty() {
        let sel = s.models.iter().find(|m| **m == p.model).cloned();
        model_row = model_row.push(pick_list(s.models.clone(), sel, Message::AiSettingsModel).text_size(12).padding([5, 8]));
    }
    let status: El<'_> = match &s.status {
        Some(Ok(m)) => text(format!("✓ {m}")).size(11.5).color(theme::OK).into(),
        Some(Err(e)) => text(format!("✗ {e}")).size(11.5).color(theme::ERR_TX).into(),
        None => Space::new().into(),
    };
    let form = column![
        label(tr("이름")),
        text_input("", &p.name).on_input(Message::AiSettingsName).size(12.5).padding([6, 10]).style(input_style),
        label(tr("API 형식")),
        pick_list(formats, Some(FormatChoice(p.format)), |f: FormatChoice| Message::AiSettingsFormat(f.0)).text_size(12).padding([5, 8]),
        muted(match p.format {
            Format::Anthropic => tr("x-api-key 헤더 · system 프롬프트 별도 필드 · max_tokens 필수"),
            Format::OpenAi => tr("Bearer 인증 · OpenAI, Ollama, vLLM, LiteLLM, OpenRouter 등 호환 서버"),
        }),
        label("Base URL"),
        text_input(p.format.default_base(), &p.base_url).on_input(Message::AiSettingsBase).font(Font::MONOSPACE).size(12).padding([6, 10]).style(input_style),
        mono(
            format!("POST {}", endpoint(p, match p.format { Format::Anthropic => "messages", Format::OpenAi => "chat/completions" })),
            10.5,
            theme::TEXT_3
        ),
        label(tr("API 키")),
        text(key_text).size(11.5).color(key_color),
        row![
            text_input(tr("새 키 입력 (화면에 다시 표시되지 않음)"), s.key_input.as_str())
                .on_input(Message::AiSettingsKey)
                .on_submit(Message::AiSettingsSaveKey)
                .secure(true)
                .font(Font::MONOSPACE)
                .size(12)
                .padding([6, 10])
                .style(input_style),
            btn(tr("키 저장"), Btn::Plain, (!s.key_input.is_empty()).then_some(Message::AiSettingsSaveKey)),
            btn(tr("키 삭제"), Btn::Ghost, (s.key_status.get(s.selected) == Some(&KeySource::Keychain)).then_some(Message::AiSettingsDeleteKey)),
        ]
        .spacing(8),
        label(tr("모델")),
        model_row,
        row![btn(tr("연결 테스트"), Btn::Plain, (!s.busy).then_some(Message::AiSettingsFetch(true))), status]
            .spacing(10)
            .align_y(Alignment::Center),
    ]
    .spacing(6);

    let footer = row![
        btn(tr("이 프로필 사용"), Btn::Ghost, (p.id != s.draft.active).then_some(Message::AiSettingsUse)),
        btn(tr("프로필 삭제"), Btn::Ghost, (s.draft.profiles.len() > 1).then_some(Message::AiSettingsRemove)),
        Space::new().width(Length::Fill),
        btn(tr("취소"), Btn::Ghost, Some(Message::AiSettingsClose)),
        btn(tr("저장"), Btn::Primary, Some(Message::AiSettingsSave)),
    ]
    .spacing(8);

    container(
        column![
            row![crate::icons::icon(&crate::icons::API_BRIDGE, 18.0), text(tr("AI 연결 설정")).size(14)].spacing(8).align_y(Alignment::Center),
            row![container(list).width(200), container(scrollable(form)).width(Length::Fill).max_height(520)].spacing(18),
            footer,
        ]
        .spacing(14),
    )
    .padding(18)
    .width(SETTINGS_W)
    .style(card_style(theme::LINE_2))
    .into()
}
