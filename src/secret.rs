// SPDX-License-Identifier: MPL-2.0

//! A string that must not end up in logs, panics or `Debug` output.
//!
//! Every Plex credential in this crate is wrapped in [`Secret`]. Reading the
//! value back requires calling [`Secret::expose`], which is deliberately
//! awkward to type, and the `Debug` implementation redacts unconditionally so
//! that deriving `Debug` on a containing struct — a config entry, an
//! application message — can never leak it by accident.
//!
//! This is also the single place to change if credentials later move out of the
//! COSMIC configuration and into the Secret Service.

use serde::{Deserialize, Serialize};

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    /// Read the underlying credential. Call sites should pass the result
    /// straight to the consumer rather than storing or formatting it.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.trim().is_empty()
    }
}

impl From<String> for Secret {
    fn from(value: String) -> Self {
        Self(value.trim().to_owned())
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.is_empty() {
            "Secret(empty)"
        } else {
            "Secret(redacted)"
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Secret;

    #[test]
    fn debug_never_reveals_the_credential() {
        let secret = Secret::from("xyzzy-super-secret-token".to_owned());
        assert_eq!(format!("{secret:?}"), "Secret(redacted)");
        assert!(!format!("{secret:?}").contains("xyzzy"));
    }

    #[test]
    fn debug_of_a_containing_struct_is_also_redacted() {
        #[derive(Debug)]
        struct Holder {
            token: Secret,
        }

        let holder = Holder {
            token: Secret::from("xyzzy".to_owned()),
        };
        assert_eq!(holder.token.expose(), "xyzzy");
        assert!(!format!("{holder:?}").contains("xyzzy"));
    }

    #[test]
    fn surrounding_whitespace_is_trimmed_away() {
        // Tokens are typically pasted, and a trailing newline would otherwise
        // produce an invalid header value.
        assert_eq!(Secret::from("  abc \n".to_owned()).expose(), "abc");
    }

    #[test]
    fn blank_input_counts_as_empty() {
        assert!(Secret::from("   ".to_owned()).is_empty());
        assert!(Secret::default().is_empty());
        assert!(!Secret::from("t".to_owned()).is_empty());
    }
}
