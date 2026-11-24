//! Block view of a live terminal: each command as a card (who ran it, prompt,
//! time, output) and the prompt being typed on as an input row. The real
//! terminal stays underneath and keeps the keyboard.

use crate::i18n::{tf, tr};
use iced::widget::text::Span;
use iced::widget::{Space, button, column, container, mouse_area, row, span, text, text_input};
use iced::{Alignment, Background, Border, Color, Font, Length, Padding};

use crate::app::{ASK_INPUT, CoShell, Message, RUN_INPUT, SEARCH_INPUT, Tab};
use crate::blocks::{self, Block, Mark, Prompt, Screen, Source};
use crate::fonts::MONO;
use crate::selectable::Selectable;
use crate::icons::{COPY, SEARCH, SEND, X};
use crate::theme::{self, alpha};
use crate::ui::{El, kbd, mono};

const TEXT: f32 = 13.0;
/// Command cards and the composer dock.
const R_DOCK: f32 = 16.0;
/// Output line height (the design's `leading-6`).
const OUTPUT_LINE_H: f32 = 24.0;
const MONO_SEMIBOLD: Font = crate::fonts::weight(MONO, iced::font::Weight::Semibold);
const MONO_MEDIUM: Font = crate::fonts::weight(MONO, iced::font::Weight::Medium);

fn badge<'a>(source: Source) -> El<'a> {
    let (content, bg, line, fg): (El<'a>, Color, Color, Color) = match source {
        Source::Ai => (
            row![crate::ui::face(10.0, crate::ui::Face::STILL), text("AI").size(10.5).font(crate::ui::SEMIBOLD).color(theme::PURPLE)].spacing(5).align_y(Alignment::Center).into(),
            alpha(theme::PURPLE, 0.12),
            alpha(theme::PURPLE, 0.25),
            theme::PURPLE,
        ),
        Source::User => (text(tr("사용자")).size(10.5).font(crate::ui::MEDIUM).color(theme::TEXT_2).into(), theme::CHIP, Color::TRANSPARENT, theme::TEXT_2),
    };
    let _ = fg;
    container(content)
        .padding([2, 8])
        .style(move |_| container::Style {
            background: Some(Background::Color(bg)),
            border: Border { color: line, width: 1.0, radius: crate::ui::R_PILL.into() },
            ..Default::default()
        })
        .into()
}

/// `user@host : cwd $` as the design draws it.
fn prompt_parts<'a>(p: &Prompt) -> Vec<El<'a>> {
    let mut parts: Vec<El<'a>> = vec![text(p.user_host.clone()).size(TEXT).font(MONO_SEMIBOLD).color(theme::OK).into()];
    if !p.sym.is_empty() {
        parts.push(mono(":", TEXT, theme::TEXT_3));
        parts.push(mono(if p.cwd.is_empty() { "~".to_string() } else { p.cwd.clone() }, TEXT, theme::INFO));
        parts.push(mono(p.sym.clone(), TEXT, theme::TEXT_2));
    }
    parts
}

/// Command output, selectable and copyable (⌘C) like the terminal underneath.
fn output_text<'a>(lines: &[String], query: &str) -> El<'a> {
    let body = lines.join("\n");
    let mut rows: Vec<Vec<Span<'a, (), Font>>> = vec![Vec::new()];
    for (s, m) in blocks::highlight(body.trim_end(), query) {
        for (k, part) in s.split('\n').enumerate() {
            if k > 0 {
                rows.push(Vec::new());
            }
            if part.is_empty() {
                continue;
            }
            let sp = span(part.to_string());
            let sp = match m {
                Mark::Plain => sp.color(alpha(theme::TEXT, 0.9)),
                Mark::High | Mark::Warn => sp.color(theme::WARN).font(MONO_SEMIBOLD),
                Mark::Err => sp.color(theme::ERR_TX).font(MONO_SEMIBOLD),
                Mark::Ok => sp.color(theme::OK).font(MONO_MEDIUM),
                Mark::Path => sp.color(theme::INFO),
                Mark::Found => sp.color(theme::AMBER_200).background(alpha(theme::WARN, 0.3)),
            };
            rows.last_mut().expect("one row").push(sp);
        }
    }
    let out = Selectable::new(rows, Message::AiCopy).font(MONO).size(TEXT).line_height(OUTPUT_LINE_H).selection(alpha(theme::ACCENT, 0.35));
    container(out).padding(Padding::ZERO.left(4.0)).into()
}

