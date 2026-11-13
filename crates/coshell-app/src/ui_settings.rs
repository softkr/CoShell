//! Settings dialog (⌘,): terminal look, behaviour and the shortcut reference.

use crate::i18n::{tf, tr};
use iced::widget::{Space, button, checkbox, column, container, row, scrollable, text, text_input};
use iced::{Alignment, Background, Border, Color, Font, Length};

use crate::app::{Message, SettingsDraft};
use crate::settings::{self, CursorKind, Language, Scheme};
use crate::theme::{self, alpha};
use crate::ui::{Btn, El, btn, card_style, input_style, mono, muted};

const DIALOG_W: f32 = 600.0;
/// The form scrolls beyond this, so the title and the buttons always stay visible.
const FORM_MAX_H: f32 = 460.0;
const NUMBER_W: f32 = 120.0;
const SHORTCUT_KEYS_W: f32 = 108.0;

/// (keys, what they do). macOS glyphs; ⌘ is Ctrl on other systems.
const SHORTCUTS: [(&str, &str); 15] = [
    ("⌘K", "서버 검색 · 명령"),
    ("⌘O", "서버 목록"),
    ("⌘T", "새 로컬 셸"),
    ("⌘W", "탭 닫기"),
    ("⌘1–9", "탭 선택"),
    ("⌘⇧[ / ⌘⇧]", "이전 / 다음 탭"),
    ("⌘⇧← / ⌘⇧→", "탭 순서 변경"),
    ("⌘L", "AI 패널"),
    ("⌘F", "터미널 출력 검색"),
    ("⌘G / ⌘⇧G", "이전 / 다음 결과"),
    ("⌘C / ⌘V", "복사 / 붙여넣기"),
    ("⌘ 클릭", "링크 열기"),
    ("⌘+ / ⌘−", "글꼴 크게 / 작게"),
    ("⌘0", "글꼴 크기 초기화"),
    ("⌘,", "설정"),
];

fn label<'a>(s: &'a str) -> El<'a> {
    text(s).size(11.5).color(theme::TEXT_2).into()
}

fn section<'a>(title: &'a str) -> El<'a> {
    text(title).size(12.5).font(Font { weight: iced::font::Weight::Semibold, ..Font::DEFAULT }).into()
}

/// One-of-many buttons.
fn segment<'a, T: Copy + PartialEq + 'a>(all: &[T], current: T, name: fn(T) -> &'static str, msg: fn(T) -> Message) -> El<'a> {
    let buttons = all.iter().map(|&v| {
        let on = v == current;
        button(text(name(v)).size(12).color(if on { theme::TEXT } else { theme::TEXT_2 }))
            .padding([5, 12])
            .on_press(msg(v))
            .style(move |_, _| button::Style {
                background: Some(Background::Color(if on { theme::BG_3 } else { Color::TRANSPARENT })),
                text_color: theme::TEXT,
                border: Border { color: if on { alpha(theme::ACCENT, 0.7) } else { theme::LINE_2 }, width: 1.0, radius: 6.0.into() },
                ..Default::default()
            })
            .into()
    });
    row(buttons.collect::<Vec<El<'a>>>()).spacing(6).into()
}

fn number<'a>(value: &'a str, on_input: fn(String) -> Message) -> El<'a> {
    text_input("", value)
        .on_input(on_input)
        .on_submit(Message::SettingsSave)
        .font(Font::MONOSPACE)
        .size(12)
        .padding([6, 10])
        .width(NUMBER_W)
        .style(input_style)
        .into()
}

fn shortcuts<'a>() -> El<'a> {
    let cell = |(keys, what): (&'a str, &'a str)| -> El<'a> {
        row![container(mono(tr(keys), 11.5, theme::TEXT)).width(SHORTCUT_KEYS_W), text(tr(what)).size(11.5).color(theme::TEXT_2)]
            .align_y(Alignment::Center)
            .width(Length::Fill)
            .into()
    };
    let rows = SHORTCUTS.chunks(2).map(|pair| {
        let right: El<'a> = pair.get(1).map_or_else(|| Space::new().width(Length::Fill).into(), |p| cell(*p));
        row![cell(pair[0]), right].spacing(12).into()
    });
    column(rows.collect::<Vec<El<'a>>>()).spacing(5).into()
}

