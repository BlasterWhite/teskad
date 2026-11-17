//! GitHub, as the sign-in meets it: where the browser is sent to consent,
//! and the calls the server then makes itself, once per sign-in: the code
//! exchanged for an access token, the profile read with it
//! (docs/native/cloud.md, "Signing in").
//!
//! The access token lives in this module's local variables and nowhere else:
//! it is not stored, not logged, and dropped when the profile has been read.
//! What goes wrong is said on stderr as a step and a status, never as a body,
//! a code or a token.
//!
//! The calls are ureq's, which blocks: [`identify`] is run on a blocking
//! thread by the callback.

use std::sync::OnceLock;
use std::time::Duration;

use hangar_cloud_api::Provider;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::account::NewIdentity;
use crate::config::OAuthClient;

/// The longest display name the server keeps, in characters.
const MAX_NAME_CHARS: usize = 100;
/// The longest address there is (RFC 5321).
const MAX_EMAIL_CHARS: usize = 254;
/// The longest avatar URL the server keeps.
const MAX_URL_CHARS: usize = 2000;

/// Where the provider is. The real one, always, outside of the tests: no
/// variable of the environment moves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    pub github_authorize: String,
    pub github_token: String,
    /// `/user` and `/user/emails` are under it.
    pub github_api: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            github_authorize: "https://github.com/login/oauth/authorize".into(),
            github_token: "https://github.com/login/oauth/access_token".into(),
            github_api: "https://api.github.com".into(),
        }
    }
}

impl Endpoints {
    /// A stand-in for the provider at `base` (`http://127.0.0.1:<port>`).
    #[cfg(test)]
    pub fn fake(base: &str) -> Self {
        Self {
            github_authorize: format!("{base}/github/authorize"),
            github_token: format!("{base}/github/token"),
            github_api: format!("{base}/github/api"),
        }
    }
}

/// What the server asks the provider for: who the person is and their
/// address, and nothing they own.
pub fn scopes(provider: Provider) -> &'static str {
    match provider {
        Provider::Github => "read:user user:email",
    }
}

/// The provider's consent page for this sign-in: the server's own state and
/// its own PKCE challenge (S256), never the desktop's.
pub fn authorize_url(
    endpoints: &Endpoints,
    provider: Provider,
    client: &OAuthClient,
    redirect_uri: &str,
    state: &str,
    challenge: &str,
) -> String {
    let base = match provider {
        Provider::Github => &endpoints.github_authorize,
    };
    let query = [
        ("response_type", "code"),
        ("client_id", client.client_id.as_str()),
        ("redirect_uri", redirect_uri),
        ("scope", scopes(provider)),
        ("state", state),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
    ]
    .map(|(name, value)| format!("{name}={}", percent_encode(value)))
    .join("&");
    format!("{base}?{query}")
}

/// `text` as a query's value: everything but RFC 3986's unreserved
/// characters is escaped, so nothing in it can end the value or start
/// another.
pub fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// The provider did not say who the person is. Why is on stderr.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Failed;

/// Exchanges `code` (with the client's secret and the server's PKCE verifier)
/// and reads who the person is. Blocks.
pub fn identify(
    endpoints: &Endpoints,
    provider: Provider,
    client: &OAuthClient,
    redirect_uri: &str,
    code: &str,
    verifier: &str,
) -> Result<NewIdentity, Failed> {
    let token_url = match provider {
        Provider::Github => &endpoints.github_token,
    };
    let token = access_token(provider, token_url, client, redirect_uri, code, verifier)?;
    match provider {
        Provider::Github => github_identity(&endpoints.github_api, &token),
    }
}

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            // The tests' stand-in speaks plain HTTP on the loopback; the
            // real endpoints are all HTTPS and nothing else is ever called.
            .https_only(!cfg!(test))
            // A redirect would carry the secret or the token somewhere the
            // server did not choose.
            .max_redirects(0)
            // GitHub's API refuses a request without one.
            .user_agent("hangar-cloud")
            .timeout_connect(Some(Duration::from_secs(5)))
            .timeout_global(Some(Duration::from_secs(10)))
            .max_idle_connections(0)
            .build()
            .new_agent()
    })
}

