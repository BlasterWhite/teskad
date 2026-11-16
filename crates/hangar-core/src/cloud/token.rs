//! Where the device token is kept: the operating system's keychain
//! (Credential Manager, the macOS Keychain, the Secret Service), and nowhere
//! else. When no keychain answers, every call says so: the token is never
//! written to a file instead (docs/native/cloud.md, "Deleting").
//!
//! Behind a trait, so that the tests keep theirs in memory and never touch
//! the keychain of the machine they run on.

use std::sync::{Arc, Mutex};

use keyring::{Entry, Error as KeyringError};

use super::api;
use crate::paths::PRODUCT_NAME;

/// The keychain entry of the token, under the service named after the app.
const ENTRY: &str = "hangar-account";

/// Somewhere a device token can be kept. Errors are sentences for the person:
/// what did not answer, and what to do about it where that is knowable.
pub trait TokenStore: Send + Sync {
    /// `Ok(None)` when no token is kept.
    fn load(&self) -> Result<Option<String>, String>;
    fn save(&self, token: &str) -> Result<(), String>;
    /// Deleting a token that is not there is fine.
    fn delete(&self) -> Result<(), String>;
}

/// The operating system's keychain: service `Hangar.AI`, entry
/// `hangar-account`.
pub struct Keychain {
    entry: String,
}

impl Keychain {
    /// The entry of the token for the server at `base_url`.
    ///
    /// The public server's is `hangar-account`. Another server's (a local
    /// one, through `HANGAR_CLOUD_URL`) gets an entry of its own, named after
    /// it: a build tried against a development server must neither replace
    /// the person's real token nor send it there.
    pub fn for_server(base_url: &str) -> Self {
        let base = base_url.trim().trim_end_matches('/');
        let entry = if base == api::DEFAULT_BASE_URL {
            ENTRY.to_string()
        } else {
            format!("{ENTRY}@{base}")
        };
        Self { entry }
    }

    fn entry(&self) -> Result<Entry, String> {
        Entry::new(PRODUCT_NAME, &self.entry).map_err(unavailable)
    }
}

/// The keychain's own error, plus what to do about it where that is knowable.
fn unavailable(err: KeyringError) -> String {
    if cfg!(all(unix, not(target_os = "macos"))) {
        format!(
            "no Secret Service answered ({err}). Start GNOME Keyring, KWallet or another \
             Secret Service provider, then try again"
        )
    } else {
        format!("the system keychain refused the request: {err}")
    }
}

impl TokenStore for Keychain {
    fn load(&self) -> Result<Option<String>, String> {
        match self.entry()?.get_password() {
            Ok(token) if token.trim().is_empty() => Ok(None),
            Ok(token) => Ok(Some(token.trim().to_string())),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(err) => Err(unavailable(err)),
        }
    }

    fn save(&self, token: &str) -> Result<(), String> {
        self.entry()?.set_password(token).map_err(unavailable)
    }

    fn delete(&self) -> Result<(), String> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(err) => Err(unavailable(err)),
        }
    }
}

/// A token kept in memory, gone with the process: what the tests sign in
/// with. Clones share the one token, so a test keeps a clone to look at.
#[derive(Clone, Default)]
pub struct MemoryTokens {
    token: Arc<Mutex<Option<String>>>,
}

impl MemoryTokens {
    pub fn new() -> Self {
        Self::default()
    }

    /// A store that already holds `token`.
    pub fn holding(token: &str) -> Self {
        Self {
            token: Arc::new(Mutex::new(Some(token.to_string()))),
        }
    }

    /// The token kept, if any.
    pub fn peek(&self) -> Option<String> {
        self.token.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl TokenStore for MemoryTokens {
    fn load(&self) -> Result<Option<String>, String> {
        Ok(self.peek())
    }

    fn save(&self, token: &str) -> Result<(), String> {
        *self.token.lock().unwrap_or_else(|e| e.into_inner()) = Some(token.to_string());
        Ok(())
    }

    fn delete(&self) -> Result<(), String> {
        *self.token.lock().unwrap_or_else(|e| e.into_inner()) = None;
        Ok(())
    }
}
