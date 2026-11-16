//! How many requests one caller may make: a token bucket per IP address for
//! everything under `/v1`, a tighter one per address for the exchange (the
//! one route that takes a secret from somebody not yet known) and for the
//! start of a sign-in (the one route that stores something for somebody not
//! yet known), and one per device for what a token signs.
//!
//! In memory only, and forgotten as soon as a bucket is full again: the
//! server stores no IP address (docs/native/cloud.md, "What the server
//! stores"), and a restart forgets them all.
//!
//! The server runs behind a reverse proxy, so the address of the connection
//! is the proxy's. The caller's is read from `X-Forwarded-For`, and only as
//! far as the configuration says there are proxies to trust: each proxy
//! appends the address it received from, so with `n` trusted proxies the
//! caller is the `n`th entry from the end, and everything before it is what
//! the caller chose to send and is worth nothing.

use std::collections::HashMap;
use std::hash::Hash;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::HeaderMap;
use axum::http::header::HeaderName;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::refusal::Refusal;
use crate::state::AppState;

const X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");

/// `burst` requests at once, refilled evenly over `per`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rate {
    pub burst: u32,
    pub per: Duration,
}

impl Rate {
    pub const fn per_minute(burst: u32) -> Self {
        Self {
            burst,
            per: Duration::from_secs(60),
        }
    }
}

/// The server's three limits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rates {
    /// Every request under `/v1`, per address. Several machines of one
    /// household share an address, and a first sync sends a year of rows in
    /// several calls.
    pub ip: Rate,
    /// `POST /v1/auth/exchange`, per address: a person signs in a few times
    /// a day at most.
    pub exchange: Rate,
    /// `GET /v1/auth/start`, per address: each one stores a sign-in in
    /// progress for ten minutes, and a person starts a few a day at most.
    pub start: Rate,
    /// Every request a device token signs, per device.
    pub device: Rate,
}

impl Default for Rates {
    fn default() -> Self {
        Self {
            ip: Rate::per_minute(300),
            exchange: Rate::per_minute(10),
            start: Rate::per_minute(10),
            device: Rate::per_minute(120),
        }
    }
}

pub struct Limits {
    pub ip: Limiter<IpAddr>,
    pub exchange: Limiter<IpAddr>,
    pub start: Limiter<IpAddr>,
    pub device: Limiter<String>,
}

impl Limits {
    pub fn new(rates: Rates) -> Self {
        Self {
            ip: Limiter::new(rates.ip),
            exchange: Limiter::new(rates.exchange),
            start: Limiter::new(rates.start),
            device: Limiter::new(rates.device),
        }
    }
}

/// The most callers one limiter remembers at a time. Past it, after dropping
/// every bucket that is full again, a caller it does not know is refused
/// rather than let through uncounted: a flood of new addresses must not be a
/// way to switch the limit off.
const MAX_KEYS: usize = 100_000;

pub struct Limiter<K> {
    /// The time one request costs: `per / burst`.
    cost: Duration,
    /// How far ahead of now a caller's bucket may be booked: a whole burst
    /// less the request being counted.
    tolerance: Duration,
    per: Duration,
    max_keys: usize,
    inner: Mutex<Inner<K>>,
}

struct Inner<K> {
    /// For each caller, the moment its bucket is full again. Whole
    /// durations, no fractions of a token: nothing here rounds.
    full_at: HashMap<K, Instant>,
    swept_at: Option<Instant>,
}

impl<K: Hash + Eq> Limiter<K> {
    pub fn new(rate: Rate) -> Self {
        Self::with_max_keys(rate, MAX_KEYS)
    }

    fn with_max_keys(rate: Rate, max_keys: usize) -> Self {
        let cost = rate.per / rate.burst.max(1);
        Self {
            cost,
            tolerance: rate.per.saturating_sub(cost),
            per: rate.per,
            max_keys,
            inner: Mutex::new(Inner {
                full_at: HashMap::new(),
                swept_at: None,
            }),
        }
    }

