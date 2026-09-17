// SPDX-License-Identifier: MPL-2.0

//! Telling the desktop when the number of streams crosses the alert level.
//!
//! Two separate jobs live here. [`StreamWatcher`] decides *whether* something
//! worth reporting just happened, which is pure logic and the part that can
//! actually be tested. [`send`] delivers the message over the session bus.
//!
//! Like [`crate::plex`], nothing in this module may depend on libcosmic, and it
//! holds no user-facing text — the caller passes in an already-localized line.

use std::collections::HashMap;
use zbus::Connection;
use zbus::zvariant::Value;

/// What just changed, and how many streams are playing now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alert {
    /// The count fell to the alert level or below.
    Quiet(usize),
    /// The count climbed back above it.
    Busy(usize),
}

/// Which side of the alert level a count sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Zone {
    /// At or below the level. "0 or fewer" is how nobody-is-watching is spelled.
    Quiet,
    Busy,
}

fn zone(playing: usize, threshold: u32) -> Zone {
    if playing as u64 <= u64::from(threshold) {
        Zone::Quiet
    } else {
        Zone::Busy
    }
}

/// Remembers enough about the last poll to spot a crossing.
///
/// Polling means the raw condition "at or below the level" is true on every
/// cycle while it lasts. Only the transition is an event, so the previous count
/// has to be carried between polls.
#[derive(Debug, Default)]
pub struct StreamWatcher {
    last_playing: Option<usize>,
}

impl StreamWatcher {
    /// Record the latest count, reporting a crossing if one just happened.
    ///
    /// `threshold` is `None` when alerts are switched off — the count is still
    /// recorded so that switching them back on cannot fire retroactively.
    pub fn observe(&mut self, playing: usize, threshold: Option<u32>) -> Option<Alert> {
        let previous = self.last_playing.replace(playing);

        let threshold = threshold?;
        // Nothing to compare against on the first reading after a restart. A
        // quiet server at startup is a state, not an event.
        let previous = previous?;

        // Both zones are derived from the current threshold rather than stored,
        // so moving the level is never mistaken for a crossing.
        let before = zone(previous, threshold);
        let now = zone(playing, threshold);
        if before == now {
            return None;
        }

        Some(match now {
            Zone::Quiet => Alert::Quiet(playing),
            Zone::Busy => Alert::Busy(playing),
        })
    }
}

#[zbus::proxy(
    interface = "org.freedesktop.Notifications",
    default_service = "org.freedesktop.Notifications",
    default_path = "/org/freedesktop/Notifications"
)]
trait Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[&str],
        hints: HashMap<&str, Value<'_>>,
        expire_timeout: i32,
    ) -> zbus::Result<u32>;
}

/// Show `summary` in the desktop's notification area.
///
/// Passing the id returned by a previous call replaces that notification
/// instead of stacking another one, so the tray holds a single entry showing
/// the current state. Zero means "this is a new notification".
///
/// Returns the new id, or `None` if the desktop has no notification daemon —
/// which is not a reason to disturb the applet.
pub async fn send(summary: &str, replaces: u32) -> Option<u32> {
    // Crossings are rare enough that a connection per call is cheaper than
    // keeping one alive in the application model.
    let connection = Connection::session().await.ok()?;
    let proxy = NotificationsProxy::new(&connection).await.ok()?;

    proxy
        .notify(
            "Now Plexing",
            replaces,
            crate::APP_ID,
            summary,
            "",
            &[],
            HashMap::new(),
            // Let the desktop decide how long to show it.
            -1,
        )
        .await
        .ok()
}

#[cfg(test)]
mod tests {
    use super::{Alert, StreamWatcher};

    /// Alerts on, firing at this many streams or fewer.
    const AT_ZERO: Option<u32> = Some(0);
    const AT_ONE: Option<u32> = Some(1);
    const AT_TWO: Option<u32> = Some(2);
    /// Alerts switched off.
    const OFF: Option<u32> = None;

    #[test]
    fn the_first_reading_is_never_an_event() {
        // Starting the applet while nobody is watching is a state, not news.
        let mut watcher = StreamWatcher::default();
        assert_eq!(watcher.observe(0, AT_ZERO), None);
    }

