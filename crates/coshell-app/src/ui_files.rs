//! Sidebar file panel: the active SSH tab's remote folder, transfers and edits.

use iced::widget::{Space, button, column, container, progress_bar, row, text, text_input};
use iced::{Alignment, Background, Border, Color, Length, Padding};

use coshell_ssh::{FileEntry, FileKind};

use crate::app::{CoShell, Message, Tab, TabId};
use crate::context_menu::ContextMenu;
use crate::files::{FileConfirm, FileInput, Files, FilesMsg, Link, Transfer, TransferState};
use crate::i18n::{tf, tr};
use crate::icons::{ARROW_UP, DOWNLOAD, FILE, FOLDER, FOLDER_PLUS, REFRESH, UPLOAD, X};
use crate::theme::{self, alpha};
use crate::ui::{Btn, El, R_CARD, R_CONTROL, SEMIBOLD, btn, btn_style, card_style, input_style, mono, muted};

/// Width of an entry's right-click menu.
const MENU_W: f32 = 150.0;
const CONFIRM_W: f32 = 440.0;
/// Text field of the one-line questions (new folder, rename, permissions).
pub const FILE_INPUT: &str = "file-panel-input";
/// Finished transfers and edits scroll beyond this so the folder keeps most of the panel.
const ACTIVITY_MAX_H: f32 = 190.0;
const BYTES_PER_UNIT: f64 = 1024.0;

/// `1.4 MB`, `812 B`.
pub fn human_size(bytes: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= BYTES_PER_UNIT && u + 1 < units.len() {
        v /= BYTES_PER_UNIT;
        u += 1;
    }
    if u == 0 { format!("{bytes} B") } else { format!("{v:.1} {}", units[u]) }
}

fn msg(id: TabId, m: FilesMsg) -> Message {
    Message::Files(id, m)
}

fn icon_btn<'a>(icon: &'static std::sync::LazyLock<iced::widget::svg::Handle>, label: &'a str, on_press: Option<Message>) -> El<'a> {
    let color = if on_press.is_some() { theme::TEXT_2 } else { theme::TEXT_3 };
    let b = button(container(crate::icons::tinted(icon, 14.0, color)).center(26)).padding(0).on_press_maybe(on_press).style(|_, st| button::Style {
        background: Some(Background::Color(if matches!(st, button::Status::Hovered) { theme::BG_3 } else { theme::BG_2 })),
        border: Border { color: theme::LINE, width: 1.0, radius: R_CONTROL.into() },
        ..Default::default()
    });
    crate::ui::tip(b, label, iced::widget::tooltip::Position::Bottom)
}

fn small_btn<'a>(label: &'a str, primary: bool, on_press: Option<Message>) -> El<'a> {
    button(text(label).size(10.5))
        .padding([2, 9])
        .on_press_maybe(on_press)
        .style(crate::ui::tint_style(if primary { theme::ACCENT_TX } else { theme::TEXT_2 }, primary))
        .into()
}

fn centered<'a>(content: impl Into<El<'a>>) -> El<'a> {
    container(content).center_x(Length::Fill).padding([32, 14]).into()
}

pub fn panel(app: &CoShell) -> El<'_> {
    let Some(tab) = app.active_tab().filter(|_| !app.host_view) else {
        return centered(muted(tr("SSH 서버에 접속하면 파일을 볼 수 있습니다")));
    };
    if tab.is_local() {
        return centered(muted(tr("로컬 셸에는 파일 패널이 없습니다. SSH 탭에서 사용하세요.")));
    }
    let f = &tab.files;
    let id = tab.id;
    match &f.link {
        Link::Idle if tab.conn.is_none() => centered(muted(tr("서버에 연결되면 파일이 표시됩니다"))),
        Link::Idle | Link::Opening => centered(muted(tr("파일 목록을 여는 중…"))),
        Link::Unavailable(e) => centered(
            column![text(e.clone()).size(12).color(theme::ERR_TX), btn(tr("다시 시도"), Btn::Plain, Some(msg(id, FilesMsg::Retry)))]
                .spacing(10)
                .align_x(Alignment::Center),
        ),
        Link::Ready => ready(id, f),
    }
}

