//! Sidebar file panel: the active SSH tab's remote folder, transfers and edits.

use iced::widget::{Space, button, column, container, progress_bar, row, text, text_input};
use iced::{Alignment, Background, Border, Color, Length, Padding};

use coshell_ssh::{FileEntry, FileKind};

use crate::app::{CoShell, Message, Tab, TabId};
use crate::context_menu::ContextMenu;
use crate::files::{FileConfirm, FileInput, Files, FilesMsg, Link, Transfer, TransferState};
use crate::i18n::{tf, tr};
use crate::icons::{ARROW_UP, CHEVRON_RIGHT, DOWNLOAD, EXTERNAL_LINK, FILE, FILE_ARCHIVE, FILE_CODE, FILE_CONFIG, FILE_IMAGE, FILE_TERMINAL, FILE_TEXT, FOLDER, FOLDER_PLUS, KEY, PENCIL, REFRESH, UPLOAD, X};
use crate::theme::{self, alpha};
use crate::ui::{Btn, El, R_CARD, R_CONTROL, SEMIBOLD, btn, btn_style, card_style, input_style, muted};

/// Width of an entry's right-click menu.
const MENU_W: f32 = 150.0;
const CONFIRM_W: f32 = 440.0;
/// Text field of the one-line questions (new folder, rename, permissions).
pub const FILE_INPUT: &str = "file-panel-input";
/// Finished transfers and edits scroll beyond this so the folder keeps most of the panel.
const ACTIVITY_MAX_H: f32 = 190.0;
const BYTES_PER_UNIT: f64 = 1024.0;
/// Width of the light round the panel while files are dragged over the window.
const DROP_GLOW_W: f32 = 1.5;

type Icon = &'static std::sync::LazyLock<iced::widget::svg::Handle>;

/// How an entry is drawn: its icon and colour by what it is (a folder, code, settings, a key…).
pub fn file_look(e: &FileEntry) -> (Icon, Color) {
    if e.kind == FileKind::Dir {
        return (&FOLDER, theme::ACCENT_TX);
    }
    let name = e.name.to_lowercase();
    let ext = name.rsplit_once('.').map_or("", |(stem, ext)| if stem.is_empty() { "" } else { ext });
    let keys = ["id_rsa", "id_ed25519", "id_ecdsa", "authorized_keys", "known_hosts"];
    if keys.iter().any(|k| name.starts_with(k)) || matches!(ext, "pem" | "key" | "crt" | "cer" | "pub" | "p12" | "pfx") {
        return (&KEY, theme::AMBER_300);
    }
    match ext {
        "rs" | "py" | "js" | "mjs" | "ts" | "tsx" | "jsx" | "go" | "java" | "kt" | "c" | "h" | "cc" | "cpp" | "hpp" | "rb" | "php" | "swift" | "lua" | "pl" | "cs" | "scala" | "sql" | "html" | "css" | "vue" => (&FILE_CODE, theme::BLUE),
        "sh" | "bash" | "zsh" | "fish" | "command" | "bat" | "ps1" => (&FILE_TERMINAL, theme::GREEN),
        "conf" | "cfg" | "ini" | "yml" | "yaml" | "toml" | "json" | "xml" | "env" | "properties" | "service" | "timer" | "plist" => (&FILE_CONFIG, theme::ORANGE),
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "7z" | "rar" | "deb" | "rpm" | "jar" | "whl" => (&FILE_ARCHIVE, theme::PURPLE),
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "ico" | "bmp" | "tif" | "tiff" | "heic" => (&FILE_IMAGE, theme::TEAL),
        "log" | "txt" | "md" | "out" | "csv" | "tsv" | "err" | "rst" => (&FILE_TEXT, theme::TEXT_2),
        // A shell's own settings: `.bashrc`, `.profile`, `.vimrc`, `.gitconfig`.
        _ if name.starts_with('.') && (name.ends_with("rc") || name.ends_with("profile") || name.ends_with("config") || name.ends_with("_logout")) => (&FILE_CONFIG, theme::ORANGE),
        // Anything its owner may run.
        _ if e.permissions.is_some_and(|m| m & 0o100 != 0) => (&FILE_TERMINAL, theme::GREEN),
        _ => (&FILE, theme::TEXT_3),
    }
}

