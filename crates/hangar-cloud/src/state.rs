//! What every route shares: the database, the rate limits, how many proxies
//! are in front, and the clock.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::db::Db;
use crate::limit::{Limits, Rates};

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub limits: Arc<Limits>,
    /// How many reverse proxies are in front of the server, which is how far
    /// `X-Forwarded-For` is believed (`limit::client_ip`).
    pub trusted_proxies: usize,
    /// Milliseconds since the Unix epoch. The tests hand in their own, to
    /// let five minutes pass.
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
}

impl AppState {
    pub fn new(db: Db, rates: Rates, trusted_proxies: usize) -> Self {
        Self {
            db,
            limits: Arc::new(Limits::new(rates)),
            trusted_proxies,
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
