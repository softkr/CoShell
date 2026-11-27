//! User preferences and remembered window state (`~/.config/coshell/settings.toml`).
//!
//! The file is user-editable, so everything read from it is clamped to sane
//! ranges before use ([`Settings::sanitize`]).

use crate::i18n::tr;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const FONT_SIZE_DEFAULT: f32 = 13.0;
pub const FONT_SIZE_MIN: f32 = 9.0;
pub const FONT_SIZE_MAX: f32 = 28.0;
pub const SCROLLBACK_DEFAULT: usize = 10_000;
pub const SCROLLBACK_MIN: usize = 100;
pub const SCROLLBACK_MAX: usize = 100_000;
/// One press of the scrollback's −/+.
pub const SCROLLBACK_STEP: usize = 1000;
pub const ATTACH_LINES_DEFAULT: usize = 60;
pub const ATTACH_LINES_MIN: usize = 10;
pub const ATTACH_LINES_MAX: usize = 500;
/// One press of the attached lines' −/+.
pub const ATTACH_LINES_STEP: usize = 10;
pub const SIDEBAR_W_DEFAULT: f32 = 256.0;
pub const SIDEBAR_W_MIN: f32 = 210.0;
pub const SIDEBAR_W_MAX: f32 = 380.0;
const FONT_FAMILY_MAX_CHARS: usize = 64;
pub const SNIPPETS_MAX: usize = 200;
const SNIPPET_TITLE_MAX_CHARS: usize = 60;
const SNIPPET_CATEGORY_MAX_CHARS: usize = 20;
const SNIPPET_CMD_MAX_CHARS: usize = 2000;
const WINDOW_DEFAULT: (f32, f32) = (1200.0, 760.0);
const WINDOW_MIN: (f32, f32) = (640.0, 400.0);
const WINDOW_MAX: (f32, f32) = (8192.0, 8192.0);

/// UI language.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    /// Follow the system: Korean on a Korean system, English otherwise.
    #[default]
    Auto,
    Ko,
    En,
}

impl Language {
    pub const ALL: [Language; 3] = [Language::Auto, Language::Ko, Language::En];

    pub fn label(self) -> &'static str {
        match self {
            Language::Auto => tr("시스템 설정"),
            // Each language is named in itself.
            Language::Ko => "한국어",
            Language::En => "English",
        }
    }
}

/// Terminal colour scheme (the app chrome stays dark).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scheme {
    #[default]
    Dark,
    SolarizedDark,
    Light,
}

impl Scheme {
    pub const ALL: [Scheme; 3] = [Scheme::Dark, Scheme::SolarizedDark, Scheme::Light];

    pub fn label(self) -> &'static str {
        match self {
            Scheme::Dark => tr("CoShell 다크"),
            Scheme::SolarizedDark => tr("Solarized 다크"),
            Scheme::Light => tr("라이트"),
        }
    }
}

/// Default cursor shape (programs such as vim may still change it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CursorKind {
    #[default]
    Block,
    Beam,
    Underline,
}

impl CursorKind {
    pub const ALL: [CursorKind; 3] = [CursorKind::Block, CursorKind::Beam, CursorKind::Underline];

    pub fn label(self) -> &'static str {
        match self {
            CursorKind::Block => tr("블록"),
            CursorKind::Beam => tr("막대"),
            CursorKind::Underline => tr("밑줄"),
        }
    }
}

/// A saved command: typed into the terminal, optionally with Enter.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Snippet {
    pub title: String,
    pub category: String,
    pub cmd: String,
}

/// `s` trimmed, on one line and at most `max` characters.
fn one_line(s: &str, max: usize) -> String {
    s.trim().chars().filter(|c| !c.is_control()).take(max).collect()
}

impl Snippet {
    /// Trimmed, single-line and bounded; `None` without a name or a command.
    pub fn sanitize(&self) -> Option<Self> {
        let s = Self {
            title: one_line(&self.title, SNIPPET_TITLE_MAX_CHARS),
            category: one_line(&self.category, SNIPPET_CATEGORY_MAX_CHARS),
            cmd: one_line(&self.cmd, SNIPPET_CMD_MAX_CHARS),
        };
        (!s.title.is_empty() && !s.cmd.is_empty()).then_some(s)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowSize {
    pub width: f32,
    pub height: f32,
}

impl Default for WindowSize {
    fn default() -> Self {
        Self { width: WINDOW_DEFAULT.0, height: WINDOW_DEFAULT.1 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub language: Language,
    pub font_size: f32,
    /// Empty = the system monospace font.
    pub font_family: String,
    pub scheme: Scheme,
    pub cursor: CursorKind,
    pub scrollback: usize,
    /// Option/Alt sends ESC-prefixed keys (word jumps in shells) instead of special characters.
    pub option_as_meta: bool,
    /// Terminal lines attached to an AI question.
    pub ai_attach_lines: usize,
    /// Remembered toggle: the assistant runs read-only commands itself and reads the results.
    pub ai_agent: bool,
    /// Look for a new release on start and every 10 minutes.
    pub update_check: bool,
    /// Remembered state: the server sidebar on the left.
    pub sidebar_open: bool,
    pub sidebar_width: f32,
    /// Remembered state: the status bar at the bottom.
    pub statusbar_open: bool,
    /// Show commands and their output as blocks (the raw terminal for full-screen programs).
    pub block_view: bool,
    pub window: WindowSize,
    /// Sidebar snippets; `None` = the built-in set.
    pub snippets: Option<Vec<Snippet>>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: Language::default(),
            font_size: FONT_SIZE_DEFAULT,
            font_family: String::new(),
            scheme: Scheme::default(),
            cursor: CursorKind::default(),
            scrollback: SCROLLBACK_DEFAULT,
            option_as_meta: true,
            ai_attach_lines: ATTACH_LINES_DEFAULT,
            ai_agent: true,
            update_check: true,
            sidebar_open: true,
            statusbar_open: true,
            sidebar_width: SIDEBAR_W_DEFAULT,
            block_view: true,
            window: WindowSize::default(),
            snippets: None,
        }
    }
}

fn clamp_or(v: f32, min: f32, max: f32, default: f32) -> f32 {
    if v.is_finite() { v.clamp(min, max) } else { default }
}

impl Settings {
    pub fn default_path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::home_dir().map(|h| h.join(".config")))?;
        Some(base.join("coshell").join("settings.toml"))
    }