/// Says a failed step on stderr: the provider, the step, and the status or
/// the kind of failure. ureq's errors name no body and no header.
fn failed(provider: Provider, step: &str, why: impl std::fmt::Display) -> Failed {
    eprintln!("hangar-cloud: sign-in with {provider} failed at {step}: {why}");
    Failed
}

fn why(err: &ureq::Error) -> String {
    match err {
        ureq::Error::StatusCode(status) => format!("status {status}"),
        ureq::Error::Timeout(_) => "timeout".into(),
        ureq::Error::Json(_) => "an answer that is not the JSON expected".into(),
        _ => "no answer".into(),
    }
}

#[derive(Deserialize)]
struct TokenAnswer {
    access_token: Option<String>,
    token_type: Option<String>,
    /// GitHub answers a refused code with `200` and this.
    error: Option<String>,
}

fn access_token(
    provider: Provider,
    url: &str,
    client: &OAuthClient,
    redirect_uri: &str,
    code: &str,
    verifier: &str,
) -> Result<String, Failed> {
    let step = "the code's exchange";
    let answer: TokenAnswer = agent()
        .post(url)
        // GitHub answers a form unless asked for JSON.
        .header("Accept", "application/json")
        .send_form([
            ("grant_type", "authorization_code"),
            ("client_id", client.client_id.as_str()),
            ("client_secret", client.client_secret.as_str()),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", verifier),
        ])
        .and_then(|mut response| response.body_mut().read_json())
        .map_err(|err| failed(provider, step, why(&err)))?;
    if answer.error.is_some() {
        // The error's name is the provider's own word (`bad_verification_code`),
        // but it is its text: only that there was one is said.
        return Err(failed(provider, step, "the provider refused the code"));
    }
    let bearer = answer
        .token_type
        .is_none_or(|kind| kind.eq_ignore_ascii_case("bearer"));
    match answer.access_token {
        Some(token) if bearer && !token.is_empty() => Ok(token),
        _ => Err(failed(provider, step, "no bearer token in the answer")),
    }
}

