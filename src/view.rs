// SPDX-License-Identifier: MPL-2.0

//! Everything the applet draws.
//!
//! These functions take domain values and produce widgets. They hold no state
//! and never talk to Plex, which keeps layout decisions out of [`crate::app`]
//! and localization out of [`crate::plex`].

use crate::app::{Message, Status};
use crate::fl;
use crate::plex::{PlaybackState, PlexError, Session, format_duration};
use crate::secret::Secret;
use cosmic::cosmic_theme::Spacing;
use cosmic::iced::{Alignment, Length};
use cosmic::prelude::*;
use cosmic::{applet, theme, widget};

/// Our own mark, tinted by the panel's foreground colour at draw time.
const PANEL_ICON: &[u8] = include_bytes!("../resources/icons/now-plexing-symbolic.svg");

/// Tall lists scroll rather than growing the popup without bound.
const MAX_SESSION_LIST_HEIGHT: f32 = 360.0;

/// Whether the token field masks what it holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenVisibility {
    Hidden,
    Revealed,
}

impl TokenVisibility {
    #[must_use]
    pub fn toggled(self) -> Self {
        match self {
            Self::Hidden => Self::Revealed,
            Self::Revealed => Self::Hidden,
        }
    }

    fn is_hidden(self) -> bool {
        self == Self::Hidden
    }
}

/// The button that lives in the panel: our icon, and how many people are
/// watching right now.
pub fn panel<'a>(applet: &applet::Context, status: &Status) -> Element<'a, Message> {
    let icon_size = applet.suggested_size(true).0;
    let (along_panel, across_panel) = applet.suggested_padding(true);

    let mut children: Vec<Element<'a, Message>> = vec![
        widget::icon::from_svg_bytes(PANEL_ICON)
            .symbolic(true)
            .icon()
            .size(icon_size)
            .into(),
    ];

    // Only a real answer earns a number; "no token" and "unreachable" would be
    // lying if they showed 0.
    if let Status::Ready(sessions) = status {
        let playing = sessions
            .iter()
            .filter(|session| session.is_playing())
            .count();
        children.push(applet.text(playing.to_string()).into());
    }

    let (content, padding): (Element<'a, Message>, [u16; 2]) = if applet.is_horizontal() {
        (
            widget::row::with_children(children)
                .spacing(across_panel)
                .align_y(Alignment::Center)
                .into(),
            [across_panel, along_panel],
        )
    } else {
        (
            widget::column::with_children(children)
                .spacing(across_panel)
                .align_x(Alignment::Center)
                .into(),
            [along_panel, across_panel],
        )
    };

    // `autosize_window` lets the button grow and shrink as the count changes
    // width, which a fixed-size applet button cannot do.
    applet
        .autosize_window(
            widget::button::custom(content)
                .on_press_down(Message::TogglePopup)
                .class(theme::Button::AppletIcon)
                .padding(padding),
        )
        .into()
}

/// The popup's default screen: who is watching what.
pub fn sessions<'a>(status: &Status) -> Element<'a, Message> {
    let Spacing {
        space_xxs, space_s, ..
    } = theme::active().cosmic().spacing;

    let header = header_row(
        widget::text::heading(fl!("app-title")),
        widget::button::icon(widget::icon::from_name("emblem-system-symbolic").size(16))
            .on_press(Message::ShowSettings),
    );

    let body: Element<'a, Message> = match status {
        Status::NoToken => notice(fl!("error-no-token")),
        Status::Loading => notice(fl!("loading")),
        Status::Failed(error) => notice(describe(*error)),
        Status::Ready(sessions) if sessions.is_empty() => notice(fl!("nothing-playing")),
        Status::Ready(sessions) => {
            widget::column::with_children(sessions.iter().map(session_row).collect::<Vec<_>>())
                .spacing(space_s)
                .apply(widget::scrollable)
                .apply(widget::container)
                .max_height(MAX_SESSION_LIST_HEIGHT)
                .into()
        }
    };

    widget::column::with_children(vec![
        applet::padded_control(header).into(),
        applet::padded_control(widget::divider::horizontal::default()).into(),
        body,
    ])
    .spacing(space_xxs)
    .padding([space_xxs, 0])
    .into()
}