fn small_btn<'a>(content: impl Into<El<'a>>, msg: Message) -> El<'a> {
    button(content)
        .padding([3, 10])
        .on_press(msg)
        .style(|_, st| {
            let hover = matches!(st, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: Some(Background::Color(if hover { theme::ACCENT } else { alpha(theme::ACCENT, 0.12) })),
                text_color: if hover { theme::ON_ACCENT } else { theme::ACCENT_TX },
                border: Border { radius: crate::ui::R_PILL.into(), ..Default::default() },
                ..Default::default()
            }
        })
        .into()
}

fn block_card<'a>(tab: &Tab, i: usize, b: &Block, meta: Option<&blocks::Meta>, hovered: bool, running: bool, query: &str) -> El<'a> {
    let mut head = row![badge(meta.map_or(Source::User, |m| m.source))].spacing(8).align_y(Alignment::Center);
    for p in prompt_parts(&b.prompt) {
        head = head.push(p);
    }
    head = head.push(container(text(b.cmd.clone()).size(TEXT).font(MONO_MEDIUM).color(theme::SLATE_100)).width(Length::Fill));
    let mut right = row![].spacing(8).align_y(Alignment::Center);
    if let Some(m) = meta {
        let mut info = row![mono(m.time.clone(), 10.0, theme::TEXT_3), mono("·", 10.0, theme::TEXT_3)].spacing(6).align_y(Alignment::Center);
        match m.took {
            Some(d) => info = info.push(mono(format!("{}ms", d.as_millis()), 10.0, theme::TEXT_3)),
            None if running => info = info.push(mono(tr("실행 중…"), 10.0, theme::INFO)),
            None => {}
        }
        if let Some(code) = m.exit {
            let (fg, bg) = if code == 0 { (theme::OK, alpha(theme::OK, 0.12)) } else { (theme::ERR_TX, alpha(theme::ERR, 0.1)) };
            info = info.push(container(mono(format!("exit {code}"), 10.0, fg)).padding([1, 6]).style(move |_| container::Style {
                background: Some(Background::Color(bg)),
                border: Border { radius: 6.0.into(), ..Default::default() },
                ..Default::default()
            }));
        }
        right = right.push(info);
    }
    // Hover actions keep their room so nothing moves when they appear.
    let output = b.output.join("\n");
    let actions: El<'a> = if hovered {
        row![
            small_btn(text(tr("AI 분석")).size(10), Message::BlockAsk(tab.id, b.cmd.clone(), output.clone())),
            button(crate::icons::tinted(&COPY, 12.0, theme::TEXT_2))
                .padding(5)
                .on_press(Message::AiCopy(format!("{}\n{}", b.cmd, output)))
                .style(|_, st| button::Style {
                    background: Some(Background::Color(if matches!(st, button::Status::Hovered) { alpha(Color::WHITE, 0.15) } else { theme::CHIP })),
                    border: Border { radius: crate::ui::R_PILL.into(), ..Default::default() },
                    ..Default::default()
                }),
        ]
        .spacing(4)
        .align_y(Alignment::Center)
        .into()
    } else {
        Space::new().width(94).height(22).into()
    };
    right = right.push(actions);
    head = head.push(right);
    let mut col = column![head].spacing(10);
    if !b.output.iter().all(|l| l.trim().is_empty()) {
        col = col.push(output_text(&b.output, query));
    }
    let card = container(col).padding(16).width(Length::Fill).style(move |_| container::Style {
        background: Some(Background::Color(theme::CARD)),
        border: Border { color: if hovered { theme::LINE_2 } else { theme::LINE }, width: 1.0, radius: R_DOCK.into() },
        ..Default::default()
    });
    mouse_area(card).on_enter(Message::BlockHover(Some(i))).on_exit(Message::BlockHover(None)).into()
}

