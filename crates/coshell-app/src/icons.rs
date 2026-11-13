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
// Symbolic UI icons (drawn black, tinted where used).
svg_handle!(NEW_CHAT, "icons/new-chat.svg");
svg_handle!(PAPERCLIP, "icons/paperclip.svg");
svg_handle!(SEND, "icons/send.svg");
svg_handle!(STOP, "icons/stop.svg");
svg_handle!(COPY, "icons/copy.svg");
svg_handle!(KEY, "icons/key.svg");
svg_handle!(PLAY, "icons/play.svg");
svg_handle!(ZAP, "icons/zap.svg");
svg_handle!(TRASH, "icons/trash.svg");

/// 256px PNG for the window / taskbar icon (Windows, Linux; macOS uses the bundle's .icns).
const WINDOW_ICON_PNG: &[u8] = asset!("png/coshell-256.png");

/// A square icon element.
pub fn icon<'a, Message: 'a>(handle: &LazyLock<Handle>, size: f32) -> Element<'a, Message> {
    svg::Svg::new(LazyLock::force(handle).clone()).width(Length::Fixed(size)).height(Length::Fixed(size)).into()
}

/// A symbolic icon drawn in `color`.
pub fn tinted<'a, Message: 'a>(handle: &LazyLock<Handle>, size: f32, color: iced::Color) -> Element<'a, Message> {
    svg::Svg::new(LazyLock::force(handle).clone())
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .style(move |_, _| svg::Style { color: Some(color) })
        .into()
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