/// The folder as steps to click: `~` for home, then each folder on the way. Each step is
/// (what it shows, the folder it opens).
pub fn crumbs(path: &str, home: &str) -> Vec<(String, String)> {
    let (mut out, rest, mut at) = match path.strip_prefix(home).filter(|r| !home.is_empty() && home != "/" && (r.is_empty() || r.starts_with('/'))) {
        Some(rest) => (vec![("~".to_string(), home.to_string())], rest, home.to_string()),
        None => (vec![("/".to_string(), "/".to_string())], path, String::new()),
    };
    for part in rest.split('/').filter(|p| !p.is_empty()) {
        at = format!("{at}/{part}");
        out.push((part.to_string(), at.clone()));
    }
    out
}

fn breadcrumb(id: TabId, f: &Files) -> El<'_> {
    let steps = crumbs(&f.path, &f.home);
    let last = steps.len() - 1;
    let mut r = row![].spacing(2).align_y(Alignment::Center);
    for (i, (label, to)) in steps.into_iter().enumerate() {
        if i > 0 {
            r = r.push(crate::icons::tinted(&CHEVRON_RIGHT, 10.0, theme::TEXT_3));
        }
        let here = i == last;
        let step = button(text(label).size(11.5).font(if here { crate::fonts::weight(crate::fonts::MONO, iced::font::Weight::Bold) } else { crate::fonts::MONO }).wrapping(text::Wrapping::None))
            .padding([2, 5])
            .on_press_maybe((!here).then(|| msg(id, FilesMsg::Go(to))))
            .style(move |_, st| {
                let hover = !here && matches!(st, button::Status::Hovered | button::Status::Pressed);
                button::Style {
                    background: hover.then_some(Background::Color(alpha(Color::WHITE, 0.08))),
                    text_color: if here { theme::TEXT } else if hover { theme::ACCENT_TX } else { theme::TEXT_2 },
                    border: Border { radius: 6.0.into(), ..Default::default() },
                    ..Default::default()
                }
            });
        r = r.push(step);
    }
    // A deep folder keeps its end in view: the steps scroll, starting at the right.
    iced::widget::scrollable(r)
        .direction(iced::widget::scrollable::Direction::Horizontal(iced::widget::scrollable::Scrollbar::new().width(0).scroller_width(0)))
        .anchor_right()
        .width(Length::Fill)
        .into()
}

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

/// A `small_btn` with an icon before its label.
fn small_icon_btn<'a>(icon: &'static std::sync::LazyLock<iced::widget::svg::Handle>, label: &'a str, primary: bool, on_press: Option<Message>) -> El<'a> {
    let color = if primary { theme::ACCENT_TX } else { theme::TEXT_2 };
    button(row![crate::icons::tinted(icon, 10.0, color), text(label).size(10.5)].spacing(4).align_y(Alignment::Center))
        .padding([2, 9])
        .on_press_maybe(on_press)
        .style(crate::ui::tint_style(color, primary))
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
        Link::Ready => ready(app, id, f),
    }
}