    /// Bring every value into its supported range.
    pub fn sanitize(mut self) -> Self {
        self.font_size = clamp_or(self.font_size, FONT_SIZE_MIN, FONT_SIZE_MAX, FONT_SIZE_DEFAULT);
        self.font_family = self.font_family.trim().chars().filter(|c| !c.is_control()).take(FONT_FAMILY_MAX_CHARS).collect();
        self.scrollback = self.scrollback.clamp(SCROLLBACK_MIN, SCROLLBACK_MAX);
        self.ai_attach_lines = self.ai_attach_lines.clamp(ATTACH_LINES_MIN, ATTACH_LINES_MAX);
        self.sidebar_width = clamp_or(self.sidebar_width, SIDEBAR_W_MIN, SIDEBAR_W_MAX, SIDEBAR_W_DEFAULT);
        self.window.width = clamp_or(self.window.width, WINDOW_MIN.0, WINDOW_MAX.0, WINDOW_DEFAULT.0);
        self.window.height = clamp_or(self.window.height, WINDOW_MIN.1, WINDOW_MAX.1, WINDOW_DEFAULT.1);
        self.snippets = self.snippets.map(|v| v.iter().filter_map(Snippet::sanitize).take(SNIPPETS_MAX).collect());
        self
    }

    /// Load (sanitized), or defaults when the file does not exist.
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(t) => toml::from_str::<Self>(&t).map(Self::sanitize).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_defaults_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("coshell").join("settings.toml");
        assert_eq!(Settings::load(&path).unwrap(), Settings::default());
        let s = Settings { font_size: 15.0, font_family: "JetBrains Mono".into(), scheme: Scheme::Light, cursor: CursorKind::Beam, ..Settings::default() };
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), s);
    }

    #[test]
    fn partial_file_keeps_other_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "scheme = \"solarized-dark\"\n").unwrap();
        let s = Settings::load(&path).unwrap();
        assert_eq!(s, Settings { scheme: Scheme::SolarizedDark, ..Settings::default() });
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(
            &path,
            "font_size = 900.0\nfont_family = \"  Menlo\\u0007  \"\nscrollback = 0\nai_attach_lines = 99999\n[window]\nwidth = nan\nheight = 1.0\n",
        )
        .unwrap();
        let s = Settings::load(&path).unwrap();
        assert_eq!(s.font_size, FONT_SIZE_MAX);
        assert_eq!(s.font_family, "Menlo");
        assert_eq!(s.scrollback, SCROLLBACK_MIN);
        assert_eq!(s.ai_attach_lines, ATTACH_LINES_MAX);
        assert_eq!((s.window.width, s.window.height), (WINDOW_DEFAULT.0, WINDOW_MIN.1));
    }

    #[test]
    fn snippets_round_trip_and_are_sanitized() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let sn = Snippet { title: "disk".into(), category: "diag".into(), cmd: "df -h".into() };
        let s = Settings { snippets: Some(vec![sn.clone()]), ..Settings::default() };
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), s);
        // An emptied list stays empty instead of bringing the built-ins back.
        let none = Settings { snippets: Some(Vec::new()), ..Settings::default() };
        none.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap().snippets, Some(Vec::new()));
        // Hand-edited: control characters go, entries without a command are dropped.
        std::fs::write(&path, "[[snippets]]\ntitle = \"  a\\u0007 \"\ncmd = \"ls\\nrm -rf x\"\n[[snippets]]\ntitle = \"no command\"\n").unwrap();
        let got = Settings::load(&path).unwrap().snippets.unwrap();
        assert_eq!(got, vec![Snippet { title: "a".into(), category: String::new(), cmd: "lsrm -rf x".into() }]);
    }

    #[test]
    fn broken_file_is_an_error_not_a_reset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "scheme = \"neon\"\n").unwrap();
        assert!(Settings::load(&path).is_err());
    }
}
