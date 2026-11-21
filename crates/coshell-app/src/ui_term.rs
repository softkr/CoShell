//! Block view of a live terminal: each command as a card (who ran it, prompt,
//! time, output) and the prompt being typed on as an input row. The real
//! terminal stays underneath and keeps the keyboard.

use crate::i18n::{tf, tr};
use iced::widget::text::{LineHeight, Span};
use iced::widget::{Space, button, column, container, mouse_area, rich_text, row, scrollable, span, text, text_input};
use iced::{Alignment, Background, Border, Color, Font, Length, Padding};

use crate::app::{ASK_INPUT, CoShell, Message, SEARCH_INPUT, Tab};
use crate::blocks::{self, Block, Mark, Prompt, Screen, Source};
use crate::fonts::MONO;
use crate::icons::{COPY, SEARCH, SPARKLES, X};
use crate::theme::{self, alpha};
use crate::ui::{El, R_CARD, kbd, mono};

const TEXT: f32 = 13.0;
/// Output line height (the design's `leading-6`).
const OUTPUT_LINE_H: f32 = 24.0;
const MONO_SEMIBOLD: Font = crate::fonts::weight(MONO, iced::font::Weight::Semibold);
const MONO_MEDIUM: Font = crate::fonts::weight(MONO, iced::font::Weight::Medium);

fn badge<'a>(source: Source) -> El<'a> {
    let (content, bg, line, fg): (El<'a>, Color, Color, Color) = match source {
        Source::Ai => (
            row![crate::icons::tinted(&SPARKLES, 12.0, theme::ACCENT_TX), text("AI").size(11).font(crate::ui::MEDIUM).color(theme::ACCENT_TX)]
                .spacing(4)
                .align_y(Alignment::Center)
                .into(),
            alpha(theme::ACCENT, 0.15),
            alpha(theme::ACCENT, 0.3),
            theme::ACCENT_TX,
        ),
        Source::User => (text(tr("사용자")).size(11).font(crate::ui::MEDIUM).color(theme::SLATE_300).into(), theme::BG_2, theme::LINE_2, theme::SLATE_300),
    };
    let _ = fg;
    container(content)
        .padding([2, 8])
        .style(move |_| container::Style {
            background: Some(Background::Color(bg)),
            border: Border { color: line, width: 1.0, radius: 6.0.into() },
            ..Default::default()
        })
        .into()
}

/// `user@host : cwd $` as the design draws it.
fn prompt_parts<'a>(p: &Prompt) -> Vec<El<'a>> {
    let mut parts: Vec<El<'a>> = vec![text(p.user_host.clone()).size(TEXT).font(MONO_SEMIBOLD).color(theme::ACCENT_TX).into()];
    if !p.sym.is_empty() {
        parts.push(mono(":", TEXT, theme::TEXT_3));
        parts.push(mono(if p.cwd.is_empty() { "~".to_string() } else { p.cwd.clone() }, TEXT, theme::INFO));
        parts.push(mono(p.sym.clone(), TEXT, theme::SLATE_300));
    }
    parts
}

fn output_text<'a>(lines: &[String], query: &str) -> El<'a> {
    let body = lines.join("\n");
    let spans: Vec<Span<'a, (), Font>> = blocks::highlight(body.trim_end(), query)
        .into_iter()
        .map(|(s, m)| {
            let sp = span(s);
            match m {
                Mark::Plain => sp.color(alpha(theme::SLATE_300, 0.9)),
                Mark::High | Mark::Warn => sp.color(theme::WARN).font(MONO_SEMIBOLD),
                Mark::Err => sp.color(theme::ERR_TX).font(MONO_SEMIBOLD),
                Mark::Ok => sp.color(theme::ACCENT_TX).font(MONO_MEDIUM),
                Mark::Path => sp.color(alpha(theme::SKY_300, 0.9)),
                Mark::Found => sp.color(theme::AMBER_200).background(alpha(theme::WARN, 0.3)),
            }
        })
        .collect();
    container(rich_text(spans).font(MONO).size(TEXT).line_height(LineHeight::Absolute(OUTPUT_LINE_H.into())))
        .padding(Padding::ZERO.left(4.0))
        .into()
}

