//! AI panel and AI settings dialog.

use crate::i18n::{tf, tr};
use coshell_ai::profile::KeySource;
use coshell_ai::{AuthStyle, Format, Role, chat_url, models_url};
use iced::keyboard::key::Named;
use iced::widget::text::Span;
use iced::widget::text_editor::{self, Binding, KeyPress};
use iced::widget::{Space, button, column, container, pick_list, rich_text, row, scrollable, span, text, text_input};
use iced::{Alignment, Background, Border, Color, Font, Length, Padding};

use crate::ai::{AI_INPUT, Chat, Exec, ExecState, Settings, Turn};
use crate::app::{CoShell, Message, Phase, Tab, TabId};
use crate::assist::{self, Risk, Segment};
use crate::theme::{self, alpha};
use crate::icons::{COPY, NEW_CHAT, PAPERCLIP, PLAY, SEND, STOP, ZAP};
use crate::ui::{Btn, El, btn, icon_btn, input_style, mono, muted};

/// The first code block's run button (tests click it by this id).
pub const RUN_BTN: &str = "ai-run";

const COMPOSER_H: f32 = 64.0;
const COMPOSER_R: f32 = 16.0;
/// Height of the multi-line fields in the AI advanced settings.
const ADV_EDITOR_H: f32 = 64.0;
/// Output of a command the assistant ran scrolls beyond this.
const EXEC_OUTPUT_MAX_H: f32 = 240.0;
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
        text_color: Some(theme::TEXT),
        ..Default::default()
    }
}

/// What the assistant is told it is looking at.
/// Title length passed on (it is set by the remote shell, so it is kept short and plain).
const TITLE_MAX: usize = 80;

pub fn target_label(tab: &Tab) -> String {
    let mut label = if tab.is_local() {
        "local shell on this computer".to_string()
    } else {
        let mut r = tab.route.clone();
        let target = r.pop().unwrap_or_default();
        if r.is_empty() { format!("SSH {target}") } else { format!("SSH {target} (via {})", r.join(" -> ")) }
    };
    // Usually `user@host: ~/dir`, i.e. where commands will run.
    let title: String = tab.title.chars().filter(|c| !c.is_control() && *c != '"').take(TITLE_MAX).collect();
    if !title.trim().is_empty() {
        label.push_str(&format!(", terminal title \"{}\"", title.trim()));
    }
    label
}

pub fn panel(app: &CoShell) -> El<'_> {
    let tab = app.active_tab();
    let mut header = row![
        crate::icons::icon(&crate::icons::AI_SPARK, 16.0),
        text(tab.map_or(tr("없음").to_string(), |t| t.display_name().to_string()))
            .size(12.5)
            .color(theme::TEXT)
            .wrapping(text::Wrapping::None),
        Space::new().width(Length::Fill),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    if let Some(t) = tab {
        header = header.push(icon_btn(&NEW_CHAT, tr("새 대화"), Btn::Ghost, Some(Message::AiNewChat(t.id))));
    }

    let body: El<'_> = match tab {
        None => container(muted(tr("세션을 열면 해당 서버를 기준으로 답합니다"))).center(Length::Fill).into(),
        Some(t) => conversation(t, app.settings.ai_panel_width),
    };
    let composer: El<'_> = match tab {
        Some(t) => composer(app, t),
        None => Space::new().into(),
    };
    container(column![container(header).padding(Padding { top: 10.0, right: 10.0, bottom: 6.0, left: 16.0 }).clip(true), body, composer].height(Length::Fill))
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
        if t.hidden {
            continue; // a command result fed back to the model; shown on the command itself
        }
        col = col.push(match t.role {
            Role::User => user_turn(t, panel_w),
            Role::Assistant => assistant_turn(tab, i, t, i == last && chat.is_streaming() && t.execs.is_empty()),
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
        crate::icons::icon(&crate::icons::AI_SPARK, 24.0),
        Space::new().height(4),
        text(tf("{0}에 대해 물어보세요", &[&tab.display_name()])).size(14),
        muted(tr("터미널 출력은 직접 첨부할 때만 전송됩니다")),
        Space::new().height(12),
    ]
    .spacing(4)
    .align_x(Alignment::Center);
    for (s, attach) in suggestions {
        col = col.push(
            button(text(*s).size(12.5).color(theme::TEXT_2))
                .width(Length::Fill)
                .padding([10, 14])
                .on_press(Message::AiSuggest(id, (*s).to_string(), *attach))
                .style(|_, st| iced::widget::button::Style {
                    background: Some(Background::Color(if matches!(st, iced::widget::button::Status::Hovered) {
                        theme::BG_3
                    } else {
                        theme::BG_2
                    })),
                    text_color: theme::TEXT_2,
                    border: Border { radius: crate::ui::R_CARD.into(), ..Default::default() },
                    ..Default::default()
                }),
        );
        col = col.push(Space::new().height(2));
    }
    container(col).padding(16).center(Length::Fill).into()
}

