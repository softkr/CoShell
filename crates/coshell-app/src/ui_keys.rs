//! SSH keys: the settings page, the import dialog and the server editor's key picker.

use iced::widget::{Space, column, container, pick_list, row, scrollable, text, text_editor, text_input};
use iced::{Alignment, Background, Border, Font, Length, Padding};

use crate::app::{CoShell, HostField, KeyImport, Message};
use crate::i18n::{tf, tr};
use crate::icons::{COPY, KEY, TRASH};
use crate::keys::{SshKey, display_path, expand};
use crate::theme::{self, alpha};
use crate::ui::{Btn, El, R_CARD, R_CONTROL, btn, card_style, icon_btn, input_style, muted};

const CARD_W: f32 = 300.0;
const IMPORT_W: f32 = 560.0;
const PRIVATE_H: f32 = 130.0;

fn tag<'a>(label: String, color: iced::Color) -> El<'a> {
    container(text(label).size(10.5).font(Font::MONOSPACE).color(color))
        .padding([1, 6])
        .style(move |_| container::Style {
            background: Some(Background::Color(alpha(color, 0.12))),
            border: Border { radius: 5.0.into(), ..Default::default() },
            ..Default::default()
        })
        .into()
}

fn key_card<'a>(app: &'a CoShell, k: &'a SshKey) -> El<'a> {
    let mut actions = row![].spacing(2).align_y(Alignment::Center);
    if let Some(p) = &k.info.public {
        actions = actions.push(icon_btn(&COPY, tr("공개 키 복사"), Btn::Ghost, Some(Message::KeyCopyPublic(p.clone()))));
    }
    if k.managed {
        actions = actions.push(if app.key_delete.as_deref() == Some(k.path.as_path()) {
            btn(tr("정말 삭제"), Btn::Danger, Some(Message::KeyDelete(k.path.clone())))
        } else {
            icon_btn(&TRASH, tr("삭제"), Btn::Ghost, Some(Message::KeyDeleteAsk(k.path.clone())))
        });
    }
    let head = row![
        crate::icons::tinted(&KEY, 15.0, theme::TEXT_2),
        text(k.name.clone()).size(13.5).color(theme::TEXT).wrapping(text::Wrapping::None),
        Space::new().width(Length::Fill),
        actions,
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    let rule = container(Space::new()).height(1).width(Length::Fill).style(|_| container::Style {
        background: Some(Background::Color(theme::LINE_2)),
        ..Default::default()
    });
    let mut tags = row![tag(k.info.algorithm.clone(), theme::ACCENT)].spacing(6).align_y(Alignment::Center);
    if k.info.encrypted {
        tags = tags.push(tag(tr("암호").into(), theme::WARN));
    }
    tags = tags.push(Space::new().width(Length::Fill)).push(muted(if k.managed { "CoShell".to_string() } else { "~/.ssh".into() }));
    let mut body = column![head, rule, tags].spacing(10);
    if let Some(n) = &k.note {
        body = body.push(text(n.clone()).size(11.5).color(theme::TEXT_2));
    }
    container(body)
        .width(CARD_W)
        .padding([12, 14])
        .style(|_| container::Style {
            background: Some(Background::Color(theme::BG_2)),
            border: Border { color: theme::LINE, width: 1.0, radius: R_CARD.into() },
            ..Default::default()
        })
        .into()
}

/// Settings › SSH keys.
pub fn page(app: &CoShell) -> El<'_> {
    let header = row![
        text(tr("SSH 키")).size(13),
        muted(tf("{0}개", &[&app.keys.len()])),
        Space::new().width(Length::Fill),
        btn(tr("+ 키 가져오기"), Btn::Primary, Some(Message::KeyImportOpen)),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    let body: El<'_> = if app.keys.is_empty() {
        container(
            column![
                text(tr("저장된 키가 없습니다")).size(13),
                muted(tr("‘+ 키 가져오기’로 서버 접속에 쓰는 개인 키를 추가하세요.")),
            ]
            .spacing(6)
            .align_x(Alignment::Center),
        )
        .center_x(Length::Fill)
        .padding(40)
        .into()
    } else {
        let cards: Vec<El<'_>> = app.keys.iter().map(|k| key_card(app, k)).collect();
        scrollable(container(row(cards).spacing(12).wrap().vertical_spacing(12)).padding(Padding::ZERO.right(12.0)))
            .height(Length::Fill)
            .into()
    };
    column![header, muted(tr("~/.ssh 의 키와 CoShell에 저장한 키입니다. 서버 편집 › 고급 설정에서 골라 쓸 수 있습니다.")), body]
        .spacing(10)
        .height(Length::Fill)
        .into()
}

fn label<'a>(s: &'a str) -> El<'a> {
    text(s).size(11.5).color(theme::TEXT_2).into()
}

