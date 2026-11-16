//! The server of the optional Hangar account (docs/native/cloud.md).
//!
//! One process, one SQLite file, behind a reverse proxy that terminates TLS.
//! It signs people in with GitHub or Google, keeps the machines linked to
//! each account and the usage totals those machines choose to send: nothing
//! else, and nothing a Hangar without an account ever talks to.
//!
//! This is the skeleton: the configuration read from the environment and a
//! router that answers `GET /health`. The routes of the protocol come next,
//! each in a module of its own beside `routes.rs`.

mod config;
mod routes;

use std::process::ExitCode;

use config::Config;
use hangar_cloud_api::{Provider, paths};

#[tokio::main]
async fn main() -> ExitCode {
    let config = match Config::from_env() {
        Ok(config) => config,
        Err(err) => {
            eprintln!("hangar-cloud: {err}");
            return ExitCode::from(2);
        }
    };

    let listener = match tokio::net::TcpListener::bind(config.bind).await {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("hangar-cloud: could not listen on {}: {err}", config.bind);
            return ExitCode::FAILURE;
        }
    };
    announce(&config);

    let served = axum::serve(listener, routes::router())
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
        "hangar-cloud {} listening on {}, public at {}, database {}",
        env!("CARGO_PKG_VERSION"),
        config.bind,
        config.base_url,
        config.db.display()
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
