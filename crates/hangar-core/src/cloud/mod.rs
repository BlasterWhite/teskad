//! The optional Hangar account ("Compte Hangar"), seen from the desktop:
//! signing in with GitHub or Google, the machines linked to the account, and
//! the usage statistics they share (docs/native/cloud.md).
//!
//! Not `accounts`: that module is the Claude accounts a pane runs under, and
//! has nothing to do with this one.
//!
//! All of it is opt-in. A Hangar nobody signed in with never calls the account
//! server, and nothing else in the app waits on it or behaves differently
//! without it. Signed in, statistics are sent only while their sync is on,
//! and what is sent is totals per day, model and project name: never a path,
//! never a line of a transcript. The types say so (`StatsRow` has nowhere to
//! put either), and so does the server, which refuses a project that looks
//! like a path.
//!
//! The protocol's types are hangar-cloud-api's, the crate the server builds
//! its answers from; they are re-exported here so the rest of the app has one
//! place to name them. The device token lives in the operating system's
//! keychain, as the Groq key does (voice/key.rs), and nowhere else.
//!
//! [`Cloud`] is the whole of it, with no window: the calls (client.rs), the
//! sign-in through the system browser (signin.rs), the token (token.rs), the
//! machine's id and name (machine.rs) and the statistics sync (sync.rs).
//! Nothing here starts by itself: every call is made by whoever holds a
//! `Cloud`, and the ones that reach the server refuse to without a token.
//!
//! Nothing here writes a token, a link code, a verifier or an email address
//! to a log, and no error carries one: an error is a status, the server's own
//! code and sentence, or what the network said.

mod client;
mod machine;
mod signin;
mod sync;
mod token;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub use hangar_cloud_api as api;
pub use hangar_cloud_api::{
    Account, ApiError, Device, DeviceStatsRow, DevicesResponse, ErrorCode, ExchangeRequest,
    ExchangeResponse, MachineInfo, MeResponse, Provider, StatsResponse, StatsRow, API_VERSION,
};

pub use signin::{Cancel, SignIn, SignedIn, SIGN_IN_TIMEOUT};
pub use sync::{project_name, rows_from, Schedule, Synced, SYNC_EVERY};
pub use token::{Keychain, MemoryTokens, TokenStore};

use client::Client;

/// The account server's address, without a trailing slash: the public one,
/// unless `HANGAR_CLOUD_URL` names another (a local server, while developing).
pub fn base_url() -> String {
    resolve_base_url(std::env::var(api::BASE_URL_ENV).ok())
}

fn resolve_base_url(from_env: Option<String>) -> String {
    from_env
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .unwrap_or(api::DEFAULT_BASE_URL)
        .trim_end_matches('/')
        .to_string()
}

/// Why something of the account did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Nobody is signed in on this machine. Nothing was asked of the server.
    NotSignedIn,
    /// The statistics sync is off. Nothing was asked of the server.
    SyncOff,
    /// The server no longer knows this machine's token (`401`): signed out
    /// from another machine, forgotten, or the account deleted. The token has
    /// been removed from the keychain.
    SignedOut,
    /// The server refused, with its status, code and sentence.
    Refused {
        status: u16,
        error: ApiError,
        /// What `Retry-After` said, when the server is asking for patience.
        retry_after: Option<Duration>,
    },
    /// The server could not be reached, or stopped answering.
    Unreachable(String),
    /// The server answered something this version cannot read.
    Answer(String),
    /// The operating system's keychain did not answer, or refused.
    Keychain(String),
    /// Something on this machine: a file, a socket, the random source.
    Local(String),
    /// Refused here, before anything was sent: a name too long, a day that is
    /// not a day.
    Invalid(String),
    /// The person said no on the provider's page.
    Denied,
    /// The sign-in failed on the provider's side.
    Provider,
    /// The sign-in was cancelled from Hangar.
    Cancelled,
    /// Nobody came back from the browser in time.
    TimedOut,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotSignedIn => f.write_str("not signed in to a Hangar account"),
            Error::SyncOff => f.write_str("the statistics sync is off"),
            Error::SignedOut => f.write_str("this machine was signed out of the Hangar account"),
            Error::Refused { status, error, .. } => {
                write!(f, "the account server refused ({status}): {error}")
            }
            Error::Unreachable(why) => write!(f, "the account server could not be reached: {why}"),
            Error::Answer(what) => {
                write!(f, "the account server's answer could not be read: {what}")
            }
            Error::Keychain(why) => f.write_str(why),
            Error::Local(why) => f.write_str(why),
            Error::Invalid(why) => f.write_str(why),
            Error::Denied => f.write_str("the sign-in was refused on the provider's page"),
            Error::Provider => f.write_str("the sign-in failed on the provider's side"),
            Error::Cancelled => f.write_str("the sign-in was cancelled"),
            Error::TimedOut => f.write_str("nobody came back from the browser in time"),
        }
    }
}

impl std::error::Error for Error {}

/// The Hangar account of this machine: the server it talks to, where its
/// token is kept, and the app data folder its two small files live in (the
/// machine's id, and what the sync last sent). Cheap to clone.
#[derive(Clone)]
pub struct Cloud {
    inner: Arc<Inner>,
}

struct Inner {
    client: Client,
    tokens: Box<dyn TokenStore>,
    data_dir: PathBuf,
    /// One sync at a time.
    syncing: Mutex<()>,
    /// When what the server holds was last compared with what was sent.
    verified: Mutex<Option<Instant>>,
}

