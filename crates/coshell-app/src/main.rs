//! CoShell — terminal (iced) with local shells and SSH jump-chain sessions.

mod ai;
mod app;
mod assist;
mod conn;
mod hosts;
mod i18n;
mod icons;
mod launch;
#[cfg(target_os = "macos")]
mod menu;
mod settings;
mod terminal;
mod theme;
mod ui;
mod ui_ai;
mod ui_hosts;
mod ui_servers;
mod ui_settings;
mod ui_tunnel;
#[cfg(test)]
mod tests;

use std::sync::Mutex;

use app::CoShell;
use launch::Launch;

/// App version (workspace `version` in Cargo.toml).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// macOS: hide the native title bar and draw our tab bar in its place
/// (traffic lights stay; the tab bar handles dragging / zoom).
fn window_settings(size: settings::WindowSize) -> iced::window::Settings {
    iced::window::Settings {
        size: iced::Size::new(size.width, size.height),
        icon: icons::window_icon(),
        #[cfg(target_os = "macos")]
        platform_specific: iced::window::settings::PlatformSpecific {
            title_hidden: true,
            titlebar_transparent: true,
            fullsize_content_view: true,
        },
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
    let user = std::env::var("USER").unwrap_or_else(|_| "root".into());
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
    .subscription(CoShell::subscription)
    .window(window_settings(window))
    .theme(|_: &CoShell| iced::Theme::Dark)
    .run()
}
