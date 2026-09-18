// SPDX-License-Identifier: MPL-2.0

//! Application state and the update loop.
//!
//! The applet polls Plex on a fixed interval. Rendering lives in [`crate::view`]
//! and talking to Plex in [`crate::plex`]; this module owns the state that
//! connects them.

use crate::config::Config;
use crate::notify::{self, Alert, StreamWatcher};
use crate::plex::{PlexClient, PlexError, Server, Session};
use crate::secret::Secret;
use crate::view::{self, TokenVisibility};
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::iced::{Subscription, time, window::Id};
use cosmic::prelude::*;
use cosmic::surface::action::{LiveSettings, app_popup, destroy_popup};
use std::mem;
use std::time::Duration;

/// Plex reports playback positions in whole seconds, so there is nothing to
/// gain from asking more often than this.
const REFRESH_INTERVAL: Duration = Duration::from_secs(5);

/// Which screen the popup is currently showing. Both are drawn into the same
/// popup window rather than opening a second surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Sessions,
    Settings,
}

/// What the applet currently knows about Plex.
#[derive(Debug)]
pub enum Status {
    /// Nothing configured yet. We never touch the network in this state.
    NoToken,
    /// A first answer has not arrived.
    Loading,
    /// A successful query. An empty list means nobody is watching.
    Ready(Vec<Session>),
    Failed(PlexError),
}

pub struct AppModel {
    core: cosmic::Core,
    popup: Option<Id>,
    view_mode: ViewMode,
    config: Config,
    config_handle: Option<cosmic_config::Config>,
    plex: PlexClient,
    /// The server we last reached, reused so that a poll is normally one
    /// request rather than a round trip through plex.tv.
    server: Option<Server>,
    status: Status,
    /// Guards against a slow Plex accumulating a backlog of polls.
    refreshing: bool,
    /// Draft token in the settings view, separate from the saved one.
    token_input: Secret,
    token_visibility: TokenVisibility,
    /// Spots when the stream count crosses the alert level.
    watcher: StreamWatcher,
    /// Id of the notification we last posted, so the next one replaces it
    /// instead of stacking. Zero means there is nothing to replace.
    notification_id: u32,
}

#[derive(Debug, Clone)]
pub enum Message {
    TogglePopup,
    PopupClosed(Id),
    UpdateConfig(Config),
    Tick,
    Refreshed(Result<(Server, Vec<Session>), PlexError>),
    ShowSettings,
    ShowSessions,
    TokenInput(Secret),
    ToggleTokenVisibility,
    SaveToken,
    ToggleAlerts(bool),
    AlertLevel(u32),
    Notified(Option<u32>),
}

impl AppModel {
    /// Ask Plex for the current sessions, unless there is nothing to ask with
    /// or a request is already in flight.
    fn refresh(&mut self) -> Task<cosmic::Action<Message>> {
        if self.config.token.is_empty() {
            self.status = Status::NoToken;
            self.server = None;
            return Task::none();
        }

        if self.refreshing {
            return Task::none();
        }
        self.refreshing = true;

        let plex = self.plex.clone();
        let token = self.config.token.clone();
        let server = self.server.clone();

        cosmic::task::future(async move { Message::Refreshed(plex.poll(&token, server).await) })
    }

    /// Open the popup through libcosmic's surface system rather than by issuing
    /// a bare Wayland `get_popup`.
    ///
    /// This matters for more than style. Only surfaces created this way are
    /// registered with the runtime, and an unregistered surface is treated as
    /// "not a popup" — which makes libcosmic skip the compositor's background
    /// blur for it, so the popup renders opaque no matter what the system's
    /// frosted-glass setting says.
    fn open_popup(&mut self) -> Task<cosmic::Action<Message>> {
        let popup = cosmic::surface::surface_task(app_popup::<Self>(
            // Take the theme's own blur and corner behaviour for applet popups.
            |_: &Self| LiveSettings::default(),
            |app: &mut Self| {
                let id = Id::unique();
                app.popup = Some(id);
                app.view_mode = ViewMode::Sessions;

                // The popup's sizing comes from `popup_container`; overriding
                // the positioner's limits here would only contradict it.
                app.core.applet.get_popup_settings(
                    app.core.main_window_id().unwrap_or(Id::RESERVED),
                    id,
                    None,
                    None,
                    None,
                )
            },
            // No view closure: the popup is drawn by `view_window`.
            None,
        ));

        // Show something current immediately instead of waiting out the rest of
        // the polling interval.
        Task::batch([popup, self.refresh()])
    }

    fn save_token(&mut self) -> Task<cosmic::Action<Message>> {
        let token = mem::take(&mut self.token_input);

        match &self.config_handle {
            Some(handle) => {
                let _ = self.config.store_token(handle, token);
            }
            // Without a config handle the token still works for this run, it
            // just will not survive a restart.
            None => self.config.token = token,
        }

        // A different token may well mean a different account.
        self.server = None;
        self.view_mode = ViewMode::Sessions;
        self.status = Status::Loading;
        self.refresh()
    }