    /// Counts one request of `key` at `now`. `Err` is the number of seconds
    /// after which the next one would pass.
    pub fn check(&self, key: K, now: Instant) -> Result<(), u64> {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if !inner.full_at.contains_key(&key) && inner.full_at.len() >= self.max_keys {
            // At most one sweep a second: a flood must not buy a walk of the
            // whole map with each request.
            let due = inner
                .swept_at
                .is_none_or(|at| now.saturating_duration_since(at) >= Duration::from_secs(1));
            if due {
                inner.full_at.retain(|_, full_at| *full_at > now);
                inner.swept_at = Some(now);
            }
            if inner.full_at.len() >= self.max_keys {
                return Err(whole_secs(self.per));
            }
        }

        let full_at = inner.full_at.entry(key).or_insert(now);
        let booked = full_at.saturating_duration_since(now);
        if booked > self.tolerance {
            // A refused request costs nothing.
            return Err(whole_secs(booked - self.tolerance));
        }
        *full_at = now + booked + self.cost;
        Ok(())
    }

    /// Forgets every caller whose bucket is full again: they are counted
    /// from scratch when they come back, which is the same thing.
    pub fn sweep(&self, now: Instant) {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner.full_at.retain(|_, full_at| *full_at > now);
        inner.swept_at = Some(now);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.inner.lock().unwrap().full_at.len()
    }
}

/// A wait in seconds, rounded up, and never zero.
fn whole_secs(wait: Duration) -> u64 {
    (wait.as_secs() + u64::from(wait.subsec_nanos() > 0)).max(1)
}

/// The address a request is counted under, put in the request's extensions
/// by [`per_ip`] for the routes that count it again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientIp(pub IpAddr);

/// The caller's address: the connection's, or with `trusted_proxies` proxies
/// in front, the entry of `X-Forwarded-For` that the outermost of them wrote.
/// When the header has fewer entries than there are proxies (a health check
/// from inside, a configuration that counts one proxy too many), the
/// connection's address is the only one nobody chose.
pub fn client_ip(headers: &HeaderMap, peer: Option<IpAddr>, trusted_proxies: usize) -> IpAddr {
    let forwarded = || {
        // Several headers are one list, in order.
        let entries: Vec<&str> = headers
            .get_all(X_FORWARDED_FOR)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(','))
            .map(str::trim)
            .collect();
        let index = entries.len().checked_sub(trusted_proxies)?;
        entries[index].parse::<IpAddr>().ok()
    };
    let ip = (trusted_proxies > 0)
        .then(forwarded)
        .flatten()
        .or(peer)
        .unwrap_or(IpAddr::V6(Ipv6Addr::UNSPECIFIED));
    bucket_address(ip)
}

/// What an address is counted as. One customer of an IPv6 provider holds a
/// whole /64 and can take any address in it: the /64 is the caller. An IPv4
/// address written as IPv6 is the IPv4 address.
fn bucket_address(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(_) => ip,
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(Ipv6Addr::from_bits(v6.to_bits() & (u128::MAX << 64))),
        },
    }
}

