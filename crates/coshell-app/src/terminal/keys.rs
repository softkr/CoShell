//! Keyboard → terminal byte encoding (xterm conventions).

use iced::keyboard::key::Named;
use iced::keyboard::{Key, Modifiers};

const ESC: u8 = 0x1b;

/// Inputs needed to encode one key press.
pub struct KeyInput<'a> {
    pub key: &'a Key,
    /// Latin equivalent of the key (works under non-latin layouts, e.g. 한글 자판).
    pub latin: Option<char>,
    /// Text the key would produce, if any.
    pub text: Option<&'a str>,
    pub mods: Modifiers,
    /// DECCKM (application cursor keys) is active.
    pub app_cursor: bool,
    /// Treat Option/Alt as Meta (ESC prefix).
    pub option_as_meta: bool,
}

/// xterm modifier parameter: 1 + shift + 2*alt + 4*ctrl.
fn modifier_param(m: Modifiers) -> u8 {
    1 + m.shift() as u8 + 2 * m.alt() as u8 + 4 * m.control() as u8
}

fn has_mods(m: Modifiers) -> bool {
    m.shift() || m.alt() || m.control()
}

/// Cursor-style key: `ESC [ A` / `ESC O A` / `ESC [ 1 ; m A`.
fn cursor_key(code: char, m: Modifiers, app_cursor: bool) -> Vec<u8> {
    if has_mods(m) {
        format!("\x1b[1;{}{code}", modifier_param(m)).into_bytes()
    } else if app_cursor {
        format!("\x1bO{code}").into_bytes()
    } else {
        format!("\x1b[{code}").into_bytes()
    }
}

/// Tilde-style key: `ESC [ n ~` / `ESC [ n ; m ~`.
fn tilde_key(n: u8, m: Modifiers) -> Vec<u8> {
    if has_mods(m) {
        format!("\x1b[{n};{}~", modifier_param(m)).into_bytes()
    } else {
        format!("\x1b[{n}~").into_bytes()
    }
}

/// F1–F4 use SS3 without modifiers, CSI with modifiers.
fn ss3_fkey(code: char, m: Modifiers) -> Vec<u8> {
    if has_mods(m) {
        format!("\x1b[1;{}{code}", modifier_param(m)).into_bytes()
    } else {
        format!("\x1bO{code}").into_bytes()
    }
}

/// Control byte for Ctrl+<c>.
fn ctrl_byte(c: char) -> Option<u8> {
    match c.to_ascii_lowercase() {
        c @ 'a'..='z' => Some(c as u8 - b'a' + 1),
        '@' | ' ' | '2' => Some(0x00),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '/' | '7' | '-' => Some(0x1f),
        '8' | '?' => Some(0x7f),
        _ => None,
    }
}

fn with_alt(m: Modifiers, mut bytes: Vec<u8>) -> Vec<u8> {
    if m.alt() {
        bytes.insert(0, ESC);
    }
    bytes
}

/// Encode a key press. `None` means "not for the terminal" (e.g. ⌘ shortcuts,
/// or keys that produce no bytes).
pub fn encode(k: &KeyInput) -> Option<Vec<u8>> {
    let m = k.mods;
    if m.logo() {
        return None;
    }
    match k.key {
        Key::Named(named) => named_key(*named, m, k.app_cursor),
        Key::Character(_) => {
            if m.control() {
                let b = ctrl_byte(k.latin?)?;
                return Some(with_alt(m, vec![b]));
            }
            if m.alt() && k.option_as_meta {
                let c = k.latin?;
                let c = if m.shift() { c.to_ascii_uppercase() } else { c };
                return Some(vec![ESC, c as u8]);
            }
            let text = k.text?;
            (!text.is_empty() && !text.chars().any(char::is_control)).then(|| text.as_bytes().to_vec())
        }
        Key::Unidentified => None,
    }
}

fn named_key(n: Named, m: Modifiers, app_cursor: bool) -> Option<Vec<u8>> {
    Some(match n {
        Named::Enter => with_alt(m, vec![b'\r']),
        Named::Backspace if m.control() => with_alt(m, vec![0x08]),
        Named::Backspace => with_alt(m, vec![0x7f]),
        Named::Tab if m.shift() => b"\x1b[Z".to_vec(),
        Named::Tab => with_alt(m, vec![b'\t']),
        Named::Escape => vec![ESC],
        Named::Space if m.control() => with_alt(m, vec![0x00]),
        Named::Space => with_alt(m, vec![b' ']),
        Named::ArrowUp => cursor_key('A', m, app_cursor),
        Named::ArrowDown => cursor_key('B', m, app_cursor),
        Named::ArrowRight => cursor_key('C', m, app_cursor),
        Named::ArrowLeft => cursor_key('D', m, app_cursor),
        Named::Home => cursor_key('H', m, app_cursor),
        Named::End => cursor_key('F', m, app_cursor),
        Named::Insert => tilde_key(2, m),
        Named::Delete => tilde_key(3, m),
        Named::PageUp => tilde_key(5, m),
        Named::PageDown => tilde_key(6, m),
        Named::F1 => ss3_fkey('P', m),
        Named::F2 => ss3_fkey('Q', m),
        Named::F3 => ss3_fkey('R', m),
        Named::F4 => ss3_fkey('S', m),
        Named::F5 => tilde_key(15, m),
        Named::F6 => tilde_key(17, m),
        Named::F7 => tilde_key(18, m),
        Named::F8 => tilde_key(19, m),
        Named::F9 => tilde_key(20, m),
        Named::F10 => tilde_key(21, m),
        Named::F11 => tilde_key(23, m),
        Named::F12 => tilde_key(24, m),
        _ => return None,
    })
}