fn ready(id: TabId, f: &Files) -> El<'_> {
    let path_bar = row![
        icon_btn(&ARROW_UP, tr("상위 폴더"), (f.path != "/").then(|| msg(id, FilesMsg::Up))),
        container(mono(f.path.clone(), 11.5, theme::TEXT)).width(Length::Fill).clip(true),
        icon_btn(&REFRESH, tr("새로 고침"), Some(msg(id, FilesMsg::Refresh))),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    let actions = row![
        icon_btn(&UPLOAD, tr("파일 올리기"), Some(msg(id, FilesMsg::Upload))),
        icon_btn(&FOLDER_PLUS, tr("새 폴더"), Some(msg(id, FilesMsg::NewFolder))),
        Space::new().width(Length::Fill),
        follow_toggle(id, f.follow),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    let mut head = column![path_bar, actions].spacing(8);
    if let Some(input) = &f.input {
        head = head.push(input_row(id, input));
    }
    if let Some(e) = &f.error {
        head = head.push(text(e.clone()).size(11).color(theme::ERR_TX));
    }

    let mut list = column![].spacing(1);
    for e in &f.entries {
        list = list.push(entry_row(id, f, e));
    }
    if f.entries.is_empty() {
        list = list.push(centered(muted(if f.loading { tr("불러오는 중…") } else { tr("빈 폴더입니다") })));
    } else {
        list = list.push(container(muted(tr("파일을 끌어다 놓으면 올라갑니다 · 오른쪽 클릭으로 이름 변경·권한·삭제"))).padding([8, 6]));
    }

    let mut col = column![
        container(head).padding(Padding { top: 0.0, right: 12.0, bottom: 8.0, left: 12.0 }),
        crate::ui::scroll(container(list).padding(Padding { top: 0.0, right: 10.0, bottom: 8.0, left: 10.0 })).height(Length::Fill),
    ];
    if !f.transfers.is_empty() || !f.edits.is_empty() {
        col = col.push(crate::ui::rule(true)).push(container(crate::ui::scroll(container(activity(id, f)).padding([8, 12]))).max_height(ACTIVITY_MAX_H));
    }
    col.into()
}

/// "Follow the terminal": the panel shows the folder the shell is in.
fn follow_toggle<'a>(id: TabId, on: bool) -> El<'a> {
    let b = button(text(tr("터미널 따라가기")).size(10.5)).padding([3, 9]).on_press(msg(id, FilesMsg::Follow(!on))).style(crate::ui::tint_style(
        if on { theme::ACCENT_TX } else { theme::TEXT_2 },
        on,
    ));
    crate::ui::tip(b, tr("터미널에서 폴더를 옮기면 이 목록도 따라갑니다"), iced::widget::tooltip::Position::Bottom)
}

fn input_row<'a>(id: TabId, input: &'a FileInput) -> El<'a> {
    let (label, placeholder, value) = match input {
        FileInput::NewFolder(name) => (tr("새 폴더").to_string(), tr("폴더 이름"), name.as_str()),
        FileInput::Rename { from, to } => (tf("이름 변경 · {0}", &[from]), tr("새 이름"), to.as_str()),
        FileInput::Chmod { name, mode } => (tf("권한 · {0}", &[name]), "644", mode.as_str()),
    };
    column![
        container(text(label).size(11).color(theme::TEXT_2)).clip(true),
        row![
            text_input(placeholder, value)
                .id(FILE_INPUT)
                .on_input(move |v| msg(id, FilesMsg::InputChanged(v)))
                .on_submit(msg(id, FilesMsg::InputSubmit))
                .font(crate::fonts::MONO)
                .size(12)
                .padding([5, 8])
                .style(input_style),
            small_btn(tr("확인"), true, Some(msg(id, FilesMsg::InputSubmit))),
            small_btn(tr("취소"), false, Some(msg(id, FilesMsg::InputCancel))),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    ]
    .spacing(4)
    .into()
}

