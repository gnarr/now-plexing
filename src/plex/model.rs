// SPDX-License-Identifier: MPL-2.0

//! What Now Plexing needs to know about Plex, and nothing more.
//!
//! These types are what the user interface renders. They carry no serde
//! attributes and know nothing about Plex's wire format; the mapping lives in
//! [`super::resources`] and [`super::sessions`].

use crate::secret::Secret;
use std::time::Duration;

/// A Plex Media Server we have picked, together with the connection we reached
/// it on and the token that server issued us.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    pub name: String,
    /// Origin of the chosen connection, without a trailing slash.
    pub base_url: String,
    /// Server-specific access token. Not necessarily the account token.
    pub token: Secret,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Movie,
    Episode,
    Track,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    Playing,
    Paused,
    /// Buffering, or anything else Plex reports that we do not model.
    Other,
}

/// One active playback session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// Absent when Plex did not name the viewer; the view supplies a
    /// localized stand-in rather than the domain inventing English text.
    pub user: Option<String>,
    /// The headline: a film's title, a show's name, a track's title.
    pub primary_title: String,
    /// Optional context under the headline, such as `S02E04 · Woe's Hollow`.
    pub secondary_title: Option<String>,
    pub kind: MediaKind,
    pub state: PlaybackState,
    pub position: Duration,
    /// Absent for live content, which has no end.
    pub duration: Option<Duration>,
}

impl Session {
    /// How far through the item playback is, as `0.0..=1.0`.
    ///
    /// `None` when there is nothing meaningful to draw — live content, or an
    /// item Plex has not reported a duration for.
    #[must_use]
    pub fn progress(&self) -> Option<f32> {
        let duration = self.duration?.as_secs_f32();
        if duration <= 0.0 {
            return None;
        }
        Some((self.position.as_secs_f32() / duration).clamp(0.0, 1.0))
    }

    #[must_use]
    pub fn is_playing(&self) -> bool {
        self.state == PlaybackState::Playing
    }
}

/// Render a playback time the way media players do: `32:18` under an hour,
/// `2:17:42` at or above one.
#[must_use]
pub fn format_duration(duration: Duration) -> String {
    let total = duration.as_secs();
    let (hours, minutes, seconds) = (total / 3600, (total % 3600) / 60, total % 60);

    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::{MediaKind, PlaybackState, Session, format_duration};
    use std::time::Duration;

    fn session(position_secs: u64, duration: Option<u64>) -> Session {
        Session {
            user: Some("Gunnar".to_owned()),
            primary_title: "Blade Runner 2049".to_owned(),
            secondary_title: None,
            kind: MediaKind::Movie,
            state: PlaybackState::Playing,
            position: Duration::from_secs(position_secs),
            duration: duration.map(Duration::from_secs),
        }
    }

    #[test]
    fn progress_is_the_fraction_watched() {
        let progress = session(50, Some(200)).progress().unwrap();
        assert!((progress - 0.25).abs() < f32::EPSILON);
    }

    #[test]
    fn progress_is_absent_without_a_usable_duration() {
        assert_eq!(session(50, None).progress(), None);
        assert_eq!(session(50, Some(0)).progress(), None);
    }

    #[test]
    fn progress_is_clamped_when_plex_overshoots() {
        // Plex can report a viewOffset slightly past the end of the item.
        assert_eq!(session(210, Some(200)).progress(), Some(1.0));
    }

    #[test]
    fn durations_gain_an_hours_field_only_when_needed() {
        assert_eq!(format_duration(Duration::from_secs(0)), "0:00");
        assert_eq!(format_duration(Duration::from_secs(9)), "0:09");
        assert_eq!(format_duration(Duration::from_secs(1938)), "32:18");
        assert_eq!(format_duration(Duration::from_secs(3599)), "59:59");
        assert_eq!(format_duration(Duration::from_hours(1)), "1:00:00");
        assert_eq!(format_duration(Duration::from_secs(8262)), "2:17:42");
    }
}