/// Mouse button codes of the xterm mouse protocol.
pub const MOUSE_LEFT: u8 = 0;
pub const MOUSE_MIDDLE: u8 = 1;
pub const MOUSE_RIGHT: u8 = 2;
/// Legacy release / "no button" for motion reports.
pub const MOUSE_NONE: u8 = 3;
pub const MOUSE_MOTION: u8 = 32;
pub const MOUSE_WHEEL_UP: u8 = 64;
pub const MOUSE_WHEEL_DOWN: u8 = 65;
/// Largest coordinate the legacy (non-SGR) encoding can carry.
const LEGACY_MOUSE_MAX: usize = 222;

/// Modifier bits added to a mouse button code.
pub fn mouse_mods(m: Modifiers) -> u8 {
    4 * m.shift() as u8 + 8 * m.alt() as u8 + 16 * m.control() as u8
}

/// Encode a mouse report for 0-based cell (`col`, `row`). `None` when the
/// legacy encoding cannot express the position.
pub fn mouse_report(code: u8, col: usize, row: usize, pressed: bool, sgr: bool) -> Option<Vec<u8>> {
    if sgr {
        let end = if pressed { 'M' } else { 'm' };
        return Some(format!("\x1b[<{code};{};{}{end}", col + 1, row + 1).into_bytes());
    }
    if col > LEGACY_MOUSE_MAX || row > LEGACY_MOUSE_MAX {
        return None;
    }
    // Legacy releases do not say which button: low bits become 3.
    let code = if pressed { code } else { (code & !MOUSE_NONE) | MOUSE_NONE };
    Some(vec![ESC, b'[', b'M', 32 + code, 32 + col as u8 + 1, 32 + row as u8 + 1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_reports_sgr_and_legacy() {
        assert_eq!(mouse_report(MOUSE_LEFT, 0, 0, true, true), Some(b"\x1b[<0;1;1M".to_vec()));
        assert_eq!(mouse_report(MOUSE_LEFT, 9, 4, false, true), Some(b"\x1b[<0;10;5m".to_vec()));
        assert_eq!(mouse_report(MOUSE_WHEEL_DOWN, 300, 2, true, true), Some(b"\x1b[<65;301;3M".to_vec()));
        assert_eq!(mouse_report(MOUSE_RIGHT, 0, 0, true, false), Some(vec![0x1b, b'[', b'M', 34, 33, 33]));
        assert_eq!(mouse_report(MOUSE_RIGHT, 0, 0, false, false), Some(vec![0x1b, b'[', b'M', 35, 33, 33]));
        assert_eq!(mouse_report(MOUSE_LEFT, 300, 0, true, false), None);
        assert_eq!(mouse_mods(Modifiers::CTRL | Modifiers::ALT), 24);
    }

    fn enc(key: Key, latin: Option<char>, text: Option<&str>, mods: Modifiers, app: bool) -> Option<Vec<u8>> {
        encode(&KeyInput { key: &key, latin, text, mods, app_cursor: app, option_as_meta: true })
    }
    fn ch(c: &str) -> Key {
        Key::Character(c.into())
    }

    #[test]
    fn plain_text_passes_through() {
        assert_eq!(enc(ch("a"), Some('a'), Some("a"), Modifiers::empty(), false), Some(b"a".to_vec()));
    }

    #[test]
    fn ctrl_letters_and_korean_layout() {
        assert_eq!(enc(ch("c"), Some('c'), None, Modifiers::CTRL, false), Some(vec![0x03]));
        // 한글 자판에서 Ctrl+ㅊ → physical C → ^C
        assert_eq!(enc(ch("ㅊ"), Some('c'), None, Modifiers::CTRL, false), Some(vec![0x03]));
        assert_eq!(enc(ch("["), Some('['), None, Modifiers::CTRL, false), Some(vec![0x1b]));
    }

    #[test]
    fn cursor_keys_respect_decckm_and_modifiers() {
        let up = Key::Named(Named::ArrowUp);
        assert_eq!(enc(up.clone(), None, None, Modifiers::empty(), false), Some(b"\x1b[A".to_vec()));
        assert_eq!(enc(up.clone(), None, None, Modifiers::empty(), true), Some(b"\x1bOA".to_vec()));
        assert_eq!(enc(up, None, None, Modifiers::CTRL, false), Some(b"\x1b[1;5A".to_vec()));
    }

    #[test]
    fn editing_and_function_keys() {
        assert_eq!(enc(Key::Named(Named::Backspace), None, None, Modifiers::empty(), false), Some(vec![0x7f]));
        assert_eq!(enc(Key::Named(Named::Tab), None, None, Modifiers::SHIFT, false), Some(b"\x1b[Z".to_vec()));
        assert_eq!(enc(Key::Named(Named::Delete), None, None, Modifiers::empty(), false), Some(b"\x1b[3~".to_vec()));
        assert_eq!(enc(Key::Named(Named::F1), None, None, Modifiers::empty(), false), Some(b"\x1bOP".to_vec()));
        assert_eq!(enc(Key::Named(Named::F5), None, None, Modifiers::SHIFT, false), Some(b"\x1b[15;2~".to_vec()));
    }

    #[test]
    fn alt_as_meta_and_cmd_ignored() {
        assert_eq!(enc(ch("b"), Some('b'), Some("∫"), Modifiers::ALT, false), Some(vec![0x1b, b'b']));
        assert_eq!(enc(ch("v"), Some('v'), Some("v"), Modifiers::LOGO, false), None);
    }
}
