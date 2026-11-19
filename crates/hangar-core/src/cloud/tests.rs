//! The account, end to end, against a server of the tests' own: an axum
//! router on a loopback port the system picks, answering the protocol as
//! docs/native/cloud.md words it and keeping every request it saw.
//!
//! The token is kept in memory (`MemoryTokens`): no test touches the keychain
//! of the machine it runs on, or the real server.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde_json::json;

use super::api::{
    paths, AuthStartQuery, DeleteDeviceQuery, Health, PutStatsRequest, PutStatsResponse,
    RenameDeviceRequest, StatsQuery,
};
use super::signin::challenge_of;
use super::sync::rows_with;
use super::*;
use crate::stats::{Bucket, ModelInfo, ScanInfo, SessionInfo, UsageStats, ACTIVE_GAP_CAP_MS};

// ---------------------------------------------------------------------------
// The server
// ---------------------------------------------------------------------------

/// One request the server saw.
#[derive(Debug, Clone)]
struct Seen {
    /// `PUT /v1/stats`, with its query when it has one.
    route: String,
    body: String,
}

struct Fake {
    seen: Vec<Seen>,
    /// Link codes waiting for their exchange, with the challenge of the
    /// start they came from.
    codes: HashMap<String, String>,
    issued: u32,
    /// What the provider's page is said to have answered, when not "yes".
    provider_error: Option<&'static str>,
    /// The one token that works.
    token: Option<String>,
    device: Device,
    exchanges: Vec<ExchangeRequest>,
    rows: BTreeMap<(String, String, String), StatsRow>,
    /// Rows of another machine of the account.
    elsewhere: Vec<DeviceStatsRow>,
    /// The next call with a token is refused this way instead.
    refuse_next: Option<(ErrorCode, Option<u64>)>,
    account_deleted: bool,
}

type Shared = Arc<Mutex<Fake>>;

fn account() -> Account {
    Account {
        id: "acc_1".into(),
        display_name: "Ada".into(),
        email: Some("ada@example.org".into()),
        avatar_url: None,
        providers: vec![Provider::Github],
    }
}

fn device(id: &str, name: &str) -> Device {
    Device {
        id: id.into(),
        name: name.into(),
        os: "windows".into(),
        created_at: 1_794_000_000_000,
        last_seen_at: 1_794_000_600_000,
        revoked_at: None,
    }
}

fn refusal(code: ErrorCode, message: &str) -> Response {
    let status = StatusCode::from_u16(code.status()).unwrap();
    (status, Json(ApiError::new(code, message))).into_response()
}

impl Fake {
    fn see(&mut self, route: impl Into<String>, body: &str) {
        self.seen.push(Seen {
            route: route.into(),
            body: body.to_string(),
        });
    }

    /// The refusal, for a request whose token is not the one that works.
    fn gate(&mut self, headers: &HeaderMap) -> Option<Response> {
        let given = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));
        if given.is_none() || given != self.token.as_deref() {
            return Some(refusal(ErrorCode::Unauthorized, "unknown or revoked token"));
        }
        if let Some((code, retry_after)) = self.refuse_next.take() {
            let mut answer = refusal(code, "refused by the test");
            if let Some(seconds) = retry_after {
                answer
                    .headers_mut()
                    .insert(header::RETRY_AFTER, seconds.into());
            }
            return Some(answer);
        }
        None
    }
}

fn lock(fake: &Shared) -> MutexGuard<'_, Fake> {
    fake.lock().unwrap_or_else(|e| e.into_inner())
}

async fn health(State(fake): State<Shared>) -> Response {
    lock(&fake).see("GET /health", "");
    Json(Health {
        status: "ok".into(),
        api_version: API_VERSION,
        version: "2.3.0".into(),
    })
    .into_response()
}

/// The start page, and with it the provider's consent and the server's own
/// callback: the browser is sent straight back to the desktop's port.
async fn auth_start(State(fake): State<Shared>, Query(query): Query<AuthStartQuery>) -> Response {
    let mut fake = lock(&fake);
    fake.see("GET /v1/auth/start", "");
    let back = format!(
        "http://127.0.0.1:{}{}",
        query.port,
        paths::LOOPBACK_CALLBACK
    );
    let location = match fake.provider_error {
        Some(error) => format!("{back}?error={error}&state={}", query.state),
        None => {
            fake.issued += 1;
            let code = format!("link-code-{}", fake.issued);
            fake.codes.insert(code.clone(), query.challenge);
            format!("{back}?code={code}&state={}", query.state)
        }
    };
    (StatusCode::FOUND, [(header::LOCATION, location)]).into_response()
}

async fn exchange(State(fake): State<Shared>, body: String) -> Response {
    let mut fake = lock(&fake);
    fake.see("POST /v1/auth/exchange", &body);
    let Ok(request) = serde_json::from_str::<ExchangeRequest>(&body) else {
        return refusal(ErrorCode::BadRequest, "not an exchange");
    };
    // The code is consumed whether or not the verifier matches.
    let challenge = fake.codes.remove(&request.code);
    fake.exchanges.push(request.clone());
    if challenge != Some(challenge_of(&request.verifier)) {
        return refusal(
            ErrorCode::InvalidCode,
            "the link code is unknown, used or expired",
        );
    }
    let token = format!("hgr_test-token-{}", fake.issued);
    fake.token = Some(token.clone());
    fake.device.name = request.machine.name.clone();
    fake.device.os = request.machine.os.clone();
    Json(ExchangeResponse {
        token,
        account: account(),
        device: fake.device.clone(),
    })
    .into_response()
}

async fn me(State(fake): State<Shared>, headers: HeaderMap) -> Response {
    let mut fake = lock(&fake);
    fake.see("GET /v1/me", "");
    if let Some(refused) = fake.gate(&headers) {
        return refused;
    }
    Json(MeResponse {
        account: account(),
        device: fake.device.clone(),
    })
    .into_response()
}

async fn devices(State(fake): State<Shared>, headers: HeaderMap) -> Response {
    let mut fake = lock(&fake);
    fake.see("GET /v1/devices", "");
    if let Some(refused) = fake.gate(&headers) {
        return refused;
    }
    Json(DevicesResponse {
        devices: vec![fake.device.clone(), device("dev_2", "Portable")],
    })
    .into_response()
}

async fn rename_device(
    State(fake): State<Shared>,
    UrlPath(id): UrlPath<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let mut fake = lock(&fake);
    fake.see(format!("PATCH /v1/devices/{id}"), &body);
    if let Some(refused) = fake.gate(&headers) {
        return refused;
    }
    let Ok(request) = serde_json::from_str::<RenameDeviceRequest>(&body) else {
        return refusal(ErrorCode::BadRequest, "not a rename");
    };
    if id != fake.device.id {
        return refusal(ErrorCode::NotFound, "no such device");
    }
    fake.device.name = request.name;
    Json(fake.device.clone()).into_response()
}

