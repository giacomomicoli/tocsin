//! API keys: where they come from and where they are kept.
//!
//! Resolution order for every key:
//! 1. environment variable (`TOCSIN_*`) — handy for CI and power users;
//! 2. the OS credential store (Windows Credential Manager, macOS Keychain,
//!    Secret Service on Linux) under the service name `tocsin`;
//! 3. the legacy `curseforge_api_key` field in `tocsin.toml`.
//!
//! The GUI writes to (2) only. Keys are never logged and never serialized
//! anywhere else.

use std::collections::BTreeMap;

use crate::manifest::Manifest;
use crate::sources::Credentials;

const SERVICE: &str = "tocsin";

/// A credential slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Secret {
    /// `x-api-key` for api.curseforge.com.
    CurseForge,
    /// Optional GitHub token (rate limit only).
    Github,
}

impl Secret {
    /// All slots.
    pub const ALL: [Secret; 2] = [Secret::CurseForge, Secret::Github];

    /// Environment variable name.
    pub fn env_var(self) -> &'static str {
        match self {
            Secret::CurseForge => "TOCSIN_CURSEFORGE_KEY",
            Secret::Github => "TOCSIN_GITHUB_TOKEN",
        }
    }

    /// Account name inside the credential store.
    pub fn account(self) -> &'static str {
        match self {
            Secret::CurseForge => "curseforge",
            Secret::Github => "github",
        }
    }

    /// Human label.
    pub fn label(self) -> &'static str {
        match self {
            Secret::CurseForge => "CurseForge API key",
            Secret::Github => "GitHub token (optional)",
        }
    }

    /// Where the user obtains it.
    pub fn help_url(self) -> &'static str {
        match self {
            Secret::CurseForge => "https://console.curseforge.com/",
            Secret::Github => "https://github.com/settings/tokens",
        }
    }
}

/// Errors from the credential store.
#[derive(Debug, thiserror::Error)]
#[error("credential store: {0}")]
pub struct StoreError(String);

impl StoreError {
    /// Only the store's Display text is kept, never the error itself: some
    /// keyring variants (`BadEncoding`, `BadDataFormat`) carry the stored
    /// secret's bytes, which their `Debug` prints. The malformed-data case
    /// gets a fixed message because its Display embeds a platform error's
    /// `Debug`, whose content we do not control.
    fn from_keyring(e: &keyring::Error) -> Self {
        match e {
            keyring::Error::BadEncoding(_) | keyring::Error::BadDataFormat(..) => {
                StoreError("the stored key is unreadable".into())
            }
            other => StoreError(other.to_string()),
        }
    }
}

fn entry(secret: Secret) -> Result<keyring::Entry, StoreError> {
    keyring::Entry::new(SERVICE, secret.account()).map_err(|e| StoreError::from_keyring(&e))
}

fn clean(s: String) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// Whether `secret` is forced by the environment (so the store is ignored).
pub fn from_env(secret: Secret) -> Option<String> {
    std::env::var(secret.env_var()).ok().and_then(clean)
}

/// Read from the OS credential store only.
pub fn from_store(secret: Secret) -> Option<String> {
    entry(secret).ok()?.get_password().ok().and_then(clean)
}

/// Save to the OS credential store. An empty value deletes the entry.
pub fn store(secret: Secret, value: &str) -> Result<(), StoreError> {
    let e = entry(secret)?;
    match clean(value.to_string()) {
        Some(v) => e.set_password(&v).map_err(|err| StoreError::from_keyring(&err)),
        None => match e.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(StoreError::from_keyring(&err)),
        },
    }
}

/// Where a key comes from, as the Settings page shows it. Only presence is
/// kept, never the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyStatus {
    /// Forced by the environment variable; the form cannot change it.
    Env,
    /// In the OS credential store.
    Stored,
    /// The legacy `curseforge_api_key` field in `tocsin.toml`.
    Config,
    /// Not set anywhere.
    NotSet,
}

/// Where each key in `creds` (from [`credentials`]) came from.
pub fn key_statuses(manifest: &Manifest, creds: &Credentials) -> BTreeMap<Secret, KeyStatus> {
    Secret::ALL
        .into_iter()
        .map(|s| {
            let value = match s {
                Secret::CurseForge => creds.curseforge.as_deref(),
                Secret::Github => creds.github.as_deref(),
            };
            let legacy = match s {
                Secret::CurseForge => manifest.settings.curseforge_api_key.as_deref(),
                Secret::Github => None,
            };
            (s, key_status(from_env(s).is_some(), value, legacy))
        })
        .collect()
}

/// The environment wins; otherwise a resolved value equal to the legacy
/// config field came from there (the store is read before it, so a stored
/// key differing from it is the store's).
fn key_status(in_env: bool, value: Option<&str>, legacy: Option<&str>) -> KeyStatus {
    match value {
        _ if in_env => KeyStatus::Env,
        None => KeyStatus::NotSet,
        Some(v) if Some(v) == legacy => KeyStatus::Config,
        Some(_) => KeyStatus::Stored,
    }
}

/// Resolve every credential following the order documented above.
pub fn credentials(manifest: &Manifest) -> Credentials {
    let get = |s: Secret| from_env(s).or_else(|| from_store(s));
    Credentials {
        curseforge: get(Secret::CurseForge).or_else(|| manifest.settings.curseforge_api_key.clone()),
        github: get(Secret::Github),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_status_follows_resolution_order() {
        // The environment wins even over a stored or legacy value.
        assert_eq!(key_status(true, Some("k"), Some("k")), KeyStatus::Env);
        assert_eq!(key_status(false, None, None), KeyStatus::NotSet);
        // Equal to the legacy `tocsin.toml` field: that is where it came from.
        assert_eq!(key_status(false, Some("k"), Some("k")), KeyStatus::Config);
        // The store is read before the legacy field and differs from it.
        assert_eq!(key_status(false, Some("new"), Some("old")), KeyStatus::Stored);
        assert_eq!(key_status(false, Some("tok"), None), KeyStatus::Stored);
    }
}
