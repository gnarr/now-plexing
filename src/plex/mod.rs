// SPDX-License-Identifier: MPL-2.0

//! Everything Now Plexing knows about talking to Plex.
//!
//! Nothing in this module may depend on libcosmic or on any user interface
//! type: it deals in [`model`] values and [`PlexError`], and the view layer
//! decides how to present them.

mod client;
pub mod model;
mod resources;
mod sessions;

pub use client::PlexClient;
pub use model::{PlaybackState, Server, Session, format_duration};

/// The failures a user can act on, deliberately coarse.
///
/// Anything finer would be noise in a panel popup, and error values from
/// `reqwest` can carry URLs, so they are mapped to these variants at the
/// boundary rather than being passed along.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlexError {
    /// plex.tv or the server rejected the token.
    Unauthorized,
    /// The account has no Plex Media Server we can use.
    NoServer,
    /// The server did not answer. Usually transient.
    Unreachable,
    /// A response arrived but was not what the Plex API documents.
    Protocol,
}
