// SPDX-License-Identifier: MPL-2.0

//! Keeping playback timers moving forward smoothly between polls.
//!
//! Plex clients report their position to the server roughly every ten seconds,
//! but we ask the server every five. Two consecutive polls therefore often
//! carry the *same* reading, and treating each poll as a fresh one makes the
//! display count up, snap back, and count up again:
//!
//! ```text
//! poll 1   4:10   →  ticks to 4:15
//! poll 2   4:10   →  snaps back, ticks to 4:15 again
//! poll 3   4:20   →  jumps forward
//! ```
//!
//! The fix is to anchor on the moment a reading *changed* rather than the
//! moment it arrived. A repeated reading keeps its original anchor, so the
//! stream carries on from where the display had already reached.

use crate::plex::Session;
use crate::plex::model::PlaybackState;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// The last distinct reading seen for one stream, and when it first appeared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Anchor {
    position: Duration,
    state: PlaybackState,
    since: Instant,
}

/// Remembers, per stream, when its reported position last actually moved.
#[derive(Debug, Default)]
pub struct PlaybackAnchors {
    anchors: HashMap<String, Anchor>,
}

impl PlaybackAnchors {
    /// Bring a freshly polled set of sessions up to date.
    ///
    /// Each playing stream is advanced by however long its reading has already
    /// been stale, so the position handed to the interface is an estimate of
    /// where playback stands *now* rather than whenever Plex last refreshed it.
    /// Streams that have ended are forgotten.
    pub fn reconcile(&mut self, sessions: &mut [Session], now: Instant) {
        let mut current = HashMap::with_capacity(sessions.len());

        for session in sessions.iter_mut() {
            let reading = (session.position, session.state);

            let since = match self.anchors.get(&session.id) {
                // Plex is repeating a reading it has not refreshed yet, so the
                // stream has really moved on since this reading first appeared.
                Some(anchor) if (anchor.position, anchor.state) == reading => anchor.since,
                // Either a new stream, or one whose position or state finally
                // changed. Start timing from here.
                _ => now,
            };

            current.insert(
                session.id.clone(),
                Anchor {
                    position: session.position,
                    state: session.state,
                    since,
                },
            );

            // `position_after` leaves anything that is not playing alone and
            // refuses to run past the end of the item.
            session.position = session.position_after(now.saturating_duration_since(since));
        }

        self.anchors = current;
    }
}

#[cfg(test)]
mod tests {
    use super::PlaybackAnchors;
    use crate::plex::Session;
    use crate::plex::model::{MediaKind, PlaybackState};
    use std::time::{Duration, Instant};

    fn session(id: &str, position_secs: u64, state: PlaybackState) -> Session {
        Session {
            id: id.to_owned(),
            user: Some("Gunnar".to_owned()),
            primary_title: "Blade Runner 2049".to_owned(),
            secondary_title: None,
            kind: MediaKind::Movie,
            state,
            position: Duration::from_secs(position_secs),
            duration: Some(Duration::from_secs(10_000)),
        }
    }

    fn playing(id: &str, position_secs: u64) -> Session {
        session(id, position_secs, PlaybackState::Playing)
    }

    fn secs(session: &Session) -> u64 {
        session.position.as_secs()
    }

    #[test]
    fn a_repeated_reading_keeps_counting_instead_of_snapping_back() {
        // The bug this module exists for: Plex refreshes every ten seconds
        // while we poll every five, so the middle poll repeats the reading.
        let start = Instant::now();
        let mut anchors = PlaybackAnchors::default();

        let mut first = [playing("a", 250)];
        anchors.reconcile(&mut first, start);
        assert_eq!(secs(&first[0]), 250);

        // Five seconds later Plex still says 4:10 — but five seconds have
        // genuinely passed, so the display must read 4:15, not snap back.
        let mut second = [playing("a", 250)];
        anchors.reconcile(&mut second, start + Duration::from_secs(5));
        assert_eq!(secs(&second[0]), 255);

        // At ten seconds Plex catches up, and its own figure agrees with ours.
        let mut third = [playing("a", 260)];
        anchors.reconcile(&mut third, start + Duration::from_secs(10));
        assert_eq!(secs(&third[0]), 260);
    }