/// The popup's other screen. Replaces the session list in place rather than
/// opening a second window.
pub fn settings<'a>(token: &Secret, visibility: TokenVisibility) -> Element<'a, Message> {
    let Spacing { space_xxs, .. } = theme::active().cosmic().spacing;

    let header = widget::row::with_children(vec![
        widget::button::icon(widget::icon::from_name("go-previous-symbolic").size(16))
            .on_press(Message::ShowSessions)
            .into(),
        widget::text::heading(fl!("settings")).into(),
    ])
    .spacing(space_xxs)
    .align_y(Alignment::Center);

    // `secure_input` obscures the value and offers the reveal toggle itself, so
    // there is no hand-rolled password handling here.
    let field = widget::text_input::secure_input(
        fl!("plex-token-placeholder"),
        token.expose().to_owned(),
        Some(Message::ToggleTokenVisibility),
        visibility.is_hidden(),
    )
    .label(fl!("plex-token"))
    .helper_text(fl!("plex-token-help"))
    .on_input(|value| Message::TokenInput(Secret::from(value)))
    .on_submit(|_| Message::SaveToken);

    let save = widget::button::suggested(fl!("save")).on_press(Message::SaveToken);

    widget::column::with_children(vec![
        applet::padded_control(header).into(),
        applet::padded_control(widget::divider::horizontal::default()).into(),
        applet::padded_control(field).into(),
        applet::padded_control(widget::row::with_children(vec![
            widget::space::horizontal().into(),
            save.into(),
        ]))
        .into(),
    ])
    .spacing(space_xxs)
    .padding([space_xxs, 0])
    .into()
}

fn header_row<'a>(
    title: impl Into<Element<'a, Message>>,
    action: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    widget::row::with_children(vec![
        title.into(),
        widget::space::horizontal().into(),
        action.into(),
    ])
    .align_y(Alignment::Center)
    .into()
}

/// One stream: who, what, and how far in.
fn session_row<'a>(session: &Session) -> Element<'a, Message> {
    let Spacing { space_xxxs, .. } = theme::active().cosmic().spacing;

    let mut lines: Vec<Element<'a, Message>> = Vec::with_capacity(5);

    lines.push(
        widget::text::caption(session.user.clone().unwrap_or_else(|| fl!("unknown-user"))).into(),
    );
    lines.push(widget::text::body(session.primary_title.clone()).into());

    if let Some(secondary) = &session.secondary_title {
        lines.push(widget::text::caption(secondary.clone()).into());
    }

    // Live content has no end, so there is no meaningful bar to draw.
    if let Some(progress) = session.progress() {
        lines.push(
            widget::determinate_linear(progress)
                .width(Length::Fill)
                .into(),
        );
    }

    lines.push(playback_times(session));

    applet::padded_control(widget::column::with_children(lines).spacing(space_xxxs)).into()
}

/// `2:17:42 / 2:43:48`, preceded by a pause glyph when the stream is paused.
fn playback_times<'a>(session: &Session) -> Element<'a, Message> {
    let elapsed = format_duration(session.position);
    let label = match session.duration {
        Some(total) => format!("{elapsed} / {}", format_duration(total)),
        None => elapsed,
    };

    let mut parts: Vec<Element<'a, Message>> = Vec::with_capacity(2);
    if session.state == PlaybackState::Paused {
        // An icon rather than a word, so there is nothing extra to translate.
        parts.push(
            widget::icon::from_name("media-playback-pause-symbolic")
                .size(12)
                .into(),
        );
    }
    parts.push(widget::text::caption(label).into());

    widget::row::with_children(parts)
        .spacing(4)
        .align_y(Alignment::Center)
        .into()
}

/// A single centred line, used for every state that is not a session list.
fn notice<'a>(message: String) -> Element<'a, Message> {
    applet::padded_control(
        widget::text::body(message)
            .width(Length::Fill)
            .align_x(Alignment::Center),
    )
    .into()
}

fn describe(error: PlexError) -> String {
    match error {
        PlexError::Unauthorized => fl!("error-unauthorized"),
        PlexError::NoServer => fl!("error-no-server"),
        PlexError::Unreachable => fl!("error-unreachable"),
        PlexError::Protocol => fl!("error-protocol"),
    }
}
