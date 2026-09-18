// SPDX-License-Identifier: MPL-2.0

//! Reading `GET /status/sessions` and turning it into [`Session`]s.
//!
//! Plex returns a large, loosely typed document per stream. We pick out the
//! handful of fields Now Plexing renders and drop the rest here, so the user
//! interface never sees Plex's wire format.

use super::model::{MediaKind, PlaybackState, Session};
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Deserialize)]
pub struct SessionsResponse {
    #[serde(rename = "MediaContainer")]
    media_container: MediaContainer,
}

#[derive(Debug, Deserialize)]
struct MediaContainer {
    /// Omitted entirely by Plex when nothing is playing, which is the common case.
    #[serde(default, rename = "Metadata")]
    metadata: Vec<Metadata>,
}

#[derive(Debug, Deserialize)]
struct Metadata {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    title: String,
    #[serde(default, rename = "grandparentTitle")]
    grandparent_title: Option<String>,
    #[serde(default, rename = "parentTitle")]
    parent_title: Option<String>,
    /// Episode or track number.
    #[serde(default)]
    index: Option<i64>,
    /// Season or disc number.
    #[serde(default, rename = "parentIndex")]
    parent_index: Option<i64>,
    #[serde(default)]
    year: Option<i64>,
    /// Total runtime in milliseconds.
    #[serde(default)]
    duration: Option<i64>,
    /// Playback position in milliseconds.
    #[serde(default, rename = "viewOffset")]
    view_offset: Option<i64>,
    #[serde(default, rename = "User")]
    user: Option<User>,
    #[serde(default, rename = "Player")]
    player: Option<Player>,
    #[serde(default, rename = "Session")]
    session: Option<SessionInfo>,
}

