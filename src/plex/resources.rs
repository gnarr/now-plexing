// SPDX-License-Identifier: MPL-2.0

//! Discovering which Plex Media Server to talk to.
//!
//! `GET https://clients.plex.tv/api/v2/resources` answers with a flat array of
//! every device on the account — servers, phones, TVs. We keep the owned media
//! servers, choose one, and rank its connections so the caller can try them
//! best-first.

use super::model::Server;
use crate::secret::Secret;
use serde::Deserialize;

/// One entry of the `/api/v2/resources` array. Only the fields we act on.
#[derive(Debug, Deserialize)]
pub struct Device {
    #[serde(default)]
    pub name: String,
    /// Comma-separated capability list; a media server advertises `server`.
    #[serde(default)]
    pub provides: String,
    #[serde(default)]
    pub owned: bool,
    /// Whether plex.tv currently believes the device is online.
    #[serde(default)]
    pub presence: bool,
    #[serde(default, rename = "clientIdentifier")]
    pub client_identifier: String,
    /// The server's own token, which is not necessarily the account token.
    #[serde(default, rename = "accessToken")]
    pub access_token: Option<Secret>,
    #[serde(default)]
    pub connections: Vec<Connection>,
}

#[derive(Debug, Deserialize)]
pub struct Connection {
    #[serde(default)]
    pub uri: String,
    #[serde(default)]
    pub protocol: String,
    #[serde(default)]
    pub local: bool,
    /// Routed through Plex's relay — works anywhere, but slow and bandwidth capped.
    #[serde(default)]
    pub relay: bool,
    #[serde(default, rename = "IPv6")]
    pub ipv6: bool,
}

impl Device {
    fn is_usable_server(&self) -> bool {
        self.owned
            && self.provides.split(',').any(|p| p.trim() == "server")
            && self.access_token.is_some()
            && self.connections.iter().any(|c| !c.uri.is_empty())
    }
}

/// Pick the server to use.
///
/// Owned media servers only. Among those the rule is deliberately small and
/// deterministic: servers plex.tv reports as online come first, then
/// alphabetical by name, with the client identifier breaking exact ties. When
/// server choice becomes a user-facing setting, this function is the only place
/// that has to change.
#[must_use]
pub fn select_server(devices: &[Device]) -> Option<&Device> {
    devices
        .iter()
        .filter(|device| device.is_usable_server())
        .min_by_key(|device| {
            (
                !device.presence,
                device.name.to_lowercase(),
                device.client_identifier.as_str(),
            )
        })
}

/// Order a server's connections best-first.
///
/// Local addresses beat remote ones, a relay is the last resort, HTTPS is
/// preferred over plaintext, and IPv4 comes before IPv6 because it is more
/// likely to be routable. The caller tries them in order, so a ranking that
/// turns out to be unreachable costs a timeout rather than a failure.
#[must_use]
pub fn ranked_connections(device: &Device) -> Vec<&Connection> {
    let mut connections: Vec<&Connection> = device
        .connections
        .iter()
        .filter(|c| !c.uri.is_empty())
        .collect();

    connections.sort_by_key(|c| (!c.local, c.relay, c.protocol != "https", c.ipv6));
    connections
}