fn chip<'a>(label: String) -> El<'a> {
    container(text(label).size(11).color(theme::TEXT_2))
        .padding([2, 8])
        .style(|_| container::Style {
            background: Some(Background::Color(theme::BG_3)),
            border: Border { radius: 6.0.into(), ..Default::default() },
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
            border: Border { radius: 14.0.into(), ..Default::default() },
            text_color: Some(theme::TEXT),
            ..Default::default()
        })
    ]
    .into()
}

const BOLD: Font = Font { weight: iced::font::Weight::Bold, ..Font::DEFAULT };

/// One line of Markdown inline text: `code` and **bold**.
fn inline<'a>(s: &str, size: f32, bold_all: bool) -> El<'a> {
    let mut spans: Vec<Span<'a, (), Font>> = Vec::new();
    for (i, part) in s.split('`').enumerate() {
        if i % 2 == 1 {
            spans.push(span(part.to_string()).font(Font::MONOSPACE).color(theme::TEXT).background(theme::BG_3));
            continue;
        }
        for (j, piece) in part.split("**").enumerate() {
            if piece.is_empty() {
                continue;
            }
            let sp = span(piece.to_string()).color(theme::TEXT);
            spans.push(if j % 2 == 1 || bold_all { sp.font(BOLD) } else { sp });
        }
    }
    rich_text(spans).size(size).into()
}

fn table_cells(line: &str) -> Vec<String> {
    line.trim().trim_matches('|').split('|').map(|c| c.trim().to_string()).collect()
}

fn is_table_rule(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|') && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

fn table<'a>(rows: Vec<Vec<String>>) -> El<'a> {
    let width = rows.iter().map(Vec::len).max().unwrap_or(1);
    let mut col = column![];
    for (r, cells) in rows.into_iter().enumerate() {
        let head = r == 0;
        let line = row((0..width).map(|c| {
            let v = cells.get(c).cloned().unwrap_or_default();
            container(inline(&v, 12.0, head)).padding([5, 8]).width(Length::FillPortion(1)).into()
        }));
        col = col.push(container(line).width(Length::Fill).style(move |_| container::Style {
            background: head.then_some(Background::Color(theme::BG_2)),
            border: Border { color: theme::LINE, width: if head { 0.0 } else { 0.5 }, radius: 0.0.into() },
            ..Default::default()
        }));
    }
    container(col)
        .width(Length::Fill)
        .clip(true)
        .style(|_| container::Style {
            border: Border { color: theme::LINE_2, width: 1.0, radius: crate::ui::R_CONTROL.into() },
            ..Default::default()
        })
        .into()
}

/// The model's prose as light Markdown: headings, lists, tables, **bold**, `code`.
fn prose<'a>(s: &str) -> El<'a> {
    let mut col = column![].spacing(5);
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    let flush_para = |col: iced::widget::Column<'a, Message>, para: &mut Vec<&str>| {
        if para.is_empty() {
            return col;
        }
        let text = para.join(" ");
        para.clear();
        col.push(inline(&text, 12.5, false))
    };
    for line in s.lines() {
        let t = line.trim();
        if t.starts_with('|') {
            col = flush_para(col, &mut para);
            if !is_table_rule(t) {
                rows.push(table_cells(t));
            }
            continue;
        }
        if !rows.is_empty() {
            col = col.push(table(std::mem::take(&mut rows)));
        }
        if t.is_empty() {
            col = flush_para(col, &mut para);
        } else if let Some(h) = t.strip_prefix("### ").or_else(|| t.strip_prefix("## ")).or_else(|| t.strip_prefix("# ")) {
            col = flush_para(col, &mut para);
            col = col.push(container(inline(h, 13.0, true)).padding(Padding::ZERO.top(4.0)));
        } else if let Some(item) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
            col = flush_para(col, &mut para);
            col = col.push(row![text("•").size(12.5).color(theme::TEXT_3), inline(item, 12.5, false)].spacing(8));
        } else if let Some((n, item)) = t.split_once(". ").filter(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())) {
            col = flush_para(col, &mut para);
            col = col.push(row![text(format!("{n}.")).size(12.5).color(theme::TEXT_3), inline(item, 12.5, false)].spacing(6));
        } else {
            para.push(t);
        }
    }
    col = flush_para(col, &mut para);
    if !rows.is_empty() {
        col = col.push(table(rows));
    }
    col.into()
}

