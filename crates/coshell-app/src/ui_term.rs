//! Block view of a live terminal: each command as a card (who ran it, prompt,
//! time, output) and the prompt being typed on as an input row. The real
//! terminal stays underneath and keeps the keyboard.

use crate::i18n::{tf, tr};
use iced::widget::text::Span;
use iced::widget::{Space, button, column, container, mouse_area, row, span, text, text_input};
use iced::{Alignment, Background, Border, Color, Font, Length, Padding};

use crate::app::{ASK_INPUT, CoShell, Message, RUN_INPUT, SEARCH_INPUT, Tab, TabId};
use crate::blocks::{self, Block, Mark, Prompt, Screen, Source};
use crate::fonts::MONO;
use crate::selectable::Selectable;
use crate::icons::{COPY, FOLDER, GIT_BRANCH, OS_LINUX, OS_MAC, OS_WINDOWS, SEARCH, SEND, STOP, X};
use crate::theme::{self, alpha};
use coshell_term::alacritty_terminal::vte::ansi::Color as AnsiColor;
use coshell_term::{Run, StyledLine};
use crate::ui::{El, kbd, mono};

const TEXT: f32 = 13.0;
/// Command cards and the composer dock.
const R_DOCK: f32 = 16.0;
/// Output line height (the design's `leading-6`).
const OUTPUT_LINE_H: f32 = 24.0;
const MONO_SEMIBOLD: Font = crate::fonts::weight(MONO, iced::font::Weight::Semibold);
const MONO_MEDIUM: Font = crate::fonts::weight(MONO, iced::font::Weight::Medium);
const MONO_BOLD: Font = crate::fonts::weight(MONO, iced::font::Weight::Bold);

/// Turns a program's colour code into the colour the terminal underneath would draw.
type Paint<'p> = &'p dyn Fn(AnsiColor) -> Color;

