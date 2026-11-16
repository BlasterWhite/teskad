//! What every route shares: the database, the rate limits, how many proxies
//! are in front, the sign-in's configuration, and the clock.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use hangar_cloud_api::Provider;

use crate::config::{Config, OAuthClient};
use crate::db::Db;
use crate::limit::{Limits, Rates};
use crate::provider::Endpoints;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub limits: Arc<Limits>,
    /// How many reverse proxies are in front of the server, which is how far
    /// `X-Forwarded-For` is believed (`limit::client_ip`).
    pub trusted_proxies: usize,
    /// What the sign-in with the providers needs.
    pub sign_in: Arc<SignIn>,
    /// Milliseconds since the Unix epoch. The tests hand in their own, to
    /// let five minutes pass.
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
}

/// The server's side of the sign-in: its public address, its OAuth clients,
/// and where the providers are.
#[derive(Debug, Clone)]
pub struct SignIn {
    /// Without a trailing slash; the redirect URIs are built from it.
    pub base_url: String,
    pub github: Option<OAuthClient>,
    pub google: Option<OAuthClient>,
    pub endpoints: Endpoints,
}

impl SignIn {
    /// The configuration's clients, at the real providers.
    pub fn from_config(config: &Config) -> Self {
        Self {
            base_url: config.base_url.clone(),
            github: config.github.clone(),
            google: config.google.clone(),
            endpoints: Endpoints::default(),
        }
    }

    /// The provider's OAuth application, when this server has one.
    pub fn client(&self, provider: Provider) -> Option<&OAuthClient> {
        match provider {
            Provider::Github => self.github.as_ref(),
            Provider::Google => self.google.as_ref(),
        }
    }

    /// The redirect URI registered with the provider: the server's own
    /// address and path, never anything of a request's.
    pub fn redirect_uri(&self, provider: Provider) -> String {
        format!(
            "{}{}",
            self.base_url,
            hangar_cloud_api::paths::auth_callback(provider)
        )
    }
}

impl AppState {
    pub fn new(db: Db, rates: Rates, trusted_proxies: usize, sign_in: SignIn) -> Self {
        Self {
            db,
            limits: Arc::new(Limits::new(rates)),
            trusted_proxies,
            sign_in: Arc::new(sign_in),
            clock: Arc::new(system_now),
        }
    }

    #[cfg(test)]
    pub fn with_clock(mut self, clock: impl Fn() -> i64 + Send + Sync + 'static) -> Self {
        self.clock = Arc::new(clock);
        self
    }

    pub fn now(&self) -> i64 {
        (self.clock)()
    }
}

fn system_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|since| i64::try_from(since.as_millis()).ok())
        .unwrap_or(0)
}
