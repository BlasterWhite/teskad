//! A stand-in for GitHub and Google: a real HTTP server on the loopback, which
//! the server under test calls as it calls the real ones (`provider.rs`), and
//! which is as strict as they are about what it is sent: the client's id and
//! secret, the redirect URI, the scope, a code that works once, and the PKCE
//! verifier checked against the challenge the consent page was asked with.
//!
//! One for the whole test binary, on one port; each test adds the people it
//! needs, under keys nobody else knows.

use std::collections::HashMap;
use std::net::{SocketAddr, TcpListener};
use std::sync::{Arc, Mutex, OnceLock};

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// The OAuth applications the stand-in knows: an id and its secret.
pub const GITHUB_CLIENT: (&str, &str) = ("gh-client", "gh-secret-7f3a");
pub const GOOGLE_CLIENT: (&str, &str) = ("g-client.apps.test", "g-secret-91bc");

/// Somebody with an account at the stand-in, as each provider describes them.
#[derive(Debug, Clone)]
pub struct Person {
    /// GitHub's `/user`.
    pub github_user: Value,
    /// GitHub's `/user/emails`.
    pub github_emails: Value,
    /// Google's userinfo.
    pub google_userinfo: Value,
    /// Says no on the consent page.
    pub denies: bool,
    /// The step at which the provider breaks for them: `token`, `user`,
    /// `emails` or `userinfo`.
    pub breaks_at: Option<&'static str>,
}

impl Default for Person {
    fn default() -> Self {
        Self {
            github_user: Value::Null,
            github_emails: json!([]),
            google_userinfo: Value::Null,
            denies: false,
            breaks_at: None,
        }
    }
}

/// A consent given: what the code that came out of it is bound to.
struct Grant {
    person: String,
    provider: String,
    redirect_uri: String,
    challenge: String,
}

#[derive(Default)]
struct Inner {
    people: HashMap<String, Person>,
    grants: HashMap<String, Grant>,
    /// Access token -> (provider, person).
    tokens: HashMap<String, (String, String)>,
    /// How many times each person's code was presented to a token endpoint.
    exchanges: HashMap<String, u32>,
}

pub struct Fake {
    base: String,
    inner: Mutex<Inner>,
}

impl Fake {
    /// `http://127.0.0.1:<port>`.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Adds a person, and gives the key the tests' browser is "logged in"
    /// with at the consent page (`&login_as=<key>`).
    pub fn add(&self, person: Person) -> String {
        let key = uuid::Uuid::new_v4().simple().to_string();
        self.inner
            .lock()
            .unwrap()
            .people
            .insert(key.clone(), person);
        key
    }

    /// Replaces what the providers say about a person.
    pub fn update(&self, key: &str, person: Person) {
        self.inner
            .lock()
            .unwrap()
            .people
            .insert(key.to_string(), person);
    }

    /// How many times a code of this person's reached a token endpoint.
    pub fn exchanges(&self, key: &str) -> u32 {
        self.inner
            .lock()
            .unwrap()
            .exchanges
            .get(key)
            .copied()
            .unwrap_or(0)
    }
}

/// The stand-in, started the first time a test asks for it.
pub fn provider() -> &'static Fake {
    static FAKE: OnceLock<Arc<Fake>> = OnceLock::new();
    FAKE.get_or_init(|| {
        let listener = listen();
        let port = listener.local_addr().unwrap().port();
        let fake = Arc::new(Fake {
            base: format!("http://127.0.0.1:{port}"),
            inner: Mutex::default(),
        });
        let served = Arc::clone(&fake);
        // A thread and a runtime of its own: each test's runtime ends with
        // its test.
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                listener.set_nonblocking(true).unwrap();
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                let router = Router::new()
                    .route("/{provider}/authorize", get(authorize))
                    .route("/{provider}/token", post(token))
                    .route("/github/api/user", get(github_user))
                    .route("/github/api/user/emails", get(github_emails))
                    .route("/google/userinfo", get(google_userinfo))
                    .with_state(served);
                axum::serve(listener, router).await.unwrap();
            });
        });
        fake
    })
}

/// A loopback port: one of the range Hangar gives a worker
/// (`HANGAR_PORT_BASE`, `HANGAR_PORT_COUNT`) when there is one, so that
/// agents working side by side never meet; any free port otherwise.
fn listen() -> TcpListener {
    let var = |name: &str| std::env::var(name).ok().and_then(|v| v.parse::<u16>().ok());
    if let Some(base) = var("HANGAR_PORT_BASE") {
        let count = var("HANGAR_PORT_COUNT").unwrap_or(20);
        for port in base..base.saturating_add(count) {
            if let Ok(listener) = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port))) {
                return listener;
            }
        }
        panic!("no free port in {base}..{}", base.saturating_add(count));
    }
    TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).unwrap()
}

fn client_of(provider: &str) -> Option<(&'static str, &'static str)> {
    match provider {
        "github" => Some(GITHUB_CLIENT),
        "google" => Some(GOOGLE_CLIENT),
        _ => None,
    }
}

fn scope_of(provider: &str) -> &'static str {
    match provider {
        "github" => "read:user user:email",
        _ => "openid email profile",
    }
}

fn refuse(status: StatusCode, why: &str) -> Response {
    (status, why.to_string()).into_response()
}