fn entry_row<'a>(id: TabId, f: &'a Files, e: &'a FileEntry) -> El<'a> {
    let folder = e.kind == FileKind::Dir;
    let selected = f.selected.as_deref() == Some(e.name.as_str());
    let menu_open = f.menu.as_deref() == Some(e.name.as_str());
    let (icon, tint) = if folder { (&FOLDER, theme::ACCENT_TX) } else { (&FILE, theme::TEXT_3) };
    let mut line = row![
        crate::icons::tinted(icon, 14.0, tint),
        container(text(e.name.clone()).size(12).color(theme::TEXT).wrapping(text::Wrapping::None)).width(Length::Fill).clip(true),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    if e.link {
        line = line.push(text("↗").size(10).color(theme::TEXT_3));
    }
    if !folder {
        line = line.push(text(human_size(e.size)).size(10).font(crate::fonts::MONO).color(theme::TEXT_3));
    }
    // A folder opens; a file shows what can be done with it.
    let on_press = if folder { FilesMsg::Go(crate::files::join(&f.path, &e.name)) } else { FilesMsg::Select(e.name.clone()) };
    let row_btn = button(line).width(Length::Fill).padding([5, 8]).on_press(msg(id, on_press)).style(move |_, st| {
        let hover = menu_open || matches!(st, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(if selected {
                alpha(theme::ACCENT, 0.22)
            } else if hover {
                alpha(Color::WHITE, 0.075)
            } else {
                Color::TRANSPARENT
            })),
            text_color: theme::TEXT,
            border: Border { radius: 8.0.into(), ..Default::default() },
            ..Default::default()
        }
    });
    let with_menu: El<'a> = ContextMenu::new(row_btn, msg(id, FilesMsg::Menu(Some(e.name.clone()))), msg(id, FilesMsg::Menu(None)))
        .menu(menu_open.then(|| entry_menu(id, f, e)))
        .into();
    if !selected || folder {
        return with_menu;
    }
    let actions = row![
        Space::new().width(Length::Fill),
        small_btn(tr("편집"), false, Some(msg(id, FilesMsg::Edit(e.name.clone())))),
        small_btn(tr("다운로드"), true, Some(msg(id, FilesMsg::Download(e.name.clone())))),
    ]
    .spacing(6);
    column![with_menu, container(actions).padding(Padding { top: 2.0, right: 8.0, bottom: 6.0, left: 8.0 })].into()
}

fn entry_menu<'a>(id: TabId, f: &'a Files, e: &'a FileEntry) -> El<'a> {
    let item = |label: &'a str, color: Color, m: FilesMsg| {
        button(text(label).size(12.5)).width(Length::Fill).padding([5, 10]).on_press(msg(id, m)).style(move |_, st| {
            let hover = matches!(st, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: hover.then_some(Background::Color(theme::ACCENT)),
                text_color: if hover { theme::ON_ACCENT } else { color },
                border: Border { radius: 6.0.into(), ..Default::default() },
                ..Default::default()
            }
        })
    };
    let name = e.name.clone();
    let mut col = column![].spacing(2);
    if e.kind == FileKind::Dir {
        col = col.push(item(tr("열기"), theme::TEXT, FilesMsg::Go(crate::files::join(&f.path, &name))));
    } else {
        col = col.push(item(tr("다운로드"), theme::TEXT, FilesMsg::Download(name.clone())));
        col = col.push(item(tr("편집"), theme::TEXT, FilesMsg::Edit(name.clone())));
    }
    col = col
        .push(item(tr("이름 변경"), theme::TEXT, FilesMsg::RenameAsk(name.clone())))
        .push(item(tr("권한 변경"), theme::TEXT, FilesMsg::ChmodAsk(name.clone())))
        .push(item(tr("삭제"), theme::ERR_TX, FilesMsg::DeleteAsk(name)));
    container(col)
        .padding(4)
        .width(MENU_W)
        .style(|_| container::Style {
            background: Some(Background::Color(theme::BG_2)),
            border: Border { color: theme::LINE_2, width: 1.0, radius: R_CONTROL.into() },
            shadow: iced::Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.45), offset: iced::Vector::new(0.0, 8.0), blur_radius: 24.0 },
            ..Default::default()
        })
        .into()
}

/// Transfers (with progress) and files being edited locally.
fn activity(id: TabId, f: &Files) -> El<'_> {
    let mut col = column![].spacing(8);
    if !f.transfers.is_empty() {
        let finished = f.transfers.iter().any(|t| t.state != TransferState::Running);
        col = col.push(
            row![
                text(tr("전송")).size(11).font(SEMIBOLD).color(theme::TEXT_3),
                Space::new().width(Length::Fill),
                if finished { small_btn(tr("지우기"), false, Some(msg(id, FilesMsg::TransfersClear))) } else { Space::new().into() },
            ]
            .align_y(Alignment::Center),
        );
        // Newest first: what was just started is what is being watched.
        for t in f.transfers.iter().rev() {
            col = col.push(transfer_row(id, t));
        }
    }
    if !f.edits.is_empty() {
        col = col.push(text(tr("편집 중")).size(11).font(SEMIBOLD).color(theme::TEXT_3));
        for (i, e) in f.edits.iter().enumerate() {
            let mut line = row![container(text(e.name().to_string()).size(11.5).color(theme::TEXT).wrapping(text::Wrapping::None)).width(Length::Fill).clip(true)]
                .spacing(6)
                .align_y(Alignment::Center);
            if e.changed {
                line = line.push(small_btn(tr("서버에 올리기"), true, Some(msg(id, FilesMsg::EditUpload(i)))));
            } else {
                line = line.push(text(tr("저장하면 표시됩니다")).size(10).color(theme::TEXT_3));
            }
            line = line.push(
                button(crate::icons::tinted(&X, 11.0, theme::TEXT_3)).padding(2).style(btn_style(Btn::Ghost)).on_press(msg(id, FilesMsg::EditClose(i))),
            );
            col = col.push(line);
        }
    }
    col.into()
}