    #[test]
    fn a_stale_reading_keeps_accruing_across_several_polls() {
        let start = Instant::now();
        let mut anchors = PlaybackAnchors::default();

        for elapsed in [0, 5, 10, 15] {
            let mut sessions = [playing("a", 100)];
            anchors.reconcile(&mut sessions, start + Duration::from_secs(elapsed));
            assert_eq!(secs(&sessions[0]), 100 + elapsed);
        }
    }

    #[test]
    fn a_paused_stream_does_not_creep_forward() {
        let start = Instant::now();
        let mut anchors = PlaybackAnchors::default();

        let mut first = [session("a", 250, PlaybackState::Paused)];
        anchors.reconcile(&mut first, start);

        let mut later = [session("a", 250, PlaybackState::Paused)];
        anchors.reconcile(&mut later, start + Duration::from_mins(1));
        assert_eq!(secs(&later[0]), 250);
    }

    #[test]
    fn resuming_restarts_the_clock_rather_than_leaping_forward() {
        // The position is unchanged across the pause, so only the state tells
        // us this is a new reading. Without that check the stream would jump
        // forward by the entire length of the pause.
        let start = Instant::now();
        let mut anchors = PlaybackAnchors::default();

        let mut paused = [session("a", 250, PlaybackState::Paused)];
        anchors.reconcile(&mut paused, start);

        let mut resumed = [playing("a", 250)];
        anchors.reconcile(&mut resumed, start + Duration::from_mins(1));
        assert_eq!(secs(&resumed[0]), 250);
    }

    #[test]
    fn seeking_backwards_is_followed_immediately() {
        let start = Instant::now();
        let mut anchors = PlaybackAnchors::default();

        let mut before = [playing("a", 250)];
        anchors.reconcile(&mut before, start);

        let mut after = [playing("a", 30)];
        anchors.reconcile(&mut after, start + Duration::from_secs(5));
        assert_eq!(secs(&after[0]), 30);
    }

    #[test]
    fn streams_are_tracked_independently() {
        let start = Instant::now();
        let mut anchors = PlaybackAnchors::default();

        let mut first = [playing("a", 100), playing("b", 900)];
        anchors.reconcile(&mut first, start);

        // Only "a" repeats its reading; "b" has moved on.
        let mut second = [playing("a", 100), playing("b", 910)];
        anchors.reconcile(&mut second, start + Duration::from_secs(5));
        assert_eq!(secs(&second[0]), 105);
        assert_eq!(secs(&second[1]), 910);
    }

    #[test]
    fn a_stream_that_ends_is_forgotten() {
        let start = Instant::now();
        let mut anchors = PlaybackAnchors::default();

        anchors.reconcile(&mut [playing("a", 100)], start);
        anchors.reconcile(&mut [], start + Duration::from_secs(5));

        // Same id, same reading, but it is a new stream as far as we know, so
        // it must not inherit the old anchor and jump forward.
        let mut restarted = [playing("a", 100)];
        anchors.reconcile(&mut restarted, start + Duration::from_mins(10));
        assert_eq!(secs(&restarted[0]), 100);
    }

    #[test]
    fn carrying_forward_stops_at_the_end_of_the_item() {
        let start = Instant::now();
        let mut anchors = PlaybackAnchors::default();

        let mut first = [playing("a", 9_990)];
        anchors.reconcile(&mut first, start);

        let mut later = [playing("a", 9_990)];
        anchors.reconcile(&mut later, start + Duration::from_mins(1));
        assert_eq!(secs(&later[0]), 10_000);
    }
}
