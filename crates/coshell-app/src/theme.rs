//! Design tokens from design-v2.html (dark theme).

use iced::Color;

const fn hex(rgb: u32) -> Color {
    Color::from_rgb8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

// surface
pub const BG_0: Color = hex(0x0B0E14);
pub const BG_1: Color = hex(0x10141B);
pub const BG_2: Color = hex(0x161B24);
pub const BG_3: Color = hex(0x1E2430);
pub const BG_4: Color = hex(0x272E3C);
pub const LINE: Color = hex(0x222935);
pub const LINE_2: Color = hex(0x2F3747);
// text
pub const TEXT: Color = hex(0xE6EAF2);
pub const TEXT_2: Color = hex(0xA6AFBF);
pub const TEXT_3: Color = hex(0x7C8698);
// semantic
pub const ACCENT: Color = hex(0x5B9DFF);
pub const ON_ACCENT: Color = hex(0x0B0E14);
pub const OK: Color = hex(0x3FB950);
pub const WARN: Color = hex(0xE3A008);
pub const ERR: Color = hex(0xF25555);
pub const ERR_TX: Color = hex(0xFF9A9A);
// environment
pub const ENV_PROD: Color = hex(0xF0506E);
pub const ENV_STAGE: Color = hex(0xE3A008);
pub const ENV_DEV: Color = hex(0x7D8799);
pub const SCRIM: Color = Color { r: 0.016, g: 0.024, b: 0.04, a: 0.62 };

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
    cursor: TEXT,
    selection: hex(0x27466E),
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
