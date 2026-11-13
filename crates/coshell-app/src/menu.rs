//! Native macOS menu bar.
//!
//! Menu key equivalents are handled by AppKit before the window sees the key,
//! so every shortcut listed here arrives as a menu event, not a key press.
//! Copy / Paste carry no key equivalent on purpose: ⌘C / ⌘V must keep reaching
//! the focused widget (text fields handle them themselves).

use crate::i18n::tr;
use std::sync::{LazyLock, Mutex};

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures::{Stream, StreamExt};
use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};

use crate::app::{Message, Shortcut};

/// Taken once by the [`events`] stream.
type Receiver = Mutex<Option<UnboundedReceiver<String>>>;

/// Selected menu item ids, from AppKit's callback to the [`events`] stream.
/// Exists before the menu does: the subscription starts first.
static CHANNEL: LazyLock<(UnboundedSender<String>, Receiver)> = LazyLock::new(|| {
    let (tx, rx) = unbounded();
    (tx, Mutex::new(Some(rx)))
});

/// What a menu item does.
pub fn message(id: &str) -> Option<Message> {
    let shortcut = |s| Some(Message::Shortcut(s));
    match id {
        "about" => Some(Message::About),
        "settings" => shortcut(Shortcut::Settings),
        "new-local" => shortcut(Shortcut::NewLocal),
        "palette" => shortcut(Shortcut::Palette),
        "servers" => shortcut(Shortcut::Servers),
        "close-tab" => shortcut(Shortcut::CloseTab),
        "copy" => Some(Message::TermCopy),
        "paste" => Some(Message::TermPaste),
        "find" => shortcut(Shortcut::Find),
        "find-up" => shortcut(Shortcut::FindNext(false)),
        "find-down" => shortcut(Shortcut::FindNext(true)),
        "ai-panel" => shortcut(Shortcut::AiPanel),
        "zoom-in" => shortcut(Shortcut::Zoom(1)),
        "zoom-out" => shortcut(Shortcut::Zoom(-1)),
        "zoom-reset" => shortcut(Shortcut::Zoom(0)),
        "tab-prev" => shortcut(Shortcut::TabStep(-1)),
        "tab-next" => shortcut(Shortcut::TabStep(1)),
        "tab-left" => shortcut(Shortcut::TabMove(-1)),
        "tab-right" => shortcut(Shortcut::TabMove(1)),
        _ => None,
    }
}

fn item(id: &str, label: &str, key: Option<(Modifiers, Code)>) -> MenuItem {
    debug_assert!(message(id).is_some(), "menu item {id} has no action");
    MenuItem::with_id(id, label, true, key.map(|(mods, code)| Accelerator::new(Some(mods), code)))
}

fn build() -> muda::Result<Menu> {
    const CMD: Modifiers = Modifiers::SUPER;
    let cmd_shift = Modifiers::SUPER | Modifiers::SHIFT;
    let sep = PredefinedMenuItem::separator;

    let app = Submenu::with_items(
        "CoShell",
        true,
        &[
            &item("about", tr("CoShell 정보"), None),
            &sep(),
            &item("settings", tr("설정…"), Some((CMD, Code::Comma))),
            &sep(),
            &PredefinedMenuItem::services(Some(tr("서비스"))),
            &sep(),
            &PredefinedMenuItem::hide(Some(tr("CoShell 가리기"))),
            &PredefinedMenuItem::hide_others(Some(tr("기타 가리기"))),
            &PredefinedMenuItem::show_all(Some(tr("모두 보기"))),
            &sep(),
            &PredefinedMenuItem::quit(Some(tr("CoShell 종료"))),
        ],
    )?;
    let shell = Submenu::with_items(
        tr("셸"),
        true,
        &[
            &item("new-local", tr("새 로컬 셸"), Some((CMD, Code::KeyT))),
            &item("palette", tr("서버 열기…"), Some((CMD, Code::KeyK))),
            &item("servers", tr("서버 목록"), Some((CMD, Code::KeyO))),
            &sep(),
            &item("close-tab", tr("탭 닫기"), Some((CMD, Code::KeyW))),
        ],
    )?;
    let edit = Submenu::with_items(
        tr("편집"),
        true,
        &[
            &item("copy", tr("터미널 선택 복사"), None),
            &item("paste", tr("터미널에 붙여넣기"), None),
            &sep(),
            &item("find", tr("찾기…"), Some((CMD, Code::KeyF))),
            &item("find-up", tr("위로 찾기"), Some((CMD, Code::KeyG))),
            &item("find-down", tr("아래로 찾기"), Some((cmd_shift, Code::KeyG))),
        ],
    )?;
    let view = Submenu::with_items(
        tr("보기"),
        true,
        &[
            &item("ai-panel", tr("AI 패널"), Some((CMD, Code::KeyL))),
            &sep(),
            &item("zoom-in", tr("글꼴 크게"), Some((CMD, Code::Equal))),
            &item("zoom-out", tr("글꼴 작게"), Some((CMD, Code::Minus))),
            &item("zoom-reset", tr("기본 글꼴 크기"), Some((CMD, Code::Digit0))),
            &sep(),
            &PredefinedMenuItem::fullscreen(Some(tr("전체 화면"))),
        ],
    )?;
    let window = Submenu::with_items(
        tr("윈도우"),
        true,
        &[
            &PredefinedMenuItem::minimize(Some(tr("최소화"))),
            &PredefinedMenuItem::maximize(Some(tr("확대/축소"))),
            &sep(),
            &item("tab-prev", tr("이전 탭"), Some((cmd_shift, Code::BracketLeft))),
            &item("tab-next", tr("다음 탭"), Some((cmd_shift, Code::BracketRight))),
            &item("tab-left", tr("탭을 왼쪽으로"), Some((cmd_shift, Code::ArrowLeft))),
            &item("tab-right", tr("탭을 오른쪽으로"), Some((cmd_shift, Code::ArrowRight))),
            &sep(),
            &PredefinedMenuItem::bring_all_to_front(Some(tr("모두 앞으로 가져오기"))),
        ],
    )?;
    Menu::with_items(&[&app, &shell, &edit, &view, &window])
}

/// Put the menu bar up. Must run on the main thread, once the app is launched.
pub fn install() -> Result<(), String> {
    let menu = build().map_err(|e| e.to_string())?;
    MenuEvent::set_event_handler(Some(|e: MenuEvent| {
        let _ = CHANNEL.0.unbounded_send(e.id.0);
    }));
    menu.init_for_nsapp();
    // AppKit keeps using the menu for the life of the process.
    std::mem::forget(menu);
    Ok(())
}

/// Menu selections as app messages (for a subscription).
pub fn events() -> impl Stream<Item = Message> {
    let rx = CHANNEL.1.lock().ok().and_then(|mut rx| rx.take());
    futures::stream::iter(rx).flatten().filter_map(|id| async move { message(&id) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_ids_do_nothing() {
        assert!(message("no-such-item").is_none());
        assert!(matches!(message("zoom-out"), Some(Message::Shortcut(Shortcut::Zoom(-1)))));
        assert!(matches!(message("copy"), Some(Message::TermCopy)));
    }
}