/// A command the assistant ran itself: one line, output on demand.
fn exec_panel<'a>(tab: &'a Tab, index: usize, run: usize, e: &'a Exec) -> El<'a> {
    let (color, status) = match &e.state {
        ExecState::Running => (theme::ACCENT, tr("실행 중…").to_string()),
        ExecState::Typed => (theme::TEXT_2, tr("터미널에 입력함").to_string()),
        ExecState::Done { exit: Some(0) | None, output, .. } => (theme::OK, tf("실행함 · {0}줄", &[&output.lines().count()])),
        ExecState::Done { exit: Some(code), .. } => (theme::WARN, tf("종료 코드 {0}", &[code])),
        ExecState::Failed(_) => (theme::ERR, tr("실행하지 못함").to_string()),
    };
    let one_line: String = e.cmd.lines().next().unwrap_or("").chars().take(70).collect();
    let head = button(
        row![
            crate::icons::tinted(if e.manual { &PLAY } else { &ZAP }, 12.0, color),
            container(mono(one_line, 11.5, theme::TEXT)).width(Length::Fill).clip(true),
            text(status).size(10.5).color(color),
            text(if e.open { "▾" } else { "▸" }).size(11).color(theme::TEXT_3),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .padding([7, 10])
    .on_press(Message::AgentToggle(tab.id, index, run))
    .style(|_, st| button::Style {
        background: Some(Background::Color(if matches!(st, button::Status::Hovered) { theme::BG_3 } else { theme::BG_2 })),
        border: Border { radius: crate::ui::R_CONTROL.into(), ..Default::default() },
        ..Default::default()
    });
    let mut col = column![head].spacing(6);
    if e.open {
        let body = match &e.state {
            ExecState::Done { output, .. } if output.trim().is_empty() => tr("(출력 없음)").to_string(),
            ExecState::Done { output, .. } => output.clone(),
            ExecState::Failed(err) => err.clone(),
            ExecState::Running | ExecState::Typed => String::new(),
        };
        col = col.push(
            container(scrollable(container(mono(body, 11.0, theme::TEXT_2)).padding(10)).height(Length::Shrink))
                .max_height(EXEC_OUTPUT_MAX_H)
                .width(Length::Fill)
                .style(|_| container::Style {
                    background: Some(Background::Color(theme::BG_0)),
                    border: Border { color: theme::LINE, width: 1.0, radius: crate::ui::R_CONTROL.into() },
                    ..Default::default()
                }),
        );
    }
    let waiting = tab.env == crate::hosts::Env::Prod && !e.sent && matches!(e.state, ExecState::Done { .. } | ExecState::Failed(_));
    if waiting {
        col = col.push(
            row![
                muted(tr("운영 서버라 결과를 AI에 보내기 전에 확인합니다")),
                Space::new().width(Length::Fill),
                btn(tr("AI에 보내기"), Btn::Primary, (!tab.chat.is_streaming()).then_some(Message::AgentSend(tab.id))),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
    }
    col.into()
}

fn assistant_turn<'a>(tab: &'a Tab, index: usize, t: &'a Turn, streaming: bool) -> El<'a> {
    let mut col = column![
        row![crate::icons::icon(&crate::icons::AI_SPARK, 12.0), mono(t.model.clone(), 10.5, theme::TEXT_3)].spacing(6).align_y(Alignment::Center)
    ]
    .spacing(8);
    for seg in assist::segments(&t.text) {
        col = col.push(match seg {
            Segment::Text(s) => prose(&s),
            Segment::Code { lang, code, closed } => {
                // A command that was run shows its run (the latest) instead of a ▶ card.
                let cmd = assist::command_text(&code);
                match t.execs.iter().enumerate().rev().find(|(_, e)| e.cmd == cmd) {
                    Some((j, e)) => exec_panel(tab, index, j, e),
                    None => command_card(tab, index, lang, code, closed),
                }
            }
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

fn command_card<'a>(tab: &'a Tab, index: usize, lang: String, code: String, closed: bool) -> El<'a> {
    let shell = assist::is_shell(&lang);
    let cmd = assist::command_text(&code);
    let r = assist::risk(&cmd);
    let bracketed = match &tab.phase {
        Phase::Live(s) => {
            s.term().lock().mode().contains(coshell_term::alacritty_terminal::term::TermMode::BRACKETED_PASTE)
        }
        _ => false,
    };
    let can_run = shell && closed && !tab.exited && matches!(tab.phase, Phase::Live(_))
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
    let multiline_blocked = shell && cmd.contains('\n') && !bracketed;
    let placeholder = shell && assist::has_placeholder(&cmd);
    let footer = row![
        warn,
        Space::new().width(Length::Fill),
        icon_btn(&COPY, tr("복사"), Btn::Ghost, Some(Message::AiCopy(cmd.clone()))),
        run_button(tab, index, &cmd, can_run && !placeholder, multiline_blocked, placeholder),
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
        border: Border { color: border, width: 1.0, radius: crate::ui::R_CARD.into() },
        ..Default::default()
    })
    .into()
}

/// Run = insert + Enter. A risky command first turns the button into "click again to run".
fn run_button<'a>(tab: &'a Tab, index: usize, cmd: &str, enabled: bool, multiline_blocked: bool, placeholder: bool) -> El<'a> {
    // While a reply is still coming in, or a command is running, there is nothing to press yet
    // (the assistant may be about to run this very command).
    if tab.chat.is_streaming() {
        return Space::new().into();
    }
    let msg = enabled.then(|| Message::AiRun(tab.id, index, cmd.to_string()));
    if enabled && tab.chat.run_confirm.as_deref() == Some(cmd) {
        return container(btn(tr("한 번 더 눌러 실행"), Btn::Danger, msg)).id(RUN_BTN).into();
    }
    let tip = if placeholder {
        tr("<…> 부분을 실제 값으로 바꿔야 실행할 수 있습니다. 복사해서 고쳐 쓰세요")
    } else if multiline_blocked {
        tr("여러 줄은 복사해서 사용")
    } else if assist::run_needs_confirm(cmd, tab.env == crate::hosts::Env::Prod) {
        tr("실행 (확인 후)")
    } else {
        tr("실행")
    };
    container(icon_btn(&PLAY, tip, Btn::Primary, msg)).id(RUN_BTN).into()
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
        border: Border { color: alpha(theme::ERR, 0.45), width: 1.0, radius: crate::ui::R_CARD.into() },
        ..Default::default()
    })
    .into()
}

/// ⚡ Auto / manual. Lit while auto-run is on.
fn auto_toggle<'a>(on: bool) -> El<'a> {
    let b = button(crate::icons::tinted(&ZAP, 14.0, if on { theme::ACCENT } else { theme::TEXT_3 }))
        .padding(6)
        .on_press(Message::AiAutoRun)
        .style(move |_, st| button::Style {
            background: Some(Background::Color(if on {
                alpha(theme::ACCENT, 0.16)
            } else if matches!(st, button::Status::Hovered) {
                theme::BG_3
            } else {
                Color::TRANSPARENT
            })),
            border: Border { radius: crate::ui::R_CONTROL.into(), ..Default::default() },
            ..Default::default()
        });
    crate::ui::tip(
        b,
        if on {
            tr("AI가 직접 확인 켜짐 · 조회 명령은 AI가 바로 실행하고 결과를 읽어 설명합니다 (변경·위험 명령은 ▶로 직접)")
        } else {
            tr("AI가 직접 확인 꺼짐 · AI는 명령만 제안하고, ▶를 눌러 실행합니다")
        },
        iced::widget::tooltip::Position::Top,
    )
}

/// One card: optional attachment, the text box, then a toolbar (attach · profile · send).
fn composer<'a>(app: &'a CoShell, tab: &'a Tab) -> El<'a> {
    let id = tab.id;
    let chat: &Chat = &tab.chat;
    let mut card = column![];
    if let Some(a) = &chat.attachment {
        let masked = if a.secrets > 0 { tf(" · 마스킹 {0}", &[&a.secrets]) } else { String::new() };
        card = card.push(
            container(
                row![
                    chip(format!("📎 {}{masked}", a.label)),
                    button(text("×").size(12).color(theme::TEXT_3)).padding([0, 6]).style(crate::ui::btn_style(Btn::Ghost)).on_press(Message::AiDetach(id)),
                ]
                .align_y(Alignment::Center),
            )
            .padding(Padding { top: 10.0, right: 10.0, bottom: 0.0, left: 10.0 }),
        );
    }
    let editor = iced::widget::text_editor(&chat.draft)
        .id(AI_INPUT)
        .placeholder(if app.composing { "" } else { tr("이 세션에 대해 질문하거나 명령을 요청하세요") })
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
        .size(13)
        .padding(Padding { top: 12.0, right: 14.0, bottom: 8.0, left: 14.0 })
        .style(|_, _| text_editor::Style {
            background: Background::Color(Color::TRANSPARENT),
            border: Border::default(),
            placeholder: theme::TEXT_3,
            value: theme::TEXT,
            selection: alpha(theme::ACCENT, 0.35),
        });

    let ai = &app.ai;
    let active = ai.active();
    let choices: Vec<ProfileChoice> =
        ai.store.profiles.iter().map(|p| ProfileChoice { id: p.id.clone(), name: p.name.clone() }).collect();
    let selected = choices.iter().find(|c| c.id == active.id).cloned();
    let profile = pick_list(choices, selected, |c: ProfileChoice| Message::AiProfile(c.id))
        .text_size(12)
        .padding([4, 10])
        .style(crate::ui::pick_style)
        .menu_style(crate::ui::menu_style);
    let model = container(mono(active.model.clone(), 10.5, theme::TEXT_3)).width(Length::Fill).clip(true);

    let can_attach = matches!(tab.phase, Phase::Live(_));
    let empty = chat.draft.text().trim().is_empty();
    let action = if chat.is_streaming() {
        icon_btn(&STOP, tr("중지"), Btn::Plain, Some(Message::AiStop(id)))
    } else {
        icon_btn(&SEND, tr("전송 (⏎) · 줄바꿈 (⇧⏎)"), Btn::Primary, (!empty).then_some(Message::AiSend(id)))
    };
    let toolbar = row![
        icon_btn(&PAPERCLIP, tr("출력 첨부"), Btn::Ghost, can_attach.then_some(Message::AiAttach(id))),
        auto_toggle(app.settings.ai_agent),
        profile,
        model,
        action,
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    let rule = container(Space::new()).height(1).width(Length::Fill).style(|_| container::Style {
        background: Some(Background::Color(theme::LINE)),
        ..Default::default()
    });
    card = card.push(editor).push(rule).push(container(toolbar).padding([6, 8]));
    // BG_0 like every other text field (the IME caret relies on fields sharing one background).
    let card = container(card).style(|_| container::Style {
        background: Some(Background::Color(theme::BG_0)),
        border: Border { color: theme::LINE_2, width: 1.0, radius: COMPOSER_R.into() },
        shadow: iced::Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.25), offset: iced::Vector::new(0.0, 4.0), blur_radius: 16.0 },
        ..Default::default()
    });
    container(card).padding(Padding { top: 6.0, right: 12.0, bottom: 12.0, left: 12.0 }).into()
}