fn badge<'a>(source: Source) -> El<'a> {
    let (content, bg, line, fg): (El<'a>, Color, Color, Color) = match source {
        Source::Ai => (
            row![crate::ui::face(8.0, crate::ui::Face::STILL), text("AI").size(10.5).font(crate::ui::SEMIBOLD).color(theme::PURPLE)].spacing(5).align_y(Alignment::Center).into(),
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

/// Height of the prompt's segments, and how far their slanted edges lean.
const SEG_H: f32 = 24.0;
const SEG_SLANT: f32 = 10.0;
/// Segment fills: the system, the folder, the git branch.
const SEG_SYSTEM: Color = theme::SLATE_600;
const SEG_FOLDER: Color = theme::BLUE;
const SEG_BRANCH: Color = theme::GREEN;
/// The slanted edge between two segments: `left`'s colour above the diagonal, `right`'s below.
struct Slant {
    left: Color,
    right: Color,
}

impl iced::widget::canvas::Program<Message> for Slant {
    type State = ();

    fn draw(&self, _: &(), renderer: &iced::Renderer, _: &iced::Theme, bounds: iced::Rectangle, _: iced::mouse::Cursor) -> Vec<iced::widget::canvas::Geometry> {
        use iced::Point;
        use iced::widget::canvas::{Frame, Path};
        let mut frame = Frame::new(renderer, bounds.size());
        let (w, h) = (bounds.width, bounds.height);
        let triangle = |a: Point, b: Point, c: Point| {
            Path::new(|p| {
                p.move_to(a);
                p.line_to(b);
                p.line_to(c);
                p.close();
            })
        };
        frame.fill(&triangle(Point::ORIGIN, Point::new(w, 0.0), Point::new(0.0, h)), self.left);
        frame.fill(&triangle(Point::new(w, 0.0), Point::new(w, h), Point::new(0.0, h)), self.right);
        vec![frame.into_geometry()]
    }
}

/// A slanted edge, to sit between two segments' fills.
fn slant<'a>(left: Color, right: Color) -> El<'a> {
    // The drawn edges are soft where they meet the segments beside them; a hair of each
    // side's colour underneath keeps the background from showing through there.
    let hair = |c: Color| container(Space::new()).width(1).height(SEG_H).style(crate::ui::fill(c));
    iced::widget::stack![
        row![hair(left), Space::new().width(Length::Fill), hair(right)].width(SEG_SLANT).height(SEG_H),
        iced::widget::canvas::Canvas::new(Slant { left, right }).width(SEG_SLANT).height(SEG_H),
    ]
    .into()
}

/// The kinds of system the prompt's first segment tells apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum System {
    Mac,
    Windows,
    Linux,
}

/// The kind of system named `os` (as a server or this computer reports it); servers are Linux unless they say otherwise.
fn system_kind(os: &str) -> System {
    let os = os.to_lowercase();
    if os.contains("mac") || os.contains("darwin") {
        System::Mac
    } else if os.contains("windows") {
        System::Windows
    } else {
        System::Linux
    }
}

/// The mark of the system named `os`.
pub(crate) fn system_icon(os: &str) -> &'static std::sync::LazyLock<iced::widget::svg::Handle> {
    match system_kind(os) {
        System::Mac => &OS_MAC,
        System::Windows => &OS_WINDOWS,
        System::Linux => &OS_LINUX,
    }
}

/// The system a tab's shell runs on and its full name, once known. A local shell runs on
/// this computer's system even before its name has been read.
fn system_of(tab: &Tab) -> Option<(System, String)> {
    let here = if cfg!(target_os = "macos") { (System::Mac, "macOS") } else if cfg!(target_os = "windows") { (System::Windows, "Windows") } else { (System::Linux, "Linux") };
    match tab.stats.as_ref().and_then(|s| s.os.as_deref()).map(str::trim) {
        Some(os) if tab.is_local() => Some((here.0, os.to_string())),
        Some(os) => Some((system_kind(os), os.to_string())),
        None if tab.is_local() => Some((here.0, here.1.to_string())),
        None => None,
    }
}

/// Where a past command ran: the dock's folder segment in small, on a card's head line.
fn folder_chip<'a>(cwd: &str) -> El<'a> {
    let folder = if cwd.is_empty() { "~".to_string() } else { cwd.to_string() };
    let label = row![crate::icons::tinted(&FOLDER, 11.0, theme::ACCENT_HI), text(folder).size(12).font(MONO_MEDIUM).color(theme::ACCENT_HI).wrapping(text::Wrapping::None)]
        .spacing(5)
        .align_y(Alignment::Center);
    container(label)
        .padding([2, 8])
        .style(|_| container::Style {
            background: Some(Background::Color(alpha(SEG_FOLDER, 0.16))),
            border: Border { radius: 6.0.into(), ..Default::default() },
            ..Default::default()
        })
        .into()
}

/// The prompt as segments with slanted edges: the system, the folder, and the folder's git branch.
fn prompt_segments<'a>(tab: &Tab, p: &Prompt) -> El<'a> {
    let label = |icon: &std::sync::LazyLock<iced::widget::svg::Handle>, s: String, color: Color| -> El<'a> {
        row![crate::icons::tinted(icon, 12.0, color), text(s).size(TEXT).font(MONO_SEMIBOLD).color(color).wrapping(text::Wrapping::None)].spacing(6).align_y(Alignment::Center).into()
    };
    let mut segments: Vec<(El<'a>, Color)> = Vec::new();
    if let Some((system, name)) = system_of(tab) {
        let icon = match system {
            System::Mac => &OS_MAC,
            System::Windows => &OS_WINDOWS,
            System::Linux => &OS_LINUX,
        };
        // The picture says which kind; the name (which Linux, which version) is a hover away.
        segments.push((crate::ui::tip(crate::icons::tinted(icon, 14.0, theme::TEXT), name, iced::widget::tooltip::Position::Top), SEG_SYSTEM));
    }
    segments.push((label(&FOLDER, if p.cwd.is_empty() { "~".to_string() } else { p.cwd.clone() }, theme::ON_ACCENT), SEG_FOLDER));
    if let Some(branch) = &tab.git.branch {
        segments.push((label(&GIT_BRANCH, branch.clone(), theme::BG_0), SEG_BRANCH));
    }
    let fills: Vec<Color> = segments.iter().map(|(_, fill)| *fill).collect();
    let mut r = row![].align_y(Alignment::Center);
    for (i, (content, fill)) in segments.into_iter().enumerate() {
        // The first one's left end is rounded like the dock it sits in.
        let (left, round) = if i == 0 { (10.0, 8.0) } else { (4.0, 0.0) };
        r = r.push(container(content).height(SEG_H).center_y(SEG_H).padding(Padding { top: 0.0, right: 4.0, bottom: 0.0, left }).style(move |_| container::Style {
            background: Some(Background::Color(fill)),
            border: Border { radius: iced::border::Radius { top_left: round, bottom_left: round, ..Default::default() }, ..Default::default() },
            ..Default::default()
        }));
        r = r.push(slant(fill, fills.get(i + 1).copied().unwrap_or(Color::TRANSPARENT)));
    }
    r.into()
}

/// Command output, selectable and copyable (⌘C) like the terminal underneath.
fn output_text<'a>(lines: &[StyledLine], query: &str, paint: Paint<'_>) -> El<'a> {
    let end = lines.iter().rposition(|l| !l.text.trim().is_empty()).map_or(0, |i| i + 1);
    let rows: Vec<Vec<Span<'a, (), Font>>> = lines[..end].iter().map(|l| line_spans(l, query, paint)).collect();
    let out = Selectable::new(rows, Message::AiCopy).font(MONO).size(TEXT).line_height(OUTPUT_LINE_H).selection(alpha(theme::ACCENT, 0.35));
    container(out).padding(Padding::ZERO.left(4.0)).into()
}

