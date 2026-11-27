//! Settings dialog (⌘,), laid out like macOS's System Settings: a sidebar of pages (general,
//! shortcuts, terminal, servers, SSH keys, AI), each a form of right-aligned labels beside
//! their controls in an inset box; the terminal page has a segmented control.

use crate::i18n::{tf, tr};
use iced::widget::{Space, button, checkbox, column, container, row, text, text_input};
use iced::{Alignment, Background, Border, Color, Length, Padding};

use crate::app::{CoShell, Message, SettingsDraft, SettingsTab, TermTab};
use crate::settings::{self, CursorKind, Language, Scheme};
use crate::icons::{DOCK_TERMINAL, GRID, KEY, MINUS, PLUG, PLUS, SPARKLES, ZAP};
use crate::theme::{self, alpha};
use crate::form::group;
use crate::ui::{Btn, El, btn, card_style, input_style, mono, muted};

const DIALOG_W: f32 = 880.0;
/// Pages scroll inside this, so the title and the buttons always stay visible.
const PAGE_H: f32 = 500.0;
const NAV_W: f32 = 176.0;
/// The coloured tile a page's icon sits on in the sidebar, as in macOS's System Settings.
const NAV_TILE: f32 = 22.0;
const NUMBER_W: f32 = 96.0;
/// The form's label column (labels are right-aligned against their controls).
const LABEL_W: f32 = 170.0;
/// Widest a drop-down or text field gets in the form.
const FIELD_W: f32 = 320.0;
const STEP_W: f32 = 30.0;
const CONTROL_H: f32 = 30.0;

/// (keys, what they do). macOS glyphs; ⌘ is Ctrl on other systems.
const SHORTCUTS: [(&str, &str); 15] = [
    ("⌘K", "서버 검색 · 명령"),
    ("⌘O", "서버 목록"),
    ("⌘T", "새 로컬 셸"),
    ("⌘W", "탭 닫기"),
    ("⌘1–9", "탭 선택"),
    ("⌘⇧[ / ⌘⇧]", "이전 / 다음 탭"),
    ("⌘⇧← / ⌘⇧→", "탭 순서 변경"),
    ("⌘L", "AI 대화"),
    ("⌘F", "터미널 출력 검색"),
    ("⌘G / ⌘⇧G", "이전 / 다음 결과"),
    ("⌘C / ⌘V", "복사 / 붙여넣기"),
    ("⌘ 클릭", "링크 열기"),
    ("⌘+ / ⌘−", "글꼴 크게 / 작게"),
    ("⌘0", "글꼴 크기 초기화"),
    ("⌘,", "설정"),
];

/// A label against its control, in this dialog's label column.
fn field<'a>(label: &str, control: impl Into<El<'a>>) -> El<'a> {
    crate::form::field(LABEL_W, label, control)
}

/// A line of explanation under a control, no wider than the fields.
fn hint<'a>(s: impl Into<String>) -> El<'a> {
    crate::form::hint(LABEL_W, FIELD_W, s)
}

/// One of `all`, picked from a drop-down as wide as the fields.
fn dropdown<'a, T: Copy + PartialEq + 'a>(all: &[T], current: T, name: fn(T) -> &'static str, msg: fn(T) -> Message) -> El<'a> {
    crate::form::dropdown(all, current, name, msg, FIELD_W)
}

/// A number with − and + joined to its right, the way GTK draws one.
fn stepper<'a>(value: El<'a>, minus: Option<Message>, plus: Option<Message>) -> El<'a> {
    let step = |icon, msg: Option<Message>| -> El<'a> {
        let color = if msg.is_some() { theme::TEXT } else { alpha(theme::TEXT, 0.3) };
        button(container(crate::icons::tinted(icon, 11.0, color)).center(Length::Fill))
            .width(STEP_W)
            .height(CONTROL_H)
            .padding(0)
            .on_press_maybe(msg)
            .style(|_, st| button::Style {
                background: Some(Background::Color(if matches!(st, button::Status::Hovered | button::Status::Pressed) { theme::BG_3 } else { theme::BG_2 })),
                ..Default::default()
            })
            .into()
    };
    let seam = || container(Space::new()).width(1).height(CONTROL_H).style(crate::ui::fill(theme::LINE_2));
    container(row![container(value).width(NUMBER_W).height(CONTROL_H).center_y(CONTROL_H).padding([0, 10]), seam(), step(&MINUS, minus), seam(), step(&PLUS, plus)])
        .clip(true)
        .style(|_| container::Style {
            background: Some(Background::Color(theme::BG_0)),
            border: Border { color: theme::LINE_2, width: 1.0, radius: crate::ui::R_CONTROL.into() },
            ..Default::default()
        })
        .into()
}

