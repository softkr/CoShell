//! Bundled fonts (see `assets/fonts/README.md`): Inter + Noto Sans KR (merged as
//! "CoShell Sans") for the interface, and Hack Nerd Font for the terminal.

use iced::Font;

macro_rules! font_file {
    ($path:literal) => {
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/fonts/", $path)).as_slice()
    };
}

pub const FILES: [&[u8]; 6] = [
    font_file!("CoShellSans-Regular.ttf"),
    font_file!("CoShellSans-Medium.ttf"),
    font_file!("CoShellSans-SemiBold.ttf"),
    font_file!("CoShellSans-Bold.ttf"),
    // Regular and bold only: lighter emphasis (medium) draws as regular, semibold as bold.
    font_file!("HackNerdFont-Regular.ttf"),
    font_file!("HackNerdFont-Bold.ttf"),
];

pub const SANS: Font = Font::with_name("CoShell Sans");
pub const MONO: Font = Font::with_name("Hack Nerd Font");

pub const fn weight(f: Font, w: iced::font::Weight) -> Font {
    Font { weight: w, ..f }
}

#[cfg(test)]
mod tests {
    #[test]
    fn bundled_fonts_parse_with_the_expected_families() {
        let mut db = iced::advanced::graphics::text::cosmic_text::fontdb::Database::new();
        for f in super::FILES {
            db.load_font_data(f.to_vec());
        }
        let families: std::collections::BTreeSet<String> = db.faces().flat_map(|f| f.families.iter().map(|(n, _)| n.clone())).collect();
        assert_eq!(families.into_iter().collect::<Vec<_>>(), ["CoShell Sans", "Hack Nerd Font"]);
        let weights: Vec<u16> = db.faces().map(|f| f.weight.0).collect();
        assert_eq!(weights, [400, 500, 600, 700, 400, 700]);
    }
}