/// One output line: the program's colours where it chose them, the app's marks on
/// the rest, and search matches over both.
fn line_spans<'a>(line: &StyledLine, query: &str, paint: Paint<'_>) -> Vec<Span<'a, (), Font>> {
    let mut out = Vec::new();
    let mut at = 0;
    for (piece, mark) in blocks::highlight(&line.text, query) {
        let end = at + piece.len();
        // Cut the piece where coloured runs start or end inside it.
        let mut cuts = vec![at, end];
        cuts.extend(line.runs.iter().flat_map(|r| [r.start, r.end]).filter(|c| (at + 1..end).contains(c)));
        cuts.sort_unstable();
        for w in cuts.windows(2) {
            let run = line.runs.iter().find(|r| r.start <= w[0] && w[1] <= r.end);
            out.push(paint_span(span(line.text[w[0]..w[1]].to_string()), mark, run, paint));
        }
        at = end;
    }
    out
}

fn paint_span<'a>(sp: Span<'a, (), Font>, mark: Mark, run: Option<&Run>, paint: Paint<'_>) -> Span<'a, (), Font> {
    match (mark, run) {
        (Mark::Found, _) => sp.color(theme::AMBER_200).background(alpha(theme::WARN, 0.3)),
        (_, Some(r)) => sp.color(paint(r.fg)).font(if r.bold { MONO_BOLD } else { MONO }),
        (Mark::Plain, None) => sp.color(alpha(theme::TEXT, 0.9)),
        (Mark::High | Mark::Warn, None) => sp.color(theme::WARN).font(MONO_SEMIBOLD),
        (Mark::Err, None) => sp.color(theme::ERR_TX).font(MONO_SEMIBOLD),
        (Mark::Ok, None) => sp.color(theme::OK).font(MONO_MEDIUM),
        (Mark::Path, None) => sp.color(theme::INFO),
    }
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

#[allow(clippy::too_many_arguments)]
fn block_card<'a>(tab: &Tab, i: usize, b: &Block, meta: Option<&blocks::Meta>, hovered: bool, running: bool, query: &str, paint: Paint<'_>) -> El<'a> {
    let mut head = row![badge(meta.map_or(Source::User, |m| m.source))].spacing(8).align_y(Alignment::Center);
    if b.prompt.sym.is_empty() {
        // A prompt of the user's own making: shown as it is.
        for p in prompt_parts(&b.prompt) {
            head = head.push(p);
        }
    } else {
        head = head.push(folder_chip(&b.prompt.cwd)).push(mono(b.prompt.sym.clone(), TEXT, theme::TEXT_2));
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
    let output = b.output.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
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
    if !b.output.iter().all(|l| l.text.trim().is_empty()) {
        col = col.push(output_text(&b.output, query, paint));
    }
    let card = container(col).padding(16).width(Length::Fill).style(move |_| container::Style {
        background: Some(Background::Color(theme::CARD)),
        border: Border { color: if hovered { theme::LINE_2 } else { theme::LINE }, width: 1.0, radius: R_DOCK.into() },
        ..Default::default()
    });
    let card = crate::ui::lit(card, R_DOCK);
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
    let placeholder = if secret {
        tr("비밀번호 입력 후 Enter (화면에 표시되지 않음)")
    } else if tab.sig.is_none() {
        // No prompt yet: the shell is still starting.
        tr("셸을 준비하는 중…")
    } else {
        tr("실행 중인 프로그램에 입력 후 Enter · '#'으로 AI 질문")
    };
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
    let ai_mode = typed.trim_start().starts_with('#');
    let mut r = row![].spacing(6).align_y(Alignment::Center);
    if ai_mode {
        r = r.push(
            container(row![crate::ui::face(8.0, app.face.idle()), text(tr("AI 질문")).size(10.5).font(crate::ui::SEMIBOLD).color(theme::PURPLE)].spacing(5).align_y(Alignment::Center)).padding([2, 8]).style(|_| container::Style {
                background: Some(Background::Color(alpha(theme::PURPLE, 0.15))),
                border: Border { color: alpha(theme::PURPLE, 0.3), width: 1.0, radius: crate::ui::R_PILL.into() },
                ..Default::default()
            }),
        );
    }
    if p.sym.is_empty() {
        // A prompt of the user's own making: shown as it is.
        for part in prompt_parts(p) {
            r = r.push(part);
        }
    } else {
        r = r.push(prompt_segments(tab, p)).push(mono(p.sym.clone(), TEXT, theme::TEXT_2));
    }
    let caret = || -> El<'a> { container(Space::new()).width(2).height(16).style(crate::ui::fill(theme::ACCENT_TX)).into() };
    let chars: Vec<char> = typed.chars().collect();
    let split = chars.len().saturating_sub(caret_tail);
    let before: String = chars[..split].iter().collect();
    let after: String = chars[split..].iter().collect();
    let ghost = blocks::ghost(typed, &tab.metas);
    let mut line = row![].align_y(Alignment::Center);
    if typed.is_empty() {
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
    if ghost.is_some() {
        r = r.push(kbd("Tab ⇥"));
    }
    // The conversation (⌘L, or `#`), for the mouse.
    let chat = button(container(crate::ui::face(13.0, app.face.idle())).center(28)).padding(0).on_press(Message::AiToggle).style(|_, st| button::Style {
        background: matches!(st, button::Status::Hovered | button::Status::Pressed).then_some(Background::Color(theme::BG_3)),
        border: Border { radius: 10.0.into(), ..Default::default() },
        ..Default::default()
    });
    r = r.push(crate::ui::tip(chat, tr("AI 대화 (⌘L)"), iced::widget::tooltip::Position::Top));
    // Return: Enter to the shell.
    let send = button(container(crate::icons::tinted(&SEND, 13.0, theme::ON_ACCENT)).center(28))
        .padding(0)
        .on_press(Message::Input(tab.id, b"\r".to_vec()))
        .style(|_, st| button::Style {
            background: Some(Background::Color(alpha(theme::ACCENT, if matches!(st, button::Status::Hovered | button::Status::Pressed) { 0.88 } else { 1.0 }))),
            border: Border { radius: 10.0.into(), ..Default::default() },
            ..Default::default()
        });
    r = r.push(send);
    let focused = app.terminal_active(tab);
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

/// Width of the lit edge around the dock in question mode.
const GLOW_W: f32 = 1.5;
/// Where the edge's light rests while nothing is being answered.
const GLOW_REST: f32 = 2.2;
/// Widest the chat sheet gets: about 80 characters of a reply per line.
const SHEET_W: f32 = 700.0;
/// The sheet's height is what was said, up to this part of the window, so the
/// commands the assistant runs stay in view above it.
const SHEET_MAX_PART: f32 = 0.5;
const SHEET_R: f32 = 20.0;
/// Space kept clear on each side of the sheet.
const SHEET_MARGIN: f32 = 28.0;
/// Width of the divider beside the side panel.
const SIDE_EDGE: f32 = 4.0;
/// Narrowest the sheet gets in a small window.
const SHEET_MIN_W: f32 = 260.0;

/// Gentle at both ends.
fn ease_in_out(t: f32) -> f32 {
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
}

/// The orb over the sheet, as a face height (`ui::face`): large, so its light can be seen.
const SHEET_ORB: f32 = 52.0;
/// How wide (and tall) that makes it.
const SHEET_ORB_D: f32 = SHEET_ORB * crate::ui::FACE_ASPECT;

/// Height of the fade under the sheet's top edge.
const SHEET_FADE: f32 = 22.0;

/// What is left of an entrance at `progress` (0 → 1): all of it at first, easing out to nothing.
fn settle(progress: f32) -> f32 {
    (1.0 - progress.clamp(0.0, 1.0)).powi(3)
}

/// The light along the dock's edge. It rests at one angle and travels round while a reply is coming.
fn glow(app: &CoShell, working: bool) -> iced::gradient::Linear {
    // On opening the light makes one turn of the edge before it comes to rest.
    let entrance = settle(app.chat_intro()) * std::f32::consts::TAU;
    let angle = if working { app.fold / crate::ui_status::CYCLE_SECS * std::f32::consts::TAU } else { GLOW_REST + entrance };
    iced::gradient::Linear::new(iced::Radians(angle)).add_stop(0.0, theme::PURPLE).add_stop(0.5, theme::BLUE).add_stop(1.0, theme::TEAL)
}

/// Question mode (`#`): the dock is the chat's input. Enter sends and it stays open for the
/// next question; while a reply is coming the send button is a stop button.
fn ask_row<'a>(app: &'a CoShell, tab: &'a Tab, q: &'a str) -> El<'a> {
    let working = tab.chat.is_streaming();
    // The question is kept with its `#` (what opened the conversation); the field shows it without.
    let field = text_input(tr("무엇이든 물어보세요"), q.strip_prefix('#').unwrap_or(q))
        .id(ASK_INPUT)
        .on_input(|typed| Message::AskInput(format!("#{typed}")))
        .on_submit(Message::AskSubmit)
        .font(MONO)
        .size(TEXT)
        .padding(0)
        .width(Length::Fill)
        .style(|t, st| text_input::Style { value: theme::SLATE_100, ..crate::ui::bare_input_style(t, st) });
    let close = button(crate::icons::tinted(&X, 12.0, theme::TEXT_2)).padding(6).style(crate::ui::btn_style(crate::ui::Btn::Ghost)).on_press(Message::AskCancel);
    let round = |bg: Background| button::Style { background: Some(bg), border: Border { radius: 10.0.into(), ..Default::default() }, ..Default::default() };
    let action: El<'a> = if working {
        let stop = button(container(crate::icons::tinted(&STOP, 12.0, theme::TEXT)).center(28)).padding(0).on_press(Message::AiStop(tab.id)).style(move |_, st| {
            round(Background::Color(alpha(Color::WHITE, if matches!(st, button::Status::Hovered | button::Status::Pressed) { 0.22 } else { 0.14 })))
        });
        crate::ui::tip(stop, tr("중지"), iced::widget::tooltip::Position::Top)
    } else {
        let edge = glow(app, false);
        button(container(crate::icons::tinted(&SEND, 13.0, theme::ON_ACCENT)).center(28)).padding(0).on_press(Message::AskSubmit).style(move |_, _| round(Background::Gradient(edge.into()))).into()
    };
    let r = row![
        field,
        mono(tr("Enter 질문 · Esc 닫기"), 11.0, theme::TEXT_3),
        crate::ui::tip(close, tr("대화 닫기"), iced::widget::tooltip::Position::Top),
        action,
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    // The same size as the prompt's dock, so nothing moves when `#` is pressed.
    let inner = container(r).padding(Padding { top: 8.0 - GLOW_W, right: 8.0 - GLOW_W, bottom: 8.0 - GLOW_W, left: 14.0 - GLOW_W }).width(Length::Fill).style(|_| container::Style {
        background: Some(Background::Color(theme::BG_2)),
        border: Border { radius: (R_DOCK - GLOW_W).into(), ..Default::default() },
        ..Default::default()
    });
    let edge = glow(app, working);
    let flare = settle(app.chat_intro());
    container(inner)
        .padding(GLOW_W)
        .width(Length::Fill)
        .style(move |_| container::Style {
            background: Some(Background::Gradient(edge.into())),
            border: Border { radius: R_DOCK.into(), ..Default::default() },
            // It flares as the mode changes, then dims to its resting glow.
            shadow: iced::Shadow { color: alpha(theme::PURPLE, if working { 0.45 } else { 0.25 } + 0.5 * flare), offset: iced::Vector::new(0.0, 4.0), blur_radius: 28.0 + 36.0 * flare },
            ..Default::default()
        })
        .into()
}

