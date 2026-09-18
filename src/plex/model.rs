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
    /// Identifies this stream across polls, so its playback can be followed
    /// from one reading to the next.
    pub id: String,
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
    /// Where playback has reached `elapsed` after this reading was taken.
    ///
    /// Plex is polled every few seconds, so between polls a playing stream is
    /// carried forward in real time rather than sitting still until the next
    /// answer arrives. Anything not actually playing stays exactly where Plex
    /// left it.
    #[must_use]
    pub fn position_after(&self, elapsed: Duration) -> Duration {
        if !self.is_playing() {
            return self.position;
        }

        let position = self.position.saturating_add(elapsed);
        match self.duration {
            // Never run past the end of the item while waiting for the poll
            // that will tell us it stopped.
            Some(duration) => position.min(duration),
            None => position,
        }
    }

    /// How far through the item playback is, as `0.0..=1.0`.
    ///
    /// `None` when there is nothing meaningful to draw — live content, or an
    /// item Plex has not reported a duration for.
    #[must_use]
    pub fn progress_after(&self, elapsed: Duration) -> Option<f32> {
        let duration = self.duration?.as_secs_f32();
        if duration <= 0.0 {
            return None;
        }
        Some((self.position_after(elapsed).as_secs_f32() / duration).clamp(0.0, 1.0))
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
            id: "session-1".to_owned(),
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
        let progress = session(50, Some(200))
            .progress_after(Duration::ZERO)
            .unwrap();
        assert!((progress - 0.25).abs() < f32::EPSILON);
    }

    #[test]
    fn progress_is_absent_without_a_usable_duration() {
        assert_eq!(session(50, None).progress_after(Duration::ZERO), None);
        assert_eq!(session(50, Some(0)).progress_after(Duration::ZERO), None);
    }

    #[test]
    fn progress_is_clamped_when_plex_overshoots() {
        // Plex can report a viewOffset slightly past the end of the item.
        assert_eq!(
            session(210, Some(200)).progress_after(Duration::ZERO),
            Some(1.0)
        );
    }

    #[test]
    fn a_playing_stream_carries_forward_between_polls() {
        // Plex is only asked every few seconds; the display should not freeze
        // in between.
        let session = session(50, Some(200));
        assert_eq!(
            session.position_after(Duration::from_secs(3)),
            Duration::from_secs(53)
        );
    }

    #[test]
    fn a_paused_stream_stays_where_plex_left_it() {
        let mut session = session(50, Some(200));
        session.state = PlaybackState::Paused;

        assert_eq!(
            session.position_after(Duration::from_secs(3)),
            Duration::from_secs(50)
        );
    }

    #[test]
    fn an_unrecognised_state_is_not_assumed_to_be_playing() {
        let mut session = session(50, Some(200));
        session.state = PlaybackState::Other;

        assert_eq!(
            session.position_after(Duration::from_secs(3)),
            Duration::from_secs(50)
        );
    }

    #[test]
    fn carrying_forward_never_runs_past_the_end() {
        // A stream that finishes between polls must not show a time beyond its
        // own duration, or a progress bar past full.
        let session = session(195, Some(200));

        assert_eq!(
            session.position_after(Duration::from_secs(30)),
            Duration::from_secs(200)
        );
        assert_eq!(session.progress_after(Duration::from_secs(30)), Some(1.0));
    }

    #[test]
    fn live_content_has_no_end_to_stop_at() {
        let session = session(50, None);
        assert_eq!(
            session.position_after(Duration::from_secs(30)),
            Duration::from_secs(80)
        );
    }

    #[test]
    fn progress_advances_with_the_carried_position() {
        let session = session(50, Some(200));
        let still = session.progress_after(Duration::ZERO).unwrap();
        let later = session.progress_after(Duration::from_secs(50)).unwrap();

        assert!((still - 0.25).abs() < f32::EPSILON);
        assert!((later - 0.50).abs() < f32::EPSILON);
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