/// The middleware of every route under `/v1`: counts the request against its
/// address, and leaves that address in the extensions.
pub async fn per_ip(State(state): State<AppState>, mut request: Request, next: Next) -> Response {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.ip());
    let ip = client_ip(request.headers(), peer, state.trusted_proxies);
    if let Err(secs) = state.limits.ip.check(ip, Instant::now()) {
        return Refusal::rate_limited(secs).into_response();
    }
    request.extensions_mut().insert(ClientIp(ip));
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn a_burst_passes_and_the_next_request_is_told_when_to_come_back() {
        let limiter = Limiter::new(Rate::per_minute(3));
        let start = Instant::now();
        for _ in 0..3 {
            assert_eq!(limiter.check("a", start), Ok(()));
        }
        // One request every twenty seconds from here.
        assert_eq!(limiter.check("a", start), Err(20));
        assert_eq!(limiter.check("a", start + secs(5)), Err(15));
        assert_eq!(limiter.check("a", start + secs(20)), Ok(()));
        assert_eq!(limiter.check("a", start + secs(20)), Err(20));
        // A refused request costs nothing: after a quiet minute the whole
        // burst is back.
        let later = start + secs(80);
        for _ in 0..3 {
            assert_eq!(limiter.check("a", later), Ok(()));
        }
        assert!(limiter.check("a", later).is_err());
    }

    #[test]
    fn each_caller_has_its_own_bucket() {
        let limiter = Limiter::new(Rate::per_minute(1));
        let now = Instant::now();
        assert_eq!(limiter.check("a", now), Ok(()));
        assert!(limiter.check("a", now).is_err());
        assert_eq!(limiter.check("b", now), Ok(()));
    }

    #[test]
    fn a_full_bucket_is_forgotten() {
        let limiter = Limiter::new(Rate::per_minute(2));
        let now = Instant::now();
        limiter.check("a", now).unwrap();
        limiter.check("b", now + secs(40)).unwrap();
        limiter.sweep(now + secs(45));
        // `a` is full again after thirty seconds, `b` is not yet.
        assert_eq!(limiter.len(), 1);
        limiter.sweep(now + secs(70));
        assert_eq!(limiter.len(), 0);
    }

    #[test]
    fn past_its_size_a_limiter_refuses_callers_it_does_not_know() {
        let limiter = Limiter::with_max_keys(Rate::per_minute(2), 2);
        let now = Instant::now();
        limiter.check("a", now).unwrap();
        limiter.check("b", now).unwrap();
        assert_eq!(limiter.check("c", now), Err(60));
        // The ones it knows are still served.
        assert_eq!(limiter.check("a", now), Ok(()));
        // And room is made as soon as a bucket is full again.
        assert_eq!(limiter.check("c", now + secs(31)), Ok(()));
        assert_eq!(limiter.len(), 2);
    }

    fn headers(values: &[&str]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for value in values {
            headers.append(X_FORWARDED_FOR, HeaderValue::from_str(value).unwrap());
        }
        headers
    }

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn without_a_trusted_proxy_the_header_is_not_read() {
        let peer = Some(ip("203.0.113.9"));
        assert_eq!(
            client_ip(&headers(&["198.51.100.1"]), peer, 0),
            ip("203.0.113.9")
        );
    }

    #[test]
    fn behind_proxies_the_caller_is_the_entry_the_outermost_one_wrote() {
        let peer = Some(ip("10.0.0.1"));
        // One proxy: the last entry, whatever the caller put before it.
        assert_eq!(
            client_ip(&headers(&["1.2.3.4, 198.51.100.7"]), peer, 1),
            ip("198.51.100.7")
        );
        assert_eq!(
            client_ip(&headers(&["198.51.100.7"]), peer, 1),
            ip("198.51.100.7")
        );
        // Two proxies, and a list split over two headers.
        assert_eq!(
            client_ip(&headers(&["1.2.3.4, 198.51.100.7", "172.16.0.2"]), peer, 2),
            ip("198.51.100.7")
        );
    }

    #[test]
    fn a_header_that_cannot_be_trusted_falls_back_to_the_connection() {
        let peer = Some(ip("10.0.0.1"));
        assert_eq!(client_ip(&headers(&[]), peer, 1), ip("10.0.0.1"));
        // Fewer entries than proxies.
        assert_eq!(
            client_ip(&headers(&["198.51.100.7"]), peer, 2),
            ip("10.0.0.1")
        );
        assert_eq!(
            client_ip(&headers(&["not an address"]), peer, 1),
            ip("10.0.0.1")
        );
        assert_eq!(client_ip(&headers(&[]), None, 1), ip("::"));
    }

    #[test]
    fn an_ipv6_caller_is_its_64_and_a_mapped_address_is_ipv4() {
        let peer = Some(ip("10.0.0.1"));
        assert_eq!(
            client_ip(&headers(&["2001:db8:1:2:aaaa:bbbb:cccc:dddd"]), peer, 1),
            ip("2001:db8:1:2::")
        );
        assert_eq!(
            client_ip(&headers(&["::ffff:198.51.100.7"]), peer, 1),
            ip("198.51.100.7")
        );
    }
}
