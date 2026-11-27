//! Settings dialog (⌘,): a sidebar of pages (general, shortcuts, terminal, servers, SSH keys, AI),
//! each a form of right-aligned labels beside their controls; the terminal page has tabs.

use crate::i18n::{tf, tr};
use iced::widget::{Space, button, checkbox, column, container, row, text, text_input};
use iced::{Background, Border, Color, Length, Padding};

use crate::app::{CoShell, Message, SettingsDraft, SettingsTab, TermTab};
use crate::settings::{self, CursorKind, Language, Scheme};
use crate::icons::{MINUS, PLUS};
use crate::theme::{self, alpha};
use crate::form::{group, section};
use crate::ui::{Btn, El, btn, card_style, input_style, mono, muted};

const DIALOG_W: f32 = 880.0;
/// Pages scroll inside this, so the title and the buttons always stay visible.
const PAGE_H: f32 = 500.0;
const NAV_W: f32 = 150.0;
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

/// The sidebar: groups of pages under bold headings; the page shown is filled with the accent.
fn nav<'a>(current: SettingsTab) -> El<'a> {
    let item = |tab: SettingsTab, name: &'a str| -> El<'a> {
        let on = tab == current;
        button(text(name).size(12.5).color(if on { theme::ON_ACCENT } else { theme::TEXT }))
            .width(Length::Fill)
            .padding([8, 14])
            .on_press(Message::SettingsTab(tab))
            .style(move |_, status| button::Style {
                background: Some(Background::Color(if on {
                    theme::ACCENT
                } else if matches!(status, button::Status::Hovered) {
                    theme::BG_3
                } else {
                    Color::TRANSPARENT
                })),
                text_color: theme::TEXT,
                ..Default::default()
            })
            .into()
    };
    let heading = |name: &'a str| -> El<'a> { container(section(name)).padding(Padding { top: 12.0, right: 14.0, bottom: 4.0, left: 14.0 }).into() };
    let list = column![
        heading(tr("전체")),
        item(SettingsTab::General, tr("일반")),
        item(SettingsTab::Shortcuts, tr("바로 가기")),
        item(SettingsTab::Terminal, tr("터미널")),
        heading(tr("연결")),
        item(SettingsTab::Servers, tr("서버")),
        item(SettingsTab::Keys, tr("SSH 키")),
        item(SettingsTab::Ai, "AI"),
    ];
    framed(list.width(NAV_W).height(Length::Fill))
}

/// A sunken frame around the sidebar and the page.
fn framed<'a>(content: impl Into<El<'a>>) -> El<'a> {
    container(content)
        .clip(true)
        .style(|_| container::Style {
            background: Some(Background::Color(theme::BG_1)),
            border: Border { color: theme::LINE_2, width: 1.0, radius: 6.0.into() },
            ..Default::default()
        })
        .into()
}

/// Tabs along the top of a page, joined to it.
fn tabs<'a>(current: TermTab) -> El<'a> {
    let tab = |t: TermTab, name: &'a str| -> El<'a> {
        let on = t == current;
        button(text(name).size(12.5).color(if on { theme::TEXT } else { theme::TEXT_2 }))
            .padding([7, 16])
            .on_press(Message::SettingsTermTab(t))
            .style(move |_, st| button::Style {
                background: Some(Background::Color(if on {
                    theme::BG_2
                } else if matches!(st, button::Status::Hovered) {
                    alpha(theme::BG_3, 0.6)
                } else {
                    Color::TRANSPARENT
                })),
                border: Border { color: if on { theme::LINE_2 } else { Color::TRANSPARENT }, width: 1.0, radius: iced::border::Radius { top_left: 6.0, top_right: 6.0, ..Default::default() } },
                ..Default::default()
            })
            .into()
    };
    row![
        tab(TermTab::Text, tr("텍스트")),
        tab(TermTab::Colors, tr("색")),
        tab(TermTab::Scrolling, tr("스크롤")),
        tab(TermTab::Compatibility, tr("호환성")),
    ]
    .into()
}

/// A page's form, scrolling inside the frame.
fn form<'a>(content: impl Into<El<'a>>) -> El<'a> {
    crate::ui::scroll(container(content).padding(Padding { top: 18.0, right: 22.0, bottom: 18.0, left: 18.0 }).width(Length::Fill)).height(Length::Fill).into()
}

pub fn dialog<'a>(app: &'a CoShell, d: &'a SettingsDraft) -> El<'a> {
    let page: El<'a> = match d.tab {
        SettingsTab::General => framed(form(general(d))),
        SettingsTab::Shortcuts => framed(form(shortcuts())),
        SettingsTab::Terminal => column![tabs(d.term_tab), framed(form(terminal(d)))].into(),
        SettingsTab::Servers => framed(container(crate::ui_servers::manage(app)).padding(14)),
        SettingsTab::Keys => framed(container(crate::ui_keys::page(app)).padding(14)),
        SettingsTab::Ai => match &app.ai.settings {
            Some(s) => framed(container(crate::ui_ai::settings_page(s)).padding(14)),
            None => Space::new().into(),
        },
    };
    let mut body = column![row![nav(d.tab), container(page).width(Length::Fill)].spacing(12).height(PAGE_H)].spacing(12);
    if let Some(e) = &d.error {
        body = body.push(text(e.clone()).size(11.5).color(theme::ERR_TX));
    }
    body = body.push(
        row![
            Space::new().width(Length::Fill),
            btn(tr("취소"), Btn::Ghost, Some(Message::SettingsClose)),
            btn(tr("저장"), Btn::Primary, Some(Message::SettingsSave)),
        ]
        .spacing(8),
    );
    let card = column![crate::form::title_bar(tr("설정")), container(body).padding(14)];
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