/// A typed number with − / + beside it.
fn number<'a>(value: &'a str, on_input: fn(String) -> Message, on_step: fn(i8) -> Message) -> El<'a> {
    let input = text_input("", value).on_input(on_input).on_submit(Message::SettingsSave).font(crate::fonts::MONO).size(12.5).padding(0).style(crate::ui::bare_input_style);
    stepper(input.into(), Some(on_step(-1)), Some(on_step(1)))
}

fn shortcuts<'a>() -> El<'a> {
    let rows = SHORTCUTS.iter().map(|&(keys, what)| field(tr(what), mono(tr(keys), 12.0, theme::TEXT)));
    group(tr("단축키"), rows.collect())
}

/// The sidebar, as macOS's System Settings draws one: each page a rounded row with its icon
/// on a coloured tile, the page shown filled with the accent, under quiet group headings.
fn nav<'a>(current: SettingsTab) -> El<'a> {
    let item = |tab: SettingsTab, name: &'a str, icon: &'static std::sync::LazyLock<iced::widget::svg::Handle>, tint: Color| -> El<'a> {
        let on = tab == current;
        // The tile: its colour, a little lighter at the top, like a key cap.
        let tile = container(crate::icons::tinted(icon, 13.0, Color::WHITE)).center(NAV_TILE).style(move |_| {
            let top = Color { r: (tint.r + 0.12).min(1.0), g: (tint.g + 0.12).min(1.0), b: (tint.b + 0.12).min(1.0), a: 1.0 };
            let face = iced::gradient::Linear::new(iced::Radians(std::f32::consts::PI)).add_stop(0.0, top).add_stop(1.0, tint);
            container::Style {
                background: Some(Background::Gradient(face.into())),
                border: Border { radius: 6.0.into(), ..Default::default() },
                shadow: iced::Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.25), offset: iced::Vector::new(0.0, 1.0), blur_radius: 2.0 },
                ..Default::default()
            }
        });
        let label = text(name).size(13).font(if on { crate::ui::MEDIUM } else { crate::fonts::SANS }).color(if on { theme::ON_ACCENT } else { theme::TEXT });
        button(row![tile, label].spacing(9).align_y(Alignment::Center))
            .width(Length::Fill)
            .padding([5, 8])
            .on_press(Message::SettingsTab(tab))
            .style(move |_, status| button::Style {
                background: Some(Background::Color(if on {
                    theme::ACCENT
                } else if matches!(status, button::Status::Hovered) {
                    alpha(Color::WHITE, 0.06)
                } else {
                    Color::TRANSPARENT
                })),
                text_color: theme::TEXT,
                border: Border { radius: 7.0.into(), ..Default::default() },
                ..Default::default()
            })
            .into()
    };
    let heading = |name: &'a str| -> El<'a> {
        container(text(name).size(11).font(crate::ui::SEMIBOLD).color(theme::TEXT_3)).padding(Padding { top: 10.0, right: 8.0, bottom: 3.0, left: 8.0 }).into()
    };
    let list = column![
        heading(tr("전체")),
        item(SettingsTab::General, tr("일반"), &GRID, Color::from_rgb8(0x8E, 0x8E, 0x93)),
        item(SettingsTab::Shortcuts, tr("바로 가기"), &ZAP, Color::from_rgb8(0xF0, 0x8A, 0x24)),
        item(SettingsTab::Terminal, tr("터미널"), &DOCK_TERMINAL, Color::from_rgb8(0x3A, 0x3A, 0x44)),
        heading(tr("연결")),
        item(SettingsTab::Servers, tr("서버"), &PLUG, Color::from_rgb8(0x0A, 0x84, 0xFF)),
        item(SettingsTab::Keys, tr("SSH 키"), &KEY, Color::from_rgb8(0x30, 0xB0, 0x5A)),
        item(SettingsTab::Ai, "AI", &SPARKLES, Color::from_rgb8(0xA8, 0x55, 0xE8)),
    ]
    .spacing(2);
    // A darker pane down the dialog's left, a hairline between it and the page.
    container(list.width(NAV_W)).padding(Padding { top: 6.0, right: 10.0, bottom: 10.0, left: 10.0 }).height(Length::Fill).style(|_| container::Style {
        background: Some(Background::Color(alpha(Color::BLACK, 0.2))),
        border: Border { radius: iced::border::Radius { bottom_left: crate::ui::R_DIALOG - 1.0, ..Default::default() }, ..Default::default() },
        ..Default::default()
    })
    .into()
}