// ------------------------------------------------------------------ settings

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AuthChoice(AuthStyle, Format);
impl std::fmt::Display for AuthChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match (self.0, self.1) {
            (AuthStyle::Auto, Format::Anthropic) => tr("기본 (x-api-key 헤더)"),
            (AuthStyle::Auto, Format::OpenAi) => tr("기본 (Authorization: Bearer)"),
            (AuthStyle::Bearer, _) => "Authorization: Bearer",
            (AuthStyle::Header, _) => tr("헤더 이름 지정"),
            (AuthStyle::None, _) => tr("보내지 않음"),
        })
    }
}

/// Multi-line field of the advanced section.
fn adv_editor<'a>(content: &'a text_editor::Content, placeholder: &'a str, on: fn(text_editor::Action) -> Message) -> El<'a> {
    iced::widget::text_editor(content)
        .placeholder(placeholder)
        .on_action(on)
        .font(Font::MONOSPACE)
        .size(11.5)
        .height(ADV_EDITOR_H)
        .padding(8)
        .style(|_, st| text_editor::Style {
            background: Background::Color(theme::BG_0),
            border: Border {
                color: if matches!(st, text_editor::Status::Focused { .. }) { alpha(theme::ACCENT, 0.7) } else { theme::LINE_2 },
                width: 1.0,
                radius: crate::ui::R_CONTROL.into(),
            },
            placeholder: theme::TEXT_3,
            value: theme::TEXT,
            selection: alpha(theme::ACCENT, 0.35),
        })
        .into()
}