    /// Post a desktop notification for a crossing of the alert level.
    fn announce(&self, alert: Alert) -> Task<cosmic::Action<Message>> {
        let summary = view::describe_alert(alert);
        let replaces = self.notification_id;

        cosmic::task::future(
            async move { Message::Notified(notify::send(&summary, replaces).await) },
        )
    }
}

impl cosmic::Application for AppModel {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;

    const APP_ID: &'static str = crate::APP_ID;

    fn core(&self) -> &cosmic::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }

    fn init(
        core: cosmic::Core,
        _flags: Self::Flags,
    ) -> (Self, Task<cosmic::Action<Self::Message>>) {
        let config_handle = cosmic_config::Config::new(Self::APP_ID, Config::VERSION).ok();

        // On a first run the key files do not exist yet, so `get_entry`
        // reports errors while still handing back usable defaults. That is the
        // normal path here, not a failure.
        let mut config = config_handle
            .as_ref()
            .map_or_else(Config::default, |handle| {
                Config::get_entry(handle).unwrap_or_else(|(_errors, config)| config)
            });
        let client_id = config.ensure_client_id(config_handle.as_ref());

        let mut app = AppModel {
            core,
            popup: None,
            view_mode: ViewMode::Sessions,
            plex: PlexClient::new(&client_id),
            server: None,
            status: Status::Loading,
            refreshing: false,
            token_input: Secret::default(),
            token_visibility: TokenVisibility::Hidden,
            watcher: StreamWatcher::default(),
            notification_id: 0,
            config,
            config_handle,
        };

        let task = app.refresh();
        (app, task)
    }

    fn on_close_requested(&self, id: Id) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    fn view(&self) -> Element<'_, Self::Message> {
        view::panel(&self.core.applet, &self.status)
    }

    fn view_window(&self, _id: Id) -> Element<'_, Self::Message> {
        let content = match self.view_mode {
            ViewMode::Sessions => view::sessions(&self.status),
            ViewMode::Settings => {
                view::settings(&self.token_input, self.token_visibility, &self.config)
            }
        };

        self.core.applet.popup_container(content).into()
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        Subscription::batch([
            time::every(REFRESH_INTERVAL).map(|_| Message::Tick),
            self.core()
                .watch_config::<Config>(Self::APP_ID)
                .map(|update| Message::UpdateConfig(update.config)),
        ])
    }

    fn update(&mut self, message: Self::Message) -> Task<cosmic::Action<Self::Message>> {
        match message {
            Message::TogglePopup => {
                return match self.popup.take() {
                    Some(id) => cosmic::surface::surface_task(destroy_popup(id)),
                    None => self.open_popup(),
                };
            }

            Message::PopupClosed(id) => {
                if self.popup == Some(id) {
                    self.popup = None;
                    self.token_input = Secret::default();
                }
            }

            Message::UpdateConfig(config) => {
                let token_changed = config.token != self.config.token;
                self.config = config;
                if token_changed {
                    self.server = None;
                    return self.refresh();
                }
            }

            Message::Tick => return self.refresh(),

            Message::Refreshed(result) => {
                self.refreshing = false;
                match result {
                    Ok((server, sessions)) => {
                        let playing = sessions.iter().filter(|s| s.is_playing()).count();
                        // Only successful polls reach the watcher, so the count
                        // from before an outage is what recovery compares to.
                        let alert = self.watcher.observe(playing, self.config.alert_level());

                        self.server = Some(server);
                        self.status = Status::Ready(sessions);

                        if let Some(alert) = alert {
                            return self.announce(alert);
                        }
                    }
                    // Keep the cached server: a blip is usually transient, and
                    // retrying a known address beats rediscovery every cycle.
                    Err(error) => self.status = Status::Failed(error),
                }
            }

            Message::ShowSettings => {
                self.view_mode = ViewMode::Settings;
                self.token_input = self.config.token.clone();
                self.token_visibility = TokenVisibility::Hidden;
            }

            Message::ShowSessions => {
                self.view_mode = ViewMode::Sessions;
                self.token_input = Secret::default();
            }

            Message::TokenInput(token) => self.token_input = token,

            Message::ToggleTokenVisibility => {
                self.token_visibility = self.token_visibility.toggled();
            }

            Message::SaveToken => return self.save_token(),

            // Both alert settings apply the moment they change, so closing the
            // popup cannot silently discard them.
            Message::ToggleAlerts(enabled) => match &self.config_handle {
                Some(handle) => {
                    let _ = self.config.set_alerts_enabled(handle, enabled);
                }
                None => self.config.alerts_enabled = enabled,
            },

            Message::AlertLevel(level) => match &self.config_handle {
                Some(handle) => {
                    let _ = self.config.set_alert_threshold(handle, level);
                }
                None => self.config.alert_threshold = level,
            },

            Message::Notified(id) => {
                // Hold on to the previous id if the desktop refused, so we do
                // not lose the ability to replace what is already on screen.
                if let Some(id) = id {
                    self.notification_id = id;
                }
            }
        }

        Task::none()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}