/// "Import key": name, private key (paste or file), derived public key, passphrase, note.
pub fn import_dialog(k: &KeyImport, composing: bool) -> El<'_> {
    let private = text_editor(&k.private)
        .placeholder(if composing { "" } else { tr("-----BEGIN OPENSSH PRIVATE KEY----- 로 시작하는 내용을 붙여 넣거나 파일을 선택하세요") })
        .on_action(Message::KeyImportPrivate)
        .font(Font::MONOSPACE)
        .size(11.5)
        .height(PRIVATE_H)
        .padding(10)
        .style(|_, st| text_editor::Style {
            background: Background::Color(theme::BG_0),
            border: Border {
                color: if matches!(st, text_editor::Status::Focused { .. }) { alpha(theme::ACCENT, 0.7) } else { theme::LINE_2 },
                width: 1.0,
                radius: R_CONTROL.into(),
            },
            placeholder: theme::TEXT_3,
            value: theme::TEXT,
            selection: alpha(theme::ACCENT, 0.35),
        });
    let (status, public): (El<'_>, El<'_>) = match &k.info {
        Some(Ok(i)) => {
            let mut s = tf("✓ {0} 키", &[&i.algorithm]);
            if i.encrypted {
                s.push_str(tr(" · 암호로 보호됨"));
            }
            (
                text(s).size(11.5).color(theme::OK).into(),
                match &i.public {
                    Some(p) => container(text(p.clone()).size(11).font(Font::MONOSPACE).color(theme::TEXT_2))
                        .padding([8, 10])
                        .width(Length::Fill)
                        .style(|_| container::Style {
                            background: Some(Background::Color(theme::BG_2)),
                            border: Border { radius: R_CONTROL.into(), ..Default::default() },
                            ..Default::default()
                        })
                        .into(),
                    None => muted(tr("암호를 입력해야 공개 키를 볼 수 있는 형식입니다")),
                },
            )
        }
        Some(Err(e)) => (text(e.clone()).size(11.5).color(theme::ERR_TX).into(), muted(tr("개인 키를 넣으면 자동으로 채워집니다"))),
        None => (Space::new().into(), muted(tr("개인 키를 넣으면 자동으로 채워집니다"))),
    };
    let ready = matches!(k.info, Some(Ok(_))) && !k.name.trim().is_empty();
    let mut col = column![
        row![crate::icons::tinted(&KEY, 16.0, theme::TEXT_2), text(tr("키 가져오기")).size(14)].spacing(8).align_y(Alignment::Center),
        Space::new().height(2),
        label(tr("이름")),
        text_input(tr("예: 회사 서버 키"), &k.name).on_input(Message::KeyImportName).size(12.5).padding([7, 10]).style(input_style),
        Space::new().height(2),
        row![label(tr("개인 키")), Space::new().width(Length::Fill), btn(tr("파일 선택…"), Btn::Plain, Some(Message::KeyImportBrowse))]
            .align_y(Alignment::Center),
        private,
        status,
        Space::new().height(2),
        label(tr("공개 키")),
        public,
        Space::new().height(2),
        label(tr("암호 (선택)")),
        text_input(tr("키에 암호가 있을 때만 입력"), k.passphrase.as_str())
            .on_input(Message::KeyImportPassphrase)
            .secure(true)
            .size(12.5)
            .padding([7, 10])
            .style(input_style),
        muted(tr("키체인에만 저장되고, 접속할 때 자동으로 쓰입니다. 비워 두면 접속할 때 물어봅니다.")),
        Space::new().height(2),
        label(tr("메모")),
        text_input(tr("예: 운영 서버 배포용"), &k.note).on_input(Message::KeyImportNote).size(12.5).padding([7, 10]).style(input_style),
    ]
    .spacing(6);
    if let Some(e) = &k.error {
        col = col.push(text(e.clone()).size(11.5).color(theme::ERR_TX));
    }
    col = col.push(Space::new().height(6)).push(
        row![
            Space::new().width(Length::Fill),
            btn(tr("취소"), Btn::Ghost, Some(Message::KeyImportClose)),
            btn(tr("저장"), Btn::Primary, ready.then_some(Message::KeyImportSave)),
        ]
        .spacing(8),
    );
    container(scrollable(col)).padding(20).width(IMPORT_W).style(card_style(theme::LINE_2)).into()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct KeyChoice {
    label: String,
    /// Stored in the host's IdentityFile; empty = automatic.
    value: String,
}
impl std::fmt::Display for KeyChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

/// Options and the current one: automatic, every known key, or the configured path as is.
fn key_choices(app: &CoShell, current: Option<&str>) -> (Vec<KeyChoice>, KeyChoice) {
    let home = app.home();
    let auto = KeyChoice { label: tr("자동 (ssh-agent · 기본 키)").into(), value: String::new() };
    let mut choices = vec![auto.clone()];
    choices.extend(app.keys.iter().map(|k| KeyChoice {
        label: format!("{} · {}", k.name, k.info.algorithm),
        value: display_path(&k.path, home),
    }));
    let selected = match current {
        None => auto,
        Some(cur) => {
            let want = expand(cur, home);
            match app.keys.iter().position(|k| k.path == want) {
                Some(i) => choices[i + 1].clone(),
                None => {
                    let other = KeyChoice { label: tf("직접 지정: {0}", &[&cur]), value: cur.to_string() };
                    choices.push(other.clone());
                    other
                }
            }
        }
    };
    (choices, selected)
}

/// Label of the key the server editor shows as chosen.
#[cfg(test)]
pub(crate) fn chosen_key_label(app: &CoShell, current: Option<&str>) -> String {
    key_choices(app, current).1.label
}

/// Server editor: which key to log in with.
pub fn picker<'a>(app: &'a CoShell, current: Option<&'a str>) -> El<'a> {
    let (choices, selected) = key_choices(app, current);
    column![
        label(tr("SSH 키")),
        pick_list(choices, Some(selected), |c: KeyChoice| Message::HostField(HostField::Identity, c.value))
            .text_size(12)
            .padding([7, 10])
            .width(Length::Fill)
            .style(crate::ui::pick_style)
            .menu_style(crate::ui::menu_style),
        muted(tr("설정 › SSH 키에서 키를 가져올 수 있습니다")),
    ]
    .spacing(4)
    .into()
}