/// Paths, auth, headers and parameters for proxies and compatible services (both formats).
fn advanced(s: &Settings) -> El<'_> {
    use crate::app::AdvField;
    let p = s.profile();
    let a = &p.advanced;
    let count = usize::from(!a.chat_path.is_empty())
        + usize::from(!a.models_path.is_empty())
        + usize::from(a.auth != AuthStyle::Auto)
        + a.headers.len()
        + usize::from(a.max_tokens.is_some())
        + usize::from(a.temperature.is_some())
        + usize::from(!a.anthropic_version.is_empty())
        + usize::from(!a.extra_body.is_empty());
    let title = if count > 0 { tf("고급 설정 ({0})", &[&count]) } else { tr("고급 설정").to_string() };
    let toggle = button(
        row![text(if s.advanced_open { "▾" } else { "▸" }).size(10).color(theme::TEXT_3), text(title).size(12).color(theme::TEXT_2)]
            .spacing(6)
            .align_y(Alignment::Center),
    )
    .padding([4, 0])
    .style(crate::ui::btn_style(Btn::Ghost))
    .on_press(Message::AiSettingsAdvanced);
    if !s.advanced_open {
        return column![toggle, muted(tr("프록시·호환 서버용: 요청 경로, 키 전달 방식, 추가 헤더, 최대 토큰, temperature, 추가 JSON"))].spacing(2).into();
    }
    let label = |t: &str| text(t.to_string()).size(11.5).color(theme::TEXT_2);
    let input = |placeholder: &str, value: &str, f: AdvField| {
        text_input(placeholder, value)
            .on_input(move |v| Message::AiSettingsAdv(f, v))
            .font(Font::MONOSPACE)
            .size(12)
            .padding([6, 10])
            .style(input_style)
    };
    let default_chat = match p.format {
        Format::Anthropic => "/v1/messages",
        Format::OpenAi => "/v1/chat/completions",
    };
    let auths: Vec<AuthChoice> = AuthStyle::ALL.iter().map(|&x| AuthChoice(x, p.format)).collect();
    let mut auth_row = row![
        pick_list(auths, Some(AuthChoice(a.auth, p.format)), |c: AuthChoice| Message::AiSettingsAuth(c.0))
            .text_size(12)
            .padding([5, 8])
            .style(crate::ui::pick_style)
            .menu_style(crate::ui::menu_style)
    ]
    .spacing(8);
    if a.auth == AuthStyle::Header {
        auth_row = auth_row.push(input(tr("예: api-key"), &a.auth_header, AdvField::AuthHeader));
    }
    let mut col = column![
        toggle,
        row![
            column![label(tr("채팅 요청 경로")), input(default_chat, &a.chat_path, AdvField::ChatPath)].spacing(4).width(Length::Fill),
            column![label(tr("모델 목록 경로")), input("/v1/models", &a.models_path, AdvField::ModelsPath)].spacing(4).width(Length::Fill),
        ]
        .spacing(10),
        muted(tr("Base URL 뒤에 붙습니다. http(s)://로 시작하면 그 주소를 그대로 씁니다 (예: Azure ?api-version=…).")),
        mono(format!("GET {}", models_url(p)), 10.5, theme::TEXT_3),
        label(tr("API 키 전달 방식")),
        auth_row,
        label(tr("추가 헤더")),
        adv_editor(&s.adv.headers, "HTTP-Referer: https://example.com\nX-Title: CoShell", Message::AiSettingsHeaders),
        muted(tr("한 줄에 ‘이름: 값’ 하나씩. 파일에 그대로 저장되니 키는 위 API 키 칸에 넣으세요.")),
        row![
            column![label(tr("최대 토큰")), input("4096", &s.adv.max_tokens, AdvField::MaxTokens)].spacing(4).width(Length::Fill),
            column![label("temperature"), input(tr("모델 기본값"), &s.adv.temperature, AdvField::Temperature)].spacing(4).width(Length::Fill),
        ]
        .spacing(10),
    ]
    .spacing(6);
    if p.format == Format::Anthropic {
        col = col.push(label("anthropic-version")).push(input("2023-06-01", &a.anthropic_version, AdvField::AnthropicVersion));
    }
    col = col
        .push(label(tr("추가 요청 JSON")))
        .push(adv_editor(&s.adv.extra, "{\"top_p\": 0.9}", Message::AiSettingsExtra))
        .push(muted(tr("요청 본문에 합쳐집니다. 같은 키는 이 값이 이깁니다 (예: thinking, reasoning_effort).")));
    if let Some(e) = &s.adv.error {
        col = col.push(text(format!("✗ {e}")).size(11.5).color(theme::ERR_TX));
    }
    col.into()
}

