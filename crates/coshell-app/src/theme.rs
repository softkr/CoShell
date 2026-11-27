//! Design tokens (dark appearance): macOS system colours on near-black surfaces with a
//! little aubergine in them, hairline separators drawn as translucent white.

use iced::Color;

const fn hex(rgb: u32) -> Color {
    Color::from_rgb8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

const WHITE: Color = Color::WHITE;

// surfaces: terminal canvas / bars and panels / elevated controls / hover
pub const BG_0: Color = hex(0x100E14);
pub const BG_1: Color = hex(0x17141C);
pub const BG_2: Color = hex(0x211D29);
pub const BG_3: Color = hex(0x2B2734);
/// The dock's lit item: an ember against the aubergine (not the amber that marks production).
pub const EMBER: Color = hex(0xE95420);
pub const EMBER_HI: Color = hex(0xFF7043);
/// The assistant's dock item.
pub const SKY: Color = hex(0x38BDF8);
/// Cards on the canvas (command blocks, chat bubbles): a faint lift.
pub const CARD: Color = alpha(WHITE, 0.035);
/// Chips and quiet buttons inside cards.
pub const CHIP: Color = alpha(WHITE, 0.07);
// hairline / stronger border
pub const LINE: Color = alpha(WHITE, 0.085);
pub const LINE_2: Color = alpha(WHITE, 0.15);
// text: primary / secondary / tertiary labels
pub const TEXT: Color = hex(0xF5F5F7);
pub const TEXT_2: Color = hex(0xA1A1A6);
pub const TEXT_3: Color = hex(0x8A8A8F);
// brighter and dimmer neutrals
pub const SLATE_100: Color = hex(0xFFFFFF);
pub const SLATE_300: Color = hex(0xD1D1D6);
pub const SLATE_600: Color = hex(0x545458);
// macOS system colours (dark)
pub const BLUE: Color = hex(0x0A84FF);
pub const GREEN: Color = hex(0x30D158);
pub const ORANGE: Color = hex(0xFF9F0A);
pub const RED: Color = hex(0xFF453A);
pub const PURPLE: Color = hex(0xBF5AF2);
pub const TEAL: Color = hex(0x64D2FF);
pub const AMBER_200: Color = hex(0xFFD08A);
pub const AMBER_300: Color = ORANGE;
// semantic
pub const ACCENT: Color = BLUE;
/// Accent as text on dark surfaces.
pub const ACCENT_TX: Color = BLUE;
/// Lighter accent: hovered links and labels.
pub const ACCENT_HI: Color = hex(0x5EB0FF);
pub const ON_ACCENT: Color = WHITE;
pub const OK: Color = GREEN;
pub const WARN: Color = ORANGE;
pub const WARN_500: Color = ORANGE;
pub const ERR: Color = RED;
pub const ERR_TX: Color = hex(0xFF6961);
/// Working directories and paths.
pub const INFO: Color = BLUE;
// environment: production orange, staging teal, development green
pub const ENV_PROD: Color = ORANGE;
pub const ENV_STAGE: Color = TEAL;
pub const ENV_DEV: Color = GREEN;
/// The sidebar: on macOS a tint over the blurred desktop behind the window (the window is
/// transparent there, see `main.rs`); a plain panel elsewhere.
/// Dense enough that a bright window or wallpaper behind it still leaves a dark panel:
/// over white it comes out near #2B2B2D, so tertiary labels stay legible.
pub const SIDEBAR: Color = if cfg!(target_os = "macos") { alpha(hex(0x17141C), 0.92) } else { hex(0x141118) };
pub const SCRIM: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 0.45 };

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
    selection: hex(0x1F3A5F),
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