/// How the conversation's sheet stands for `tab`: how far it is drawn into the dock (0: all
/// out), and whether it is the live conversation (on its way out it takes no input). None:
/// not on screen.
fn sheet_motion(app: &CoShell, tab: TabId) -> Option<(f32, bool)> {
    if app.ask.as_ref().is_some_and(|(id, _)| *id == tab) {
        return Some((1.0 - ease_in_out(app.chat_intro()), true));
    }
    app.chat_outro().filter(|(id, _)| *id == tab).map(|(_, o)| (ease_in_out(o), false))
}

/// The conversation, on a sheet of dark glass over the terminal. Like a window from the
/// Dock it pours out of the AI's face in the dock as it opens and is drawn back into it as
/// it closes (`sucked`: how far in it is).
fn chat_sheet<'a>(app: &'a CoShell, tab: &'a Tab, sucked: f32, live: bool) -> El<'a> {
    let max_height = app.settings.window.height * SHEET_MAX_PART;
    let talk = crate::ui_ai::sheet_conversation(tab, app.prod_guard, app.fold, max_height);
    // What has scrolled up fades into the sheet instead of being cut at its edge.
    let fade = container(Space::new()).width(Length::Fill).height(SHEET_FADE).style(|_| {
        let into_sheet = iced::gradient::Linear::new(iced::Radians(std::f32::consts::PI)).add_stop(0.0, theme::BG_3).add_stop(1.0, alpha(theme::BG_3, 0.0));
        container::Style { background: Some(Background::Gradient(into_sheet.into())), ..Default::default() }
    });
    // Room at the top for the lower half of the orb that floats over the sheet's edge.
    let talk = column![Space::new().height(SHEET_ORB_D / 2.0 + 4.0), iced::widget::stack![talk, fade], crate::ui::rule(true), crate::ui_ai::sheet_tools(app, tab)];
    let side = if app.settings.sidebar_open { app.settings.sidebar_width + SIDE_EDGE } else { 0.0 };
    let full_w = (app.settings.window.width - crate::ui_sidebar::DOCK_W - side - 2.0 * SHEET_MARGIN).clamp(SHEET_MIN_W, SHEET_W);
    let sheet = container(container(talk).width(full_w)).clip(true).style(|_| {
        let glass = iced::gradient::Linear::new(iced::Radians(std::f32::consts::PI)).add_stop(0.0, alpha(theme::BG_3, 0.9)).add_stop(1.0, alpha(theme::BG_1, 0.94));
        container::Style {
            background: Some(Background::Gradient(glass.into())),
            border: Border { color: theme::LINE_2, width: 1.0, radius: SHEET_R.into() },
            shadow: iced::Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.55), offset: iced::Vector::new(0.0, 16.0), blur_radius: 44.0 },
            text_color: Some(theme::TEXT),
            ..Default::default()
        }
    });
    // Clicks on the sheet stay on it; around it the terminal is still within reach.
    let sheet: El<'a> = if live { iced::widget::opaque(sheet) } else { sheet.into() };
    // The AI itself floats over the sheet's top edge, half above it.
    let orb = container(crate::ui::face(SHEET_ORB, app.face)).center_x(Length::Fill);
    let sheet = iced::widget::stack![column![Space::new().height(SHEET_ORB_D / 2.0), sheet], orb];
    crate::genie::Genie::new(sheet, sucked, FACE_SPOT, Padding { top: 24.0, right: SHEET_MARGIN, bottom: 4.0, left: SHEET_MARGIN }, live).into()
}

