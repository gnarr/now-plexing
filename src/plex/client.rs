// SPDX-License-Identifier: MPL-2.0

//! The two HTTP calls Now Plexing makes.
//!
//! Discovery asks plex.tv which servers the account owns; polling asks the
//! chosen server who is watching. The token always travels in a header marked
//! sensitive, never in a URL, and never reaches a log or an error value.

use super::PlexError;
use super::model::{Server, Session};
use super::resources::{Device, ranked_connections, select_server, server_at};
use super::sessions::SessionsResponse;
use crate::secret::Secret;
use reqwest::StatusCode;
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue};
use std::time::Duration;

const RESOURCES_URL: &str =
    "https://clients.plex.tv/api/v2/resources?includeHttps=1&includeRelay=1&includeIPv6=1";

/// How many of a server's addresses we are willing to try before giving up for
/// this cycle. Bounds the worst case when a LAN address is advertised to a
/// machine that has since left the LAN.
const MAX_CONNECTION_ATTEMPTS: usize = 4;

/// Long enough for a busy server to answer, short enough that a poll cannot
/// outlive several refresh intervals.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Deliberately short: an unreachable LAN address should fail fast so the next
/// candidate connection gets its turn.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone)]
pub struct PlexClient {
    http: reqwest::Client,
}

impl PlexClient {
    /// `client_id` must be stable across restarts — Plex treats a new
    /// identifier as a new device.
    #[must_use]
    pub fn new(client_id: &str) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert("X-Plex-Product", HeaderValue::from_static("Now Plexing"));
        headers.insert(
            "X-Plex-Version",
            HeaderValue::from_static(env!("CARGO_PKG_VERSION")),
        );
        headers.insert("X-Plex-Platform", HeaderValue::from_static("Linux"));
        headers.insert("X-Plex-Device", HeaderValue::from_static("COSMIC"));
        headers.insert(
            "X-Plex-Device-Name",
            HeaderValue::from_static("Now Plexing"),
        );
        if let Ok(value) = HeaderValue::from_str(client_id) {
            headers.insert("X-Plex-Client-Identifier", value);
        }

        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .default_headers(headers)
            .build()
            // Only fails if the TLS backend cannot start, in which case no
            // request would succeed anyway.
            .unwrap_or_else(|_| reqwest::Client::new());

        Self { http }
    }

    /// Fetch the current sessions, rediscovering the server when needed.
    ///
    /// Returns the server it ended up using so the caller can cache it and skip
    /// discovery on the next cycle.
    pub async fn poll(
        &self,
        account_token: &Secret,
        cached: Option<Server>,
    ) -> Result<(Server, Vec<Session>), PlexError> {
        if let Some(server) = cached {
            match self.sessions(&server).await {
                Ok(sessions) => return Ok((server, sessions)),
                // A cached server goes stale in two ways: it moves address, or
                // the token it issued us is rotated. Neither means the
                // account token is wrong, so rediscover before reporting
                // anything to the user.
                Err(PlexError::Unreachable | PlexError::Unauthorized) => {}
                Err(other) => return Err(other),
            }
        }

        self.discover(account_token).await
    }

    async fn discover(&self, account_token: &Secret) -> Result<(Server, Vec<Session>), PlexError> {
        let devices: Vec<Device> = self
            .http
            .get(RESOURCES_URL)
            .header("X-Plex-Token", token_header(account_token)?)
            .send()
            .await
            .map_err(|_| PlexError::Unreachable)
            .and_then(ok_or_status)?
            .json()
            .await
            .map_err(|_| PlexError::Protocol)?;

        let device = select_server(&devices).ok_or(PlexError::NoServer)?;

        // Try the addresses best-first. The first one that answers is both the
        // connection we keep and the sessions we were after.
        let mut furthest = PlexError::Unreachable;
        for connection in ranked_connections(device)
            .into_iter()
            .take(MAX_CONNECTION_ATTEMPTS)
        {
            let server = server_at(device, connection);
            match self.sessions(&server).await {
                Ok(sessions) => return Ok((server, sessions)),
                // The token came straight from plex.tv, so a rejection here is
                // real and trying the other addresses would not help.
                Err(PlexError::Unauthorized) => return Err(PlexError::Unauthorized),
                Err(other) => furthest = other,
            }
        }

        Err(furthest)
    }

    async fn sessions(&self, server: &Server) -> Result<Vec<Session>, PlexError> {
        let response = self
            .http
            .get(format!("{}/status/sessions", server.base_url))
            .header("X-Plex-Token", token_header(&server.token)?)
            .send()
            .await
            .map_err(|_| PlexError::Unreachable)
            .and_then(ok_or_status)?;

        response
            .json::<SessionsResponse>()
            .await
            .map(SessionsResponse::into_sessions)
            .map_err(|_| PlexError::Protocol)
    }
}

/// Build the token header, flagged so it is kept out of any HTTP/2 index or
/// debug rendering of the request.
fn token_header(token: &Secret) -> Result<HeaderValue, PlexError> {
    // A token that cannot even be expressed as a header is not a usable
    // credential, which is the same thing to the user as a rejected one.
    let mut value = HeaderValue::from_str(token.expose()).map_err(|_| PlexError::Unauthorized)?;
    value.set_sensitive(true);
    Ok(value)
}

fn ok_or_status(response: reqwest::Response) -> Result<reqwest::Response, PlexError> {
    match response.status() {
        status if status.is_success() => Ok(response),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(PlexError::Unauthorized),
        _ => Err(PlexError::Unreachable),
    }
}