fn transfer_row(id: TabId, t: &Transfer) -> El<'_> {
    let (status, color): (String, Color) = match &t.state {
        TransferState::Running => (format!("{:.0}%", t.fraction() * 100.0), theme::ACCENT_TX),
        TransferState::Done => (tr("완료").into(), theme::OK),
        TransferState::Cancelled => (tr("취소됨").into(), theme::TEXT_3),
        TransferState::Failed(_) => (tr("실패").into(), theme::ERR_TX),
    };
    let mut line = row![
        crate::icons::tinted(if t.upload { &UPLOAD } else { &DOWNLOAD }, 12.0, color),
        container(text(t.name.clone()).size(11.5).color(theme::TEXT).wrapping(text::Wrapping::None)).width(Length::Fill).clip(true),
        text(status).size(10.5).font(crate::fonts::MONO).color(color),
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    if t.state == TransferState::Running {
        line = line.push(crate::ui::tip(
            button(crate::icons::tinted(&X, 11.0, theme::TEXT_3)).padding(2).style(btn_style(Btn::Ghost)).on_press(msg(id, FilesMsg::TransferCancel(t.id))),
            tr("전송 취소"),
            iced::widget::tooltip::Position::Left,
        ));
    }
    let mut col = column![line].spacing(4);
    match &t.state {
        TransferState::Running => {
            col = col.push(progress_bar(0.0..=1.0, t.fraction()).girth(3).style(|_| progress_bar::Style {
                background: Background::Color(alpha(Color::WHITE, 0.08)),
                bar: Background::Color(theme::ACCENT),
                border: Border { radius: 2.0.into(), ..Default::default() },
            }));
            col = col.push(muted(format!("{} / {}", human_size(t.progress.done()), human_size(t.progress.total()))));
        }
        TransferState::Failed(e) => col = col.push(text(e.clone()).size(10.5).color(theme::ERR_TX)),
        _ => {}
    }
    container(col)
        .padding(8)
        .width(Length::Fill)
        .style(|_| container::Style {
            background: Some(Background::Color(theme::CARD)),
            border: Border { color: theme::LINE, width: 1.0, radius: R_CARD.into() },
            ..Default::default()
        })
        .into()
}

/// "Delete?" / "Replace?" for the file panel: both cannot be undone.
pub fn confirm_dialog<'a>(tab: &'a Tab, c: &'a FileConfirm) -> El<'a> {
    let id = tab.id;
    let (title, detail, action): (String, String, &str) = match c {
        FileConfirm::Delete { name, folder: true } => {
            (tf("{0} 폴더를 삭제할까요?", &[name]), tr("폴더 안의 파일과 하위 폴더가 모두 서버에서 지워집니다. 되돌릴 수 없습니다.").into(), tr("삭제"))
        }
        FileConfirm::Delete { name, folder: false } => (tf("{0} 파일을 삭제할까요?", &[name]), tr("서버에서 지워지며 되돌릴 수 없습니다.").into(), tr("삭제")),
        FileConfirm::Overwrite(paths) => {
            let names: Vec<String> = paths
                .iter()
                .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .filter(|n| tab.files.entry(n).is_some())
                .collect();
            (tr("같은 이름의 파일을 덮어쓸까요?").into(), tf("서버의 기존 파일이 바뀝니다: {0}", &[&names.join(", ")]), tr("덮어쓰기"))
        }
    };
    let col = column![
        text(title).size(14),
        muted(tf("{0} · {1}", &[&tab.display_name(), &tab.files.path])),
        text(detail).size(12).color(theme::TEXT_2),
        Space::new().height(4),
        row![
            Space::new().width(Length::Fill),
            btn(tr("취소"), Btn::Ghost, Some(msg(id, FilesMsg::ConfirmNo))),
            btn(action, Btn::Danger, Some(msg(id, FilesMsg::ConfirmYes))),
        ]
        .spacing(8),
    ]
    .spacing(8);
    let border = crate::ui_hosts::env_color(tab.env).map_or(theme::LINE_2, |c| alpha(c, 0.6));
    container(col).padding(18).width(CONFIRM_W).style(card_style(border)).into()
}
