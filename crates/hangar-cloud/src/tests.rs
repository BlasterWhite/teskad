//! The server through its router, as a desktop meets it: every route, who
//! may see and change what, and what each refusal looks like. Accounts and
//! link codes are made directly, as the sign-in routes will make them.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hangar_cloud_api::{
    API_VERSION, Account, ApiError, Device, DevicesResponse, ErrorCode, ExchangeResponse, Health,
    MAX_BODY_BYTES, MAX_STATS_ROWS, MeResponse, Provider, PutStatsResponse, StatsResponse, paths,
};
use http_body_util::BodyExt;
use rusqlite::params;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

mod fake;
mod sign_in;

use crate::account::{self, NewIdentity};
use crate::auth;
use crate::config::OAuthClient;
use crate::db::Db;
use crate::limit::{Rate, Rates};
use crate::provider::Endpoints;
use crate::routes::router;
use crate::secrets;
use crate::state::{AppState, SignIn};

/// When the tests' clock starts.
const T0: i64 = 1_794_000_000_000;
const MINUTE: i64 = 60_000;

/// A verifier as a desktop draws one, and another.
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const OTHER_VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXl";

fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

struct Server {
    router: Router,
    state: AppState,
    clock: Arc<AtomicI64>,
}

struct Answer {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl Answer {
    fn json<T: DeserializeOwned>(&self) -> T {
        assert_eq!(
            self.headers.get(header::CONTENT_TYPE).unwrap(),
            "application/json"
        );
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|err| panic!("{err}: {}", String::from_utf8_lossy(&self.body)))
    }

    /// The answer is the refusal `code`, with its status, in the protocol's
    /// shape. Gives its message.
    #[track_caller]
    fn refused(&self, code: ErrorCode) -> String {
        let error: ApiError = self.json();
        assert_eq!(error.error, code, "{}", error.message);
        assert_eq!(self.status.as_u16(), code.status());
        error.message
    }
}

impl Server {
    fn new() -> Self {
        // Limits no test reaches by accident.
        let generous = Rate::per_minute(1_000_000);
        Self::with(
            Db::in_memory(),
            Rates {
                ip: generous,
                exchange: generous,
                start: generous,
                device: generous,
            },
        )
    }

    fn with(db: Db, rates: Rates) -> Self {
        Self::with_sign_in(db, rates, sign_in())
    }

    fn with_sign_in(db: Db, rates: Rates, sign_in: SignIn) -> Self {
        let clock = Arc::new(AtomicI64::new(T0));
        let ticking = Arc::clone(&clock);
        // One trusted proxy, as deployed: a test chooses its address with
        // `X-Forwarded-For`.
        let state =
            AppState::new(db, rates, 1, sign_in).with_clock(move || ticking.load(Ordering::SeqCst));
        Self {
            router: router(state.clone()),
            state,
            clock,
        }
    }

    fn advance(&self, ms: i64) {
        self.clock.fetch_add(ms, Ordering::SeqCst);
    }