/// The dock, found on screen when a snippet's command flies into it.
pub const DOCK: &str = "term-dock";

/// Padding around the dock at the bottom of the terminal.
const DOCK_PAD: Padding = Padding { top: 8.0, right: 28.0, bottom: 16.0, left: 28.0 };
/// Height of the AI face at the dock's left.
const DOCK_FACE: f32 = 14.0;
/// The middle of that face, from the bottom left corner of the area above the dock: where
/// the sheet comes from and goes back to.
const FACE_SPOT: iced::Vector = iced::Vector::new(DOCK_PAD.left + 14.0 + DOCK_FACE * crate::ui::FACE_ASPECT / 2.0, DOCK_PAD.top + 22.0);

/// The chat over a terminal that has no block view (a full-screen program is running):
/// the sheet and the question dock along the bottom, the program still visible above.
/// On its way out only the sheet, going back into where the dock was.
pub fn chat_overlay<'a>(app: &'a CoShell, tab: &'a Tab) -> Option<El<'a>> {
    let (sucked, live) = sheet_motion(app, tab.id)?;
    let sheet = chat_sheet(app, tab, sucked, live);
    match app.ask.as_ref().filter(|(id, _)| *id == tab.id) {
        Some((_, q)) => Some(column![sheet, iced::widget::opaque(container(ask_row(app, tab, q)).padding(DOCK_PAD))].into()),
        None => Some(sheet),
    }
}

