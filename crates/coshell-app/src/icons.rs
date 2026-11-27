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
svg_handle!(PORT_FORWARDING, "features/port-forwarding.svg");
svg_handle!(SAFE_GUARD, "features/safe-guard.svg");
// Symbolic UI icons (drawn black, tinted where used).
svg_handle!(SEND, "icons/send.svg");
svg_handle!(STOP, "icons/stop.svg");
svg_handle!(COPY, "icons/copy.svg");
svg_handle!(KEY, "icons/key.svg");
svg_handle!(ZAP, "icons/zap.svg");
svg_handle!(TRASH, "icons/trash.svg");
svg_handle!(ERASER, "icons/eraser.svg");
// The dock's own icons.
svg_handle!(DOCK_TERMINAL, "icons/dock-terminal.svg");
svg_handle!(SPARKLES, "icons/sparkles.svg");
svg_handle!(GRID, "icons/grid.svg");
svg_handle!(SEARCH, "icons/search.svg");
svg_handle!(LOCK, "icons/lock.svg");
svg_handle!(LOCK_OPEN, "icons/lock-open.svg");
svg_handle!(PENCIL, "icons/pencil.svg");
svg_handle!(PLUS, "icons/plus.svg");
svg_handle!(CHEVRON_DOWN, "icons/chevron-down.svg");
svg_handle!(X, "icons/x.svg");
svg_handle!(CHECK, "icons/check.svg");
svg_handle!(PLUG, "icons/plug.svg");
svg_handle!(UNPLUG, "icons/unplug.svg");
svg_handle!(FOLDER, "icons/folder.svg");
svg_handle!(FOLDER_PLUS, "icons/folder-plus.svg");
svg_handle!(FILE, "icons/file.svg");
// What kind of file it is, in the file panel.
svg_handle!(FILE_CODE, "icons/file-code.svg");
svg_handle!(FILE_TEXT, "icons/file-text.svg");
svg_handle!(FILE_TERMINAL, "icons/file-terminal.svg");
svg_handle!(FILE_CONFIG, "icons/file-config.svg");
svg_handle!(FILE_ARCHIVE, "icons/file-archive.svg");
svg_handle!(FILE_IMAGE, "icons/file-image.svg");
svg_handle!(UPLOAD, "icons/upload.svg");
svg_handle!(DOWNLOAD, "icons/download.svg");
svg_handle!(REFRESH, "icons/refresh.svg");
svg_handle!(ARROW_UP, "icons/arrow-up.svg");
svg_handle!(ARROW_DOWN, "icons/arrow-down.svg");
svg_handle!(MINUS, "icons/minus.svg");
svg_handle!(CHEVRON_RIGHT, "icons/chevron-right.svg");
svg_handle!(EXTERNAL_LINK, "icons/external-link.svg");
svg_handle!(MORE, "icons/more.svg");
svg_handle!(PLAY, "icons/play.svg");
svg_handle!(PAPERCLIP, "icons/paperclip.svg");
svg_handle!(GIT_BRANCH, "icons/git-branch.svg");
// Marks for the system a shell runs on (Font Awesome Free brand icons, see assets/icons/README.md).
svg_handle!(OS_MAC, "icons/os-mac.svg");
svg_handle!(OS_WINDOWS, "icons/os-windows.svg");
svg_handle!(OS_LINUX, "icons/os-linux.svg");

/// Soft glows of colour on the terminal's background: small, and stretched to fit (it has no detail to lose).
pub static AURA: LazyLock<iced::widget::image::Handle> = LazyLock::new(|| iced::widget::image::Handle::from_bytes(asset!("png/aura.png").as_slice()));

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
