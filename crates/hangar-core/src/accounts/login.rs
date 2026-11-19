//! Who an account is logged in as, read from its own files on this machine
//! and never sent anywhere — and, on macOS, whether each Claude login really
//! is its own.
//!
//! No token is ever kept: Claude Code's `.credentials.json` and Codex's
//! `auth.json` are read for whether they hold a login, which account and
//! which plan, nothing else. A token is matched as `IgnoredAny`, which reads
//! past it; Codex's ID token, a JWT, has its claims decoded where it is read
//! and is dropped there.

use std::fs;
use std::path::Path;

use base64::Engine;
use serde::de::IgnoredAny;
use serde::{Deserialize, Deserializer};
use sha2::{Digest, Sha256};

use super::ClaudeHome;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Login {
    pub logged_in: bool,
    pub email: Option<String>,
    /// Claude Code's `subscriptionType`: `pro`, `max`, `team`… For Codex, the
    /// ChatGPT plan (`plus`, `pro`, `team`…), or for a login that is no
    /// ChatGPT account, Codex's own name for it (`apikey`…, see
    /// `read_codex_login`).
    pub plan: Option<String>,
}

#[derive(Deserialize)]
struct Config {
    #[serde(rename = "oauthAccount")]
    account: Option<OauthAccount>,
}

#[derive(Deserialize)]
struct OauthAccount {
    #[serde(rename = "emailAddress")]
    email: Option<String>,
    #[serde(rename = "accountUuid")]
    uuid: Option<IgnoredAny>,
}

#[derive(Deserialize)]
struct Credentials {
    #[serde(rename = "claudeAiOauth")]
    oauth: Option<Oauth>,
}

#[derive(Deserialize)]
struct Oauth {
    #[serde(rename = "accessToken")]
    access: Option<IgnoredAny>,
    #[serde(rename = "refreshToken")]
    refresh: Option<IgnoredAny>,
    #[serde(rename = "subscriptionType")]
    plan: Option<String>,
}

fn read<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(raw.trim_start_matches('\u{feff}')).ok()
}

/// The login of the account whose files are at `home`. `keychain` is
/// whether Claude Code keeps the tokens in the system keychain rather than in
/// `.credentials.json` (macOS): the account it logged in as, which it writes
/// to `.claude.json` at login and drops at logout, then says it.
pub fn read_login(home: &ClaudeHome, keychain: bool) -> Login {
    let account = read::<Config>(&home.config).and_then(|config| config.account);
    let email = account
        .as_ref()
        .and_then(|account| account.email.clone())
        .map(|email| email.trim().to_string())
        .filter(|email| !email.is_empty());
    if keychain {
        let logged_in = account.is_some_and(|account| account.uuid.is_some() || email.is_some());
        return Login {
            logged_in,
            email,
            plan: None,
        };
    }
    let oauth = read::<Credentials>(&home.credentials()).and_then(|file| file.oauth);
    let logged_in = oauth
        .as_ref()
        .is_some_and(|oauth| oauth.access.is_some() || oauth.refresh.is_some());
    let plan = oauth
        .and_then(|oauth| oauth.plan)
        .map(|plan| plan.trim().to_string())
        .filter(|plan| !plan.is_empty());
    Login {
        logged_in,
        email: if logged_in { email } else { None },
        plan: if logged_in { plan } else { None },
    }
}