/// The consent page. The tests' browser adds `login_as`, who it is at the
/// provider. Answers as the real ones do once the person has chosen: `302`
/// to the redirect URI with a code, or with `error=access_denied`.
async fn authorize(
    State(fake): State<Arc<Fake>>,
    Path(provider): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let Some((client_id, _)) = client_of(&provider) else {
        return refuse(StatusCode::NOT_FOUND, "no such provider");
    };
    let get = |name: &str| query.get(name).map(String::as_str).unwrap_or("");
    if get("response_type") != "code"
        || get("client_id") != client_id
        || get("scope") != scope_of(&provider)
        || get("code_challenge_method") != "S256"
        || get("code_challenge").len() != 43
        || get("state").is_empty()
        || get("redirect_uri").is_empty()
    {
        return refuse(StatusCode::BAD_REQUEST, "not what the consent page takes");
    }
    let mut inner = fake.inner.lock().unwrap();
    let key = get("login_as");
    let Some(person) = inner.people.get(key) else {
        return refuse(StatusCode::BAD_REQUEST, "nobody is logged in");
    };
    let back = |name: &str, value: &str| {
        let location = format!(
            "{}?{name}={value}&state={}",
            get("redirect_uri"),
            get("state")
        );
        (StatusCode::FOUND, [(header::LOCATION, location)]).into_response()
    };
    if person.denies {
        return back("error", "access_denied");
    }
    let code = uuid::Uuid::new_v4().simple().to_string();
    inner.grants.insert(
        code.clone(),
        Grant {
            person: key.to_string(),
            provider,
            redirect_uri: get("redirect_uri").to_string(),
            challenge: get("code_challenge").to_string(),
        },
    );
    back("code", &code)
}

/// The token endpoint: a form, as OAuth sends it.
async fn token(
    State(fake): State<Arc<Fake>>,
    Path(provider): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let Some((client_id, client_secret)) = client_of(&provider) else {
        return refuse(StatusCode::NOT_FOUND, "no such provider");
    };
    let form = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|kind| kind.starts_with("application/x-www-form-urlencoded"));
    let parsed = format!("/?{body}")
        .parse::<Uri>()
        .ok()
        .and_then(|uri| Query::<HashMap<String, String>>::try_from_uri(&uri).ok());
    let Some(Query(fields)) = parsed.filter(|_| form) else {
        return refuse(StatusCode::BAD_REQUEST, "not a form");
    };
    let get = |name: &str| fields.get(name).map(String::as_str).unwrap_or("");

    // GitHub says no with a 200, Google with a 400.
    let refused = |error: &str| {
        let status = if provider == "github" {
            StatusCode::OK
        } else {
            StatusCode::BAD_REQUEST
        };
        (status, Json(json!({ "error": error }))).into_response()
    };

    let mut inner = fake.inner.lock().unwrap();
    // A code works once, whatever comes with it.
    let grant = inner.grants.remove(get("code"));
    if let Some(grant) = &grant {
        *inner.exchanges.entry(grant.person.clone()).or_default() += 1;
    }
    if get("client_id") != client_id || get("client_secret") != client_secret {
        return refused("invalid_client");
    }
    let Some(grant) = grant else {
        return refused("bad_verification_code");
    };
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(get("code_verifier").as_bytes()));
    if grant.provider != provider
        || get("grant_type") != "authorization_code"
        || get("redirect_uri") != grant.redirect_uri
        || challenge != grant.challenge
    {
        return refused("invalid_grant");
    }
    if inner.people[&grant.person].breaks_at == Some("token") {
        return refuse(StatusCode::INTERNAL_SERVER_ERROR, "broken");
    }
    let access_token = format!("at-{}", uuid::Uuid::new_v4().simple());
    inner
        .tokens
        .insert(access_token.clone(), (provider.clone(), grant.person));
    Json(json!({
        "access_token": access_token,
        "token_type": if provider == "github" { "bearer" } else { "Bearer" },
        "scope": scope_of(&provider),
    }))
    .into_response()
}

/// What `provider` says at `step` about the person the request's token is
/// for.
fn profile(
    fake: &Fake,
    headers: &HeaderMap,
    provider: &str,
    step: &'static str,
    read: impl Fn(&Person) -> Value,
) -> Response {
    // GitHub refuses a request that does not say what makes it.
    if !headers.contains_key(header::USER_AGENT) {
        return refuse(StatusCode::FORBIDDEN, "no user agent");
    }
    let inner = fake.inner.lock().unwrap();
    let person = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .and_then(|token| inner.tokens.get(token))
        .filter(|(of, _)| of == provider)
        .and_then(|(_, key)| inner.people.get(key));
    match person {
        None => refuse(StatusCode::UNAUTHORIZED, "bad credentials"),
        Some(person) if person.breaks_at == Some(step) => {
            refuse(StatusCode::INTERNAL_SERVER_ERROR, "broken")
        }
        Some(person) => Json(read(person)).into_response(),
    }
}

async fn github_user(State(fake): State<Arc<Fake>>, headers: HeaderMap) -> Response {
    profile(&fake, &headers, "github", "user", |person| {
        person.github_user.clone()
    })
}

async fn github_emails(State(fake): State<Arc<Fake>>, headers: HeaderMap) -> Response {
    profile(&fake, &headers, "github", "emails", |person| {
        person.github_emails.clone()
    })
}

async fn google_userinfo(State(fake): State<Arc<Fake>>, headers: HeaderMap) -> Response {
    profile(&fake, &headers, "google", "userinfo", |person| {
        person.google_userinfo.clone()
    })
}