fn key_status_text(s: &Settings) -> (String, Color) {
    match s.key_status.get(s.selected).copied().unwrap_or(KeySource::None) {
        KeySource::Keychain => (tr("키체인에 저장됨").into(), theme::OK),
        KeySource::Env => (tr("환경 변수에서 읽음 (읽기 전용)").into(), theme::TEXT_2),
        KeySource::None if s.profile().is_local() => (tr("키 없음 · 로컬 서버는 선택 사항").into(), theme::TEXT_3),
        KeySource::None => (tr("키 없음").into(), theme::WARN),
    }
}

/// The AI page of the settings dialog. Saved with the dialog's own Save button.
pub fn settings_page(s: &Settings) -> El<'_> {
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
                border: Border { radius: crate::ui::R_CONTROL.into(), ..Default::default() },
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
        model_row = model_row.push(pick_list(s.models.clone(), sel, Message::AiSettingsModel).text_size(12).padding([5, 8]).style(crate::ui::pick_style).menu_style(crate::ui::menu_style));
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
        pick_list(formats, Some(FormatChoice(p.format)), |f: FormatChoice| Message::AiSettingsFormat(f.0)).text_size(12).padding([5, 8]).style(crate::ui::pick_style).menu_style(crate::ui::menu_style),
        muted(match p.format {
            Format::Anthropic => tr("Claude와 Anthropic 호환 서버 (DeepSeek, 프록시 등)"),
            Format::OpenAi => tr("OpenAI, Ollama, vLLM, LiteLLM, OpenRouter, Azure 등 호환 서버"),
        }),
        label("Base URL"),
        text_input(p.format.default_base(), &p.base_url).on_input(Message::AiSettingsBase).font(Font::MONOSPACE).size(12).padding([6, 10]).style(input_style),
        mono(format!("POST {}", chat_url(p)), 10.5, theme::TEXT_3),
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
        Space::new().height(4),
        advanced(s),
    ]
    .spacing(6);

    let actions = row![
        btn(tr("이 프로필 사용"), Btn::Ghost, (p.id != s.draft.active).then_some(Message::AiSettingsUse)),
        btn(tr("프로필 삭제"), Btn::Ghost, (s.draft.profiles.len() > 1).then_some(Message::AiSettingsRemove)),
    ]
    .spacing(8);

    column![
        row![crate::icons::icon(&crate::icons::API_BRIDGE, 16.0), text(tr("AI 연결 설정")).size(13)].spacing(8).align_y(Alignment::Center),
        row![
            container(scrollable(list)).width(190).height(Length::Fill),
            column![container(scrollable(container(form).padding(Padding::ZERO.right(12.0)))).height(Length::Fill), actions].spacing(8).width(Length::Fill),
        ]
        .spacing(18)
        .height(Length::Fill),
    ]
    .spacing(12)
    .height(Length::Fill)
    .into()
}