pub fn dialog(d: &SettingsDraft) -> El<'_> {
    let s = &d.s;
    let font_size = row![
        btn("−", Btn::Plain, (s.font_size > settings::FONT_SIZE_MIN).then_some(Message::SettingsFontSize(-1))),
        container(mono(format!("{} px", s.font_size), 12.0, theme::TEXT)).width(56).center_x(56),
        btn("+", Btn::Plain, (s.font_size < settings::FONT_SIZE_MAX).then_some(Message::SettingsFontSize(1))),
    ]
    .spacing(6)
    .align_y(Alignment::Center);

    let form = column![
        section(tr("언어")),
        segment(&Language::ALL, s.language, Language::label, Message::SettingsLanguage),
        muted(tr("메뉴 막대는 앱을 다시 시작한 뒤 바뀝니다.")),
        Space::new().height(6),
        section(tr("터미널 모양")),
        row![
            column![label(tr("글꼴 크기")), font_size].spacing(4),
            column![
                label(tr("글꼴")),
                text_input(tr("시스템 고정폭 글꼴"), &s.font_family)
                    .on_input(Message::SettingsFontFamily)
                    .on_submit(Message::SettingsSave)
                    .size(12.5)
                    .padding([6, 10])
                    .style(input_style),
            ]
            .spacing(4)
            .width(Length::Fill),
        ]
        .spacing(18),
        muted(tr("설치된 고정폭 글꼴 이름을 입력하세요 (예: JetBrains Mono). 없는 글꼴이면 기본 글꼴로 표시됩니다.")),
        label(tr("색 구성")),
        segment(&Scheme::ALL, s.scheme, Scheme::label, Message::SettingsScheme),
        muted(tr("터미널 영역에만 적용됩니다.")),
        label(tr("커서 모양")),
        segment(&CursorKind::ALL, s.cursor, CursorKind::label, Message::SettingsCursor),
        Space::new().height(6),
        section(tr("동작")),
        row![
            column![label(tr("스크롤백 줄 수")), number(&d.scrollback, Message::SettingsScrollback)].spacing(4),
            column![label(tr("AI에 첨부할 출력 줄 수")), number(&d.attach_lines, Message::SettingsAttachLines)].spacing(4),
        ]
        .spacing(18),
        muted(tf("스크롤백 {0}–{1} · 첨부 {2}–{3}", &[&settings::SCROLLBACK_MIN, &settings::SCROLLBACK_MAX, &settings::ATTACH_LINES_MIN, &settings::ATTACH_LINES_MAX])),
        checkbox(s.option_as_meta).label(tr("Option 키를 Meta로 사용 (셸에서 단어 단위 이동)")).on_toggle(Message::SettingsOptionMeta).size(14).text_size(12),
        muted(tr("끄면 Option 조합으로 특수 문자를 입력합니다.")),
        Space::new().height(6),
        section(tr("단축키")),
        shortcuts(),
    ]
    .spacing(6);

    let mut col = column![text(tr("설정")).size(14), container(scrollable(container(form).padding([0, 12]))).max_height(FORM_MAX_H)].spacing(12);
    if let Some(e) = &d.error {
        col = col.push(text(e.clone()).size(11.5).color(theme::ERR_TX));
    }
    col = col.push(
        row![
            Space::new().width(Length::Fill),
            btn(tr("취소"), Btn::Ghost, Some(Message::SettingsClose)),
            btn(tr("저장"), Btn::Primary, Some(Message::SettingsSave)),
        ]
        .spacing(8),
    );
    container(col).padding(18).width(DIALOG_W).style(card_style(theme::LINE_2)).into()
}
