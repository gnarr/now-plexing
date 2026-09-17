// SPDX-License-Identifier: MPL-2.0

mod app;
mod config;
mod i18n;
mod notify;
mod plex;
mod secret;
mod view;

/// Reverse-domain identifier, shared by the desktop entry, the COSMIC
/// configuration directory and the applet itself.
pub const APP_ID: &str = "com.github.gnarr.now-plexing";

fn main() -> cosmic::iced::Result {
    // Get the system's preferred languages.
    let requested_languages = i18n_embed::DesktopLanguageRequester::requested_languages();

    // Enable localizations to be applied.
    i18n::init(&requested_languages);

    // Starts the applet's event loop with `()` as the application's flags.
    cosmic::applet::run::<app::AppModel>(())
}