/// No prompt on screen (a command is running): a line for the program, sent with Enter.
/// `#…` asks the AI instead; `secret` hides what is typed (a password prompt).
fn run_row<'a>(app: &'a CoShell, tab: &Tab, running: bool, secret: bool) -> El<'a> {
    let value = app.run_input.as_ref().filter(|(id, _)| *id == tab.id).map_or("", |(_, t)| t.as_str());
    let ai_mode = value.trim_start().starts_with('#');
    let mut r = row![].spacing(8).align_y(Alignment::Center);
    if running {
        r = r.push(
            container(row![crate::ui::dot(theme::INFO, 6.0), text(tr("실행 중")).size(10.5).font(crate::ui::SEMIBOLD).color(theme::INFO)].spacing(5).align_y(Alignment::Center))
                .padding([2, 8])
                .style(|_| container::Style {
                    background: Some(Background::Color(alpha(theme::INFO, 0.12))),
                    border: Border { color: alpha(theme::INFO, 0.3), width: 1.0, radius: crate::ui::R_PILL.into() },
                    ..Default::default()
                }),
        );
    }
    let placeholder = if secret { tr("비밀번호 입력 후 Enter (화면에 표시되지 않음)") } else { tr("실행 중인 프로그램에 입력 후 Enter · '#'으로 AI 질문") };
    r = r.push(
        text_input(placeholder, value)
            .id(RUN_INPUT)
            .on_input(Message::RunInput)
            .on_submit(Message::RunSubmit)
            .secure(secret && !ai_mode)
            .font(MONO)
            .size(TEXT)
            .padding(0)
            .width(Length::Fill)
            .style(move |t, st| text_input::Style {
                value: if ai_mode { theme::PURPLE } else { theme::SLATE_100 },
                placeholder: theme::TEXT_3,
                ..crate::ui::bare_input_style(t, st)
            }),
    );
    if !value.is_empty() {
        r = r.push(mono(tr("Enter 전송 · Esc 취소"), 11.0, theme::TEXT_3));
    }
    // Ctrl+C for the program, wherever the keyboard is.
    let stop = button(text(tr("중단")).size(11.5).font(crate::ui::MEDIUM))
        .padding([5, 10])
        .on_press(Message::Input(tab.id, b"\x03".to_vec()))
        .style(crate::ui::tint_style(theme::ERR_TX, true));
    r = r.push(crate::ui::tip(stop, tr("Ctrl+C 보내기"), iced::widget::tooltip::Position::Top));
    let send = button(container(crate::icons::tinted(&SEND, 13.0, theme::ON_ACCENT)).center(28))
        .padding(0)
        .on_press(if value.is_empty() { Message::Input(tab.id, b"\r".to_vec()) } else { Message::RunSubmit })
        .style(|_, st| button::Style {
            background: Some(Background::Color(alpha(theme::ACCENT, if matches!(st, button::Status::Hovered | button::Status::Pressed) { 0.88 } else { 1.0 }))),
            border: Border { radius: 10.0.into(), ..Default::default() },
            ..Default::default()
        });
    r = r.push(send);
    let focused = app.run_input.as_ref().is_some_and(|(id, _)| *id == tab.id);
    container(r)
        .padding(Padding { top: 8.0, right: 8.0, bottom: 8.0, left: 14.0 })
        .width(Length::Fill)
        .style(move |_| container::Style {
            background: Some(Background::Color(theme::BG_2)),
            border: Border { color: if focused { alpha(theme::ACCENT, 0.45) } else { theme::LINE }, width: if focused { 1.5 } else { 1.0 }, radius: R_DOCK.into() },
            shadow: iced::Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.3), offset: iced::Vector::new(0.0, 6.0), blur_radius: 18.0 },
            ..Default::default()
        })
        .into()
}