impl Cloud {
    /// The real one: the public server (or `HANGAR_CLOUD_URL`'s), the
    /// operating system's keychain, the app's data folder.
    pub fn new(host: &crate::host::Host) -> Result<Self, Error> {
        let data_dir = host
            .paths()
            .app_data_dir()
            .map_err(|err| Error::Local(format!("no app data dir: {err}")))?;
        let base = base_url();
        let tokens = Keychain::for_server(&base);
        Ok(Self::with(&base, tokens, data_dir))
    }

    /// One that talks to `base_url`, keeps its token in `tokens` and its
    /// files in `data_dir`: for tests, which pass a server and a store of
    /// their own.
    pub fn with(base_url: &str, tokens: impl TokenStore + 'static, data_dir: PathBuf) -> Self {
        Self {
            inner: Arc::new(Inner {
                client: Client::new(base_url),
                tokens: Box::new(tokens),
                data_dir,
                syncing: Mutex::new(()),
                verified: Mutex::new(None),
            }),
        }
    }

    /// Whether this machine holds a token. Asks the keychain, never the
    /// server: `me` is what tells whether the token still works.
    pub fn signed_in(&self) -> Result<bool, Error> {
        Ok(self.token()?.is_some())
    }

    /// Which version of the protocol the server speaks (`GET /health`).
    /// Only for a person who is signed in: without a token, a Hangar never
    /// calls the server, not even to ask how it is.
    pub fn health(&self) -> Result<api::Health, Error> {
        self.with_token(|_| self.inner.client.health())
    }

    /// The machine as it presents itself at sign-in: its id, drawn once and
    /// kept in the app data folder, its host name and its system.
    pub fn machine(&self) -> Result<MachineInfo, Error> {
        machine::machine(&self.inner.data_dir)
    }

    /// The account, and this machine's device on it. What to call at launch,
    /// when a token is there, to learn whether it still works.
    pub fn me(&self) -> Result<MeResponse, Error> {
        self.with_token(|token| self.inner.client.me(token))
    }

    /// Every machine of the account, signed-out ones included.
    pub fn devices(&self) -> Result<Vec<Device>, Error> {
        self.with_token(|token| self.inner.client.devices(token))
            .map(|answer| answer.devices)
    }

    /// Renames a machine of the account, this one or another.
    pub fn rename_device(&self, id: &str, name: &str) -> Result<Device, Error> {
        let name = name.trim();
        let length = name.chars().count();
        if length == 0 || length > api::MAX_DEVICE_NAME_CHARS || name.chars().any(char::is_control)
        {
            return Err(Error::Invalid(format!(
                "a machine's name is 1 to {} characters on one line",
                api::MAX_DEVICE_NAME_CHARS
            )));
        }
        self.with_token(|token| self.inner.client.rename_device(token, id, name))
    }

    /// Signs another machine out: its token stops working, its statistics
    /// stay on the account. On this machine's own device, use `sign_out`.
    pub fn sign_out_device(&self, id: &str) -> Result<(), Error> {
        self.with_token(|token| self.inner.client.delete_device(token, id, false))
    }

    /// Deletes a machine from the account with every row it sent. When it is
    /// this one, the next call finds the token gone and forgets it here too.
    pub fn forget_device(&self, id: &str) -> Result<(), Error> {
        self.with_token(|token| self.inner.client.delete_device(token, id, true))
    }

    /// Signs this machine out: its device is revoked on the server when the
    /// server can be reached (its rows stay on the account), and the token
    /// leaves the keychain either way. Signing out while signed out is fine.
    pub fn sign_out(&self) -> Result<(), Error> {
        let Some(token) = self.token()? else {
            sync::forget(&self.inner.data_dir);
            return Ok(());
        };
        // Best effort: a server that is down, or that no longer knows the
        // token, must not keep the person signed in here.
        if let Ok(me) = self.inner.client.me(&token) {
            let _ = self
                .inner
                .client
                .delete_device(&token, &me.device.id, false);
        }
        self.forget_locally()
    }

    /// Deletes the account and everything it holds, on the server, at once;
    /// then forgets the token here.
    pub fn delete_account(&self) -> Result<(), Error> {
        self.with_token(|token| self.inner.client.delete_account(token))?;
        self.forget_locally()
    }

    fn token(&self) -> Result<Option<String>, Error> {
        self.inner.tokens.load().map_err(Error::Keychain)
    }

    /// Runs a call that needs the token. Without one, nothing is called. A
    /// `401` means the server signed this machine out: the token is dropped.
    fn with_token<T>(&self, call: impl FnOnce(&str) -> Result<T, Error>) -> Result<T, Error> {
        let token = self.token()?.ok_or(Error::NotSignedIn)?;
        let answer = call(&token);
        if matches!(answer, Err(Error::SignedOut)) {
            // The answer stays "signed out" whatever the keychain says of the
            // deletion: the token is dead either way.
            let _ = self.forget_locally();
        }
        answer
    }

    /// The token out of the keychain, and with it what the sync remembered
    /// having sent under it.
    fn forget_locally(&self) -> Result<(), Error> {
        sync::forget(&self.inner.data_dir);
        self.inner.tokens.delete().map_err(Error::Keychain)
    }
}