fn small_btn<'a>(content: impl Into<El<'a>>, msg: Message) -> El<'a> {
    button(content)
        .padding([2, 8])
        .on_press(msg)
        .style(|_, st| {
            let hover = matches!(st, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: Some(Background::Color(if hover { alpha(theme::ACCENT, 0.2) } else { theme::BG_2 })),
                text_color: if hover { theme::ACCENT_TX } else { theme::SLATE_300 },
                border: Border { color: theme::LINE, width: 1.0, radius: 4.0.into() },
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
            let (fg, bg) = if code == 0 { (alpha(theme::ACCENT_TX, 0.8), alpha(theme::ACCENT, 0.1)) } else { (theme::ERR_TX, alpha(theme::ERR, 0.1)) };
            info = info.push(container(mono(format!("exit {code}"), 10.0, fg)).padding([0, 4]).style(move |_| container::Style {
                background: Some(Background::Color(bg)),
                border: Border { radius: 3.0.into(), ..Default::default() },
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
                .padding(4)
                .on_press(Message::AiCopy(format!("{}\n{}", b.cmd, output)))
                .style(|_, st| button::Style {
                    background: Some(Background::Color(if matches!(st, button::Status::Hovered) { theme::BG_3 } else { theme::BG_2 })),
                    border: Border { color: theme::LINE, width: 1.0, radius: 4.0.into() },
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
    let mut col = column![head].spacing(8);
    if !b.output.iter().all(|l| l.trim().is_empty()) {
        col = col.push(output_text(&b.output, query));
    }
    let card = container(col).padding(10).width(Length::Fill).style(move |_| container::Style {
        background: hovered.then_some(Background::Color(alpha(theme::BG_1, 0.35))),
        border: Border { color: if hovered { alpha(theme::LINE, 0.6) } else { Color::TRANSPARENT }, width: 1.0, radius: R_CARD.into() },
        ..Default::default()
    });
    mouse_area(card).on_enter(Message::BlockHover(Some(i))).on_exit(Message::BlockHover(None)).into()
}

/// The prompt waiting for input: what is typed, the caret, and an inline suggestion.
fn prompt_row<'a>(app: &CoShell, tab: &Tab, p: &Prompt, typed: &str, caret_tail: usize) -> El<'a> {
    let asking = app.ask.as_ref().filter(|(id, _)| *id == tab.id).map(|(_, q)| q.as_str());
    let ai_mode = asking.is_some() || typed.trim_start().starts_with('#');
    let mut r = row![].spacing(6).align_y(Alignment::Center);
    if ai_mode {
        r = r.push(
            container(text(tr("AI 질문")).size(10).font(crate::ui::SEMIBOLD).color(theme::EMERALD_300)).padding([1, 6]).style(|_| container::Style {
                background: Some(Background::Color(alpha(theme::ACCENT, 0.2))),
                border: Border { color: alpha(theme::ACCENT, 0.4), width: 1.0, radius: 4.0.into() },
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
        line = line.push(caret()).push(mono(tr("명령어 입력 (Tab 자동완성 · '#'으로 바로 AI 질문)..."), TEXT, alpha(theme::TEXT_3, 0.7)));
    } else {
        line = line.push(mono(before, TEXT, theme::SLATE_100)).push(caret()).push(mono(after, TEXT, theme::SLATE_100));
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
    let focused = app.terminal_active(tab);
    container(r)
        .padding([8, 10])
        .width(Length::Fill)
        .style(move |_| container::Style {
            background: focused.then_some(Background::Color(alpha(theme::BG_1, 0.4))),
            border: Border { color: if focused { theme::LINE } else { Color::TRANSPARENT }, width: 1.0, radius: R_CARD.into() },
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
    column![container(strip).padding([8, 16]).width(Length::Fill).style(crate::ui::fill(alpha(theme::BG_1, 0.95))), crate::ui::rule(true)].into()
}

pub fn view<'a>(app: &'a CoShell, tab: &'a Tab, screen: Screen) -> El<'a> {
    let query = app.search.as_ref().map_or("", |s| s.query.as_str());
    let q = query.trim().to_lowercase();
    let metas = blocks::match_metas(&screen.blocks, &tab.metas);
    let running = screen.running();
    let last = screen.blocks.len().saturating_sub(1);
    let mut col = column![].spacing(24).width(Length::Fill);
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
    if let Some((p, typed)) = &screen.current {
        let tail = match &tab.phase {
            crate::app::Phase::Live(s) => blocks::cursor_tail(s.term()),
            _ => 0,
        };
        col = col.push(prompt_row(app, tab, p, typed, tail));
    }
    col = col.push(Space::new().height(48));
    let body = scrollable(container(col).padding(Padding { top: 20.0, right: 24.0, bottom: 0.0, left: 24.0 }).width(Length::Fill))
        .anchor_bottom()
        .height(Length::Fill)
        .width(Length::Fill);
    // A click anywhere gives the keyboard back to the shell.
    let body = mouse_area(body).on_press(Message::FocusTerminal);
    let mut layers = column![];
    if app.search.is_some() {
        layers = layers.push(search_strip(query, shown, screen.blocks.len()));
    }
    container(layers.push(body))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_| container::Style { background: Some(Background::Color(theme::BG_0)), text_color: Some(theme::TEXT), ..Default::default() })
        .into()
}