async fn delete_device(
    State(fake): State<Shared>,
    UrlPath(id): UrlPath<String>,
    Query(query): Query<DeleteDeviceQuery>,
    headers: HeaderMap,
) -> Response {
    let mut fake = lock(&fake);
    let forget = if query.forget { "?forget=true" } else { "" };
    fake.see(format!("DELETE /v1/devices/{id}{forget}"), "");
    if let Some(refused) = fake.gate(&headers) {
        return refused;
    }
    if id == fake.device.id {
        fake.token = None;
        if query.forget {
            fake.rows.clear();
        }
    } else if id != "dev_2" {
        return refusal(ErrorCode::NotFound, "no such device");
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn put_stats(State(fake): State<Shared>, headers: HeaderMap, body: String) -> Response {
    let mut fake = lock(&fake);
    fake.see("PUT /v1/stats", &body);
    if let Some(refused) = fake.gate(&headers) {
        return refused;
    }
    let Ok(request) = serde_json::from_str::<PutStatsRequest>(&body) else {
        return refusal(ErrorCode::BadRequest, "not rows");
    };
    if request.rows.len() > api::MAX_STATS_ROWS {
        return refusal(ErrorCode::TooLarge, "more than 5000 rows");
    }
    if let Some(at) = request.rows.iter().position(|row| row.validate().is_err()) {
        return refusal(ErrorCode::BadRequest, &format!("row {at} is refused"));
    }
    let mut keys = std::collections::BTreeSet::new();
    for row in request.rows {
        let key = (row.day.clone(), row.model.clone(), row.project.clone());
        keys.insert(key.clone());
        fake.rows.insert(key, row);
    }
    Json(PutStatsResponse {
        upserted: keys.len() as u64,
    })
    .into_response()
}

async fn get_stats(
    State(fake): State<Shared>,
    Query(query): Query<StatsQuery>,
    headers: HeaderMap,
) -> Response {
    let mut fake = lock(&fake);
    let ends = [("from", &query.from), ("to", &query.to)]
        .iter()
        .filter_map(|(name, day)| day.as_ref().map(|day| format!("{name}={day}")))
        .collect::<Vec<_>>()
        .join("&");
    fake.see(format!("GET /v1/stats?{ends}"), "");
    if let Some(refused) = fake.gate(&headers) {
        return refused;
    }
    let within = |day: &str| {
        query.from.as_deref().is_none_or(|from| day >= from)
            && query.to.as_deref().is_none_or(|to| day <= to)
    };
    let own = fake.rows.values().map(|row| DeviceStatsRow {
        device_id: fake.device.id.clone(),
        row: row.clone(),
    });
    let rows: Vec<DeviceStatsRow> = own
        .chain(fake.elsewhere.iter().cloned())
        .filter(|row| within(&row.row.day))
        .collect();
    Json(StatsResponse {
        devices: vec![fake.device.clone(), device("dev_2", "Portable")],
        rows,
    })
    .into_response()
}

async fn delete_account(State(fake): State<Shared>, headers: HeaderMap) -> Response {
    let mut fake = lock(&fake);
    fake.see("DELETE /v1/account", "");
    if let Some(refused) = fake.gate(&headers) {
        return refused;
    }
    fake.token = None;
    fake.rows.clear();
    fake.account_deleted = true;
    StatusCode::NO_CONTENT.into_response()
}

/// The server, running until the test ends.
struct Server {
    fake: Shared,
    url: String,
    _runtime: tokio::runtime::Runtime,
}

impl Server {
    fn start() -> Self {
        let fake: Shared = Arc::new(Mutex::new(Fake {
            seen: Vec::new(),
            codes: HashMap::new(),
            issued: 0,
            provider_error: None,
            token: None,
            device: device("dev_1", "Atelier"),
            exchanges: Vec::new(),
            rows: BTreeMap::new(),
            elsewhere: Vec::new(),
            refuse_next: None,
            account_deleted: false,
        }));
        let router = Router::new()
            .route(paths::HEALTH, get(health))
            .route(paths::AUTH_START, get(auth_start))
            .route(paths::AUTH_EXCHANGE, post(exchange))
            .route(paths::ME, get(me))
            .route(paths::DEVICES, get(devices))
            .route(paths::DEVICE, delete(delete_device).patch(rename_device))
            .route(paths::STATS, get(get_stats).put(put_stats))
            .route(paths::ACCOUNT, delete(delete_account))
            .with_state(fake.clone());
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        // Port 0: the system picks one nobody else has.
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind(("127.0.0.1", 0)))
            .unwrap();
        let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        runtime.spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            fake,
            url,
            _runtime: runtime,
        }
    }

    fn fake(&self) -> MutexGuard<'_, Fake> {
        lock(&self.fake)
    }

    /// The routes asked for so far, in order.
    fn routes(&self) -> Vec<String> {
        self.fake()
            .seen
            .iter()
            .map(|seen| seen.route.clone())
            .collect()
    }

    /// How many requests of this route were seen.
    fn count(&self, route: &str) -> usize {
        self.fake()
            .seen
            .iter()
            .filter(|seen| seen.route == route)
            .count()
    }

    /// The rows of each `PUT /v1/stats` seen, in order.
    fn puts(&self) -> Vec<Vec<StatsRow>> {
        let fake = self.fake();
        fake.seen
            .iter()
            .filter(|seen| seen.route == "PUT /v1/stats")
            .map(|seen| {
                serde_json::from_str::<PutStatsRequest>(&seen.body)
                    .unwrap()
                    .rows
            })
            .collect()
    }

    /// A desktop signed in to this server, with the token store to look at.
    fn signed_in(&self, tag: &str) -> (Cloud, MemoryTokens, PathBuf) {
        let tokens = MemoryTokens::holding("hgr_test-token-0");
        self.fake().token = Some("hgr_test-token-0".into());
        let dir = crate::testdir::scratch(&format!("cloud-{tag}")).join("data");
        (
            Cloud::with(&self.url, tokens.clone(), dir.clone()),
            tokens,
            dir,
        )
    }

    /// A desktop nobody signed in on.
    fn signed_out(&self, tag: &str) -> (Cloud, MemoryTokens, PathBuf) {
        let tokens = MemoryTokens::new();
        let dir = crate::testdir::scratch(&format!("cloud-{tag}")).join("data");
        (
            Cloud::with(&self.url, tokens.clone(), dir.clone()),
            tokens,
            dir,
        )
    }
}

// ---------------------------------------------------------------------------
// The browser
// ---------------------------------------------------------------------------

struct Page {
    status: u16,
    /// Where the page redirects, when it does.
    location: Option<String>,
    html: String,
}

/// One page fetched as a browser would, without following its redirect.
fn fetch(url: &str, language: &str) -> Page {
    let agent = ureq::Agent::config_builder()
        .max_redirects(0)
        .max_redirects_will_error(false)
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .new_agent();
    let mut response = agent
        .get(url)
        .header("accept-language", language)
        .call()
        .unwrap();
    Page {
        status: response.status().as_u16(),
        location: response
            .headers()
            .get("location")
            .map(|value| value.to_str().unwrap().to_string()),
        html: response.body_mut().read_to_string().unwrap(),
    }
}

/// The browser's whole trip: the start page, then wherever the server sends
/// it, which is the desktop's loopback port.
fn browse(sign_in: &SignIn, language: &str) -> Page {
    let start = fetch(sign_in.url(), language);
    assert_eq!(start.status, 302);
    fetch(
        &start.location.expect("a redirect to the desktop"),
        language,
    )
}

