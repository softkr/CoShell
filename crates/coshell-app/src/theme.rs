//! Design tokens from coshell_ai_native_ssh_terminal.html (dark, emerald accent).

use iced::Color;

const fn hex(rgb: u32) -> Color {
    Color::from_rgb8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

// surface (shell.bg / panel / elevated / hover)
pub const BG_0: Color = hex(0x0B0E14);
pub const BG_1: Color = hex(0x10141C);
pub const BG_2: Color = hex(0x161B26);
pub const BG_3: Color = hex(0x1C2331);
// shell.border / borderLight
pub const LINE: Color = hex(0x1F2735);
pub const LINE_2: Color = hex(0x2B3548);
// text (slate-200 / 400 / 500)
pub const TEXT: Color = hex(0xE2E8F0);
pub const TEXT_2: Color = hex(0x94A3B8);
pub const TEXT_3: Color = hex(0x64748B);
// tailwind shades the design uses directly
pub const SLATE_100: Color = hex(0xF1F5F9);
pub const SLATE_300: Color = hex(0xCBD5E1);
pub const SLATE_600: Color = hex(0x475569);
pub const SKY_300: Color = hex(0x7DD3FC);
pub const AMBER_200: Color = hex(0xFDE68A);
pub const AMBER_300: Color = hex(0xFCD34D);
pub const EMERALD_300: Color = hex(0x6EE7B7);
// semantic
/// emerald-500
pub const ACCENT: Color = hex(0x10B981);
/// emerald-400: accent text and live dots on dark surfaces
pub const ACCENT_TX: Color = hex(0x34D399);
pub const ON_ACCENT: Color = hex(0x0B0E14);
pub const OK: Color = hex(0x34D399);
/// amber-400
pub const WARN: Color = hex(0xFBBF24);
/// amber-500
pub const WARN_500: Color = hex(0xF59E0B);
pub const ERR: Color = hex(0xEF4444);
pub const ERR_TX: Color = hex(0xF87171);
/// sky-400: working directories and paths
pub const INFO: Color = hex(0x38BDF8);
// environment: production is amber like the mockup's 운영 group, development emerald
pub const ENV_PROD: Color = hex(0xFBBF24);
pub const ENV_STAGE: Color = hex(0x38BDF8);
pub const ENV_DEV: Color = hex(0x34D399);
pub const SCRIM: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 0.7 };

/// `c` at `alpha` opacity.
pub const fn alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
}

/// Palette for a terminal colour scheme.
pub fn term_palette(scheme: crate::settings::Scheme) -> &'static TermPalette {
    use crate::settings::Scheme;
    match scheme {
        Scheme::Dark => &TERM_DARK,
        Scheme::SolarizedDark => &TERM_SOLARIZED_DARK,
        Scheme::Light => &TERM_LIGHT,
    }
}

/// Terminal palette: 16 ANSI colours + default fg/bg/cursor.
pub struct TermPalette {
    pub ansi: [Color; 16],
    pub fg: Color,
    pub bg: Color,
    pub cursor: Color,
    /// Background of selected cells.
    pub selection: Color,
}

pub const TERM_DARK: TermPalette = TermPalette {
    ansi: [
        hex(0x484F58), // black
        hex(0xFF7B72), // red
        hex(0x56D364), // green
        hex(0xE3B341), // yellow
        hex(0x79C0FF), // blue
        hex(0xD2A8FF), // magenta
        hex(0x56D4DD), // cyan
        hex(0xB1BAC4), // white
        hex(0x6E7681), // bright black
        hex(0xFFA198), // bright red
        hex(0x7EE787), // bright green
        hex(0xF2CC60), // bright yellow
        hex(0xA5D6FF), // bright blue
        hex(0xE2C5FF), // bright magenta
        hex(0xB3F0FF), // bright cyan
        hex(0xF0F6FC), // bright white
    ],
    fg: TEXT,
    bg: BG_0,
    cursor: ACCENT_TX,
    selection: hex(0x134E4A),
};

pub const TERM_SOLARIZED_DARK: TermPalette = TermPalette {
    ansi: [
        hex(0x073642), // black
        hex(0xDC322F), // red
        hex(0x859900), // green
        hex(0xB58900), // yellow
        hex(0x268BD2), // blue
        hex(0xD33682), // magenta
        hex(0x2AA198), // cyan
        hex(0xEEE8D5), // white
        hex(0x586E75), // bright black (readable on the background)
        hex(0xCB4B16), // bright red
        hex(0x93A1A1), // bright green
        hex(0xB58900), // bright yellow
        hex(0x839496), // bright blue
        hex(0x6C71C4), // bright magenta
        hex(0x93A1A1), // bright cyan
        hex(0xFDF6E3), // bright white
    ],
    fg: hex(0x93A1A1),
    bg: hex(0x002B36),
    cursor: hex(0x93A1A1),
    selection: hex(0x0E4F60),
};

pub const TERM_LIGHT: TermPalette = TermPalette {
    ansi: [
        hex(0x24292F), // black
        hex(0xCF222E), // red
        hex(0x116329), // green
        hex(0x7D4E00), // yellow
        hex(0x0969DA), // blue
        hex(0x8250DF), // magenta
        hex(0x1B7C83), // cyan
        hex(0x6E7781), // white
        hex(0x57606A), // bright black
        hex(0xA40E26), // bright red
        hex(0x1A7F37), // bright green
        hex(0x633C01), // bright yellow
        hex(0x218BFF), // bright blue
        hex(0xA475F9), // bright magenta
        hex(0x3192AA), // bright cyan
        hex(0x8C959F), // bright white
    ],
    fg: hex(0x1F2328),
    bg: hex(0xF7F8FA),
    cursor: hex(0x1F2328),
    selection: hex(0xB6D7FF),
};