/// Plex's own record of the playback session, which is where its identifier
/// lives.
#[derive(Debug, Deserialize)]
struct SessionInfo {
    #[serde(default)]
    id: Option<String>,
    #[serde(default, rename = "sessionKey")]
    session_key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct User {
    #[serde(default)]
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Player {
    #[serde(default)]
    state: Option<String>,
}

impl SessionsResponse {
    #[must_use]
    pub fn into_sessions(self) -> Vec<Session> {
        self.media_container
            .metadata
            .iter()
            .map(Metadata::to_session)
            .collect()
    }
}

impl Metadata {
    fn to_session(&self) -> Session {
        let (primary_title, secondary_title) = self.titles();

        Session {
            id: self.identity(),
            user: self
                .user
                .as_ref()
                .and_then(|user| user.title.as_deref())
                .and_then(trimmed),
            primary_title,
            secondary_title,
            kind: self.media_kind(),
            state: self.playback_state(),
            position: millis(self.view_offset).unwrap_or_default(),
            duration: millis(self.duration),
        }
    }

    /// Something stable to recognise this stream by on the next poll.
    ///
    /// Plex keeps a session's id for the life of the stream. When it does not
    /// give us one, who-is-watching-what is stable enough across the few
    /// seconds between two polls.
    fn identity(&self) -> String {
        self.session
            .as_ref()
            .and_then(|session| {
                session
                    .id
                    .as_deref()
                    .or(session.session_key.as_deref())
                    .and_then(trimmed)
            })
            .unwrap_or_else(|| {
                let user = self.user.as_ref().and_then(|user| user.title.as_deref());
                format!("{}\u{1f}{}", user.unwrap_or_default(), self.title)
            })
    }

    /// The headline and the line beneath it, chosen per media type.
    fn titles(&self) -> (String, Option<String>) {
        let title = trimmed(&self.title);
        let show = self.grandparent_title.as_deref().and_then(trimmed);
        let album = self.parent_title.as_deref().and_then(trimmed);

        match self.media_kind() {
            // The year keeps a film's row the same height as an episode's.
            MediaKind::Movie => (
                title.unwrap_or_default(),
                self.year.map(|year| year.to_string()),
            ),

            // Plex nests an episode as show → season → episode, and the show is
            // what a glancing reader is looking for.
            MediaKind::Episode => match show {
                Some(show) => (show, join([self.episode_code(), title])),
                None => (title.unwrap_or_default(), self.episode_code()),
            },

            // For music the grandparent is the artist and the parent the album.
            MediaKind::Track => (title.unwrap_or_default(), join([show, album])),

            MediaKind::Other => (title.unwrap_or_default(), show),
        }
    }

    fn episode_code(&self) -> Option<String> {
        match (self.parent_index, self.index) {
            (Some(season), Some(episode)) => Some(format!("S{season:02}E{episode:02}")),
            (Some(season), None) => Some(format!("S{season:02}")),
            (None, Some(episode)) => Some(format!("E{episode:02}")),
            (None, None) => None,
        }
    }

    fn media_kind(&self) -> MediaKind {
        match self.kind.as_str() {
            "movie" => MediaKind::Movie,
            "episode" => MediaKind::Episode,
            "track" => MediaKind::Track,
            _ => MediaKind::Other,
        }
    }

    fn playback_state(&self) -> PlaybackState {
        match self
            .player
            .as_ref()
            .and_then(|player| player.state.as_deref())
        {
            Some("playing") => PlaybackState::Playing,
            Some("paused") => PlaybackState::Paused,
            _ => PlaybackState::Other,
        }
    }
}

fn trimmed(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// Join the parts that are present with a separator, or `None` if none are.
fn join<const N: usize>(parts: [Option<String>; N]) -> Option<String> {
    let parts: Vec<String> = parts.into_iter().flatten().collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// Plex reports times in milliseconds. Zero and negative values mean "unknown".
fn millis(value: Option<i64>) -> Option<Duration> {
    value
        .filter(|ms| *ms > 0)
        .map(|ms| Duration::from_millis(ms.unsigned_abs()))
}

#[cfg(test)]
mod tests {
    use super::SessionsResponse;
    use crate::plex::model::{MediaKind, PlaybackState, Session};
    use std::time::Duration;

    fn sessions(json: &str) -> Vec<Session> {
        serde_json::from_str::<SessionsResponse>(json)
            .expect("sessions fixture should deserialize")
            .into_sessions()
    }

    #[test]
    fn an_idle_server_omits_the_metadata_array_entirely() {
        // This is what every poll looks like when nobody is watching, so it has
        // to parse cleanly rather than raise a protocol error.
        let json = r#"{"MediaContainer":{"size":0}}"#;
        assert!(sessions(json).is_empty());
    }

    #[test]
    fn a_movie_shows_its_title_and_year() {
        let json = r#"{"MediaContainer":{"size":1,"Metadata":[{
          "type":"movie",
          "title":"Blade Runner 2049",
          "year":2017,
          "duration":9828000,
          "viewOffset":8262000,
          "User":{"id":"1","title":"Gunnar"},
          "Player":{"state":"playing","title":"Living Room"}
        }]}}"#;

        let session = &sessions(json)[0];
        assert_eq!(session.user.as_deref(), Some("Gunnar"));
        assert_eq!(session.primary_title, "Blade Runner 2049");
        assert_eq!(session.secondary_title.as_deref(), Some("2017"));
        assert_eq!(session.kind, MediaKind::Movie);
        assert_eq!(session.state, PlaybackState::Playing);
        assert_eq!(session.position, Duration::from_secs(8262));
        assert_eq!(session.duration, Some(Duration::from_secs(9828)));
    }