    async fn send(&self, request: Request<Body>) -> Answer {
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        Answer {
            status,
            headers,
            body: body.to_vec(),
        }
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> Answer {
        let mut request = Request::builder().method(method).uri(path);
        if let Some(token) = token {
            request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let body = match body {
            Some(body) => {
                request = request.header(header::CONTENT_TYPE, "application/json");
                Body::from(serde_json::to_vec(&body).unwrap())
            }
            None => Body::empty(),
        };
        self.send(request.body(body).unwrap()).await
    }

    async fn get(&self, path: &str, token: &str) -> Answer {
        self.call(Method::GET, path, Some(token), None).await
    }

    /// A new account, as a first sign-in with `provider` makes one.
    async fn account(&self, name: &str, provider: Provider) -> String {
        let identity = identity(name, provider);
        let now = self.state.now();
        self.state
            .db
            .call(move |conn| account::create(conn, &identity, now))
            .await
            .unwrap()
    }

    /// A link code for the account, for a desktop that holds `VERIFIER`.
    async fn link_code(&self, account_id: &str) -> String {
        let account_id = account_id.to_string();
        let now = self.state.now();
        self.state
            .db
            .call(move |conn| auth::create_link_code(conn, &account_id, &challenge(VERIFIER), now))
            .await
            .unwrap()
    }

    async fn exchange(&self, code: &str, verifier: &str, machine: Value) -> Answer {
        let body = json!({ "code": code, "verifier": verifier, "machine": machine });
        self.call(Method::POST, paths::AUTH_EXCHANGE, None, Some(body))
            .await
    }

    /// The whole end of a sign-in: a link code, exchanged by `machine_id`.
    async fn sign_in(&self, account_id: &str, machine_id: &str) -> ExchangeResponse {
        let code = self.link_code(account_id).await;
        let answer = self
            .exchange(&code, VERIFIER, machine(machine_id, machine_id))
            .await;
        assert_eq!(answer.status, StatusCode::OK);
        answer.json()
    }

    async fn put_stats(&self, token: &str, rows: Value) -> Answer {
        self.call(
            Method::PUT,
            paths::STATS,
            Some(token),
            Some(json!({ "rows": rows })),
        )
        .await
    }

    async fn stats(&self, token: &str, query: &str) -> StatsResponse {
        let answer = self.get(&format!("{}{query}", paths::STATS), token).await;
        assert_eq!(answer.status, StatusCode::OK);
        answer.json()
    }

    async fn devices(&self, token: &str) -> Vec<Device> {
        let answer = self.get(paths::DEVICES, token).await;
        assert_eq!(answer.status, StatusCode::OK);
        answer.json::<DevicesResponse>().devices
    }

    /// How many rows `table` holds, for the account or in all.
    async fn count(&self, sql: &'static str, account_id: &str) -> i64 {
        let account_id = account_id.to_string();
        self.state
            .db
            .call(move |conn| Ok(conn.query_row(sql, params![account_id], |row| row.get(0))?))
            .await
            .unwrap()
    }
}

/// The server's public address in the tests. Nothing listens there: the
/// tests play the browser, and hand the router what a browser would ask it.
const BASE_URL: &str = "https://account.test";

/// Both providers configured, at the stand-in for them (`tests/fake.rs`).
fn sign_in() -> SignIn {
    SignIn {
        base_url: BASE_URL.into(),
        github: Some(OAuthClient {
            client_id: fake::GITHUB_CLIENT.0.into(),
            client_secret: fake::GITHUB_CLIENT.1.into(),
        }),
        google: Some(OAuthClient {
            client_id: fake::GOOGLE_CLIENT.0.into(),
            client_secret: fake::GOOGLE_CLIENT.1.into(),
        }),
        endpoints: Endpoints::fake(fake::provider().base()),
    }
}

fn identity(name: &str, provider: Provider) -> NewIdentity {
    NewIdentity {
        provider,
        provider_user_id: format!("{provider}-{name}"),
        display_name: name.to_string(),
        email: Some(format!("{}@example.org", name.to_lowercase())),
        avatar_url: None,
    }
}

fn machine(id: &str, name: &str) -> Value {
    json!({ "id": id, "name": name, "os": "windows" })
}

fn row(day: &str, project: &str, input_tokens: u64) -> Value {
    json!({
        "day": day,
        "model": "claude-opus-5-5",
        "project": project,
        "input_tokens": input_tokens,
        "output_tokens": 2,
        "cache_read_tokens": 3,
        "cache_write_tokens": 4,
        "cost_usd_micros": 5,
        "active_seconds": 6,
        "sessions": 7,
    })
}

// ---------------------------------------------------------------- the router

#[tokio::test]
async fn health_says_ok_and_which_protocol_the_server_speaks() {
    let server = Server::new();
    let answer = server.call(Method::GET, paths::HEALTH, None, None).await;
    assert_eq!(answer.status, StatusCode::OK);
    let health: Health = answer.json();
    assert_eq!(health.status, "ok");
    assert_eq!(health.api_version, API_VERSION);
    assert_eq!(health.version, env!("CARGO_PKG_VERSION"));
}

#[tokio::test]
async fn a_route_that_does_not_exist_is_refused_in_the_protocol_s_words() {
    let server = Server::new();
    for path in ["/", "/v1/nothing", "/v1/devices/a/b", "/v2/me"] {
        let answer = server.call(Method::GET, path, None, None).await;
        answer.refused(ErrorCode::NotFound);
    }
}

#[tokio::test]
async fn a_method_a_route_does_not_take_is_refused_the_same_way() {
    let server = Server::new();
    for (method, path) in [
        (Method::POST, paths::HEALTH),
        (Method::POST, paths::ME),
        (Method::GET, paths::AUTH_EXCHANGE),
        (Method::POST, paths::STATS),
        (Method::GET, paths::ACCOUNT),
        (Method::PUT, "/v1/devices/some-id"),
    ] {
        let answer = server.call(method, path, None, None).await;
        answer.refused(ErrorCode::MethodNotAllowed);
    }
}

#[tokio::test]
async fn every_answer_tells_browsers_and_caches_to_keep_out() {
    let server = Server::new();
    let account = server.account("Ada", Provider::Github).await;
    let token = server.sign_in(&account, "m1").await.token;
    for answer in [
        server.call(Method::GET, paths::HEALTH, None, None).await,
        server.get(paths::ME, &token).await,
        server.call(Method::GET, paths::ME, None, None).await,
        server.call(Method::GET, "/nowhere", None, None).await,
        server.call(Method::POST, paths::HEALTH, None, None).await,
    ] {
        let header = |name| answer.headers.get(name).unwrap().to_str().unwrap();
        assert_eq!(header(header::CACHE_CONTROL), "no-store");
        assert_eq!(header(header::X_CONTENT_TYPE_OPTIONS), "nosniff");
        assert_eq!(header(header::X_FRAME_OPTIONS), "DENY");
        assert_eq!(header(header::REFERRER_POLICY), "no-referrer");
        assert!(header(header::CONTENT_SECURITY_POLICY).contains("default-src 'none'"));
        assert!(header(header::STRICT_TRANSPORT_SECURITY).starts_with("max-age="));
    }
}

// -------------------------------------------------------------- the exchange

#[tokio::test]
async fn an_exchange_gives_the_token_the_account_and_the_device() {
    let server = Server::new();
    let account = server.account("Ada", Provider::Github).await;
    let code = server.link_code(&account).await;
    let answer = server
        .exchange(&code, VERIFIER, machine("m1", "Atelier"))
        .await;
    assert_eq!(answer.status, StatusCode::OK);
    let exchanged: ExchangeResponse = answer.json();

    assert!(exchanged.token.starts_with("hgr_"));
    assert_eq!(exchanged.token.len(), 4 + 43);
    assert_eq!(
        exchanged.account,
        Account {
            id: account.clone(),
            display_name: "Ada".into(),
            email: Some("ada@example.org".into()),
            avatar_url: None,
            providers: vec![Provider::Github],
        }
    );
    assert_eq!(exchanged.device.name, "Atelier");
    assert_eq!(exchanged.device.os, "windows");
    assert_eq!(exchanged.device.created_at, T0);
    assert_eq!(exchanged.device.last_seen_at, T0);
    assert_eq!(exchanged.device.revoked_at, None);
    // The server's id, not the machine's.
    assert_ne!(exchanged.device.id, "m1");

    // And the token works.
    let me: MeResponse = server.get(paths::ME, &exchanged.token).await.json();
    assert_eq!(me.account, exchanged.account);
    assert_eq!(me.device, exchanged.device);
}

#[tokio::test]
async fn the_database_holds_neither_the_token_nor_the_code_only_their_hashes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cloud.sqlite");
    let server = Server::with(Db::open(&path).unwrap(), Rates::default());
    let account = server.account("Ada", Provider::Github).await;
    let code = server.link_code(&account).await;
    let answer = server.exchange(&code, VERIFIER, machine("m1", "m1")).await;
    let token = answer.json::<ExchangeResponse>().token;

    let hash = secrets::sha256(&token);
    let found = server
        .count(
            "SELECT count(*) FROM devices WHERE account_id = ?1 AND token_hash IS NOT NULL",
            &account,
        )
        .await;
    assert_eq!(found, 1);
    let stored: Vec<u8> = server
        .state
        .db
        .call(|conn| Ok(conn.query_row("SELECT token_hash FROM devices", [], |r| r.get(0))?))
        .await
        .unwrap();
    assert_eq!(stored, hash);

    // Not in the file either, nor in its write-ahead log.
    let contains = |haystack: &[u8], needle: &str| {
        haystack
            .windows(needle.len())
            .any(|window| window == needle.as_bytes())
    };
    let mut files = 0;
    for entry in std::fs::read_dir(dir.path()).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        files += 1;
        assert!(!contains(&bytes, &token));
        assert!(!contains(&bytes, &token["hgr_".len()..]));
        assert!(!contains(&bytes, &code));
        assert!(!contains(&bytes, VERIFIER));
    }
    assert!(files >= 1);
}

