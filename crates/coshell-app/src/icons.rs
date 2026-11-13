//! Brand and feature icons from `assets/` (embedded at build time).

use std::sync::LazyLock;

use iced::widget::svg::{self, Handle};
use iced::{Element, Length};

macro_rules! asset {
    ($path:literal) => {
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/", $path))
    };
}

macro_rules! svg_handle {
    ($name:ident, $path:literal) => {
        pub static $name: LazyLock<Handle> = LazyLock::new(|| Handle::from_memory(asset!($path).as_slice()));
    };
}

svg_handle!(APP, "svg/coshell-app.svg");
svg_handle!(AI_SPARK, "features/ai-spark.svg");
svg_handle!(PORT_FORWARDING, "features/port-forwarding.svg");
svg_handle!(SAFE_GUARD, "features/safe-guard.svg");
svg_handle!(API_BRIDGE, "features/api-bridge.svg");

/// 256px PNG for the window / taskbar icon (Windows, Linux; macOS uses the bundle's .icns).
const WINDOW_ICON_PNG: &[u8] = asset!("png/coshell-256.png");

/// A square icon element.
pub fn icon<'a, Message: 'a>(handle: &LazyLock<Handle>, size: f32) -> Element<'a, Message> {
    svg::Svg::new(LazyLock::force(handle).clone()).width(Length::Fixed(size)).height(Length::Fixed(size)).into()
}

pub fn window_icon() -> Option<iced::window::Icon> {
    iced::window::icon::from_file_data(WINDOW_ICON_PNG, None).ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn window_icon_decodes() {
        let icon = super::window_icon().expect("embedded PNG decodes");
        let (_, size) = icon.into_raw();
        assert_eq!((size.width, size.height), (256, 256));
    }
}