fn param<'a>(url: &'a str, name: &str) -> &'a str {
    url.split(['?', '&'])
        .find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
        .unwrap_or_else(|| panic!("no {name} in the address"))
}

/// Whether the listener of the sign-in that drew `state` still answers on
/// `port`. Once it is gone the port is the system's to give again, and a
/// test running beside may hold it by then: that something answers there
/// proves nothing. Only the sign-in's own listener takes a visit with its
/// state for the provider's answer: asked as a refusal (nothing to
/// exchange), it says the sign-in was not completed. Another sign-in's
/// listener turns the visit away; anything else answers something else, or
/// nothing.
fn still_listens(port: u16, state: &str) -> bool {
    use std::io::{Read, Write};
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = std::net::TcpStream::connect_timeout(&address, Duration::from_secs(10))
    else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let visit = format!(
        "GET /callback?error=access_denied&state={state} HTTP/1.1\r\n\
         Host: 127.0.0.1:{port}\r\n\
         Accept-Language: en\r\n\
         Connection: close\r\n\r\n"
    );
    if stream.write_all(visit.as_bytes()).is_err() {
        return false;
    }
    // Nothing more to send. A connection the system made to itself (the
    // port free, and given to this very socket) reads its own request back,
    // then its end, rather than waiting for an answer.
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let mut answer = Vec::new();
    let _ = stream.read_to_end(&mut answer);
    let answer = String::from_utf8_lossy(&answer);
    answer.starts_with("HTTP/1.1 200") && answer.contains("The sign-in was not completed.")
}

// ---------------------------------------------------------------------------
// Statistics to send
// ---------------------------------------------------------------------------

const HOUR: i64 = 3_600_000;
/// 2026-11-16T00:00:00Z.
const MONDAY: i64 = 1_794_787_200_000;

fn utc_day(at: i64) -> Option<String> {
    chrono::DateTime::from_timestamp_millis(at).map(|time| time.date_naive().to_string())
}

fn session(id: &str, cwd: &str) -> SessionInfo {
    SessionInfo {
        id: id.into(),
        cwd: cwd.into(),
        title: format!("a private title of {id}"),
        first_at: MONDAY,
        last_at: MONDAY + HOUR,
    }
}

/// A model's hour in a session.
fn calls(session: u32, at: i64, model: i32, output: u64, cost: f64, calls: u32) -> Bucket {
    Bucket {
        session,
        at,
        model,
        input: 10,
        output,
        cache_write: 30,
        cache_read: 40,
        cost,
        calls,
        ..Bucket::default()
    }
}

/// A session's own hour: its turns and its active time.
fn active(session: u32, at: i64, active_ms: i64) -> Bucket {
    Bucket {
        session,
        at,
        model: -1,
        turns: 2,
        active_ms,
        ..Bucket::default()
    }
}

fn usage(sessions: Vec<SessionInfo>, models: &[&str], buckets: Vec<Bucket>) -> UsageStats {
    UsageStats {
        generated_at: 0,
        scan: ScanInfo::default(),
        sources: Vec::new(),
        active_gap_cap_ms: ACTIVE_GAP_CAP_MS,
        sessions,
        models: models
            .iter()
            .map(|id| ModelInfo {
                id: id.to_string(),
                priced: true,
            })
            .collect(),
        buckets,
    }
}

/// Two projects on two systems' paths, two models, two days.
fn sample() -> UsageStats {
    usage(
        vec![
            session("s-1", "C:\\Users\\ada\\Projects\\hangar"),
            session("s-2", "/home/ada/work/site"),
        ],
        &["claude-opus-5-5", "claude-haiku-5-5"],
        vec![
            calls(0, MONDAY + 9 * HOUR, 0, 100, 1.25, 4),
            calls(0, MONDAY + 10 * HOUR, 0, 50, 0.5, 2),
            calls(0, MONDAY + 10 * HOUR, 1, 7, 0.01, 1),
            active(0, MONDAY + 9 * HOUR, 600_000),
            active(0, MONDAY + 10 * HOUR, 300_400),
            calls(1, MONDAY + 26 * HOUR, 1, 9, 0.002, 3),
            active(1, MONDAY + 26 * HOUR, 61_000),
        ],
    )
}

fn row(day: &str, model: &str, project: &str, output: u64) -> StatsRow {
    StatsRow {
        day: day.into(),
        model: model.into(),
        project: project.into(),
        input_tokens: 1,
        output_tokens: output,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        cost_usd_micros: 0,
        active_seconds: 0,
        sessions: 1,
    }
}

// ---------------------------------------------------------------------------
// Where the server is
// ---------------------------------------------------------------------------

#[test]
fn the_server_is_the_public_one_unless_the_environment_names_another() {
    assert_eq!(resolve_base_url(None), "https://account.hangarai.dev");
    assert_eq!(
        resolve_base_url(Some("  ".into())),
        "https://account.hangarai.dev"
    );
    // Paths are appended as they are, so the slash goes.
    assert_eq!(
        resolve_base_url(Some("http://127.0.0.1:21120/".into())),
        "http://127.0.0.1:21120"
    );
}

// ---------------------------------------------------------------------------
// Signing in
// ---------------------------------------------------------------------------

#[test]
fn signing_in_exchanges_the_code_and_keeps_the_token() {
    let server = Server::start();
    let (cloud, tokens, dir) = server.signed_out("sign-in");
    let sign_in = cloud.sign_in(Provider::Github).unwrap();

    // The address the browser opens: the server's start page, with this
    // sign-in's port, state and challenge, and the machine's name.
    let url = sign_in.url().to_string();
    assert!(url.starts_with(&format!("{}/v1/auth/start?", server.url)));
    assert_eq!(param(&url, "provider"), "github");
    assert_eq!(param(&url, "port"), sign_in.port().to_string());
    assert!(sign_in.port() >= api::MIN_LOOPBACK_PORT);
    let state = param(&url, "state").to_string();
    let challenge = param(&url, "challenge").to_string();
    assert_eq!(challenge.len(), api::CHALLENGE_CHARS);
    assert!(sign_in.poll().is_none());
    // Nothing was asked of the server before the browser went.
    assert!(server.routes().is_empty());

    // A visit with another state is turned away, and changes nothing.
    let callback = format!("http://127.0.0.1:{}/callback", sign_in.port());
    let stranger = fetch(&format!("{callback}?code=stolen&state=not-the-state"), "en");
    assert_eq!(stranger.status, 400);
    let near = format!("{}x", &state[..state.len() - 1]);
    assert_eq!(
        fetch(&format!("{callback}?code=stolen&state={near}"), "en").status,
        400
    );
    assert_eq!(fetch(&format!("{callback}?code=stolen"), "en").status, 400);
    assert_eq!(
        fetch(&format!("http://127.0.0.1:{}/", sign_in.port()), "en").status,
        404
    );
    assert!(sign_in.poll().is_none());
    assert!(server.routes().is_empty());
    assert_eq!(tokens.peek(), None);

    // The real trip, with the right state.
    let page = browse(&sign_in, "en-US,en;q=0.9");
    assert_eq!(page.status, 200);
    assert!(page
        .html
        .contains("You can close this tab and return to Hangar."));
    let signed = sign_in.wait().unwrap();
    assert_eq!(signed.account, account());
    assert_eq!(signed.device.id, "dev_1");

    // The exchange carried the code, the verifier of the challenge (the
    // server checked its S256) and the machine.
    let exchanges = server.fake().exchanges.clone();
    assert_eq!(exchanges.len(), 1);
    assert_eq!(exchanges[0].code, "link-code-1");
    assert_eq!(challenge_of(&exchanges[0].verifier), challenge);
    assert_ne!(exchanges[0].verifier, state);
    let machine = cloud.machine().unwrap();
    assert_eq!(exchanges[0].machine, machine);
    assert!(uuid::Uuid::parse_str(&machine.id).is_ok());
    assert_eq!(machine.name, super::machine::default_name());
    assert_eq!(machine.os, std::env::consts::OS);
    // The machine's id is kept beside the app's other files, and nowhere is
    // the token.
    assert_eq!(
        std::fs::read_to_string(dir.join("cloud-machine-id")).unwrap(),
        machine.id
    );
    for file in std::fs::read_dir(&dir).unwrap() {
        let text = std::fs::read_to_string(file.unwrap().path()).unwrap();
        assert!(!text.contains("hgr_"));
    }

    assert_eq!(tokens.peek().as_deref(), Some("hgr_test-token-1"));
    assert!(cloud.signed_in().unwrap());
    assert_eq!(cloud.me().unwrap().device.id, "dev_1");
    // The listener is gone with the sign-in.
    assert!(!still_listens(param(&url, "port").parse().unwrap(), &state));
}