/// The prompt waiting for input: what is typed, the caret, and an inline suggestion.
fn prompt_row<'a>(app: &CoShell, tab: &Tab, p: &Prompt, typed: &str, caret_tail: usize) -> El<'a> {
    let asking = app.ask.as_ref().filter(|(id, _)| *id == tab.id).map(|(_, q)| q.as_str());
    let ai_mode = asking.is_some() || typed.trim_start().starts_with('#');
    let mut r = row![].spacing(6).align_y(Alignment::Center);
    if ai_mode {
        r = r.push(
            container(row![crate::ui::face(10.0, app.face.idle()), text(tr("AI 질문")).size(10.5).font(crate::ui::SEMIBOLD).color(theme::PURPLE)].spacing(5).align_y(Alignment::Center)).padding([2, 8]).style(|_| container::Style {
                background: Some(Background::Color(alpha(theme::PURPLE, 0.15))),
                border: Border { color: alpha(theme::PURPLE, 0.3), width: 1.0, radius: crate::ui::R_PILL.into() },
                ..Default::default()
            }),
        );
    }
    for part in prompt_parts(p) {
        r = r.push(part);
    }
    let caret = || -> El<'a> { container(Space::new()).width(2).height(16).style(crate::ui::fill(theme::ACCENT_TX)).into() };
    let chars: Vec<char> = typed.chars().collect();
    let split = chars.len().saturating_sub(caret_tail);
    let before: String = chars[..split].iter().collect();
    let after: String = chars[split..].iter().collect();
    let ghost = blocks::ghost(typed, &tab.metas).filter(|_| asking.is_none());
    let mut line = row![].align_y(Alignment::Center);
    if let Some(q) = asking {
        line = line.push(
            text_input("", q)
                .id(ASK_INPUT)
                .on_input(Message::AskInput)
                .on_submit(Message::AskSubmit)
                .font(MONO)
                .size(TEXT)
                .padding(0)
                .style(|t, st| text_input::Style { value: theme::SLATE_100, ..crate::ui::bare_input_style(t, st) }),
        );
    } else if typed.is_empty() {
        line = line.push(caret()).push(mono(tr("명령어 입력 (Tab 자동완성 · '#'으로 바로 AI 질문)..."), TEXT, theme::TEXT_3));
    } else {
        // No wrapping, so a trailing space keeps its width and moves the caret.
        let typed_text = |s: String| -> El<'a> { text(s).size(TEXT).font(MONO).color(theme::SLATE_100).wrapping(text::Wrapping::None).into() };
        line = line.push(typed_text(before)).push(caret()).push(typed_text(after));
        if let Some(g) = &ghost {
            line = line.push(mono(g.clone(), TEXT, theme::SLATE_600));
        }
    }
    r = r.push(container(line).width(Length::Fill).clip(true));
    if asking.is_some() {
        r = r.push(mono(tr("Enter 질문 · Esc 취소"), 11.0, theme::TEXT_3));
    } else if ghost.is_some() {
        r = r.push(kbd("Tab ⇥"));
    }
    // Return: sends the question, or Enter to the shell.
    let send = button(container(crate::icons::tinted(&SEND, 13.0, theme::ON_ACCENT)).center(28))
        .padding(0)
        .on_press(if asking.is_some() { Message::AskSubmit } else { Message::Input(tab.id, b"\r".to_vec()) })
        .style(|_, st| button::Style {
            background: Some(Background::Color(alpha(theme::ACCENT, if matches!(st, button::Status::Hovered | button::Status::Pressed) { 0.88 } else { 1.0 }))),
            border: Border { radius: 10.0.into(), ..Default::default() },
            ..Default::default()
        });
    r = r.push(send);
    let focused = app.terminal_active(tab) || asking.is_some();
    container(r)
        .padding(Padding { top: 8.0, right: 8.0, bottom: 8.0, left: 14.0 })
        .width(Length::Fill)
        .style(move |_| container::Style {
            background: Some(Background::Color(theme::BG_2)),
            border: Border { color: if focused { alpha(theme::ACCENT, 0.45) } else { theme::LINE }, width: if focused { 1.5 } else { 1.0 }, radius: R_DOCK.into() },
            shadow: iced::Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.3), offset: iced::Vector::new(0.0, 6.0), blur_radius: 18.0 },
            ..Default::default()
        })
        .into()
}