/// The macOS Keychain entry Claude Code keeps the login of `dir` in, when it
/// is started with `CLAUDE_CONFIG_DIR=<dir>`: `Claude Code-credentials-`
/// and the first eight hex digits of the SHA-256 of the folder's path. Read
/// in Claude Code 2.1.316's bundle; the main account, started without the
/// variable, keeps the unsuffixed `Claude Code-credentials`.
pub fn keychain_service(dir: &Path) -> String {
    let digest = Sha256::digest(dir.to_string_lossy().as_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("Claude Code-credentials-{}", &hex[..8])
}

/// Whether logins are kept apart, from the profiles that say they are logged
/// in: each must have a Keychain entry of its own. A Claude Code that keeps
/// one entry for every folder (older versions did) logs a profile in over
/// the main account instead, and leaves its own entry missing.
///
/// `probe` answers whether an entry exists, `None` when it cannot tell. A
/// folder whose path is not plain ASCII is not judged: Claude Code hashes it
/// in Unicode NFC, which is not checked here.
pub fn logins_isolated(profiles: &[ClaudeHome], probe: impl Fn(&str) -> Option<bool>) -> bool {
    !profiles.iter().any(|profile| {
        let dir = profile.dir.to_string_lossy();
        dir.is_ascii()
            && read_login(profile, true).logged_in
            && probe(&keychain_service(&profile.dir)) == Some(false)
    })
}

/// Asks the Keychain for an entry's attributes only: no secret is read, so
/// nothing is prompted. `security` exits 44 when there is no such entry.
pub fn keychain_has(service: &str) -> Option<bool> {
    let status = std::process::Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", service])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;
    match status.code() {
        Some(0) => Some(true),
        Some(44) => Some(false),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Codex
// ---------------------------------------------------------------------------

/// The part of `auth.json` that says who is logged in, as Codex 0.170.0
/// writes it (`AuthDotJson`, codex-rs/login/src/auth/storage.rs). Every
/// credential is `IgnoredAny`: read past, never held.
#[derive(Deserialize)]
struct CodexAuth {
    #[serde(default)]
    auth_mode: Option<String>,
    #[serde(rename = "OPENAI_API_KEY", default)]
    api_key: Option<IgnoredAny>,
    #[serde(default)]
    tokens: Option<CodexTokens>,
    #[serde(default)]
    agent_identity: Option<AgentIdentity>,
    #[serde(default)]
    personal_access_token: Option<IgnoredAny>,
    #[serde(default)]
    bedrock_api_key: Option<IgnoredAny>,
    #[serde(default)]
    bedrock_access_keys: Option<IgnoredAny>,
}

/// `TokenData` (codex-rs/login/src/token_data.rs).
#[derive(Deserialize)]
struct CodexTokens {
    #[serde(default, deserialize_with = "id_claims")]
    id_token: Option<IdClaims>,
    #[serde(default)]
    access_token: Option<IgnoredAny>,
    #[serde(default)]
    refresh_token: Option<IgnoredAny>,
}

/// The ID token's claims Codex reads the account from (`IdClaims`,
/// token_data.rs): the e-mail at the top or in the profile claim, the plan in
/// the auth claim.
#[derive(Deserialize)]
struct IdClaims {
    #[serde(default)]
    email: Option<String>,
    #[serde(rename = "https://api.openai.com/profile", default)]
    profile: Option<ProfileClaims>,
    #[serde(rename = "https://api.openai.com/auth", default)]
    auth: Option<AuthClaims>,
}

#[derive(Deserialize)]
struct ProfileClaims {
    #[serde(default)]
    email: Option<String>,
}

#[derive(Deserialize)]
struct AuthClaims {
    #[serde(default)]
    chatgpt_plan_type: Option<String>,
}

/// `AgentIdentityStorage`: a JWT, or a record that names its account.
#[derive(Deserialize)]
#[serde(untagged)]
enum AgentIdentity {
    Record {
        #[serde(default)]
        email: Option<String>,
        #[serde(default)]
        plan_type: Option<String>,
    },
    Jwt(IgnoredAny),
}

/// The claims of the JWT in `id_token`, or nothing for a token that does not
/// decode: the login is still there, only its account unknown.
fn id_claims<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<IdClaims>, D::Error> {
    let jwt = Option::<String>::deserialize(deserializer)?;
    Ok(jwt.as_deref().and_then(jwt_claims))
}

/// A JWT's payload, read as Codex reads it (`decode_jwt_payload`,
/// token_data.rs): three parts, the second base64url without padding. The
/// signature is not checked: the file is the account's own, read for its
/// name only.
fn jwt_claims(jwt: &str) -> Option<IdClaims> {
    let mut parts = jwt.split('.');
    let (Some(header), Some(payload), Some(signature)) = (parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    if header.is_empty() || payload.is_empty() || signature.is_empty() {
        return None;
    }
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

impl CodexAuth {
    /// How Codex logs in with this file (`resolved_mode`,
    /// codex-rs/login/src/auth/manager.rs): the mode it names, else the first
    /// credential it holds, else a ChatGPT account.
    fn mode(&self) -> &str {
        if let Some(mode) = self.auth_mode.as_deref() {
            return mode;
        }
        if self.personal_access_token.is_some() {
            "personalAccessToken"
        } else if self.bedrock_api_key.is_some() {
            "bedrockApiKey"
        } else if self.bedrock_access_keys.is_some() {
            "bedrockAccessKeys"
        } else if self.api_key.is_some() {
            "apikey"
        } else {
            "chatgpt"
        }
    }
}

fn cleaned(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// The login of the Codex home whose `auth.json` is `auth`: a ChatGPT
/// account's e-mail and plan, from the claims of its ID token; for any other
/// way of logging in, no e-mail and, as the plan, Codex's name for it
/// (`apikey` for an API key, `personalAccessToken`, `agentIdentity`, …).
///
/// `codex logout` deletes the file: no file is no login. Neither is a login
/// Codex keeps in the system keyring (`cli_auth_credentials_store`, a file by
/// default): the entry holds the tokens, and is not read.
pub fn read_codex_login(auth: &Path) -> Login {
    let Some(file) = read::<CodexAuth>(auth) else {
        return Login::default();
    };
    let mode = file.mode().to_string();
    let (logged_in, email, plan) = match mode.as_str() {
        "chatgpt" | "chatgptAuthTokens" => {
            let tokens = file.tokens;
            let logged_in = tokens.as_ref().is_some_and(|tokens| {
                tokens.access_token.is_some() || tokens.refresh_token.is_some()
            });
            let claims = tokens.and_then(|tokens| tokens.id_token);
            let (email, plan) = match claims {
                Some(claims) => (
                    cleaned(claims.email).or_else(|| cleaned(claims.profile?.email)),
                    cleaned(claims.auth.and_then(|auth| auth.chatgpt_plan_type)),
                ),
                None => (None, None),
            };
            (logged_in, email, plan)
        }
        "agentIdentity" => {
            let named = Some("agentIdentity".to_string());
            match file.agent_identity {
                Some(AgentIdentity::Record { email, plan_type }) => {
                    (true, cleaned(email), cleaned(plan_type).or(named))
                }
                Some(AgentIdentity::Jwt(IgnoredAny)) => (true, None, named),
                None => (false, None, None),
            }
        }
        other => {
            let held = match other {
                "apikey" => file.api_key.is_some(),
                "personalAccessToken" => file.personal_access_token.is_some(),
                "bedrockApiKey" => file.bedrock_api_key.is_some(),
                "bedrockAccessKeys" => file.bedrock_access_keys.is_some(),
                // `headers` and modes to come keep nothing here.
                _ => false,
            };
            (held, None, Some(other.to_string()))
        }
    };
    Login {
        logged_in,
        email: if logged_in { email } else { None },
        plan: if logged_in { plan } else { None },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::testing::Scratch;
    use std::path::PathBuf;

    fn home(scratch: &Scratch, id: &str) -> ClaudeHome {
        ClaudeHome::profile(scratch.dir(&format!("accounts/{id}")))
    }

    #[test]
    fn a_login_is_read_from_the_profiles_own_files() {
        let scratch = Scratch::new("login");
        let work = home(&scratch, "work");
        assert_eq!(read_login(&work, false), Login::default());

        scratch.file(
            "accounts/work/.claude.json",
            r#"{"oauthAccount":{"emailAddress":" work@example.com ","accountUuid":"u"},"projects":{}}"#,
        );
        // An account without tokens is not logged in, whatever it remembers.
        assert_eq!(read_login(&work, false), Login::default());

        scratch.file(
            "accounts/work/.credentials.json",
            r#"{"claudeAiOauth":{"accessToken":"secret","refreshToken":"secret","subscriptionType":"max"}}"#,
        );
        assert_eq!(
            read_login(&work, false),
            Login {
                logged_in: true,
                email: Some("work@example.com".into()),
                plan: Some("max".into()),
            }
        );

        // Logged out: Claude Code empties the file.
        scratch.file("accounts/work/.credentials.json", "{}");
        assert!(!read_login(&work, false).logged_in);
        // Broken files are no login, and no panic.
        scratch.file("accounts/work/.credentials.json", "{ broken");
        assert!(!read_login(&work, false).logged_in);
    }

    #[test]
    fn with_the_keychain_the_account_in_the_config_is_the_login() {
        let scratch = Scratch::new("login-keychain");
        let work = home(&scratch, "work");
        assert!(!read_login(&work, true).logged_in);
        scratch.file(
            "accounts/work/.claude.json",
            r#"{"oauthAccount":{"accountUuid":"u","emailAddress":"w@example.com"}}"#,
        );
        let login = read_login(&work, true);
        assert!(login.logged_in);
        assert_eq!(login.email.as_deref(), Some("w@example.com"));
        assert_eq!(login.plan, None);
    }

    #[test]
    fn the_keychain_entry_is_named_after_the_folder() {
        // sha256("/Users/me/.hangar/accounts/work") starts with these digits.
        let dir = PathBuf::from("/Users/me/.hangar/accounts/work");
        let digest = Sha256::digest(b"/Users/me/.hangar/accounts/work");
        let expected = format!(
            "Claude Code-credentials-{:02x}{:02x}{:02x}{:02x}",
            digest[0], digest[1], digest[2], digest[3]
        );
        assert_eq!(keychain_service(&dir), expected);
        assert_eq!(
            keychain_service(&dir).len(),
            "Claude Code-credentials-".len() + 8
        );
        assert_ne!(
            keychain_service(&dir),
            keychain_service(Path::new("/Users/me/.hangar/accounts/perso"))
        );
    }

    #[test]
    fn a_logged_in_profile_without_its_own_entry_means_logins_are_shared() {
        let scratch = Scratch::new("login-isolated");
        let work = home(&scratch, "work");
        let perso = home(&scratch, "perso");
        scratch.file(
            "accounts/work/.claude.json",
            r#"{"oauthAccount":{"accountUuid":"u"}}"#,
        );
        let profiles = [work.clone(), perso];
        let work_entry = keychain_service(&work.dir);

        assert!(logins_isolated(&profiles, |service| Some(
            service == work_entry
        )));
        assert!(!logins_isolated(&profiles, |_| Some(false)));
        // Unknown is not proof of anything.
        assert!(logins_isolated(&profiles, |_| None));
        // Nobody logged in yet: nothing to judge.
        assert!(logins_isolated(&profiles[1..], |_| Some(false)));
    }

    // Codex ------------------------------------------------------------------

    /// A Codex home holding one of the `auth.json` files written for these
    /// tests (fixtures/codex/, each saying what it was written from).
    fn codex_auth(scratch: &Scratch, fixture: &str) -> std::path::PathBuf {
        let body = match fixture {
            "chatgpt" => include_str!("fixtures/codex/chatgpt.json"),
            "chatgpt-legacy" => include_str!("fixtures/codex/chatgpt-legacy.json"),
            "api-key" => include_str!("fixtures/codex/api-key.json"),
            other => panic!("no fixture {other}"),
        };
        scratch.file("accounts/codex/team/auth.json", body)
    }

    #[test]
    fn a_chatgpt_login_is_its_id_tokens_account_and_plan() {
        let scratch = Scratch::new("codex-login");
        let auth = codex_auth(&scratch, "chatgpt");
        assert_eq!(
            read_codex_login(&auth),
            Login {
                logged_in: true,
                email: Some("team@example.com".into()),
                plan: Some("team".into()),
            }
        );
        // An older Codex: no mode named, the e-mail in the profile claim.
        let auth = codex_auth(&scratch, "chatgpt-legacy");
        assert_eq!(
            read_codex_login(&auth),
            Login {
                logged_in: true,
                email: Some("plus@example.com".into()),
                plan: Some("plus".into()),
            }
        );
    }

    #[test]
    fn an_api_key_login_has_no_account_and_says_so() {
        let scratch = Scratch::new("codex-login-key");
        let auth = codex_auth(&scratch, "api-key");
        assert_eq!(
            read_codex_login(&auth),
            Login {
                logged_in: true,
                email: None,
                plan: Some("apikey".into()),
            }
        );
        // The same without the mode, as an older Codex wrote it.
        scratch.file(
            "accounts/codex/team/auth.json",
            r#"{"OPENAI_API_KEY":"sk-x"}"#,
        );
        assert_eq!(read_codex_login(&auth).plan.as_deref(), Some("apikey"));
    }

    #[test]
    fn no_file_a_broken_one_or_no_credential_is_no_codex_login() {
        let scratch = Scratch::new("codex-login-none");
        let auth = scratch.root.join("accounts/codex/team/auth.json");
        assert_eq!(read_codex_login(&auth), Login::default());
        for body in [
            "{ broken",
            r#"{"OPENAI_API_KEY":null}"#,
            r#"{"auth_mode":"chatgpt","OPENAI_API_KEY":null,"tokens":null}"#,
            r#"{"auth_mode":"apikey","OPENAI_API_KEY":null}"#,
            r#"{"auth_mode":"headers"}"#,
        ] {
            scratch.file("accounts/codex/team/auth.json", body);
            assert_eq!(read_codex_login(&auth), Login::default(), "{body}");
        }
        // Tokens whose ID token does not decode: logged in, account unknown.
        scratch.file(
            "accounts/codex/team/auth.json",
            r#"{"tokens":{"id_token":"not-a-jwt","access_token":"a","refresh_token":"r"}}"#,
        );
        assert_eq!(
            read_codex_login(&auth),
            Login {
                logged_in: true,
                email: None,
                plan: None,
            }
        );
    }

    #[test]
    fn an_agent_identity_names_its_account_when_it_is_a_record() {
        let scratch = Scratch::new("codex-login-agent");
        let auth = scratch.file(
            "accounts/codex/team/auth.json",
            r#"{"auth_mode":"agentIdentity","agent_identity":{"agent_runtime_id":"r","agent_private_key":"k","account_id":"a","chatgpt_user_id":"u","email":"bot@example.com","plan_type":"business","chatgpt_account_is_fedramp":false}}"#,
        );
        let login = read_codex_login(&auth);
        assert!(login.logged_in);
        assert_eq!(login.email.as_deref(), Some("bot@example.com"));
        assert_eq!(login.plan.as_deref(), Some("business"));
        scratch.file(
            "accounts/codex/team/auth.json",
            r#"{"auth_mode":"agentIdentity","agent_identity":"a.b.c"}"#,
        );
        assert_eq!(
            read_codex_login(&auth).plan.as_deref(),
            Some("agentIdentity")
        );
    }

    /// Nothing secret comes out of a login, whatever shows it.
    #[test]
    fn no_codex_token_ever_leaves_the_file() {
        let scratch = Scratch::new("codex-login-secret");
        for fixture in ["chatgpt", "chatgpt-legacy", "api-key"] {
            let auth = codex_auth(&scratch, fixture);
            let shown = format!("{:?}", read_codex_login(&auth));
            for secret in [
                "placeholder",
                "eyJ",
                "not-signed",
                "bm90LXNpZ25lZA",
                "acct-fixture",
                "user-fixture",
            ] {
                assert!(!shown.contains(secret), "{fixture}: {shown}");
            }
        }
    }
}