    #[test]
    fn an_episode_leads_with_the_show_and_labels_the_episode() {
        let json = r#"{"MediaContainer":{"size":1,"Metadata":[{
          "type":"episode",
          "title":"Woe's Hollow",
          "grandparentTitle":"Severance",
          "parentTitle":"Season 2",
          "index":4,
          "parentIndex":2,
          "duration":3064000,
          "viewOffset":1938000,
          "User":{"id":"2","title":"Terry"},
          "Player":{"state":"playing"}
        }]}}"#;

        let session = &sessions(json)[0];
        assert_eq!(session.primary_title, "Severance");
        assert_eq!(
            session.secondary_title.as_deref(),
            Some("S02E04 · Woe's Hollow")
        );
        assert_eq!(session.kind, MediaKind::Episode);
    }

    #[test]
    fn a_track_is_labelled_with_its_artist_and_album() {
        let json = r#"{"MediaContainer":{"size":1,"Metadata":[{
          "type":"track",
          "title":"Windowlicker",
          "grandparentTitle":"Aphex Twin",
          "parentTitle":"Windowlicker",
          "index":1,
          "duration":366000,
          "viewOffset":60000,
          "User":{"id":"1","title":"Gunnar"},
          "Player":{"state":"paused"}
        }]}}"#;

        let session = &sessions(json)[0];
        assert_eq!(session.primary_title, "Windowlicker");
        assert_eq!(
            session.secondary_title.as_deref(),
            Some("Aphex Twin · Windowlicker")
        );
        assert_eq!(session.kind, MediaKind::Track);
        assert_eq!(session.state, PlaybackState::Paused);
    }

    #[test]
    fn a_session_without_optional_metadata_still_parses() {
        // Everything Plex is allowed to omit, omitted at once.
        let json = r#"{"MediaContainer":{"Metadata":[{"type":"movie","title":"Unknown Film"}]}}"#;

        let session = &sessions(json)[0];
        assert_eq!(session.user, None);
        assert_eq!(session.primary_title, "Unknown Film");
        assert_eq!(session.secondary_title, None);
        assert_eq!(session.position, Duration::ZERO);
        assert_eq!(session.duration, None);
        assert_eq!(session.progress_after(Duration::ZERO), None);
        // An unreported player state must not be mistaken for playing.
        assert_eq!(session.state, PlaybackState::Other);
    }

    #[test]
    fn an_episode_missing_its_numbering_falls_back_to_the_episode_title() {
        let json = r#"{"MediaContainer":{"Metadata":[{
          "type":"episode","title":"Pilot","grandparentTitle":"Some Show"
        }]}}"#;

        let session = &sessions(json)[0];
        assert_eq!(session.primary_title, "Some Show");
        assert_eq!(session.secondary_title.as_deref(), Some("Pilot"));
    }

    #[test]
    fn an_episode_missing_its_show_does_not_repeat_its_own_title() {
        let json = r#"{"MediaContainer":{"Metadata":[{
          "type":"episode","title":"Pilot","index":1,"parentIndex":1
        }]}}"#;

        let session = &sessions(json)[0];
        assert_eq!(session.primary_title, "Pilot");
        assert_eq!(session.secondary_title.as_deref(), Some("S01E01"));
    }

    #[test]
    fn live_content_reports_a_position_but_no_duration() {
        let json = r#"{"MediaContainer":{"Metadata":[{
          "type":"clip","title":"BBC One","duration":0,"viewOffset":45000,
          "Player":{"state":"playing"}
        }]}}"#;

        let session = &sessions(json)[0];
        assert_eq!(session.kind, MediaKind::Other);
        assert_eq!(session.position, Duration::from_secs(45));
        assert_eq!(session.duration, None);
        assert_eq!(session.progress_after(Duration::ZERO), None);
    }

    #[test]
    fn blank_strings_are_treated_as_missing() {
        let json = r#"{"MediaContainer":{"Metadata":[{
          "type":"episode","title":"Ep","grandparentTitle":"   ",
          "User":{"title":"  "}
        }]}}"#;

        let session = &sessions(json)[0];
        assert_eq!(session.user, None);
        assert_eq!(session.primary_title, "Ep");
    }

    #[test]
    fn the_plex_session_id_identifies_a_stream() {
        let json = r#"{"MediaContainer":{"Metadata":[{
          "type":"movie","title":"A",
          "Session":{"id":"abc123","bandwidth":4000,"location":"lan"}
        }]}}"#;

        assert_eq!(sessions(json)[0].id, "abc123");
    }

    #[test]
    fn the_session_key_stands_in_when_there_is_no_id() {
        let json = r#"{"MediaContainer":{"Metadata":[{
          "type":"movie","title":"A","Session":{"sessionKey":"42"}
        }]}}"#;

        assert_eq!(sessions(json)[0].id, "42");
    }

    #[test]
    fn without_a_session_plex_gives_us_who_is_watching_what() {
        // Identity only has to survive the few seconds between two polls, so
        // the viewer and the title are enough to fall back on.
        let json = r#"{"MediaContainer":{"Metadata":[
          {"type":"movie","title":"A","User":{"title":"Gunnar"}},
          {"type":"movie","title":"B","User":{"title":"Gunnar"}},
          {"type":"movie","title":"A","User":{"title":"Terry"}}
        ]}}"#;

        let sessions = sessions(json);
        assert_ne!(sessions[0].id, sessions[1].id);
        assert_ne!(sessions[0].id, sessions[2].id);
    }

    #[test]
    fn every_concurrent_stream_is_reported() {
        let json = r#"{"MediaContainer":{"size":2,"Metadata":[
          {"type":"movie","title":"A","Player":{"state":"playing"}},
          {"type":"movie","title":"B","Player":{"state":"paused"}}
        ]}}"#;

        let sessions = sessions(json);
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions.iter().filter(|s| s.is_playing()).count(), 1);
    }
}
