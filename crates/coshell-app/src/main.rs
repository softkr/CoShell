//! CoShell — terminal (iced) with local shells and SSH jump-chain sessions.

// Windows release builds are a GUI app: no console window next to it.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod ai;
mod app;
mod assist;
mod blocks;
mod branch;
mod metrics;
mod conn;
mod files;
mod form;
mod genie;
mod glow;
mod context_menu;
mod fonts;
mod hangul;
mod hosts;
mod i18n;
mod icons;
mod ime;
mod keys;
mod launch;
#[cfg(target_os = "macos")]
mod menu;
mod motion;
mod selectable;
mod settings;
mod terminal;
mod theme;
mod ui;
mod ui_ai;
mod ui_files;
mod ui_hosts;
mod ui_keys;
mod ui_servers;
mod ui_settings;
mod ui_sidebar;
mod ui_status;
mod ui_term;
mod ui_tunnel;
mod update;
#[cfg(test)]
mod tests;

use std::sync::Mutex;

use app::CoShell;
use launch::Launch;

/// App version (workspace `version` in Cargo.toml).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// No native title bar on any platform: the header draws the brand, the session pill and
/// its own minimise / maximise / close buttons, and handles dragging and zoom.
fn window_settings(size: settings::WindowSize) -> iced::window::Settings {
    iced::window::Settings {
        size: iced::Size::new(size.width, size.height),
        icon: icons::window_icon(),
        // The system's own window controls. On macOS they sit in the app's header
        // (transparent title bar); elsewhere the usual title bar.
        #[cfg(target_os = "macos")]
        platform_specific: iced::window::settings::PlatformSpecific { title_hidden: true, titlebar_transparent: true, fullsize_content_view: true },
        // The sidebar lets the blurred desktop show through, like Finder's; every other
        // part of the view paints an opaque background.
        transparent: cfg!(target_os = "macos"),
        blur: cfg!(target_os = "macos"),
        ..Default::default()
    }
}

fn main() -> iced::Result {
    let settings_path = settings::Settings::default_path();
    // A broken settings file must not block startup: defaults + a warning in the status bar.
    let (prefs, broken) = match settings_path.as_deref().map(settings::Settings::load) {
        Some(Ok(s)) => (s, None),
        Some(Err(e)) => (settings::Settings::default(), Some(e)),
        None => (settings::Settings::default(), None),
    };
    // Before anything is worded for the user (command-line errors included).
    i18n::set(i18n::resolve(prefs.language));
    let warning = broken.map(|e| i18n::tf("설정 읽기 실패: {0}", &[&e]));

    let args: Vec<String> = std::env::args().skip(1).collect();
    let user = login_user();
    let launch = match launch::parse(&args, &user, coshell_ssh::ConnectOptions::default_known_hosts()) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let window = prefs.window;
    let boot = Mutex::new(Some((launch, prefs, warning)));
    iced::application(
        move || {
            let (launch, prefs, warning) = boot
                .lock()
                .ok()
                .and_then(|mut b| b.take())
                .unwrap_or((Launch::Local, settings::Settings::default(), None));
            CoShell::boot(launch, prefs, settings_path.clone(), warning)
        },
        CoShell::update,
        ui::view,
    )
    .title(CoShell::title)
    .font(fonts::FILES[0])
    .font(fonts::FILES[1])
    .font(fonts::FILES[2])
    .font(fonts::FILES[3])
    .font(fonts::FILES[4])
    .font(fonts::FILES[5])
    .subscription(CoShell::subscription)
    .window(window_settings(window))
    .default_font(fonts::SANS)
    // Smooth edges for canvas shapes (the chat's folding status mark).
    .antialiasing(true)
    .theme(|_: &CoShell| iced::Theme::Dark)
    // The window background and default text colour. Clear on macOS, where the sidebar is
    // see-through; ours rather than the stock grey elsewhere.
    .style(|_: &CoShell, _: &iced::Theme| iced::theme::Style {
        background_color: if cfg!(target_os = "macos") { iced::Color::TRANSPARENT } else { theme::BG_1 },
        text_color: theme::TEXT,
    })
    .run()
}

/// The local account name (`USER` on macOS/Linux, `USERNAME` on Windows): the default SSH user.
pub(crate) fn login_user() -> String {
    std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_else(|_| "root".into())
}
