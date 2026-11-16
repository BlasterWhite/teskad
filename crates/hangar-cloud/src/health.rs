//! `hangar-cloud health`: the image's health check (its `HEALTHCHECK`, and
//! the Compose file's `depends_on: service_healthy`).
//!
//! The image holds the server's binary and nothing else, no `curl` or `wget`
//! to call `/health` with, so the binary calls it itself: one plain HTTP/1.1
//! request to the port the server listens on (`HANGAR_CLOUD_BIND`), on the
//! loopback when it listens on every address. It answers success for a
//! `200`, and anything else, a refused connection or a timeout, is a failure
//! said in one line on stderr.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::process::ExitCode;
use std::time::Duration;

use hangar_cloud_api::paths;

use crate::config::Config;

/// What a check may take, connection and answer each.
const TIMEOUT: Duration = Duration::from_secs(3);

pub fn check() -> ExitCode {
    let bind = match Config::from_env() {
        Ok(config) => config.bind,
        Err(err) => {
            eprintln!("hangar-cloud health: {err}");
            return ExitCode::from(2);
        }
    };
    match probe(target(bind)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("hangar-cloud health: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Where the server can be reached from inside its own container: the
/// address it listens on, or the loopback when that is every address.
fn target(bind: SocketAddr) -> SocketAddr {
    let ip = match bind.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    SocketAddr::new(ip, bind.port())
}

fn probe(addr: SocketAddr) -> Result<(), String> {
    let mut stream = TcpStream::connect_timeout(&addr, TIMEOUT)
        .map_err(|err| format!("could not reach {addr}: {err}"))?;
    stream
        .set_read_timeout(Some(TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(TIMEOUT)))
        .map_err(|err| format!("{addr}: {err}"))?;
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n",
        paths::HEALTH
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|err| format!("{addr}: {err}"))?;
    // The status line is all that is read: "HTTP/1.1 200 OK".
    let mut head = [0u8; 64];
    let mut len = 0;
    while len < head.len() {
        match stream.read(&mut head[len..]) {
            Ok(0) => break,
            Ok(n) => len += n,
            Err(err) => return Err(format!("{addr}: {err}")),
        }
        if head[..len].contains(&b'\n') {
            break;
        }
    }
    let line = String::from_utf8_lossy(&head[..len]);
    let line = line.lines().next().unwrap_or_default();
    match line.split_whitespace().nth(1) {
        Some("200") => Ok(()),
        _ => Err(format!("{addr} answered {line:?}")),
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::thread;

    use super::*;

    /// A server that answers one request with `status_line` and closes.
    fn answering(status_line: &'static str) -> SocketAddr {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 512];
            let _ = stream.read(&mut request);
            let _ =
                stream.write_all(format!("{status_line}\r\nContent-Length: 0\r\n\r\n").as_bytes());
        });
        addr
    }

    #[test]
    fn a_server_listening_everywhere_is_checked_on_the_loopback() {
        assert_eq!(
            target("0.0.0.0:8080".parse().unwrap()),
            "127.0.0.1:8080".parse().unwrap()
        );
        assert_eq!(
            target("[::]:8080".parse().unwrap()),
            "[::1]:8080".parse().unwrap()
        );
        assert_eq!(
            target("127.0.0.1:21120".parse().unwrap()),
            "127.0.0.1:21120".parse().unwrap()
        );
    }

    #[test]
    fn a_200_is_healthy() {
        assert_eq!(probe(answering("HTTP/1.1 200 OK")), Ok(()));
    }

    #[test]
    fn any_other_status_is_not() {
        let err = probe(answering("HTTP/1.1 503 Service Unavailable")).unwrap_err();
        assert!(err.contains("503"), "{err}");
    }

    #[test]
    fn nobody_listening_is_not() {
        let addr = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .local_addr()
            .unwrap();
        // The listener is dropped: the port is closed.
        let err = probe(addr).unwrap_err();
        assert!(err.contains("could not reach"), "{err}");
    }

    #[test]
    fn the_real_server_s_health_route_answers_200() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)))
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let config = Config::from_lookup(|_| None).unwrap();
        let state = runtime.block_on(async {
            crate::state::AppState::new(
                crate::db::Db::in_memory(),
                crate::limit::Rates::default(),
                0,
                crate::state::SignIn::from_config(&config),
            )
        });
        runtime.spawn(async move {
            let service =
                crate::routes::router(state).into_make_service_with_connect_info::<SocketAddr>();
            axum::serve(listener, service).await.unwrap();
        });
        let checked = thread::spawn(move || probe(addr)).join().unwrap();
        assert_eq!(checked, Ok(()));
    }
}