/// The box a page's content sits in: an inset group, as macOS draws a settings form.
fn framed<'a>(content: impl Into<El<'a>>) -> El<'a> {
    container(content)
        .clip(true)
        .style(|_| container::Style {
            background: Some(Background::Color(alpha(Color::WHITE, 0.035))),
            border: Border { color: theme::LINE, width: 1.0, radius: 10.0.into() },
            ..Default::default()
        })
        .into()
}

/// The terminal page's parts as a segmented control, centred over the page.
fn tabs<'a>(current: TermTab) -> El<'a> {
    let tab = |t: TermTab, name: &'a str| -> El<'a> {
        let on = t == current;
        button(container(text(name).size(12.5).font(if on { crate::ui::MEDIUM } else { crate::fonts::SANS }).color(if on { theme::TEXT } else { theme::TEXT_2 })).center_x(Length::Fill))
            .width(92)
            .padding([4, 0])
            .on_press(Message::SettingsTermTab(t))
            .style(move |_, st| button::Style {
                background: Some(Background::Color(if on {
                    alpha(Color::WHITE, 0.16)
                } else if matches!(st, button::Status::Hovered) {
                    alpha(Color::WHITE, 0.05)
                } else {
                    Color::TRANSPARENT
                })),
                border: Border { radius: 6.0.into(), ..Default::default() },
                shadow: if on { iced::Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.3), offset: iced::Vector::new(0.0, 1.0), blur_radius: 3.0 } } else { iced::Shadow::default() },
                ..Default::default()
            })
            .into()
    };
    let segments = container(
        row![
            tab(TermTab::Text, tr("텍스트")),
            tab(TermTab::Colors, tr("색")),
            tab(TermTab::Scrolling, tr("스크롤")),
            tab(TermTab::Compatibility, tr("호환성")),
        ]
        .spacing(2),
    )
    .padding(2)
    .style(|_| container::Style {
        background: Some(Background::Color(alpha(Color::BLACK, 0.25))),
        border: Border { color: theme::LINE, width: 1.0, radius: 8.0.into() },
        ..Default::default()
    });
    container(segments).center_x(Length::Fill).into()
}

/// A page's form, scrolling inside the frame.
fn form<'a>(content: impl Into<El<'a>>) -> El<'a> {
    crate::ui::scroll(container(content).padding(Padding { top: 18.0, right: 22.0, bottom: 18.0, left: 18.0 }).width(Length::Fill)).height(Length::Fill).into()
}

pub fn dialog<'a>(app: &'a CoShell, d: &'a SettingsDraft) -> El<'a> {
    let page: El<'a> = match d.tab {
        SettingsTab::General => framed(form(general(d))),
        SettingsTab::Shortcuts => framed(form(shortcuts())),
        SettingsTab::Terminal => column![tabs(d.term_tab), framed(form(terminal(d)))].spacing(10).into(),
        SettingsTab::Servers => framed(container(crate::ui_servers::manage(app)).padding(14)),
        SettingsTab::Keys => framed(container(crate::ui_keys::page(app)).padding(14)),
        SettingsTab::Ai => match &app.ai.settings {
            Some(s) => framed(container(crate::ui_ai::settings_page(s)).padding(14)),
            None => Space::new().into(),
        },
    };
    // The sidebar runs the dialog's whole height; the page and its buttons sit beside it.
    let mut side = column![container(page).width(Length::Fill).height(Length::Fill)].spacing(12);
    if let Some(e) = &d.error {
        side = side.push(text(e.clone()).size(11.5).color(theme::ERR_TX));
    }
    side = side.push(
        row![
            Space::new().width(Length::Fill),
            btn(tr("취소"), Btn::Ghost, Some(Message::SettingsClose)),
            btn(tr("저장"), Btn::Primary, Some(Message::SettingsSave)),
        ]
        .spacing(8),
    );
    let body = row![nav(d.tab), container(Space::new()).width(1).height(Length::Fill).style(crate::ui::fill(theme::LINE)), container(side).padding(14).width(Length::Fill)].height(PAGE_H + 68.0);
    let card = column![crate::form::title_bar(tr("설정")), body];
    container(card).padding(1).width(DIALOG_W).style(card_style(theme::LINE_2)).into()
}