pub fn view<'a>(app: &'a CoShell, tab: &'a Tab, screen: Screen) -> El<'a> {
    let query = app.search.as_ref().map_or("", |s| s.query.as_str());
    let q = query.trim().to_lowercase();
    let metas = blocks::match_metas(&screen.blocks, &tab.metas);
    let running = screen.running();
    let last = screen.blocks.len().saturating_sub(1);
    let mut col = column![].spacing(16).width(Length::Fill);
    // Output in the terminal's colours (and any a program redefined). The cards are dark whatever
    // colour scheme the raw terminal has, so the dark palette is the one that reads on them.
    let palette = &theme::TERM_DARK;
    let colors = match &tab.phase {
        crate::app::Phase::Live(s) => *s.term().lock().colors(),
        _ => Default::default(),
    };
    let paint = move |c: AnsiColor| crate::terminal::resolve(c, palette, &colors);
    if !screen.preamble.iter().all(|l| l.text.trim().is_empty()) && q.is_empty() {
        col = col.push(container(output_text(&screen.preamble, "", &paint)).padding(10));
    }
    let mut shown = 0;
    for (i, b) in screen.blocks.iter().enumerate() {
        if !q.is_empty() && !b.cmd.to_lowercase().contains(&q) && !b.output.iter().any(|l| l.text.to_lowercase().contains(&q)) {
            continue;
        }
        shown += 1;
        col = col.push(block_card(tab, i, b, metas[i], app.block_hover == Some(i), running && i == last, query, &paint));
    }
    if !q.is_empty() && shown == 0 {
        col = col.push(
            container(text(tf("'{0}'에 해당하는 터미널 기록이 없습니다.", &[&query.trim()])).size(12).color(theme::TEXT_3))
                .center_x(Length::Fill)
                .padding([40, 0]),
        );
    }
    // In question mode the dock is the chat's input, whatever the shell is doing. Otherwise: at a
    // prompt it mirrors the shell's line; with none (a program running) it takes input for it.
    let asking = app.ask.as_ref().filter(|(id, _)| *id == tab.id).map(|(_, q)| q.as_str());
    let dock = match (asking, &screen.current) {
        (Some(q), _) => ask_row(app, tab, q),
        (None, Some((p, typed))) => {
            let tail = match &tab.phase {
                crate::app::Phase::Live(s) => blocks::cursor_tail(s.term()),
                _ => 0,
            };
            // It only mirrors the shell's line, so a click on it means typing to the shell.
            mouse_area(prompt_row(app, tab, p, typed, tail)).on_press(Message::FocusTerminal).into()
        }
        (None, None) => {
            let last_line = screen.blocks.last().map_or(&screen.preamble, |b| &b.output).iter().rev().find(|l| !l.text.trim().is_empty());
            run_row(app, tab, running, last_line.is_some_and(|l| blocks::asks_secret(&l.text)))
        }
    };
    let dock = Some(container(dock).padding(DOCK_PAD).id(iced::widget::Id::new(DOCK)));
    col = col.push(Space::new().height(24));
    let body = crate::ui::scroll(container(col).padding(Padding { top: 24.0, right: 28.0, bottom: 0.0, left: 28.0 }).width(Length::Fill))
        .anchor_bottom()
        .height(Length::Fill)
        .width(Length::Fill);
    // A click anywhere gives the keyboard back to the shell.
    let body: El<'a> = mouse_area(body).on_press(Message::FocusTerminal).into();
    let body = match sheet_motion(app, tab.id) {
        Some((sucked, live)) => iced::widget::stack![body, chat_sheet(app, tab, sucked, live)].into(),
        None => body,
    };
    let mut layers = column![];
    if app.search.is_some() {
        layers = layers.push(search_strip(query, shown, screen.blocks.len()));
    }
    layers = layers.push(body);
    if let Some(d) = dock {
        layers = layers.push(d);
    }
    // The glow lies on a solid ground (the terminal itself is underneath and must not show), the cards on the glow.
    let glow = iced::widget::image(std::sync::LazyLock::force(&crate::icons::AURA).clone())
        .width(Length::Fill)
        .height(Length::Fill)
        .content_fit(iced::ContentFit::Fill);
    let ground = container(glow).width(Length::Fill).height(Length::Fill).style(crate::ui::fill(theme::BG_0));
    let cards = container(layers).width(Length::Fill).height(Length::Fill).style(|_| container::Style { text_color: Some(theme::TEXT), ..Default::default() });
    iced::widget::stack![ground, cards].into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use coshell_term::alacritty_terminal::vte::ansi::NamedColor;

    fn paint(c: AnsiColor) -> Color {
        if c == AnsiColor::Named(NamedColor::Blue) { Color::from_rgb8(0, 0, 255) } else { Color::BLACK }
    }

    fn pieces(line: &StyledLine, query: &str) -> Vec<(String, Option<Color>, Option<Font>)> {
        line_spans(line, query, &paint).into_iter().map(|s| (s.text.to_string(), s.color, s.font)).collect()
    }

    #[test]
    fn program_colours_win_and_plain_text_keeps_the_app_marks() {
        let blue = AnsiColor::Named(NamedColor::Blue);
        // `ls`-style: a bold blue folder (whose name looks like a path), then uncoloured text with "error".
        let text = "/var/logs error ok";
        let line = StyledLine { text: text.into(), runs: vec![Run { start: 0, end: 9, fg: blue, bold: true }] };
        let p = pieces(&line, "");
        assert_eq!(p.iter().map(|(t, ..)| t.as_str()).collect::<String>(), text, "no text lost or repeated");
        assert_eq!(p[0], ("/var/logs".into(), Some(Color::from_rgb8(0, 0, 255)), Some(MONO_BOLD)), "the program's colour, not the path mark");
        assert!(p.contains(&("error".into(), Some(theme::ERR_TX), Some(MONO_SEMIBOLD))), "{p:?}");

        // A search match is shown over a coloured run, and the run's remaining parts keep their colour.
        let p = pieces(&line, "LOG");
        assert_eq!(p[0], ("/var/".into(), Some(Color::from_rgb8(0, 0, 255)), Some(MONO_BOLD)));
        assert_eq!(p[1].0, "log");
        assert_eq!(p[1].1, Some(theme::AMBER_200));
        assert_eq!(p[2], ("s".into(), Some(Color::from_rgb8(0, 0, 255)), Some(MONO_BOLD)));
    }
}