#[test]
fn the_page_is_french_for_a_french_browser() {
    let server = Server::start();
    let (cloud, _tokens, _dir) = server.signed_out("sign-in-fr");
    let sign_in = cloud.sign_in(Provider::Github).unwrap();
    assert_eq!(param(sign_in.url(), "provider"), "github");
    let page = browse(&sign_in, "fr-FR,fr;q=0.9,en;q=0.8");
    assert!(page
        .html
        .contains("Vous pouvez fermer cet onglet et revenir à Hangar."));
    assert!(page.html.contains("lang=\"fr\""));
    sign_in.wait().unwrap();
}

#[test]
fn a_refusal_at_the_provider_ends_the_sign_in_without_a_token() {
    let server = Server::start();
    for (error, expected) in [
        ("access_denied", Error::Denied),
        ("provider_error", Error::Provider),
    ] {
        server.fake().provider_error = Some(error);
        let (cloud, tokens, _dir) = server.signed_out("sign-in-denied");
        let sign_in = cloud.sign_in(Provider::Github).unwrap();
        let page = browse(&sign_in, "en");
        assert_eq!(page.status, 200);
        assert!(page.html.contains("The sign-in was not completed."));
        assert_eq!(sign_in.wait(), Err(expected));
        assert_eq!(tokens.peek(), None);
    }
    assert_eq!(server.count("POST /v1/auth/exchange"), 0);
}

#[test]
fn a_code_the_server_refuses_leaves_the_machine_signed_out() {
    let server = Server::start();
    let (cloud, tokens, _dir) = server.signed_out("sign-in-refused");
    let sign_in = cloud.sign_in(Provider::Github).unwrap();
    // The right state, with a code the server never gave.
    let state = param(sign_in.url(), "state").to_string();
    let page = fetch(
        &format!(
            "http://127.0.0.1:{}/callback?code=made-up&state={state}",
            sign_in.port()
        ),
        "en",
    );
    assert_eq!(page.status, 200);
    let refused = sign_in.wait().unwrap_err();
    assert!(matches!(
        &refused,
        Error::Refused { status: 400, error, .. } if error.error == ErrorCode::InvalidCode
    ));
    assert!(!refused.to_string().contains("made-up"));
    assert_eq!(tokens.peek(), None);
    assert!(!cloud.signed_in().unwrap());
}

#[test]
fn a_sign_in_can_be_cancelled_and_does_not_wait_forever() {
    let server = Server::start();
    let (cloud, tokens, _dir) = server.signed_out("sign-in-cancel");

    let sign_in = cloud.sign_in(Provider::Github).unwrap();
    let port = sign_in.port();
    let state = param(sign_in.url(), "state").to_string();
    let cancel = sign_in.canceller();
    let waiting = std::thread::spawn(move || sign_in.wait());
    cancel.cancel();
    assert_eq!(waiting.join().unwrap(), Err(Error::Cancelled));
    assert!(!still_listens(port, &state));

    let started = Instant::now();
    let sign_in = cloud
        .sign_in_within(Provider::Github, Duration::from_millis(150))
        .unwrap();
    assert_eq!(sign_in.wait(), Err(Error::TimedOut));
    assert!(started.elapsed() >= Duration::from_millis(150));

    // Letting go of the sign-in calls it off too.
    let sign_in = cloud.sign_in(Provider::Github).unwrap();
    let port = sign_in.port();
    let state = param(sign_in.url(), "state").to_string();
    drop(sign_in);
    // Its thread sees it a moment later. Waited for without a visit (one
    // would end a listener that outlived its sign-in, and hide it): until
    // the port is free, or something holds it still at the deadline,
    // which then must not be this sign-in's listener.
    let gone = Instant::now() + Duration::from_secs(10);
    while std::net::TcpListener::bind(("127.0.0.1", port)).is_err() && Instant::now() < gone {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !still_listens(port, &state),
        "the listener outlived its sign-in"
    );

    assert_eq!(tokens.peek(), None);
    assert!(server.routes().is_empty());
}

/// A keychain that does not answer.
struct NoKeychain;

impl TokenStore for NoKeychain {
    fn load(&self) -> Result<Option<String>, String> {
        Err("no Secret Service answered".into())
    }
    fn save(&self, _: &str) -> Result<(), String> {
        Err("no Secret Service answered".into())
    }
    fn delete(&self) -> Result<(), String> {
        Err("no Secret Service answered".into())
    }
}

#[test]
fn without_a_keychain_sign_in_fails_and_says_so() {
    let server = Server::start();
    let dir = crate::testdir::scratch("cloud-no-keychain");
    let cloud = Cloud::with(&server.url, NoKeychain, dir.clone());
    let Err(refused) = cloud.sign_in(Provider::Github) else {
        panic!("signed in without a keychain");
    };
    assert_eq!(
        refused,
        Error::Keychain("no Secret Service answered".into())
    );
    assert!(matches!(cloud.me(), Err(Error::Keychain(_))));
    assert!(matches!(
        cloud.sync_rows(true, &[row("2026-11-16", "m", "p", 1)]),
        Err(Error::Keychain(_))
    ));
    // Nothing went out, and nothing was written instead.
    assert!(server.routes().is_empty());
    assert!(std::fs::read_dir(&dir).unwrap().next().is_none());
}

/// A keychain that reads, and refuses to keep anything.
struct ReadOnlyKeychain;