/// Settings for the whole app.
fn general(d: &SettingsDraft) -> El<'_> {
    let s = &d.s;
    column![
        group(tr("언어"), vec![field(tr("화면 언어"), dropdown(&Language::ALL, s.language, Language::label, Message::SettingsLanguage)), hint(tr("메뉴 막대는 앱을 다시 시작한 뒤 바뀝니다."))]),
        group(
            "AI",
            vec![
                field(tr("AI에 첨부할 출력 줄 수"), number(&d.attach_lines, Message::SettingsAttachLines, Message::SettingsAttachLinesStep)),
                hint(tf("{0}–{1}줄", &[&settings::ATTACH_LINES_MIN, &settings::ATTACH_LINES_MAX])),
            ]
        ),
        group(
            tr("업데이트"),
            vec![
                checkbox(s.update_check).label(tr("업데이트 자동 확인 (10분마다)")).on_toggle(Message::SettingsUpdateCheck).size(14).text_size(12.5).into(),
                muted(tr("GitHub에서 새 버전이 있는지만 확인합니다. 설치는 상태바에서 직접 고릅니다.")),
            ]
        ),
    ]
    .spacing(22)
    .into()
}

/// How terminals look and behave, by tab.
fn terminal(d: &SettingsDraft) -> El<'_> {
    let s = &d.s;
    match d.term_tab {
        TermTab::Text => {
            let size = stepper(
                mono(format!("{}", s.font_size), 12.5, theme::TEXT),
                (s.font_size > settings::FONT_SIZE_MIN).then_some(Message::SettingsFontSize(-1)),
                (s.font_size < settings::FONT_SIZE_MAX).then_some(Message::SettingsFontSize(1)),
            );
            let family = text_input(tr("시스템 고정폭 글꼴"), &s.font_family)
                .on_input(Message::SettingsFontFamily)
                .on_submit(Message::SettingsSave)
                .size(12.5)
                .padding([6, 10])
                .width(FIELD_W)
                .style(input_style);
            column![
                group(
                    tr("글씨 모양"),
                    vec![
                        field(tr("글꼴"), family),
                        hint(tr("설치된 고정폭 글꼴 이름을 입력하세요 (예: JetBrains Mono). 없는 글꼴이면 기본 글꼴로 표시됩니다.")),
                        field(tr("글꼴 크기"), size),
                    ]
                ),
                group(tr("커서"), vec![field(tr("커서 모양"), dropdown(&CursorKind::ALL, s.cursor, CursorKind::label, Message::SettingsCursor))]),
            ]
            .spacing(22)
            .into()
        }
        TermTab::Colors => group(
            tr("색 구성"),
            vec![field(tr("색 구성"), dropdown(&Scheme::ALL, s.scheme, Scheme::label, Message::SettingsScheme)), hint(tr("터미널 영역에만 적용됩니다."))],
        ),
        TermTab::Scrolling => group(
            tr("스크롤"),
            vec![
                field(tr("스크롤백 줄 수"), number(&d.scrollback, Message::SettingsScrollback, Message::SettingsScrollbackStep)),
                hint(tf("{0}–{1}줄", &[&settings::SCROLLBACK_MIN, &settings::SCROLLBACK_MAX])),
            ],
        ),
        TermTab::Compatibility => group(
            tr("키보드"),
            vec![
                checkbox(s.option_as_meta).label(tr("Option 키를 Meta로 사용 (셸에서 단어 단위 이동)")).on_toggle(Message::SettingsOptionMeta).size(14).text_size(12.5).into(),
                muted(tr("끄면 Option 조합으로 특수 문자를 입력합니다.")),
            ],
        ),
    }
}
