//! The server of the optional Hangar account (docs/native/cloud.md).
//!
//! One process, one SQLite file, behind a reverse proxy that terminates TLS.
//! It signs people in with GitHub or Google, keeps the machines linked to
//! each account and the usage totals those machines choose to send: nothing
//! else, and nothing a Hangar without an account ever talks to.
//!
//! `routes.rs` is the router and what surrounds every route; each part of
//! the protocol is a module beside it: `auth.rs` (link codes, the exchange,
//! the token's check), `account.rs`, `devices.rs`, `stats.rs`. The sign-in
//! with the providers comes next.
//!
//! Nothing here writes a token, a link code or an email address anywhere but
//! in the database (tokens and codes as their SHA-256) and in the answer that
//! is meant to carry it: not to stdout, not to stderr.

mod account;
mod auth;
mod config;
mod db;
mod devices;
mod extract;
mod limit;
mod refusal;
mod routes;
mod secrets;
mod state;
mod stats;
#[cfg(test)]
mod tests;

use std::net::SocketAddr;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use config::Config;
use db::Db;
use hangar_cloud_api::{Provider, paths};
use limit::Rates;
use state::AppState;

#[tokio::main]
async fn main() -> ExitCode {
    let config = match Config::from_env() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("hangar-cloud: {err}");
            return ExitCode::from(2);
        }
    };

    let db = match Db::open(&config.db) {
        Ok(db) => db,
        Err(err) => {
            eprintln!("hangar-cloud: {err}");
            return ExitCode::FAILURE;
        }
    };
    let state = AppState::new(db, Rates::default(), config.trusted_proxies);

    let listener = match tokio::net::TcpListener::bind(config.bind).await {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("hangar-cloud: could not listen on {}: {err}", config.bind);
            return ExitCode::FAILURE;
        }
    };
    announce(&config);

    tokio::spawn(housekeeping(state.clone()));

    // With the connection's address, which the rate limit falls back on.
    let service = routes::router(state).into_make_service_with_connect_info::<SocketAddr>();
    let served = axum::serve(listener, service)
        .with_graceful_shutdown(stop_signal())
        .await;
    match served {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("hangar-cloud: {err}");
            ExitCode::FAILURE
        }
    }
}

/// What the server runs with, said once at start: the first thing to read
/// when a deployment does not behave. Never a secret.
fn announce(config: &Config) {
    println!(
        "hangar-cloud {} listening on {}, public at {}, database {}, {} trusted proxies in front",
        env!("CARGO_PKG_VERSION"),
        config.bind,
        config.base_url,
        config.db.display(),
        config.trusted_proxies
    );
    for provider in Provider::ALL {
        match config.provider(provider) {
            // The redirect URI is the one to register with the provider.
            Some(client) => println!(
                "  {provider}: client {}, redirect URI {}{}",
                client.client_id,
                config.base_url,
                paths::auth_callback(provider)
            ),
            None => println!("  {provider}: not configured, sign-in with it is refused"),
        }
    }
}

/// Once a minute: the sign-ins nobody can finish any more leave the database
/// (five minutes at most, as the document promises), and the rate limits
/// forget the callers that have been quiet.
async fn housekeeping(state: AppState) {
    let mut tick = tokio::time::interval(Duration::from_secs(60));
    loop {
        tick.tick().await;
        let now = state.now();
        // A failure is said on stderr by the refusal itself, and tried again
        // at the next tick.
        let _ = state
            .db
            .call(move |conn| auth::purge_expired(conn, now))
            .await;
        let now = Instant::now();
        state.limits.ip.sweep(now);
        state.limits.exchange.sweep(now);
        state.limits.device.sweep(now);
    }
}

/// Ctrl-C, or the SIGTERM `docker stop` sends: requests in flight finish, and
/// the process exits before Docker's ten seconds run out.
async fn stop_signal() {
    let interrupt = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut stream) => {
                stream.recv().await;
            }
            Err(_) => std::future::pending().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = interrupt => {}
        () = terminate => {}
    }
}