impl TokenStore for ReadOnlyKeychain {
    fn load(&self) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn save(&self, _: &str) -> Result<(), String> {
        Err("the keychain is locked".into())
    }
    fn delete(&self) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn a_token_that_cannot_be_kept_is_revoked_rather_than_left_valid() {
    let server = Server::start();
    let dir = crate::testdir::scratch("cloud-locked-keychain");
    let cloud = Cloud::with(&server.url, ReadOnlyKeychain, dir);
    let sign_in = cloud.sign_in(Provider::Github).unwrap();
    browse(&sign_in, "en");
    assert_eq!(
        sign_in.wait(),
        Err(Error::Keychain("the keychain is locked".into()))
    );
    assert_eq!(server.count("DELETE /v1/devices/dev_1"), 1);
    assert_eq!(server.fake().token, None);
}

// ---------------------------------------------------------------------------
// The calls
// ---------------------------------------------------------------------------

#[test]
fn signed_out_nothing_is_asked_of_the_server() {
    let server = Server::start();
    let (cloud, _tokens, dir) = server.signed_out("signed-out");
    assert!(!cloud.signed_in().unwrap());
    assert_eq!(cloud.me(), Err(Error::NotSignedIn));
    assert_eq!(cloud.health(), Err(Error::NotSignedIn));
    assert_eq!(cloud.devices(), Err(Error::NotSignedIn));
    assert_eq!(
        cloud.rename_device("dev_1", "Portable"),
        Err(Error::NotSignedIn)
    );
    assert_eq!(cloud.sign_out_device("dev_2"), Err(Error::NotSignedIn));
    assert_eq!(cloud.forget_device("dev_2"), Err(Error::NotSignedIn));
    assert_eq!(cloud.delete_account(), Err(Error::NotSignedIn));
    assert_eq!(
        cloud.sync_rows(true, &[row("2026-11-16", "m", "p", 1)]),
        Err(Error::NotSignedIn)
    );
    assert_eq!(cloud.pull_stats(true, None, None), Err(Error::NotSignedIn));
    assert_eq!(cloud.sign_out(), Ok(()));
    assert!(server.routes().is_empty());
    // Nor is anything written: not even the machine's id, before a sign-in.
    assert!(!dir.exists());
}

#[test]
fn every_call_reaches_its_route_with_the_token() {
    let server = Server::start();
    let (cloud, _tokens, _dir) = server.signed_in("calls");

    assert_eq!(cloud.health().unwrap().api_version, API_VERSION);
    let me = cloud.me().unwrap();
    assert_eq!((me.account, me.device.id.as_str()), (account(), "dev_1"));
    let devices = cloud.devices().unwrap();
    assert_eq!(devices.len(), 2);
    assert_eq!(devices[1].name, "Portable");

    assert_eq!(
        cloud.rename_device("dev_1", "  Bureau  ").unwrap().name,
        "Bureau"
    );
    assert!(matches!(
        cloud.rename_device("dev_9", "Bureau"),
        Err(Error::Refused { status: 404, error, .. }) if error.error == ErrorCode::NotFound
    ));
    // Names the server would refuse are refused here.
    for name in [
        "",
        "   ",
        "a\nb",
        &"n".repeat(api::MAX_DEVICE_NAME_CHARS + 1),
    ] {
        assert!(matches!(
            cloud.rename_device("dev_1", name),
            Err(Error::Invalid(_))
        ));
    }
    // An id is one path segment, whatever it holds.
    assert!(cloud
        .rename_device("dev_1/../../account", "Bureau")
        .is_err());

    cloud.sign_out_device("dev_2").unwrap();
    cloud.forget_device("dev_2").unwrap();

    assert_eq!(
        server.routes(),
        [
            "GET /health",
            "GET /v1/me",
            "GET /v1/devices",
            "PATCH /v1/devices/dev_1",
            "PATCH /v1/devices/dev_9",
            "PATCH /v1/devices/dev_1/../../account",
            "DELETE /v1/devices/dev_2",
            "DELETE /v1/devices/dev_2?forget=true",
        ]
    );
    let renamed = server.fake().seen[3].body.clone();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&renamed).unwrap(),
        json!({ "name": "Bureau" })
    );
}

#[test]
fn a_401_signs_the_machine_out_and_drops_the_token() {
    let server = Server::start();
    let (cloud, tokens, dir) = server.signed_in("401");
    cloud
        .sync_rows(true, &[row("2026-11-16", "m", "p", 1)])
        .unwrap();
    assert!(dir.join("cloud-sync.json").is_file());

    // Signed out from another machine: the token no longer works.
    server.fake().token = Some("hgr_another".into());
    let before = server.routes().len();
    assert_eq!(cloud.me(), Err(Error::SignedOut));
    assert_eq!(tokens.peek(), None);
    assert!(!cloud.signed_in().unwrap());
    // What was sent under that token is forgotten with it.
    assert!(!dir.join("cloud-sync.json").exists());

    // From then on the server is not asked anything.
    assert_eq!(cloud.me(), Err(Error::NotSignedIn));
    assert_eq!(
        cloud.sync_rows(true, &[row("2026-11-16", "m", "p", 2)]),
        Err(Error::NotSignedIn)
    );
    assert_eq!(server.routes().len(), before + 1);
}

#[test]
fn a_401_in_the_middle_of_a_sync_signs_out_too() {
    let server = Server::start();
    let (cloud, tokens, _dir) = server.signed_in("401-sync");
    server.fake().token = None;
    assert_eq!(
        cloud.sync_rows(true, &[row("2026-11-16", "m", "p", 1)]),
        Err(Error::SignedOut)
    );
    assert_eq!(tokens.peek(), None);
    assert_eq!(cloud.pull_stats(true, None, None), Err(Error::NotSignedIn));
}

#[test]
fn a_refusal_carries_the_server_s_code_and_its_patience_and_keeps_the_token() {
    let server = Server::start();
    let (cloud, tokens, _dir) = server.signed_in("refusal");
    server.fake().refuse_next = Some((ErrorCode::RateLimited, Some(120)));
    let refused = cloud.me().unwrap_err();
    assert_eq!(
        refused,
        Error::Refused {
            status: 429,
            error: ApiError::new(ErrorCode::RateLimited, "refused by the test"),
            retry_after: Some(Duration::from_secs(120)),
        }
    );
    server.fake().refuse_next = Some((ErrorCode::Internal, None));
    assert!(matches!(
        cloud.me(),
        Err(Error::Refused {
            status: 500,
            retry_after: None,
            ..
        })
    ));
    assert_eq!(tokens.peek().as_deref(), Some("hgr_test-token-0"));
    assert!(cloud.me().is_ok());
}

#[test]
fn a_server_that_is_not_there_is_unreachable_and_no_error_names_the_token() {
    // A port nobody listens on: taken, then let go.
    let port = {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.local_addr().unwrap().port()
    };
    let tokens = MemoryTokens::holding("hgr_secret-token");
    let dir = crate::testdir::scratch("cloud-unreachable");
    let cloud = Cloud::with(&format!("http://127.0.0.1:{port}/"), tokens.clone(), dir);
    let failures = [
        cloud.me().map(|_| ()).unwrap_err(),
        cloud
            .sync_rows(true, &[row("2026-11-16", "m", "p", 1)])
            .map(|_| ())
            .unwrap_err(),
        cloud.delete_account().unwrap_err(),
    ];
    for failure in &failures {
        assert!(matches!(failure, Error::Unreachable(_)), "{failure:?}");
        assert!(!format!("{failure} {failure:?}").contains("hgr_"));
    }
    // Not reaching the server is not being signed out.
    assert_eq!(tokens.peek().as_deref(), Some("hgr_secret-token"));
    // Signing out works all the same: the token goes.
    assert_eq!(cloud.sign_out(), Ok(()));
    assert_eq!(tokens.peek(), None);
}