fn get_json<T: DeserializeOwned>(
    provider: Provider,
    step: &str,
    url: &str,
    token: &str,
    accept: &str,
) -> Result<T, Failed> {
    agent()
        .get(url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", accept)
        .call()
        .and_then(|mut response| response.body_mut().read_json())
        .map_err(|err| failed(provider, step, why(&err)))
}

#[derive(Deserialize)]
struct GithubUser {
    id: u64,
    login: String,
    name: Option<String>,
    avatar_url: Option<String>,
}

#[derive(Deserialize)]
struct GithubEmail {
    email: String,
    #[serde(default)]
    primary: bool,
    #[serde(default)]
    verified: bool,
}

/// GitHub: `/user`, and the address of `/user/emails` that is both the
/// primary one and verified. The `email` of `/user` is the public one, which
/// says nothing of either, and is not read.
fn github_identity(api: &str, token: &str) -> Result<NewIdentity, Failed> {
    let provider = Provider::Github;
    let accept = "application/vnd.github+json";
    let user: GithubUser = get_json(
        provider,
        "the profile",
        &format!("{api}/user"),
        token,
        accept,
    )?;
    let emails: Vec<GithubEmail> = get_json(
        provider,
        "the addresses",
        &format!("{api}/user/emails"),
        token,
        accept,
    )?;
    let email = emails
        .into_iter()
        .find(|entry| entry.primary && entry.verified)
        .and_then(|entry| verified_email(&entry.email));
    let display_name = display_name([user.name.as_deref(), Some(&user.login)], "GitHub user");
    Ok(NewIdentity {
        provider,
        provider_user_id: user.id.to_string(),
        display_name,
        email,
        avatar_url: user.avatar_url.as_deref().and_then(avatar_url),
    })
}

/// An address the provider vouched for, as the server keeps it: trimmed and
/// in lower case. Nothing when it is not plausibly an address: it is then not
/// kept.
fn verified_email(email: &str) -> Option<String> {
    let email = email.trim().to_lowercase();
    let shaped = email.chars().count() <= MAX_EMAIL_CHARS
        && !email.chars().any(|c| c.is_control() || c.is_whitespace())
        && email
            .split_once('@')
            .is_some_and(|(local, domain)| !local.is_empty() && !domain.is_empty());
    shaped.then_some(email)
}

/// The first of `candidates` that is a name, on one line and cut to what the
/// server keeps.
fn display_name<const N: usize>(candidates: [Option<&str>; N], fallback: &str) -> String {
    candidates
        .into_iter()
        .flatten()
        .map(|name| {
            name.chars()
                .filter(|c| !c.is_control())
                .collect::<String>()
                .trim()
                .chars()
                .take(MAX_NAME_CHARS)
                .collect::<String>()
        })
        .find(|name| !name.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

/// An avatar the desktop may load: an HTTPS address, or none.
fn avatar_url(url: &str) -> Option<String> {
    let shaped = url.starts_with("https://")
        && url.chars().count() <= MAX_URL_CHARS
        && !url.chars().any(|c| c.is_control() || c.is_whitespace());
    shaped.then(|| url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_value_cannot_leave_its_place() {
        assert_eq!(percent_encode("aZ09-._~"), "aZ09-._~");
        assert_eq!(
            percent_encode("a b&c=d#e/é?"),
            "a%20b%26c%3Dd%23e%2F%C3%A9%3F"
        );
    }

    #[test]
    fn the_consent_page_is_asked_with_the_server_s_state_and_s256() {
        let client = OAuthClient {
            client_id: "the id".into(),
            client_secret: "the-secret".into(),
        };
        let url = authorize_url(
            &Endpoints::default(),
            Provider::Github,
            &client,
            "https://account.hangar.simnjs.fr/v1/auth/callback/github",
            "st",
            "ch",
        );
        assert_eq!(
            url,
            "https://github.com/login/oauth/authorize?response_type=code&client_id=the%20id\
             &redirect_uri=https%3A%2F%2Faccount.hangar.simnjs.fr%2Fv1%2Fauth%2Fcallback%2Fgithub\
             &scope=read%3Auser%20user%3Aemail&state=st&code_challenge=ch\
             &code_challenge_method=S256"
        );
        assert!(!url.contains("the-secret"));
    }

    #[test]
    fn the_real_endpoints_are_all_https() {
        let endpoints = Endpoints::default();
        for url in [
            &endpoints.github_authorize,
            &endpoints.github_token,
            &endpoints.github_api,
        ] {
            assert!(url.starts_with("https://"), "{url}");
        }
    }

    #[test]
    fn an_address_is_kept_in_lower_case_or_not_at_all() {
        assert_eq!(
            verified_email(" Ada@Example.ORG "),
            Some("ada@example.org".into())
        );
        for not_one in [
            "",
            "ada",
            "@example.org",
            "ada@",
            "a da@example.org",
            "a\n@b",
        ] {
            assert_eq!(verified_email(not_one), None, "{not_one:?}");
        }
        assert_eq!(verified_email(&format!("{}@b", "a".repeat(253))), None);
    }

    #[test]
    fn a_name_is_one_line_of_a_hundred_characters_at_most() {
        assert_eq!(display_name([Some("  Ada\n "), Some("ada")], "x"), "Ada");
        assert_eq!(display_name([Some(" \t"), Some("ada")], "x"), "ada");
        assert_eq!(display_name([None, None], "GitHub user"), "GitHub user");
        assert_eq!(
            display_name([Some(&"é".repeat(150))], "x").chars().count(),
            100
        );
    }

    #[test]
    fn an_avatar_is_an_https_address_or_none() {
        assert_eq!(
            avatar_url("https://avatars.example/u/1"),
            Some("https://avatars.example/u/1".into())
        );
        assert_eq!(avatar_url("http://avatars.example/u/1"), None);
        assert_eq!(avatar_url("javascript:alert(1)"), None);
        assert_eq!(avatar_url("https://a b"), None);
    }
}