#[tokio::test]
async fn a_link_code_works_once() {
    let server = Server::new();
    let account = server.account("Ada", Provider::Github).await;
    let code = server.link_code(&account).await;
    let first = server.exchange(&code, VERIFIER, machine("m1", "m1")).await;
    assert_eq!(first.status, StatusCode::OK);
    let second = server.exchange(&code, VERIFIER, machine("m1", "m1")).await;
    second.refused(ErrorCode::InvalidCode);
    // The first exchange's token is not disturbed by the second's failure.
    let token = first.json::<ExchangeResponse>().token;
    assert_eq!(server.get(paths::ME, &token).await.status, StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_link_code_presented_many_times_at_once_still_works_once() {
    let server = Arc::new(Server::new());
    let account = server.account("Ada", Provider::Github).await;
    let code = server.link_code(&account).await;
    let mut calls = Vec::new();
    for index in 0..12 {
        let (server, code) = (Arc::clone(&server), code.clone());
        calls.push(tokio::spawn(async move {
            let id = format!("m{index}");
            server
                .exchange(&code, VERIFIER, machine(&id, &id))
                .await
                .status
        }));
    }
    let mut passed = 0;
    for call in calls {
        match call.await.unwrap() {
            StatusCode::OK => passed += 1,
            status => assert_eq!(status, StatusCode::BAD_REQUEST),
        }
    }
    assert_eq!(passed, 1);
    assert_eq!(
        server
            .count(
                "SELECT count(*) FROM devices WHERE account_id = ?1",
                &account
            )
            .await,
        1
    );
}

#[tokio::test]
async fn a_link_code_expires_five_minutes_after_it_was_drawn() {
    let server = Server::new();
    let account = server.account("Ada", Provider::Github).await;

    let late = server.link_code(&account).await;
    server.advance(5 * MINUTE);
    server
        .exchange(&late, VERIFIER, machine("m1", "m1"))
        .await
        .refused(ErrorCode::InvalidCode);
    // And an expired code is gone from the database, not only refused.
    assert_eq!(
        server
            .count(
                "SELECT count(*) FROM link_codes WHERE account_id = ?1",
                &account
            )
            .await,
        0
    );

    let in_time = server.link_code(&account).await;
    server.advance(5 * MINUTE - 1);
    let answer = server
        .exchange(&in_time, VERIFIER, machine("m1", "m1"))
        .await;
    assert_eq!(answer.status, StatusCode::OK);
}

#[tokio::test]
async fn the_wrong_verifier_is_refused_and_costs_the_code() {
    let server = Server::new();
    let account = server.account("Ada", Provider::Github).await;
    let code = server.link_code(&account).await;
    let wrong = server
        .exchange(&code, OTHER_VERIFIER, machine("m1", "m1"))
        .await;
    let said = wrong.refused(ErrorCode::InvalidCode);
    // The right one comes too late, and is told the same thing.
    let right = server.exchange(&code, VERIFIER, machine("m1", "m1")).await;
    assert_eq!(right.refused(ErrorCode::InvalidCode), said);
    assert_eq!(
        server
            .count(
                "SELECT count(*) FROM devices WHERE account_id = ?1",
                &account
            )
            .await,
        0
    );
}

#[tokio::test]
async fn a_verifier_that_is_not_one_and_a_code_nobody_drew_are_refused_alike() {
    let server = Server::new();
    let account = server.account("Ada", Provider::Github).await;
    let said = server
        .exchange("no-such-code", VERIFIER, machine("m1", "m1"))
        .await
        .refused(ErrorCode::InvalidCode);
    for verifier in ["", "short", &"a".repeat(200)] {
        let code = server.link_code(&account).await;
        let answer = server.exchange(&code, verifier, machine("m1", "m1")).await;
        assert_eq!(answer.refused(ErrorCode::InvalidCode), said);
    }
    // The challenge itself is not a verifier.
    let code = server.link_code(&account).await;
    server
        .exchange(&code, &challenge(VERIFIER), machine("m1", "m1"))
        .await
        .refused(ErrorCode::InvalidCode);
}

#[tokio::test]
async fn a_machine_the_server_would_not_store_is_a_bad_request_and_keeps_the_code() {
    let server = Server::new();
    let account = server.account("Ada", Provider::Github).await;
    let code = server.link_code(&account).await;
    let long = "x".repeat(101);
    for machine in [
        json!({ "id": "", "name": "Atelier", "os": "windows" }),
        json!({ "id": "m1", "name": "", "os": "windows" }),
        json!({ "id": "m1", "name": "Atelier", "os": "" }),
        json!({ "id": long, "name": "Atelier", "os": "windows" }),
        json!({ "id": "m1", "name": long, "os": "windows" }),
        json!({ "id": "m1", "name": "Atelier", "os": long }),
        json!({ "id": "m1", "name": "Ate\nlier", "os": "windows" }),
    ] {
        let answer = server.exchange(&code, VERIFIER, machine).await;
        answer.refused(ErrorCode::BadRequest);
    }
    // A hundred characters, of three bytes each, fit.
    let answer = server
        .exchange(&code, VERIFIER, machine("m1", &"格".repeat(100)))
        .await;
    assert_eq!(answer.status, StatusCode::OK);
}

#[tokio::test]
async fn a_body_that_is_not_the_exchange_is_a_bad_request_without_the_parser_s_words() {
    let server = Server::new();
    let post = |body: &'static str, content_type: &'static str| {
        Request::builder()
            .method(Method::POST)
            .uri(paths::AUTH_EXCHANGE)
            .header(header::CONTENT_TYPE, content_type)
            .body(Body::from(body))
            .unwrap()
    };
    for request in [
        post("", "application/json"),
        post("{", "application/json"),
        post("[]", "application/json"),
        post(r#"{"code": "secret-code"}"#, "application/json"),
        post(
            r#"{"code": 12, "verifier": "v", "machine": {}}"#,
            "application/json",
        ),
        post(
            r#"{"code":"c","verifier":"v","machine":{"id":"m","name":"n","os":"o"}}"#,
            "text/plain",
        ),
    ] {
        let answer = server.send(request).await;
        let message = answer.refused(ErrorCode::BadRequest);
        assert_eq!(message, "the body is not the JSON this route takes");
    }
}

#[tokio::test]
async fn the_same_machine_signing_in_again_is_the_same_device_with_a_new_token() {
    let server = Server::new();
    let account = server.account("Ada", Provider::Github).await;
    let first = server.sign_in(&account, "m1").await;
    assert_eq!(
        server
            .put_stats(&first.token, json!([row("2026-11-16", "hangar", 1)]))
            .await
            .status,
        StatusCode::OK
    );

    server.advance(10 * MINUTE);
    let code = server.link_code(&account).await;
    let second: ExchangeResponse = server
        .exchange(&code, VERIFIER, machine("m1", "Renamed"))
        .await
        .json();

    assert_eq!(second.device.id, first.device.id);
    assert_eq!(second.device.created_at, first.device.created_at);
    assert_eq!(second.device.last_seen_at, T0 + 10 * MINUTE);
    assert_eq!(second.device.name, "Renamed");
    assert_ne!(second.token, first.token);
    // The previous token stops working, the rows are kept.
    server
        .get(paths::ME, &first.token)
        .await
        .refused(ErrorCode::Unauthorized);
    assert_eq!(server.devices(&second.token).await.len(), 1);
    assert_eq!(server.stats(&second.token, "").await.rows.len(), 1);
}

#[tokio::test]
async fn a_signed_out_machine_signing_in_again_finds_its_device_and_its_rows() {
    let server = Server::new();
    let account = server.account("Ada", Provider::Github).await;
    let first = server.sign_in(&account, "m1").await;
    server
        .put_stats(&first.token, json!([row("2026-11-16", "hangar", 1)]))
        .await;
    let answer = server
        .call(
            Method::DELETE,
            &paths::device(&first.device.id),
            Some(&first.token),
            None,
        )
        .await;
    assert_eq!(answer.status, StatusCode::NO_CONTENT);

    let again = server.sign_in(&account, "m1").await;
    assert_eq!(again.device.id, first.device.id);
    assert_eq!(again.device.revoked_at, None);
    assert_eq!(server.stats(&again.token, "").await.rows.len(), 1);
    // Another account's machine with the same id is another device.
    let other = server.account("Bob", Provider::Google).await;
    let bob = server.sign_in(&other, "m1").await;
    assert_ne!(bob.device.id, first.device.id);
    assert!(server.stats(&bob.token, "").await.rows.is_empty());
}

// ------------------------------------------------------------------ the token

#[tokio::test]
async fn every_route_with_a_token_refuses_without_a_valid_one() {
    let server = Server::new();
    let account = server.account("Ada", Provider::Github).await;
    let signed_in = server.sign_in(&account, "m1").await;
    let token = signed_in.token;
    let device = paths::device(&signed_in.device.id);
    let stranger = secrets::new_device_token().unwrap();

    let routes = [
        (Method::GET, paths::ME.to_string()),
        (Method::GET, paths::DEVICES.to_string()),
        (Method::PATCH, device.clone()),
        (Method::DELETE, device.clone()),
        (Method::DELETE, format!("{device}?forget=true")),
        (Method::PUT, paths::STATS.to_string()),
        (Method::GET, paths::STATS.to_string()),
        (Method::DELETE, paths::ACCOUNT.to_string()),
    ];
    let wrong_headers = [
        None,
        Some(String::new()),
        Some("Bearer".to_string()),
        Some("Bearer ".to_string()),
        Some(format!("Bearer {stranger}")),
        // A token is a token whole: not its start, not without its prefix,
        // not in another scheme.
        Some(format!("Bearer {}", &token[..token.len() - 1])),
        Some(format!("Bearer {}", &token["hgr_".len()..])),
        Some(format!("Basic {token}")),
        Some(token.clone()),
        Some(format!("Bearer {token} {token}")),
    ];
    let mut said = None;
    for (method, path) in &routes {
        for wrong in &wrong_headers {
            let mut request = Request::builder().method(method).uri(path);
            if let Some(value) = wrong {
                request = request.header(header::AUTHORIZATION, value);
            }
            // Refused before the body is looked at: this one is not JSON.
            let answer = server
                .send(request.body(Body::from("not json")).unwrap())
                .await;
            let message = answer.refused(ErrorCode::Unauthorized);
            // One sentence, whatever was wrong with the token.
            assert_eq!(said.get_or_insert(message.clone()), &message);
        }
    }
    // Nothing of that touched the account.
    let devices = server.devices(&token).await;
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].revoked_at, None);
    // And the scheme's case does not matter.
    let request = Request::builder()
        .uri(paths::ME)
        .header(header::AUTHORIZATION, format!("bearer {token}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(server.send(request).await.status, StatusCode::OK);
}

#[tokio::test]
async fn last_seen_moves_at_most_once_a_minute() {
    let server = Server::new();
    let account = server.account("Ada", Provider::Github).await;
    let token = server.sign_in(&account, "m1").await.token;
    let seen = |me: MeResponse| me.device.last_seen_at;

    server.advance(MINUTE - 1);
    assert_eq!(seen(server.get(paths::ME, &token).await.json()), T0);
    server.advance(1);
    assert_eq!(
        seen(server.get(paths::ME, &token).await.json()),
        T0 + MINUTE
    );
    server.advance(30_000);
    assert_eq!(
        seen(server.get(paths::ME, &token).await.json()),
        T0 + MINUTE
    );
    assert_eq!(server.devices(&token).await[0].last_seen_at, T0 + MINUTE);
    server.advance(30_000);
    assert_eq!(
        server.devices(&token).await[0].last_seen_at,
        T0 + 2 * MINUTE
    );
}

// ------------------------------------------------------------------------ me

#[tokio::test]
async fn me_is_the_account_with_every_provider_and_the_device_asking() {
    let server = Server::new();
    let account = server.account("Ada", Provider::Google).await;
    let later = NewIdentity {
        display_name: "Ada L.".into(),
        avatar_url: Some("https://example.org/ada.png".into()),
        ..identity("Ada", Provider::Github)
    };
    let (id, now) = (account.clone(), T0 + 1);
    server
        .state
        .db
        .call(move |conn| account::add_identity(conn, &id, &later, now))
        .await
        .unwrap();
    let first = server.sign_in(&account, "m1").await;
    let second = server.sign_in(&account, "m2").await;

    let me: MeResponse = server.get(paths::ME, &second.token).await.json();
    // The profile of the first identity, the providers of both.
    assert_eq!(me.account.display_name, "Ada");
    assert_eq!(me.account.avatar_url, None);
    assert_eq!(me.account.providers, [Provider::Google, Provider::Github]);
    assert_eq!(me.device.id, second.device.id);
    assert_ne!(me.device.id, first.device.id);

    // What the provider did not give is left out, not null.
    let bare = NewIdentity {
        email: None,
        ..identity("Bob", Provider::Github)
    };
    let bob = server
        .state
        .db
        .call(move |conn| account::create(conn, &bare, T0))
        .await
        .unwrap();
    let token = server.sign_in(&bob, "m1").await.token;
    let me: Value = server.get(paths::ME, &token).await.json();
    assert_eq!(
        me["account"],
        json!({ "id": bob, "display_name": "Bob", "providers": ["github"] })
    );
    assert!(me["device"].get("revoked_at").is_none());
}

// ------------------------------------------------------------------- devices

#[tokio::test]
async fn devices_lists_the_account_s_machines_and_no_other_s() {
    let server = Server::new();
    let ada = server.account("Ada", Provider::Github).await;
    let bob = server.account("Bob", Provider::Github).await;
    let a1 = server.sign_in(&ada, "m1").await;
    server.advance(1);
    let a2 = server.sign_in(&ada, "m2").await;
    let b1 = server.sign_in(&bob, "m1").await;

    // The oldest first, from either machine.
    for token in [&a1.token, &a2.token] {
        let ids: Vec<String> = server
            .devices(token)
            .await
            .into_iter()
            .map(|d| d.id)
            .collect();
        assert_eq!(ids, [a1.device.id.clone(), a2.device.id.clone()]);
    }
    let bobs = server.devices(&b1.token).await;
    assert_eq!(bobs, [b1.device]);
}

#[tokio::test]
async fn a_device_is_renamed_by_any_device_of_its_account() {
    let server = Server::new();
    let account = server.account("Ada", Provider::Github).await;
    let a1 = server.sign_in(&account, "m1").await;
    let a2 = server.sign_in(&account, "m2").await;
    let path = paths::device(&a1.device.id);
    let rename = |name: &str| {
        server.call(
            Method::PATCH,
            &path,
            Some(&a2.token),
            Some(json!({ "name": name })),
        )
    };

    let answer = rename("Portable").await;
    assert_eq!(answer.status, StatusCode::OK);
    let renamed: Device = answer.json();
    assert_eq!(renamed.id, a1.device.id);
    assert_eq!(renamed.name, "Portable");
    let me: MeResponse = server.get(paths::ME, &a1.token).await.json();
    assert_eq!(me.device.name, "Portable");

    // 1 to 100 characters, on one line.
    assert_eq!(rename(&"格".repeat(100)).await.status, StatusCode::OK);
    for name in ["", &"x".repeat(101), "two\nlines"] {
        rename(name).await.refused(ErrorCode::BadRequest);
    }
    let answer = server
        .call(
            Method::PATCH,
            &path,
            Some(&a2.token),
            Some(json!({ "nom": "x" })),
        )
        .await;
    answer.refused(ErrorCode::BadRequest);
    let me: MeResponse = server.get(paths::ME, &a1.token).await.json();
    assert_eq!(me.device.name, "格".repeat(100));
}

#[tokio::test]
async fn another_account_s_device_is_not_found_whatever_is_asked_of_it() {
    let server = Server::new();
    let ada = server.account("Ada", Provider::Github).await;
    let bob = server.account("Bob", Provider::Github).await;
    let a = server.sign_in(&ada, "m1").await;
    let b = server.sign_in(&bob, "m1").await;
    server
        .put_stats(&a.token, json!([row("2026-11-16", "hangar", 1)]))
        .await;
    let hers = paths::device(&a.device.id);

    let said = server
        .call(
            Method::DELETE,
            &paths::device("no-such-device"),
            Some(&b.token),
            None,
        )
        .await
        .refused(ErrorCode::NotFound);
    for answer in [
        server
            .call(
                Method::PATCH,
                &hers,
                Some(&b.token),
                Some(json!({ "name": "Mine now" })),
            )
            .await,
        server
            .call(Method::DELETE, &hers, Some(&b.token), None)
            .await,
        server
            .call(
                Method::DELETE,
                &format!("{hers}?forget=true"),
                Some(&b.token),
                None,
            )
            .await,
    ] {
        // Exactly what a device that does not exist is answered.
        assert_eq!(answer.refused(ErrorCode::NotFound), said);
    }

    // Hers is as it was: named, signed in, with its rows.
    let me: MeResponse = server.get(paths::ME, &a.token).await.json();
    assert_eq!(me.device, a.device);
    assert_eq!(server.stats(&a.token, "").await.rows.len(), 1);
    // And he sees nothing of hers.
    assert_eq!(server.devices(&b.token).await, [b.device]);
    let his = server.stats(&b.token, "").await;
    assert!(his.rows.is_empty() && his.devices.is_empty());
}

#[tokio::test]
async fn signing_a_device_out_destroys_its_token_and_keeps_its_rows() {
    let server = Server::new();
    let account = server.account("Ada", Provider::Github).await;
    let a1 = server.sign_in(&account, "m1").await;
    server.advance(1);
    let a2 = server.sign_in(&account, "m2").await;
    server
        .put_stats(&a1.token, json!([row("2026-11-16", "hangar", 1)]))
        .await;
    let path = paths::device(&a1.device.id);

    // By another device of the account.
    server.advance(1000);
    let answer = server
        .call(Method::DELETE, &path, Some(&a2.token), None)
        .await;
    assert_eq!(answer.status, StatusCode::NO_CONTENT);
    assert!(answer.body.is_empty());

    // At once, on every route.
    server
        .get(paths::ME, &a1.token)
        .await
        .refused(ErrorCode::Unauthorized);
    server
        .put_stats(&a1.token, json!([row("2026-11-17", "hangar", 1)]))
        .await
        .refused(ErrorCode::Unauthorized);
    assert_eq!(
        server
            .count(
                "SELECT count(*) FROM devices WHERE account_id = ?1 AND token_hash IS NOT NULL",
                &account
            )
            .await,
        1
    );

    // Still listed, with when it was signed out, and its rows still read.
    let devices = server.devices(&a2.token).await;
    assert_eq!(devices.len(), 2);
    assert_eq!(devices[0].id, a1.device.id);
    assert_eq!(devices[0].revoked_at, Some(T0 + 1001));
    assert_eq!(devices[1].revoked_at, None);
    let stats = server.stats(&a2.token, "").await;
    assert_eq!(stats.rows.len(), 1);
    assert_eq!(stats.devices.len(), 1);
    assert_eq!(stats.devices[0].revoked_at, Some(T0 + 1001));

    // Again changes nothing, not even the time.
    server.advance(1000);
    let answer = server
        .call(Method::DELETE, &path, Some(&a2.token), None)
        .await;
    assert_eq!(answer.status, StatusCode::NO_CONTENT);
    assert_eq!(
        server.devices(&a2.token).await[0].revoked_at,
        Some(T0 + 1001)
    );
}

#[tokio::test]
async fn a_device_signs_itself_out() {
    let server = Server::new();
    let account = server.account("Ada", Provider::Github).await;
    let a1 = server.sign_in(&account, "m1").await;
    let path = paths::device(&a1.device.id);
    let answer = server
        .call(Method::DELETE, &path, Some(&a1.token), None)
        .await;
    assert_eq!(answer.status, StatusCode::NO_CONTENT);
    server
        .call(Method::DELETE, &path, Some(&a1.token), None)
        .await
        .refused(ErrorCode::Unauthorized);
}

#[tokio::test]
async fn forgetting_a_device_deletes_it_and_every_row_it_sent() {
    let server = Server::new();
    let account = server.account("Ada", Provider::Github).await;
    let a1 = server.sign_in(&account, "m1").await;
    let a2 = server.sign_in(&account, "m2").await;
    server
        .put_stats(&a1.token, json!([row("2026-11-16", "hangar", 1)]))
        .await;
    server
        .put_stats(&a2.token, json!([row("2026-11-16", "hangar", 2)]))
        .await;
    let path = format!("{}?forget=true", paths::device(&a1.device.id));

    let answer = server
        .call(Method::DELETE, &path, Some(&a2.token), None)
        .await;
    assert_eq!(answer.status, StatusCode::NO_CONTENT);

    server
        .get(paths::ME, &a1.token)
        .await
        .refused(ErrorCode::Unauthorized);
    assert_eq!(
        server.devices(&a2.token).await,
        std::slice::from_ref(&a2.device)
    );
    let stats = server.stats(&a2.token, "").await;
    assert_eq!(stats.rows.len(), 1);
    assert_eq!(stats.rows[0].device_id, a2.device.id);
    assert_eq!(
        server
            .count("SELECT count(*) FROM stats WHERE ?1 = ?1", &account)
            .await,
        1
    );
    // It is gone: forgetting it again finds nothing.
    server
        .call(Method::DELETE, &path, Some(&a2.token), None)
        .await
        .refused(ErrorCode::NotFound);
    // A signed-out device is forgotten the same way, and `forget` is a
    // boolean or a bad request.
    let own = paths::device(&a2.device.id);
    server
        .call(
            Method::DELETE,
            &format!("{own}?forget=perhaps"),
            Some(&a2.token),
            None,
        )
        .await
        .refused(ErrorCode::BadRequest);
    let answer = server
        .call(
            Method::DELETE,
            &format!("{own}?forget=false"),
            Some(&a2.token),
            None,
        )
        .await;
    assert_eq!(answer.status, StatusCode::NO_CONTENT);
    assert_eq!(
        server
            .count("SELECT count(*) FROM stats WHERE ?1 = ?1", &account)
            .await,
        1
    );
}

// --------------------------------------------------------------------- stats

#[tokio::test]
async fn rows_are_totals_sending_them_twice_changes_nothing() {
    let server = Server::new();
    let account = server.account("Ada", Provider::Github).await;
    let a = server.sign_in(&account, "m1").await;
    let rows = json!([
        row("2026-11-15", "hangar", 10),
        row("2026-11-16", "hangar", 20),
        row("2026-11-16", "", 30),
    ]);

    let answer = server.put_stats(&a.token, rows.clone()).await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(answer.json::<PutStatsResponse>().upserted, 3);
    let first = server.stats(&a.token, "").await;
    assert_eq!(first.rows.len(), 3);
    assert_eq!(first.devices, std::slice::from_ref(&a.device));

    server.advance(1000);
    let answer = server.put_stats(&a.token, rows).await;
    assert_eq!(answer.json::<PutStatsResponse>().upserted, 3);
    assert_eq!(server.stats(&a.token, "").await, first);

    // On the wire a row is the row sent, with its device beside it.
    let wire: Value = server.get(paths::STATS, &a.token).await.json();
    let mut expected = row("2026-11-15", "hangar", 10);
    expected["device_id"] = json!(a.device.id);
    assert_eq!(wire["rows"][0], expected);
}

#[tokio::test]
async fn a_row_replaces_the_row_of_its_key_and_leaves_the_others() {
    let server = Server::new();
    let account = server.account("Ada", Provider::Github).await;
    let a = server.sign_in(&account, "m1").await;
    server
        .put_stats(
            &a.token,
            json!([
                row("2026-11-15", "hangar", 10),
                row("2026-11-16", "hangar", 20)
            ]),
        )
        .await;

    // The day in progress, sent again with its new totals; and a key sent
    // twice in one call, which keeps its last row and counts once.
    let mut other_model = row("2026-11-16", "hangar", 7);
    other_model["model"] = json!("claude-haiku-5-5");
    let answer = server
        .put_stats(
            &a.token,
            json!([
                row("2026-11-16", "hangar", 21),
                row("2026-11-16", "hangar", 25),
                other_model,
            ]),
        )
        .await;
    assert_eq!(answer.json::<PutStatsResponse>().upserted, 2);

    let stats = server.stats(&a.token, "").await;
    let totals: Vec<(&str, &str, u64)> = stats
        .rows
        .iter()
        .map(|r| (r.row.day.as_str(), r.row.model.as_str(), r.row.input_tokens))
        .collect();
    assert_eq!(
        totals,
        [
            ("2026-11-15", "claude-opus-5-5", 10),
            ("2026-11-16", "claude-haiku-5-5", 7),
            ("2026-11-16", "claude-opus-5-5", 25),
        ]
    );
    // An empty call is a call: nothing changes.
    let answer = server.put_stats(&a.token, json!([])).await;
    assert_eq!(answer.json::<PutStatsResponse>().upserted, 0);
    assert_eq!(server.stats(&a.token, "").await.rows.len(), 3);
}

#[tokio::test]
async fn a_device_writes_its_own_rows_and_reads_the_account_s() {
    let server = Server::new();
    let ada = server.account("Ada", Provider::Github).await;
    let bob = server.account("Bob", Provider::Github).await;
    let a1 = server.sign_in(&ada, "m1").await;
    server.advance(1);
    let a2 = server.sign_in(&ada, "m2").await;
    let b = server.sign_in(&bob, "m1").await;

    // The same key on three devices is three rows. A `device_id` in the body
    // is a field the server does not know: the row is the token's device's.
    server
        .put_stats(&a1.token, json!([row("2026-11-16", "hangar", 1)]))
        .await;
    let mut claiming = row("2026-11-16", "hangar", 2);
    claiming["device_id"] = json!(a1.device.id);
    server.put_stats(&a2.token, json!([claiming])).await;
    let mut claiming = row("2026-11-16", "hangar", 3);
    claiming["device_id"] = json!(a1.device.id);
    server.put_stats(&b.token, json!([claiming])).await;

    for token in [&a1.token, &a2.token] {
        let stats = server.stats(token, "").await;
        let mut seen: Vec<(String, u64)> = stats
            .rows
            .iter()
            .map(|r| (r.device_id.clone(), r.row.input_tokens))
            .collect();
        seen.sort_by_key(|(_, tokens)| *tokens);
        assert_eq!(seen, [(a1.device.id.clone(), 1), (a2.device.id.clone(), 2)]);
        assert_eq!(stats.devices, [a1.device.clone(), a2.device.clone()]);
    }
    let his = server.stats(&b.token, "").await;
    assert_eq!(his.rows.len(), 1);
    assert_eq!(his.rows[0].device_id, b.device.id);
    assert_eq!(his.rows[0].row.input_tokens, 3);
    assert_eq!(his.devices, [b.device]);
}

#[tokio::test]
async fn a_row_at_fault_is_named_and_none_of_the_call_is_written() {
    let server = Server::new();
    let account = server.account("Ada", Provider::Github).await;
    let a = server.sign_in(&account, "m1").await;
    let good = || row("2026-11-16", "hangar", 1);
    let with = |field: &str, value: Value| {
        let mut bad = good();
        bad[field] = value;
        bad
    };

    for (bad, word) in [
        (with("day", json!("2026-02-30")), "day"),
        (with("day", json!("16/11/2026")), "day"),
        (with("model", json!("")), "model"),
        (with("model", json!("m".repeat(101))), "model"),
        (with("project", json!("p".repeat(101))), "project"),
        (with("project", json!("C:\\Users\\ada\\hangar")), "project"),
        (with("project", json!("/home/ada/hangar")), "project"),
        (with("project", json!("two\nlines")), "project"),
        (
            with("sessions", json!(9_223_372_036_854_775_808_u64)),
            "counter",
        ),
    ] {
        let answer = server
            .put_stats(&a.token, json!([good(), good(), bad]))
            .await;
        let message = answer.refused(ErrorCode::BadRequest);
        assert!(message.starts_with("rows[2]: "), "{message}");
        assert!(message.contains(word), "{message}");
    }
    // What is not a row at all: a negative counter, a fraction, a missing
    // field, rows that are not a list.
    let mut missing = good();
    missing.as_object_mut().unwrap().remove("sessions");
    for rows in [
        json!([good(), with("input_tokens", json!(-1))]),
        json!([good(), with("active_seconds", json!(1.5))]),
        json!([good(), with("sessions", json!("3"))]),
        json!([good(), missing]),
        json!({ "0": good() }),
    ] {
        server
            .put_stats(&a.token, rows)
            .await
            .refused(ErrorCode::BadRequest);
    }
    assert!(server.stats(&a.token, "").await.rows.is_empty());

    // The largest counter fits, and comes back whole.
    let answer = server
        .put_stats(&a.token, json!([with("sessions", json!(i64::MAX))]))
        .await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(
        server.stats(&a.token, "").await.rows[0].row.sessions,
        i64::MAX as u64
    );
}

#[tokio::test]
async fn five_thousand_rows_pass_and_one_more_is_too_large() {
    let server = Server::new();
    let account = server.account("Ada", Provider::Github).await;
    let a = server.sign_in(&account, "m1").await;
    let rows = |count: usize| -> Value {
        (0..count)
            .map(|index| row("2026-11-16", &format!("project-{index}"), index as u64))
            .collect()
    };

    let answer = server.put_stats(&a.token, rows(MAX_STATS_ROWS + 1)).await;
    answer.refused(ErrorCode::TooLarge);
    assert!(server.stats(&a.token, "").await.rows.is_empty());

    let answer = server.put_stats(&a.token, rows(MAX_STATS_ROWS)).await;
    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(
        answer.json::<PutStatsResponse>().upserted,
        MAX_STATS_ROWS as u64
    );
    assert_eq!(server.stats(&a.token, "").await.rows.len(), MAX_STATS_ROWS);
}

#[tokio::test]
async fn a_body_over_the_limit_is_too_large() {
    let server = Server::new();
    let account = server.account("Ada", Provider::Github).await;
    let a = server.sign_in(&account, "m1").await;
    let put = |padding: usize| {
        let body = format!(r#"{{"rows":[],"padding":"{}"}}"#, "x".repeat(padding));
        Request::builder()
            .method(Method::PUT)
            .uri(paths::STATS)
            .header(header::AUTHORIZATION, format!("Bearer {}", a.token))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap()
    };
    // The frame around the padding is 24 bytes.
    let at_the_limit = server.send(put(MAX_BODY_BYTES - 24)).await;
    assert_eq!(at_the_limit.status, StatusCode::OK);
    server
        .send(put(MAX_BODY_BYTES - 23))
        .await
        .refused(ErrorCode::TooLarge);
    // The exchange has the same limit.
    let body = format!(r#"{{"code":"{}"}}"#, "x".repeat(MAX_BODY_BYTES));
    let request = Request::builder()
        .method(Method::POST)
        .uri(paths::AUTH_EXCHANGE)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap();
    server.send(request).await.refused(ErrorCode::TooLarge);
}

#[tokio::test]
async fn the_range_of_a_read_is_inclusive_and_optional_at_both_ends() {
    let server = Server::new();
    let account = server.account("Ada", Provider::Github).await;
    let a1 = server.sign_in(&account, "m1").await;
    let a2 = server.sign_in(&account, "m2").await;
    server
        .put_stats(
            &a1.token,
            json!([
                row("2026-10-31", "hangar", 1),
                row("2026-11-01", "hangar", 2),
                row("2026-11-30", "hangar", 3),
            ]),
        )
        .await;
    server
        .put_stats(&a2.token, json!([row("2026-12-01", "hangar", 4)]))
        .await;

    let days = |stats: &StatsResponse| -> Vec<String> {
        stats.rows.iter().map(|r| r.row.day.clone()).collect()
    };
    let all = server.stats(&a1.token, "").await;
    assert_eq!(
        days(&all),
        ["2026-10-31", "2026-11-01", "2026-11-30", "2026-12-01"]
    );
    assert_eq!(all.devices.len(), 2);

    let november = server
        .stats(&a1.token, "?from=2026-11-01&to=2026-11-30")
        .await;
    assert_eq!(days(&november), ["2026-11-01", "2026-11-30"]);
    // Only the devices those rows name.
    assert_eq!(november.devices, std::slice::from_ref(&a1.device));

    let since = server.stats(&a1.token, "?from=2026-11-30").await;
    assert_eq!(days(&since), ["2026-11-30", "2026-12-01"]);
    let until = server.stats(&a1.token, "?to=2026-10-31").await;
    assert_eq!(days(&until), ["2026-10-31"]);
    let one_day = server
        .stats(&a1.token, "?from=2026-12-01&to=2026-12-01")
        .await;
    assert_eq!(days(&one_day), ["2026-12-01"]);
    assert_eq!(one_day.devices, std::slice::from_ref(&a2.device));
    let backwards = server
        .stats(&a1.token, "?from=2026-12-01&to=2026-11-01")
        .await;
    assert!(backwards.rows.is_empty() && backwards.devices.is_empty());

    for query in [
        "?from=yesterday",
        "?to=2026-11-31",
        "?from=",
        "?from=2026-11-01&to=2026-11-01'%20OR%201=1",
        "?from=2026-11-01&from=2026-11-02",
    ] {
        let answer = server
            .get(&format!("{}{query}", paths::STATS), &a1.token)
            .await;
        answer.refused(ErrorCode::BadRequest);
    }
}

// ------------------------------------------------------- deleting the account

#[tokio::test]
async fn deleting_the_account_deletes_everything_of_it_and_nothing_else() {
    let server = Server::new();
    let ada = server.account("Ada", Provider::Github).await;
    let bob = server.account("Bob", Provider::Github).await;
    let second = identity("Ada", Provider::Google);
    let id = ada.clone();
    server
        .state
        .db
        .call(move |conn| account::add_identity(conn, &id, &second, T0))
        .await
        .unwrap();
    let a1 = server.sign_in(&ada, "m1").await;
    let a2 = server.sign_in(&ada, "m2").await;
    let b = server.sign_in(&bob, "m1").await;
    for token in [&a1.token, &a2.token, &b.token] {
        server
            .put_stats(token, json!([row("2026-11-16", "hangar", 1)]))
            .await;
    }
    // One device signed out, and a sign-in in progress.
    server
        .call(
            Method::DELETE,
            &paths::device(&a2.device.id),
            Some(&a1.token),
            None,
        )
        .await;
    let pending = server.link_code(&ada).await;
    let bobs_pending = server.link_code(&bob).await;

    let answer = server
        .call(Method::DELETE, paths::ACCOUNT, Some(&a1.token), None)
        .await;
    assert_eq!(answer.status, StatusCode::NO_CONTENT);
    assert!(answer.body.is_empty());

    const TABLES: [&str; 5] = [
        "SELECT count(*) FROM accounts WHERE id = ?1",
        "SELECT count(*) FROM identities WHERE account_id = ?1",
        "SELECT count(*) FROM devices WHERE account_id = ?1",
        "SELECT count(*) FROM link_codes WHERE account_id = ?1",
        // Hers had no other way to be reached than her devices: what is
        // left is his.
        "SELECT count(*) FROM stats WHERE device_id NOT IN \
         (SELECT id FROM devices WHERE account_id != ?1)",
    ];
    for sql in TABLES {
        assert_eq!(server.count(sql, &ada).await, 0, "{sql}");
    }
    // His link codes are two: the one his sign-in used, and the one pending.
    for sql in &TABLES[..3] {
        assert_eq!(server.count(sql, &bob).await, 1, "{sql}");
    }
    assert_eq!(
        server
            .count("SELECT count(*) FROM stats WHERE ?1 = ?1", &ada)
            .await,
        1
    );

    // Every token of the account is dead, on every route, and the sign-in
    // that was in progress ends nowhere.
    for token in [&a1.token, &a2.token] {
        server
            .get(paths::ME, token)
            .await
            .refused(ErrorCode::Unauthorized);
        server
            .call(Method::DELETE, paths::ACCOUNT, Some(token), None)
            .await
            .refused(ErrorCode::Unauthorized);
    }
    server
        .exchange(&pending, VERIFIER, machine("m1", "m1"))
        .await
        .refused(ErrorCode::InvalidCode);

    // His account never noticed.
    let me: MeResponse = server.get(paths::ME, &b.token).await.json();
    assert_eq!(me.account.id, bob);
    assert_eq!(server.stats(&b.token, "").await.rows.len(), 1);
    let answer = server
        .exchange(&bobs_pending, VERIFIER, machine("m2", "m2"))
        .await;
    assert_eq!(answer.status, StatusCode::OK);
}

// ---------------------------------------------------------------- the limits

fn tight(rates: impl FnOnce(&mut Rates)) -> Server {
    let generous = Rate::per_minute(1_000_000);
    let mut all = Rates {
        ip: generous,
        exchange: generous,
        start: generous,
        device: generous,
    };
    rates(&mut all);
    Server::with(Db::in_memory(), all)
}

fn from(ip: &str, method: Method, path: &str, token: Option<&str>) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("x-forwarded-for", ip);
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    request.body(Body::empty()).unwrap()
}

#[track_caller]
fn assert_rate_limited(answer: &Answer) {
    answer.refused(ErrorCode::RateLimited);
    let retry_after: u64 = answer.headers[header::RETRY_AFTER]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((1..=60).contains(&retry_after), "{retry_after}");
}

#[tokio::test]
async fn an_address_is_limited_and_told_when_to_come_back() {
    let server = tight(|rates| rates.ip = Rate::per_minute(3));
    let account = server.account("Ada", Provider::Github).await;
    let token = server.sign_in(&account, "m1").await.token;

    // The sign-in above came from no address; these come from one.
    for _ in 0..3 {
        let answer = server
            .send(from("198.51.100.7", Method::GET, paths::ME, Some(&token)))
            .await;
        assert_eq!(answer.status, StatusCode::OK);
    }
    let answer = server
        .send(from("198.51.100.7", Method::GET, paths::ME, Some(&token)))
        .await;
    assert_rate_limited(&answer);
    // Whatever it puts in front of its address in the header.
    let spoofed = from(
        "203.0.113.1, 198.51.100.7",
        Method::GET,
        paths::ME,
        Some(&token),
    );
    assert_rate_limited(&server.send(spoofed).await);
    // Without a token too: the limit comes before the token's check.
    let answer = server
        .send(from("198.51.100.7", Method::GET, paths::ME, None))
        .await;
    assert_rate_limited(&answer);

    // Another address is not concerned, and neither is the health check.
    let answer = server
        .send(from("198.51.100.8", Method::GET, paths::ME, Some(&token)))
        .await;
    assert_eq!(answer.status, StatusCode::OK);
    let answer = server
        .send(from("198.51.100.7", Method::GET, paths::HEALTH, None))
        .await;
    assert_eq!(answer.status, StatusCode::OK);
}

#[tokio::test]
async fn a_device_is_limited_wherever_it_calls_from() {
    let server = tight(|rates| rates.device = Rate::per_minute(2));
    let account = server.account("Ada", Provider::Github).await;
    let a1 = server.sign_in(&account, "m1").await.token;
    let a2 = server.sign_in(&account, "m2").await.token;

    for ip in ["198.51.100.1", "198.51.100.2"] {
        let answer = server
            .send(from(ip, Method::GET, paths::ME, Some(&a1)))
            .await;
        assert_eq!(answer.status, StatusCode::OK);
    }
    let answer = server
        .send(from("198.51.100.3", Method::GET, paths::ME, Some(&a1)))
        .await;
    assert_rate_limited(&answer);
    // The account's other device has its own count.
    let answer = server
        .send(from("198.51.100.3", Method::GET, paths::ME, Some(&a2)))
        .await;
    assert_eq!(answer.status, StatusCode::OK);
}

#[tokio::test]
async fn exchanges_have_a_tighter_limit_of_their_own() {
    let server = tight(|rates| rates.exchange = Rate::per_minute(2));
    let account = server.account("Ada", Provider::Github).await;
    let token = server.sign_in(&account, "m1").await.token;
    let attempt = |ip: &str| {
        let body = json!({ "code": "guess", "verifier": VERIFIER, "machine": machine("m", "m") });
        Request::builder()
            .method(Method::POST)
            .uri(paths::AUTH_EXCHANGE)
            .header("x-forwarded-for", ip)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap()
    };

    for _ in 0..2 {
        server
            .send(attempt("198.51.100.7"))
            .await
            .refused(ErrorCode::InvalidCode);
    }
    assert_rate_limited(&server.send(attempt("198.51.100.7")).await);
    // A real code from that address waits like the guesses.
    let code = server.link_code(&account).await;
    let body = json!({ "code": code, "verifier": VERIFIER, "machine": machine("m2", "m2") });
    let request = Request::builder()
        .method(Method::POST)
        .uri(paths::AUTH_EXCHANGE)
        .header("x-forwarded-for", "198.51.100.7")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    assert_rate_limited(&server.send(request).await);
    // The address's other requests are not the exchange's business, and
    // another address exchanges freely.
    let answer = server
        .send(from("198.51.100.7", Method::GET, paths::ME, Some(&token)))
        .await;
    assert_eq!(answer.status, StatusCode::OK);
    server
        .send(attempt("198.51.100.8"))
        .await
        .refused(ErrorCode::InvalidCode);
}