#[test]
fn signing_out_revokes_this_device_and_deletes_the_token() {
    let server = Server::start();
    let (cloud, tokens, dir) = server.signed_in("sign-out");
    cloud
        .sync_rows(true, &[row("2026-11-16", "m", "p", 1)])
        .unwrap();
    let before = server.routes().len();
    assert_eq!(cloud.sign_out(), Ok(()));
    assert_eq!(
        server.routes()[before..],
        ["GET /v1/me", "DELETE /v1/devices/dev_1"]
    );
    assert_eq!(server.fake().token, None);
    // Its rows stay on the account.
    assert_eq!(server.fake().rows.len(), 1);
    assert_eq!(tokens.peek(), None);
    assert!(!dir.join("cloud-sync.json").exists());
    assert_eq!(cloud.me(), Err(Error::NotSignedIn));
}

#[test]
fn signing_out_with_a_token_the_server_forgot_still_deletes_it() {
    let server = Server::start();
    let (cloud, tokens, _dir) = server.signed_in("sign-out-stale");
    server.fake().token = None;
    assert_eq!(cloud.sign_out(), Ok(()));
    assert_eq!(tokens.peek(), None);
}

#[test]
fn deleting_the_account_forgets_the_token() {
    let server = Server::start();
    let (cloud, tokens, _dir) = server.signed_in("delete-account");
    assert_eq!(cloud.delete_account(), Ok(()));
    assert!(server.fake().account_deleted);
    assert_eq!(tokens.peek(), None);
    assert_eq!(cloud.me(), Err(Error::NotSignedIn));
}

// ---------------------------------------------------------------------------
// The rows
// ---------------------------------------------------------------------------

#[test]
fn rows_are_totals_per_day_model_and_project_name() {
    let rows = rows_with(&sample(), utc_day);
    assert_eq!(
        rows,
        [
            StatsRow {
                day: "2026-11-16".into(),
                model: "claude-haiku-5-5".into(),
                project: "hangar".into(),
                input_tokens: 10,
                output_tokens: 7,
                cache_read_tokens: 40,
                cache_write_tokens: 30,
                cost_usd_micros: 10_000,
                // The session's day went, whole, to the model that answered
                // most of it.
                active_seconds: 0,
                sessions: 0,
            },
            StatsRow {
                day: "2026-11-16".into(),
                model: "claude-opus-5-5".into(),
                project: "hangar".into(),
                input_tokens: 20,
                output_tokens: 150,
                cache_read_tokens: 80,
                cache_write_tokens: 60,
                cost_usd_micros: 1_750_000,
                active_seconds: 900,
                sessions: 1,
            },
            StatsRow {
                day: "2026-11-17".into(),
                model: "claude-haiku-5-5".into(),
                project: "site".into(),
                input_tokens: 10,
                output_tokens: 9,
                cache_read_tokens: 40,
                cache_write_tokens: 30,
                cost_usd_micros: 2_000,
                active_seconds: 61,
                sessions: 1,
            },
        ]
    );
    for row in &rows {
        assert_eq!(row.validate(), Ok(()));
    }
}

#[test]
fn a_session_counts_once_a_day_whatever_models_it_used() {
    // One session, two models on Monday, then a Tuesday it was only typed
    // into: no call that day.
    let stats = usage(
        vec![session("s-1", "/work/app")],
        &["big", "small"],
        vec![
            calls(0, MONDAY + HOUR, 0, 1, 0.0, 2),
            calls(0, MONDAY + HOUR, 1, 1, 0.0, 5),
            active(0, MONDAY + HOUR, 10_000),
            active(0, MONDAY + 25 * HOUR, 4_000),
        ],
    );
    let rows = rows_with(&stats, utc_day);
    let brief: Vec<_> = rows
        .iter()
        .map(|r| {
            (
                r.day.as_str(),
                r.model.as_str(),
                r.active_seconds,
                r.sessions,
            )
        })
        .collect();
    assert_eq!(
        brief,
        [
            ("2026-11-16", "big", 0, 0),
            ("2026-11-16", "small", 10, 1),
            // The day without a call sits with the model the session mostly
            // ran on.
            ("2026-11-17", "small", 4, 1),
        ]
    );
    // Two models tied: the first by name, the same every time.
    let tied = usage(
        vec![session("s-1", "/work/app")],
        &["zeta", "alpha"],
        vec![
            calls(0, MONDAY, 0, 1, 0.0, 3),
            calls(0, MONDAY, 1, 1, 0.0, 3),
            active(0, MONDAY, 2_000),
        ],
    );
    let rows = rows_with(&tied, utc_day);
    assert_eq!((rows[0].model.as_str(), rows[0].sessions), ("alpha", 1));
    assert_eq!((rows[1].model.as_str(), rows[1].sessions), ("zeta", 0));
    // A session no model answered has no row.
    let silent = usage(
        vec![session("s-1", "/work/app")],
        &[],
        vec![active(0, MONDAY, 2_000)],
    );
    assert!(rows_with(&silent, utc_day).is_empty());
}

#[test]
fn names_the_protocol_refuses_are_made_ones_it_takes() {
    let stats = usage(
        vec![
            session("s-1", &format!("/work/{}", "p".repeat(300))),
            session("s-2", "D:\\"),
        ],
        &["", "tab\tbed"],
        vec![
            calls(0, MONDAY, 0, 1, 0.0, 1),
            calls(1, MONDAY, 1, 1, 0.0, 1),
        ],
    );
    let rows = rows_with(&stats, utc_day);
    assert_eq!(rows.len(), 2);
    for row in &rows {
        assert_eq!(row.validate(), Ok(()), "{row:?}");
    }
    assert_eq!(rows[0].model, "tabbed");
    assert_eq!(rows[0].project, "");
    assert_eq!(rows[1].model, "unknown");
    assert_eq!(rows[1].project, "p".repeat(api::MAX_PROJECT_CHARS));
}

#[test]
fn days_are_the_machine_s_own() {
    // Whatever the zone the tests run in, a bucket is one calendar day.
    let rows = rows_from(&sample());
    assert!(!rows.is_empty());
    assert!(rows.iter().all(|row| api::is_day(&row.day)));
}

// ---------------------------------------------------------------------------
// The sync
// ---------------------------------------------------------------------------

#[test]
fn only_the_project_s_name_leaves_never_its_path() {
    let server = Server::start();
    let (cloud, _tokens, _dir) = server.signed_in("basename");
    let stats = sample();
    let synced = cloud.sync_rows(true, &rows_with(&stats, utc_day)).unwrap();
    assert_eq!(synced.sent, 3);
    cloud
        .pull_stats(true, Some("2026-11-01"), Some("2026-11-30"))
        .unwrap();

    let fake = server.fake();
    let sent: Vec<&Seen> = fake
        .seen
        .iter()
        .filter(|seen| !seen.body.is_empty())
        .collect();
    assert_eq!(sent.len(), 1);
    let projects: Vec<String> = serde_json::from_str::<PutStatsRequest>(&sent[0].body)
        .unwrap()
        .rows
        .into_iter()
        .map(|row| row.project)
        .collect();
    assert_eq!(projects, ["hangar", "hangar", "site"]);
    // Nothing of a path, of a session or of the machine's folders is in
    // anything the server was sent: bodies and addresses alike.
    for seen in &fake.seen {
        let wire = format!("{} {}", seen.route, seen.body);
        for private in [
            "Users",
            "ada",
            "Projects",
            "home",
            "work",
            "C:",
            "\\",
            "private title",
            "s-1",
            "s-2",
        ] {
            assert!(!wire.contains(private), "{private:?} in {wire}");
        }
        // No separator in a body at all: a path cannot hide in one.
        assert!(!seen.body.contains('/'), "{}", seen.body);
        for session in &stats.sessions {
            assert!(!wire.contains(&session.cwd));
        }
    }
}

