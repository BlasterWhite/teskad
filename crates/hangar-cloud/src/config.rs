//! The server's configuration, read once from the environment at start
//! (docs/native/cloud.md, "Running the server", lists the variables).
//!
//! A variable that is set and wrong stops the server with a sentence saying
//! which: a deployment that starts on a default it did not ask for is harder
//! to notice than one that does not start.

use std::net::SocketAddr;
use std::path::PathBuf;

use hangar_cloud_api::{DEFAULT_BASE_URL, Provider};

pub const BIND: &str = "HANGAR_CLOUD_BIND";
pub const DB: &str = "HANGAR_CLOUD_DB";
pub const BASE_URL: &str = "HANGAR_CLOUD_BASE_URL";

/// Inside the image: every interface, the port the Dockerfile exposes.
const DEFAULT_BIND: &str = "0.0.0.0:8080";
/// Inside the image's volume.
const DEFAULT_DB: &str = "/data/hangar-cloud.sqlite";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub bind: SocketAddr,
    /// The SQLite file.
    pub db: PathBuf,
    /// The server's public address, without a trailing slash: what the OAuth
    /// redirect URIs are built from, since behind the proxy the server cannot
    /// know it.
    pub base_url: String,
    pub github: Option<OAuthClient>,
    pub google: Option<OAuthClient>,
}

/// The OAuth application registered with a provider.
#[derive(Clone, PartialEq, Eq)]
pub struct OAuthClient {
    pub client_id: String,
    // Read by the sign-in routes, which this skeleton does not have yet.
    #[allow(dead_code)]
    pub client_secret: String,
}

/// By hand, so that a configuration printed in a log or a panic never carries
/// the secret.
impl std::fmt::Debug for OAuthClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthClient")
            .field("client_id", &self.client_id)
            .field("client_secret", &"…")
            .finish()
    }
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// `lookup` is the environment; the tests hand in their own.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        // Set and empty is unset: what `docker run -e NAME=` and an empty
        // line of an env file give.
        let var = |name: &str| {
            lookup(name)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };

        let bind = var(BIND).unwrap_or_else(|| DEFAULT_BIND.into());
        let bind = bind
            .parse()
            .map_err(|_| format!("{BIND} is not an address and a port (\"{bind}\")"))?;

        let db = PathBuf::from(var(DB).unwrap_or_else(|| DEFAULT_DB.into()));

        let base_url = var(BASE_URL).unwrap_or_else(|| DEFAULT_BASE_URL.into());
        let base_url = base_url.trim_end_matches('/').to_string();
        let host = base_url
            .strip_prefix("https://")
            .or_else(|| base_url.strip_prefix("http://"));
        if !host.is_some_and(|host| !host.is_empty() && !host.contains(['/', '?', '#'])) {
            return Err(format!(
                "{BASE_URL} must be the server's public origin, like {DEFAULT_BASE_URL} \
                 (\"{base_url}\")"
            ));
        }

        let client = |provider: Provider| {
            let id_name = format!("{}_CLIENT_ID", provider.as_str().to_uppercase());
            let secret_name = format!("{}_CLIENT_SECRET", provider.as_str().to_uppercase());
            match (var(&id_name), var(&secret_name)) {
                (Some(client_id), Some(client_secret)) => Ok(Some(OAuthClient {
                    client_id,
                    client_secret,
                })),
                (None, None) => Ok(None),
                // Half a client is a mistake, not a provider turned off.
                (Some(_), None) => Err(format!("{id_name} is set without {secret_name}")),
                (None, Some(_)) => Err(format!("{secret_name} is set without {id_name}")),
            }
        };

        Ok(Self {
            bind,
            db,
            base_url,
            github: client(Provider::Github)?,
            google: client(Provider::Google)?,
        })
    }

    /// The provider's OAuth application, when this server has one.
    pub fn provider(&self, provider: Provider) -> Option<&OAuthClient> {
        match provider {
            Provider::Github => self.github.as_ref(),
            Provider::Google => self.google.as_ref(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(vars: &[(&str, &str)]) -> Result<Config, String> {
        Config::from_lookup(|name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        })
    }

    #[test]
    fn an_empty_environment_is_the_image_s_defaults_with_no_provider() {
        let config = config(&[]).unwrap();
        assert_eq!(config.bind, "0.0.0.0:8080".parse().unwrap());
        assert_eq!(config.db, PathBuf::from("/data/hangar-cloud.sqlite"));
        assert_eq!(config.base_url, "https://account.hangar.simnjs.fr");
        assert_eq!(config.provider(Provider::Github), None);
        assert_eq!(config.provider(Provider::Google), None);
    }

    #[test]
    fn every_variable_is_read() {
        let config = config(&[
            (BIND, "127.0.0.1:21120"),
            (DB, "/tmp/test.sqlite"),
            (BASE_URL, "http://localhost:21120/"),
            ("GITHUB_CLIENT_ID", "gh-id"),
            ("GITHUB_CLIENT_SECRET", "gh-secret"),
            ("GOOGLE_CLIENT_ID", "g-id"),
            ("GOOGLE_CLIENT_SECRET", "g-secret"),
        ])
        .unwrap();
        assert_eq!(config.bind, "127.0.0.1:21120".parse().unwrap());
        assert_eq!(config.db, PathBuf::from("/tmp/test.sqlite"));
        // Without the slash: paths are appended to it as they are.
        assert_eq!(config.base_url, "http://localhost:21120");
        let github = config.provider(Provider::Github).unwrap();
        assert_eq!(github.client_id, "gh-id");
        assert_eq!(github.client_secret, "gh-secret");
        let google = config.provider(Provider::Google).unwrap();
        assert_eq!(google.client_id, "g-id");
        assert_eq!(google.client_secret, "g-secret");
    }

    #[test]
    fn a_variable_set_to_nothing_is_unset() {
        let config = config(&[(BIND, ""), (DB, "  "), ("GITHUB_CLIENT_ID", "")]).unwrap();
        assert_eq!(config.bind, "0.0.0.0:8080".parse().unwrap());
        assert_eq!(config.db, PathBuf::from("/data/hangar-cloud.sqlite"));
        assert_eq!(config.github, None);
    }

    #[test]
    fn a_wrong_value_stops_the_server_and_names_its_variable() {
        let err = config(&[(BIND, "8080")]).unwrap_err();
        assert!(err.contains(BIND), "{err}");
        for url in [
            "account.hangar.simnjs.fr",
            "ftp://x",
            "https://",
            "https://x/api",
        ] {
            let err = config(&[(BASE_URL, url)]).unwrap_err();
            assert!(err.contains(BASE_URL), "{url}: {err}");
        }
    }

    #[test]
    fn half_an_oauth_client_is_refused() {
        let err = config(&[("GITHUB_CLIENT_ID", "id")]).unwrap_err();
        assert!(err.contains("GITHUB_CLIENT_SECRET"), "{err}");
        let err = config(&[("GOOGLE_CLIENT_SECRET", "secret")]).unwrap_err();
        assert!(err.contains("GOOGLE_CLIENT_ID"), "{err}");
    }

    #[test]
    fn the_secret_is_not_in_what_debug_prints() {
        let config = config(&[
            ("GITHUB_CLIENT_ID", "gh-id"),
            ("GITHUB_CLIENT_SECRET", "gh-secret"),
        ])
        .unwrap();
        let printed = format!("{config:?}");
        assert!(printed.contains("gh-id"), "{printed}");
        assert!(!printed.contains("gh-secret"), "{printed}");
    }
}