    #[test]
    fn the_last_stream_stopping_fires_once() {
        let mut watcher = StreamWatcher::default();
        watcher.observe(3, AT_ZERO);

        assert_eq!(watcher.observe(0, AT_ZERO), Some(Alert::Quiet(0)));
    }

    #[test]
    fn staying_quiet_does_not_keep_firing() {
        // The whole point of tracking the previous count: the raw condition is
        // true on every poll, but only the crossing is an event.
        let mut watcher = StreamWatcher::default();
        watcher.observe(3, AT_ZERO);
        watcher.observe(0, AT_ZERO);

        assert_eq!(watcher.observe(0, AT_ZERO), None);
        assert_eq!(watcher.observe(0, AT_ZERO), None);
    }

    #[test]
    fn streaming_starting_again_fires() {
        let mut watcher = StreamWatcher::default();
        watcher.observe(3, AT_ZERO);
        watcher.observe(0, AT_ZERO);

        assert_eq!(watcher.observe(2, AT_ZERO), Some(Alert::Busy(2)));
        assert_eq!(watcher.observe(2, AT_ZERO), None);
    }

    #[test]
    fn a_higher_level_fires_before_the_count_reaches_zero() {
        let mut watcher = StreamWatcher::default();
        watcher.observe(3, AT_ONE);

        // Three streams down to one is already at the level.
        assert_eq!(watcher.observe(1, AT_ONE), Some(Alert::Quiet(1)));
        // And the last one stopping is not a second event.
        assert_eq!(watcher.observe(0, AT_ONE), None);
    }

    #[test]
    fn a_count_above_the_level_is_not_a_crossing() {
        let mut watcher = StreamWatcher::default();
        watcher.observe(3, AT_ONE);

        assert_eq!(watcher.observe(2, AT_ONE), None);
    }

    #[test]
    fn switched_off_it_stays_silent_but_keeps_watching() {
        let mut watcher = StreamWatcher::default();
        watcher.observe(3, OFF);

        assert_eq!(watcher.observe(0, OFF), None);
    }

    #[test]
    fn switching_alerts_on_does_not_fire_retroactively() {
        // The count was recorded while alerts were off, so turning them on
        // mid-session reports nothing until something actually changes.
        let mut watcher = StreamWatcher::default();
        watcher.observe(3, OFF);
        watcher.observe(0, OFF);

        assert_eq!(watcher.observe(0, AT_ZERO), None);
        assert_eq!(watcher.observe(2, AT_ZERO), Some(Alert::Busy(2)));
    }

    #[test]
    fn moving_the_level_is_not_a_crossing() {
        // Both zones are recomputed against the current level, so raising it
        // over a steady count must not look like the count fell.
        let mut watcher = StreamWatcher::default();
        watcher.observe(1, AT_ZERO);

        assert_eq!(watcher.observe(1, AT_TWO), None);
        assert_eq!(watcher.observe(1, AT_ZERO), None);
    }

    #[test]
    fn a_drop_spanning_a_plex_outage_still_fires() {
        // Failed polls never reach the watcher, so the count from before the
        // outage is what the recovery reading is compared against.
        let mut watcher = StreamWatcher::default();
        watcher.observe(3, AT_ZERO);

        assert_eq!(watcher.observe(0, AT_ZERO), Some(Alert::Quiet(0)));
    }

    #[test]
    #[ignore = "sends a real desktop notification"]
    fn sends_a_real_notification() {
        // Run with `cargo test -- --ignored`. Talking to the daemon is the only
        // way to exercise the D-Bus path, and a unit test cannot fake it.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a current-thread runtime should start");

        let first = runtime.block_on(super::send("Now Plexing: nobody is streaming", 0));
        assert!(first.is_some(), "no notification daemon answered");

        // Passing the previous id should replace that notification rather than
        // stack a second one in the tray.
        let replaced = runtime.block_on(super::send(
            "Now Plexing: 2 streams are active",
            first.unwrap_or_default(),
        ));
        assert!(replaced.is_some(), "the replacement was not accepted");
    }
}