#[test]
fn a_sync_sends_only_the_rows_that_changed() {
    let server = Server::start();
    let (cloud, _tokens, dir) = server.signed_in("changes");
    let monday = row("2026-11-16", "claude-opus-5-5", "hangar", 100);
    let tuesday = row("2026-11-17", "claude-opus-5-5", "hangar", 5);
    let other = row("2026-11-16", "claude-haiku-5-5", "site", 9);

    // The first sync sends everything.
    let first = cloud
        .sync_rows(true, &[monday.clone(), tuesday.clone(), other.clone()])
        .unwrap();
    assert_eq!((first.sent, first.unchanged), (3, 0));
    assert_eq!(server.puts().len(), 1);
    assert_eq!(server.fake().rows.len(), 3);

    // Nothing changed: nothing is sent, and the server is not even called.
    let before = server.routes().len();
    let again = cloud
        .sync_rows(true, &[monday.clone(), tuesday.clone(), other.clone()])
        .unwrap();
    assert_eq!((again.sent, again.unchanged), (0, 3));
    assert_eq!(server.routes().len(), before);

    // The day in progress grew, and a new project appeared: those two go.
    let grown = StatsRow {
        output_tokens: 42,
        active_seconds: 60,
        ..tuesday.clone()
    };
    let new = row("2026-11-17", "claude-opus-5-5", "atelier", 1);
    let third = cloud
        .sync_rows(
            true,
            &[monday.clone(), grown.clone(), other.clone(), new.clone()],
        )
        .unwrap();
    assert_eq!((third.sent, third.unchanged), (2, 2));
    let puts = server.puts();
    assert_eq!(puts.len(), 2);
    assert_eq!(puts[1], [new.clone(), grown.clone()]);
    assert_eq!(server.fake().rows.len(), 4);

    // What was sent is remembered across runs: a new process, same folder.
    let restarted = Cloud::with(&server.url, MemoryTokens::holding("hgr_test-token-0"), dir);
    let fourth = restarted
        .sync_rows(
            true,
            &[monday.clone(), grown.clone(), other.clone(), new.clone()],
        )
        .unwrap();
    assert_eq!((fourth.sent, fourth.unchanged, fourth.lost), (0, 4, 0));
    assert_eq!(server.puts().len(), 2);

    // A row that is no longer here (old transcripts cleaned up) is left as
    // it is on the server: nothing says to zero it.
    let fifth = restarted.sync_rows(true, &[grown, new]).unwrap();
    assert_eq!((fifth.sent, fifth.unchanged), (0, 2));
    assert_eq!(server.fake().rows.len(), 4);
    assert_eq!(server.puts().len(), 2);
}

#[test]
fn rows_the_server_lost_are_sent_again() {
    let server = Server::start();
    let (cloud, _tokens, dir) = server.signed_in("lost");
    let rows = [
        row("2026-11-15", "m", "hangar", 1),
        row("2026-11-16", "m", "hangar", 2),
        row("2026-11-17", "m", "hangar", 3),
    ];
    cloud.sync_rows(true, &rows).unwrap();
    assert_eq!(cloud.sync_rows(true, &rows).unwrap().sent, 0);

    // The server comes back from an older copy: one row gone, one stale.
    {
        let mut fake = server.fake();
        let gone = (
            "2026-11-15".to_string(),
            "m".to_string(),
            "hangar".to_string(),
        );
        fake.rows.remove(&gone);
        let stale = (
            "2026-11-16".to_string(),
            "m".to_string(),
            "hangar".to_string(),
        );
        fake.rows.get_mut(&stale).unwrap().output_tokens = 0;
        // Another machine's rows are not this one's to judge.
        fake.elsewhere.push(DeviceStatsRow {
            device_id: "dev_2".into(),
            row: row("2026-11-15", "m", "hangar", 77),
        });
    }
    // Within a run and within the day, the memory is trusted.
    assert_eq!(cloud.sync_rows(true, &rows).unwrap().sent, 0);

    // The next run compares first, and sends again exactly what is missing.
    let restarted = Cloud::with(
        &server.url,
        MemoryTokens::holding("hgr_test-token-0"),
        dir.clone(),
    );
    let before = server.puts().len();
    let synced = restarted.sync_rows(true, &rows).unwrap();
    assert_eq!((synced.sent, synced.unchanged, synced.lost), (2, 1, 2));
    assert_eq!(server.puts()[before], rows[..2]);
    assert_eq!(
        server.fake().rows.values().cloned().collect::<Vec<_>>(),
        rows
    );

    // Everything lost: everything again.
    server.fake().rows.clear();
    let restarted = Cloud::with(&server.url, MemoryTokens::holding("hgr_test-token-0"), dir);
    let synced = restarted.sync_rows(true, &rows).unwrap();
    assert_eq!((synced.sent, synced.unchanged, synced.lost), (3, 0, 3));
    assert_eq!(server.fake().rows.len(), 3);
}

#[test]
fn a_day_whose_local_totals_drop_is_not_sent_lower() {
    let server = Server::start();
    let (cloud, _tokens, _dir) = server.signed_in("shrinking");
    let mut full = row("2026-10-01", "m", "hangar", 500);
    full.cost_usd_micros = 900;
    full.active_seconds = 60;
    cloud.sync_rows(true, &[full.clone()]).unwrap();

    // Claude Code cleaned old transcripts: the same day now adds up to less.
    let mut cleaned = row("2026-10-01", "m", "hangar", 100);
    cleaned.cost_usd_micros = 200;
    cleaned.active_seconds = 10;
    cleaned.sessions = 0;
    let before = server.puts().len();
    let synced = cloud.sync_rows(true, &[cleaned.clone()]).unwrap();
    assert_eq!((synced.sent, synced.unchanged), (0, 1));
    assert_eq!(server.puts().len(), before);
    assert_eq!(
        server.fake().rows.values().cloned().collect::<Vec<_>>(),
        [full.clone()]
    );

    // A field that grows is still sent, the others keep their higher value.
    let mut grown = cleaned;
    grown.output_tokens = 700;
    let synced = cloud.sync_rows(true, &[grown]).unwrap();
    assert_eq!(synced.sent, 1);
    let mut expected = full;
    expected.output_tokens = 700;
    assert_eq!(
        server.fake().rows.values().cloned().collect::<Vec<_>>(),
        [expected]
    );
}