/// The design's filter strip: icon, query, `n / m 블록`, close.
fn search_strip<'a>(query: &'a str, shown: usize, total: usize) -> El<'a> {
    let count = if query.trim().is_empty() { String::new() } else { tf("{0} / {1} 블록", &[&shown, &total]) };
    let strip = row![
        crate::icons::tinted(&SEARCH, 14.0, theme::TEXT_2),
        text_input(tr("현재 터미널 명령어 및 출력 내용 필터링..."), query)
            .id(SEARCH_INPUT)
            .on_input(Message::SearchQuery)
            .on_submit(Message::SearchNext(false))
            .font(MONO)
            .size(12)
            .padding([2, 0])
            .width(Length::Fill)
            .style(crate::ui::bare_input_style),
        mono(count, 11.0, theme::TEXT_2),
        button(crate::icons::tinted(&X, 14.0, theme::TEXT_2)).padding(4).style(crate::ui::btn_style(crate::ui::Btn::Ghost)).on_press(Message::SearchClose),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    column![container(strip).padding([8, 16]).width(Length::Fill).style(crate::ui::fill(theme::BG_1)), crate::ui::rule(true)].into()
}

pub fn view<'a>(app: &'a CoShell, tab: &'a Tab, screen: Screen) -> El<'a> {
    let query = app.search.as_ref().map_or("", |s| s.query.as_str());
    let q = query.trim().to_lowercase();
    let metas = blocks::match_metas(&screen.blocks, &tab.metas);
    let running = screen.running();
    let last = screen.blocks.len().saturating_sub(1);
    let mut col = column![].spacing(16).width(Length::Fill);
    if !screen.preamble.iter().all(|l| l.trim().is_empty()) && q.is_empty() {
        col = col.push(container(output_text(&screen.preamble, "")).padding(10));
    }
    let mut shown = 0;
    for (i, b) in screen.blocks.iter().enumerate() {
        if !q.is_empty() && !b.cmd.to_lowercase().contains(&q) && !b.output.iter().any(|l| l.to_lowercase().contains(&q)) {
            continue;
        }
        shown += 1;
        col = col.push(block_card(tab, i, b, metas[i], app.block_hover == Some(i), running && i == last, query));
    }
    if !q.is_empty() && shown == 0 {
        col = col.push(
            container(text(tf("'{0}'에 해당하는 터미널 기록이 없습니다.", &[&query.trim()])).size(12).color(theme::TEXT_3))
                .center_x(Length::Fill)
                .padding([40, 0]),
        );
    }
    // At a prompt the dock mirrors the shell's line; with none (a program running) it takes input for it.
    let dock = match &screen.current {
        Some((p, typed)) => {
            let tail = match &tab.phase {
                crate::app::Phase::Live(s) => blocks::cursor_tail(s.term()),
                _ => 0,
            };
            prompt_row(app, tab, p, typed, tail)
        }
        None => {
            let last_line = screen.blocks.last().map_or(&screen.preamble, |b| &b.output).iter().rev().find(|l| !l.trim().is_empty());
            run_row(app, tab, running, last_line.is_some_and(|l| blocks::asks_secret(l)))
        }
    };
    let dock = Some(container(dock).padding(Padding { top: 8.0, right: 28.0, bottom: 16.0, left: 28.0 }));
    col = col.push(Space::new().height(24));
    let body = crate::ui::scroll(container(col).padding(Padding { top: 24.0, right: 28.0, bottom: 0.0, left: 28.0 }).width(Length::Fill))
        .anchor_bottom()
        .height(Length::Fill)
        .width(Length::Fill);
    // A click anywhere gives the keyboard back to the shell.
    let body = mouse_area(body).on_press(Message::FocusTerminal);
    let mut layers = column![];
    if app.search.is_some() {
        layers = layers.push(search_strip(query, shown, screen.blocks.len()));
    }
    layers = layers.push(body);
    if let Some(d) = dock {
        layers = layers.push(d);
    }
    container(layers)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_| container::Style { background: Some(Background::Color(theme::BG_0)), text_color: Some(theme::TEXT), ..Default::default() })
        .into()
}