fn ready<'a>(app: &'a CoShell, id: TabId, f: &'a Files) -> El<'a> {
    let path_bar = row![
        icon_btn(&ARROW_UP, tr("상위 폴더"), (f.path != "/").then(|| msg(id, FilesMsg::Up))),
        breadcrumb(id, f),
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
    // A folder just opened slides in from the side it was entered from, a row at a time.
    let moved = f.moved_since();
    for (i, e) in f.entries.iter().enumerate() {
        let row = entry_row(id, f, e);
        list = list.push(match moved {
            Some((elapsed, side)) => {
                let left = crate::motion::slide(elapsed, i);
                crate::motion::Shift::new(row, 1.0, iced::Vector::new(side * crate::motion::SLIDE_PX * left, 0.0), left < 1.0).into()
            }
            None => row,
        });
    }
    if f.entries.is_empty() {
        list = list.push(centered(muted(if f.loading { tr("불러오는 중…") } else { tr("빈 폴더입니다") })));
    } else {
        list = list.push(container(muted(tr("파일·폴더를 끌어다 놓으면 올라갑니다 · 오른쪽 클릭으로 이름 변경·권한·삭제"))).padding([8, 6]));
    }

    let mut col = column![
        container(head).padding(Padding { top: 0.0, right: 12.0, bottom: 8.0, left: 12.0 }),
        crate::ui::scroll(container(list).padding(Padding { top: 0.0, right: 10.0, bottom: 8.0, left: 10.0 })).height(Length::Fill),
    ];
    if !f.transfers.is_empty() || !f.edits.is_empty() {
        col = col.push(crate::ui::rule(true)).push(container(crate::ui::scroll(container(activity(id, f)).padding([8, 12]))).max_height(ACTIVITY_MAX_H));
    }
    if !app.file_drag {
        return col.into();
    }
    // Files are being dragged over the window: the panel lights up and says where they will go.
    let hint = container(
        row![crate::icons::tinted(&UPLOAD, 13.0, theme::ACCENT_TX), text(tf("놓으면 {0}에 올라갑니다", &[&f.path])).size(11.5).color(theme::TEXT).wrapping(text::Wrapping::None)]
            .spacing(7)
            .align_y(Alignment::Center),
    )
    .padding([7, 12])
    .clip(true)
    .width(Length::Fill)
    .style(|_| container::Style { background: Some(Background::Color(alpha(theme::ACCENT, 0.16))), ..Default::default() });
    let inner = container(column![hint, col]).width(Length::Fill).height(Length::Fill).style(|_| container::Style {
        background: Some(Background::Color(theme::BG_1)),
        border: Border { radius: (R_CARD - DROP_GLOW_W).into(), ..Default::default() },
        ..Default::default()
    });
    let edge = iced::gradient::Linear::new(iced::Radians(2.2)).add_stop(0.0, theme::PURPLE).add_stop(0.5, theme::BLUE).add_stop(1.0, theme::TEAL);
    container(container(inner).padding(DROP_GLOW_W).style(move |_| container::Style {
        background: Some(Background::Gradient(edge.into())),
        border: Border { radius: R_CARD.into(), ..Default::default() },
        shadow: iced::Shadow { color: alpha(theme::BLUE, 0.35), offset: iced::Vector::ZERO, blur_radius: 18.0 },
        ..Default::default()
    }))
    .padding([0, 6])
    .into()
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

/// `a` turned `t` of the way into `b`.
fn mix(a: Color, b: Color, t: f32) -> Color {
    let l = |x: f32, y: f32| x + (y - x) * t;
    Color { r: l(a.r, b.r), g: l(a.g, b.g), b: l(a.b, b.b), a: l(a.a, b.a) }
}

fn entry_row<'a>(id: TabId, f: &'a Files, e: &'a FileEntry) -> El<'a> {
    let folder = e.kind == FileKind::Dir;
    let selected = f.selected.as_deref() == Some(e.name.as_str());
    let menu_open = f.menu.as_deref() == Some(e.name.as_str());
    let (icon, tint) = file_look(e);
    // Hidden entries (`.cache`, `.bashrc`) step back a little.
    let hidden = e.name.starts_with('.');
    let (tint, name_colour) = if hidden { (alpha(tint, 0.6), theme::TEXT_2) } else { (tint, theme::TEXT) };
    // Just uploaded or downloaded: it glows, fading.
    let glow = f.flash_of(&e.name);
    let mut line = row![
        crate::icons::tinted(icon, 14.0, tint),
        container(text(e.name.clone()).size(12).color(name_colour).wrapping(text::Wrapping::None)).width(Length::Fill).clip(true),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    if e.link {
        line = line.push(crate::icons::tinted(&EXTERNAL_LINK, 10.0, theme::TEXT_3));
    }
    if !folder {
        line = line.push(text(human_size(e.size)).size(10).font(crate::fonts::MONO).color(theme::TEXT_3));
    }
    // A folder opens; a file shows what can be done with it.
    let on_press = if folder { FilesMsg::Go(crate::files::join(&f.path, &e.name)) } else { FilesMsg::Select(e.name.clone()) };
    let row_btn = button(line).width(Length::Fill).padding([5, 8]).on_press(msg(id, on_press)).style(move |_, st| {
        let hover = menu_open || matches!(st, button::Status::Hovered | button::Status::Pressed);
        let base = if selected {
            alpha(theme::ACCENT, 0.22)
        } else if hover {
            alpha(Color::WHITE, 0.075)
        } else {
            Color::TRANSPARENT
        };
        button::Style {
            background: Some(Background::Color(if glow > 0.0 { mix(base, alpha(theme::TEAL, 0.32), glow) } else { base })),
            text_color: theme::TEXT,
            border: Border { radius: 8.0.into(), color: alpha(theme::TEAL, 0.8 * glow), width: if glow > 0.0 { 1.0 } else { 0.0 } },
            shadow: iced::Shadow { color: alpha(theme::TEAL, 0.45 * glow), offset: iced::Vector::ZERO, blur_radius: 14.0 * glow },
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
        small_icon_btn(&PENCIL, tr("편집"), false, Some(msg(id, FilesMsg::Edit(e.name.clone())))),
        small_icon_btn(&DOWNLOAD, tr("다운로드"), true, Some(msg(id, FilesMsg::Download(e.name.clone())))),
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
                line = line.push(small_icon_btn(&UPLOAD, tr("서버에 올리기"), true, Some(msg(id, FilesMsg::EditUpload(i)))));
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


#[cfg(test)]
mod look_tests {
    use super::*;

    fn entry(name: &str, kind: FileKind, permissions: u32) -> FileEntry {
        FileEntry { name: name.into(), kind, link: false, size: 0, modified: None, permissions: Some(permissions) }
    }

    #[test]
    fn files_are_told_apart_by_what_they_are() {
        let look = |name: &str, kind, mode| file_look(&entry(name, kind, mode)).1;
        assert_eq!(look("devora-ai", FileKind::Dir, 0o755), theme::ACCENT_TX);
        assert_eq!(look("main.rs", FileKind::File, 0o644), theme::BLUE);
        assert_eq!(look("deploy.sh", FileKind::File, 0o644), theme::GREEN);
        assert_eq!(look("run", FileKind::File, 0o755), theme::GREEN, "the owner may run it");
        assert_eq!(look("nginx.conf", FileKind::File, 0o644), theme::ORANGE);
        assert_eq!(look(".bashrc", FileKind::File, 0o644), theme::ORANGE);
        assert_eq!(look(".bash_logout", FileKind::File, 0o644), theme::ORANGE);
        assert_eq!(look("backup.tar.gz", FileKind::File, 0o644), theme::PURPLE);
        assert_eq!(look("logo.PNG", FileKind::File, 0o644), theme::TEAL);
        assert_eq!(look("syslog.log", FileKind::File, 0o644), theme::TEXT_2);
        assert_eq!(look("id_ed25519.pub", FileKind::File, 0o644), theme::AMBER_300);
        assert_eq!(look("authorized_keys", FileKind::File, 0o600), theme::AMBER_300);
        assert_eq!(look(".viminfo", FileKind::File, 0o600), theme::TEXT_3, "nothing special");
        assert_eq!(look(".sudo_as_admin_successful", FileKind::File, 0o644), theme::TEXT_3);
    }

    #[test]
    fn the_path_is_steps_from_home_or_from_the_root() {
        let steps = |p: &str, home: &str| crumbs(p, home).into_iter().map(|(l, to)| format!("{l}={to}")).collect::<Vec<_>>();
        assert_eq!(steps("/home/hwr", "/home/hwr"), ["~=/home/hwr"]);
        assert_eq!(steps("/home/hwr/devora-ai/src", "/home/hwr"), ["~=/home/hwr", "devora-ai=/home/hwr/devora-ai", "src=/home/hwr/devora-ai/src"]);
        assert_eq!(steps("/etc/nginx", "/home/hwr"), ["/=/", "etc=/etc", "nginx=/etc/nginx"]);
        assert_eq!(steps("/", "/home/hwr"), ["/=/"]);
        assert_eq!(steps("/home/hwr2", "/home/hwr"), ["/=/", "home=/home", "hwr2=/home/hwr2"], "a neighbour of home is not home");
        assert_eq!(steps("/root", "/"), ["/=/", "root=/root"]);
    }
}