#[test]
fn signing_in_again_sends_everything_again() {
    let server = Server::start();
    let (cloud, _tokens, _dir) = server.signed_in("again");
    let rows = [row("2026-11-16", "m", "hangar", 2)];
    cloud.sync_rows(true, &rows).unwrap();
    cloud.sign_out().unwrap();

    let sign_in = cloud.sign_in(Provider::Github).unwrap();
    browse(&sign_in, "en");
    sign_in.wait().unwrap();
    assert_eq!(cloud.sync_rows(true, &rows).unwrap().sent, 1);
}

#[test]
fn more_rows_than_a_call_takes_go_in_several() {
    let server = Server::start();
    let (cloud, _tokens, _dir) = server.signed_in("many");
    let rows: Vec<StatsRow> = (0..api::MAX_STATS_ROWS + 3)
        .map(|n| row("2026-11-16", "m", &format!("p{n:05}"), 1))
        .collect();
    let synced = cloud.sync_rows(true, &rows).unwrap();
    assert_eq!(synced.sent, rows.len());
    let sizes: Vec<usize> = server.puts().iter().map(Vec::len).collect();
    assert_eq!(sizes, [api::MAX_STATS_ROWS, 3]);
    assert_eq!(server.fake().rows.len(), rows.len());
    assert_eq!(cloud.sync_rows(true, &rows).unwrap().sent, 0);
}

#[test]
fn a_row_the_protocol_refuses_stays_here_and_the_rest_goes() {
    let server = Server::start();
    let (cloud, _tokens, _dir) = server.signed_in("skipped");
    let good = row("2026-11-16", "m", "hangar", 1);
    // Rows are built from names made safe; a path given here all the same
    // does not leave.
    let path = row("2026-11-16", "m", "C:\\Users\\ada\\hangar", 1);
    let no_day = row("yesterday", "m", "hangar", 1);
    // A key given twice keeps its last row.
    let later = StatsRow {
        output_tokens: 8,
        ..good.clone()
    };
    let synced = cloud
        .sync_rows(true, &[good, path, no_day, later.clone()])
        .unwrap();
    assert_eq!((synced.sent, synced.skipped), (1, 2));
    assert_eq!(server.puts(), [vec![later]]);
}

#[test]
fn a_failed_send_is_tried_again_at_the_next_sync() {
    let server = Server::start();
    let (cloud, _tokens, _dir) = server.signed_in("retry");
    let rows = [row("2026-11-16", "m", "hangar", 1)];
    server.fake().refuse_next = Some((ErrorCode::Internal, None));
    assert!(matches!(
        cloud.sync_rows(true, &rows),
        Err(Error::Refused { status: 500, .. })
    ));
    assert!(server.fake().rows.is_empty());
    assert_eq!(cloud.sync_rows(true, &rows).unwrap().sent, 1);
    assert_eq!(server.fake().rows.len(), 1);
}

#[test]
fn with_the_sync_off_nothing_is_sent_or_read() {
    let server = Server::start();
    let (cloud, _tokens, dir) = server.signed_in("off");
    let rows = [row("2026-11-16", "m", "hangar", 1)];
    assert_eq!(cloud.sync_rows(false, &rows), Err(Error::SyncOff));
    assert_eq!(cloud.pull_stats(false, None, None), Err(Error::SyncOff));

    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let host = crate::host::Host::with_paths(crate::paths::Paths::rooted(
        &crate::testdir::scratch("cloud-off-host"),
    ));
    // Off: returns before the transcripts are even looked at.
    assert_eq!(
        runtime.block_on(cloud.sync_stats(host.clone(), false)),
        Err(Error::SyncOff)
    );
    // On, and signed out: the same.
    let (nobody, _tokens, _dir) = server.signed_out("off-nobody");
    assert_eq!(
        runtime.block_on(nobody.sync_stats(host, true)),
        Err(Error::NotSignedIn)
    );

    assert!(server.routes().is_empty());
    assert!(!dir.exists());
}

#[test]
fn pulling_reads_every_machine_s_rows_for_the_days_asked() {
    let server = Server::start();
    let (cloud, _tokens, _dir) = server.signed_in("pull");
    cloud
        .sync_rows(
            true,
            &[
                row("2026-11-10", "m", "hangar", 1),
                row("2026-11-16", "m", "hangar", 2),
            ],
        )
        .unwrap();
    server.fake().elsewhere.push(DeviceStatsRow {
        device_id: "dev_2".into(),
        row: row("2026-11-16", "m", "site", 5),
    });

    let week = cloud
        .pull_stats(true, Some("2026-11-14"), Some("2026-11-16"))
        .unwrap();
    assert_eq!(week.devices.len(), 2);
    let brief: Vec<_> = week
        .rows
        .iter()
        .map(|r| {
            (
                r.device_id.as_str(),
                r.row.project.as_str(),
                r.row.output_tokens,
            )
        })
        .collect();
    assert_eq!(brief, [("dev_1", "hangar", 2), ("dev_2", "site", 5)]);
    assert_eq!(cloud.pull_stats(true, None, None).unwrap().rows.len(), 3);
    assert_eq!(
        cloud
            .pull_stats(true, Some("2026-11-16"), None)
            .unwrap()
            .rows
            .len(),
        2
    );

    // A day that is not one is refused here, before it is sent.
    let before = server.routes().len();
    for (from, to) in [(Some("16/11/2026"), None), (None, Some("2026-11-31"))] {
        assert!(matches!(
            cloud.pull_stats(true, from, to),
            Err(Error::Invalid(_))
        ));
    }
    assert_eq!(server.routes().len(), before);
    let routes = server.routes();
    assert!(routes.contains(&"GET /v1/stats?from=2026-11-14&to=2026-11-16".to_string()));
    assert!(routes.contains(&"GET /v1/stats?".to_string()));
    assert!(routes.contains(&"GET /v1/stats?from=2026-11-16".to_string()));
}

// ---------------------------------------------------------------------------
// The token
// ---------------------------------------------------------------------------

#[test]
fn the_stand_in_keeps_a_token_as_the_keychain_would() {
    let tokens = MemoryTokens::new();
    assert_eq!(tokens.load(), Ok(None));
    assert_eq!(tokens.delete(), Ok(()));
    tokens.save("hgr_a").unwrap();
    assert_eq!(tokens.load(), Ok(Some("hgr_a".into())));
    // A clone is the same store, as two handles on one keychain entry are.
    let other = tokens.clone();
    other.save("hgr_b").unwrap();
    assert_eq!(tokens.peek().as_deref(), Some("hgr_b"));
    tokens.delete().unwrap();
    assert_eq!(other.load(), Ok(None));
}

/// Talks to the real keychain, under an entry of its own, so it stays out of
/// CI: the Linux runner has no Secret Service. On a desktop:
/// `cargo test -p hangar-core cloud::tests::real_keychain -- --ignored`.
#[test]
#[ignore]
fn real_keychain_round_trip() {
    let keychain = Keychain::for_server("http://127.0.0.1:1/hangar-cloud-test");
    keychain.delete().unwrap();
    assert_eq!(keychain.load(), Ok(None));
    keychain.save("hgr_test").unwrap();
    assert_eq!(keychain.load(), Ok(Some("hgr_test".into())));
    keychain.delete().unwrap();
    assert_eq!(keychain.load(), Ok(None));
    assert_eq!(keychain.delete(), Ok(()));
}
