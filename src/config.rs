// SPDX-License-Identifier: MPL-2.0

//! Persisted settings.
//!
//! COSMIC stores one file per field under
//! `$XDG_CONFIG_HOME/cosmic/<app id>/v<version>/`. That is fine for the client
//! identifier, but `cosmic-config` writes world-readable files, so the token
//! gets its permissions tightened after every write. Credential handling is
//! kept to this module and [`crate::secret`] so that moving it to the Secret
//! Service later is a local change.

use crate::secret::Secret;
use cosmic::cosmic_config::{self, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::{env, fs};
use uuid::Uuid;

#[derive(Debug, Default, Clone, CosmicConfigEntry, Eq, PartialEq)]
#[version = 1]
pub struct Config {
    /// Plex account token. Redacted in `Debug` by [`Secret`], which is what
    /// keeps it out of this struct's derived formatting.
    pub token: Secret,
    /// Opaque, stable device identifier Plex uses to recognise this install.
    pub client_id: String,
}

impl Config {
    /// Store a new token and restrict who can read it.
    pub fn store_token(
        &mut self,
        handle: &cosmic_config::Config,
        token: Secret,
    ) -> Result<(), cosmic_config::Error> {
        self.set_token(handle, token)?;
        if let Some(directory) = config_directory() {
            restrict_permissions(&directory);
        }
        Ok(())
    }

    /// Return the stable client identifier, generating and persisting one on
    /// first run.
    ///
    /// If the identifier cannot be written the generated value is still used
    /// for this run; Plex simply sees a new device next time, which is a far
    /// better outcome than refusing to start.
    pub fn ensure_client_id(&mut self, handle: Option<&cosmic_config::Config>) -> String {
        if !self.client_id.is_empty() {
            return self.client_id.clone();
        }

        let client_id = Uuid::new_v4().to_string();
        match handle {
            Some(handle) => {
                let _ = self.set_client_id(handle, client_id.clone());
            }
            None => self.client_id.clone_from(&client_id),
        }
        client_id
    }
}

/// `cosmic-config` writes its key files 0644, which would leave the token
/// readable by every local user. Tighten the directory as well as the file, so
/// that the protection survives the atomic replace on the next write.
///
/// Failures are ignored: a configuration we cannot chmod is not worth a crash.
fn restrict_permissions(directory: &Path) {
    let _ = fs::set_permissions(directory, PermissionsExt::from_mode(0o700));
    let _ = fs::set_permissions(directory.join("token"), PermissionsExt::from_mode(0o600));
}

/// Mirrors where `cosmic_config::Config::new` puts our keys. The crate keeps
/// that path private, so it is reconstructed here.
fn config_directory() -> Option<PathBuf> {
    let base = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;

    Some(
        base.join("cosmic")
            .join(crate::APP_ID)
            .join(format!("v{}", Config::VERSION)),
    )
}

#[cfg(test)]
mod tests {
    use super::restrict_permissions;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};
    use std::{env, fs};

    fn scratch_directory() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after the epoch")
            .as_nanos();
        let directory =
            env::temp_dir().join(format!("now-plexing-{}-{unique}", std::process::id()));
        fs::create_dir_all(&directory).expect("scratch directory should be creatable");
        directory
    }

    fn mode(path: &std::path::Path) -> u32 {
        fs::metadata(path)
            .expect("path should exist")
            .permissions()
            .mode()
            & 0o777
    }

    #[test]
    fn a_stored_token_is_not_readable_by_other_users() {
        // Reproduces what cosmic-config leaves behind: a world-readable key
        // file in a world-traversable directory.
        let directory = scratch_directory();
        let token = directory.join("token");
        fs::write(&token, "\"plex-token\"").expect("token file should be writable");
        fs::set_permissions(&token, PermissionsExt::from_mode(0o644))
            .expect("permissions should be settable");
        fs::set_permissions(&directory, PermissionsExt::from_mode(0o755))
            .expect("permissions should be settable");

        restrict_permissions(&directory);

        assert_eq!(mode(&token), 0o600);
        assert_eq!(mode(&directory), 0o700);

        fs::remove_dir_all(&directory).expect("scratch directory should be removable");
    }

    #[test]
    fn a_missing_token_file_is_not_an_error() {
        // The first save happens before the file exists in some orderings, and
        // clearing the token is allowed too.
        let directory = scratch_directory();

        restrict_permissions(&directory);

        assert_eq!(mode(&directory), 0o700);
        fs::remove_dir_all(&directory).expect("scratch directory should be removable");
    }
}