/// Build the domain [`Server`] for a device reached over `connection`.
#[must_use]
pub fn server_at(device: &Device, connection: &Connection) -> Server {
    Server {
        name: device.name.clone(),
        base_url: connection.uri.trim_end_matches('/').to_owned(),
        token: device.access_token.clone().unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::{Device, ranked_connections, select_server, server_at};

    fn devices(json: &str) -> Vec<Device> {
        serde_json::from_str(json).expect("resources fixture should deserialize")
    }

    const TWO_SERVERS_AND_A_PHONE: &str = r#"[
      {
        "name": "Zeppelin",
        "product": "Plex Media Server",
        "provides": "server",
        "owned": true,
        "presence": true,
        "clientIdentifier": "zzz",
        "accessToken": "token-zeppelin",
        "connections": [
          {"protocol":"https","address":"10.0.0.5","port":32400,"uri":"https://10-0-0-5.abc.plex.direct:32400","local":true,"relay":false,"IPv6":false}
        ]
      },
      {
        "name": "attic",
        "product": "Plex Media Server",
        "provides": "server",
        "owned": true,
        "presence": true,
        "clientIdentifier": "aaa",
        "accessToken": "token-attic",
        "connections": [
          {"protocol":"https","address":"10.0.0.6","port":32400,"uri":"https://10-0-0-6.abc.plex.direct:32400","local":true,"relay":false,"IPv6":false}
        ]
      },
      {
        "name": "Gunnar's Phone",
        "product": "Plex for Android",
        "provides": "client,player,pubsub-player",
        "owned": true,
        "presence": true,
        "clientIdentifier": "ppp",
        "connections": []
      }
    ]"#;

    #[test]
    fn clients_are_not_servers() {
        let devices = devices(TWO_SERVERS_AND_A_PHONE);
        let chosen = select_server(&devices).unwrap();
        assert_ne!(chosen.client_identifier, "ppp");
    }

    #[test]
    fn ties_are_broken_alphabetically_ignoring_case() {
        let devices = devices(TWO_SERVERS_AND_A_PHONE);
        // "attic" sorts before "Zeppelin" only if the comparison is case-insensitive.
        assert_eq!(select_server(&devices).unwrap().name, "attic");
    }

    #[test]
    fn an_online_server_wins_over_an_alphabetically_earlier_offline_one() {
        let json = r#"[
          {"name":"attic","provides":"server","owned":true,"presence":false,
           "clientIdentifier":"aaa","accessToken":"t",
           "connections":[{"uri":"https://a:32400","protocol":"https","local":true,"relay":false,"IPv6":false}]},
          {"name":"Zeppelin","provides":"server","owned":true,"presence":true,
           "clientIdentifier":"zzz","accessToken":"t",
           "connections":[{"uri":"https://z:32400","protocol":"https","local":true,"relay":false,"IPv6":false}]}
        ]"#;
        assert_eq!(select_server(&devices(json)).unwrap().name, "Zeppelin");
    }

    #[test]
    fn servers_shared_with_us_are_skipped() {
        // v1 only manages the account's own servers.
        let json = r#"[
          {"name":"Friend's Server","provides":"server","owned":false,"presence":true,
           "clientIdentifier":"fff","accessToken":"t",
           "connections":[{"uri":"https://f:32400","protocol":"https","local":false,"relay":false,"IPv6":false}]}
        ]"#;
        assert!(select_server(&devices(json)).is_none());
    }

    #[test]
    fn a_server_without_a_token_or_connection_is_unusable() {
        let json = r#"[
          {"name":"No Token","provides":"server","owned":true,"presence":true,
           "clientIdentifier":"n1",
           "connections":[{"uri":"https://a:32400","protocol":"https","local":true,"relay":false,"IPv6":false}]},
          {"name":"No Connection","provides":"server","owned":true,"presence":true,
           "clientIdentifier":"n2","accessToken":"t","connections":[]}
        ]"#;
        assert!(select_server(&devices(json)).is_none());
    }

    #[test]
    fn no_devices_at_all_is_not_an_error() {
        assert!(select_server(&devices("[]")).is_none());
    }

    #[test]
    fn connections_are_ranked_local_https_first_and_relay_last() {
        let json = r#"[
          {"name":"Server","provides":"server","owned":true,"presence":true,
           "clientIdentifier":"s","accessToken":"t","connections":[
            {"uri":"https://relay.plex.direct:443","protocol":"https","local":false,"relay":true,"IPv6":false},
            {"uri":"https://remote:32400","protocol":"https","local":false,"relay":false,"IPv6":false},
            {"uri":"http://local-plain:32400","protocol":"http","local":true,"relay":false,"IPv6":false},
            {"uri":"https://local-v6:32400","protocol":"https","local":true,"relay":false,"IPv6":true},
            {"uri":"https://local-secure:32400","protocol":"https","local":true,"relay":false,"IPv6":false}
          ]}
        ]"#;
        let devices = devices(json);
        let order: Vec<&str> = ranked_connections(&devices[0])
            .iter()
            .map(|c| c.uri.as_str())
            .collect();

        assert_eq!(
            order,
            [
                "https://local-secure:32400",
                "https://local-v6:32400",
                "http://local-plain:32400",
                "https://remote:32400",
                "https://relay.plex.direct:443",
            ]
        );
    }

    #[test]
    fn the_chosen_connection_becomes_the_base_url() {
        let devices = devices(TWO_SERVERS_AND_A_PHONE);
        let device = select_server(&devices).unwrap();
        let server = server_at(device, ranked_connections(device)[0]);

        assert_eq!(server.name, "attic");
        assert_eq!(server.base_url, "https://10-0-0-6.abc.plex.direct:32400");
        assert_eq!(server.token.expose(), "token-attic");
    }

    #[test]
    fn a_trailing_slash_on_the_connection_uri_is_dropped() {
        let json = r#"[
          {"name":"S","provides":"server","owned":true,"presence":true,
           "clientIdentifier":"s","accessToken":"t",
           "connections":[{"uri":"https://host:32400/","protocol":"https","local":true,"relay":false,"IPv6":false}]}
        ]"#;
        let devices = devices(json);
        let device = &devices[0];
        assert_eq!(
            server_at(device, ranked_connections(device)[0]).base_url,
            "https://host:32400"
        );
    }
}