#[cfg(test)]
mod system_tests {
    use super::*;

    #[test]
    fn an_entrance_starts_whole_and_eases_out_to_nothing() {
        assert_eq!(settle(0.0), 1.0);
        assert_eq!(settle(1.0), 0.0);
        assert!(settle(0.5) < 0.5, "most of the movement comes early");
        assert!(settle(0.2) > settle(0.6));
        assert_eq!(settle(7.0), 0.0, "never past the end");
        assert_eq!((ease_in_out(0.0), ease_in_out(1.0)), (0.0, 1.0));
        assert!(ease_in_out(0.3) < ease_in_out(0.6), "always forward");
        assert!((ease_in_out(0.3) - (1.0 - ease_in_out(0.7))).abs() < 1e-6, "symmetric: an exit turned back half way meets its entrance");
    }

    #[test]
    fn systems_are_told_apart_by_name() {
        assert_eq!(system_kind("macOS 27.2"), System::Mac);
        assert_eq!(system_kind("Microsoft Windows Server 2022"), System::Windows);
        for linux in ["Ubuntu 22.04.4 LTS", "Red Hat Enterprise Linux 9.4 (Plow)", "Rocky Linux 9.9 (Blue Onyx)", ""] {
            assert_eq!(system_kind(linux), System::Linux, "{linux:?}");
        }
    }
}
