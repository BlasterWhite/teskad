//! The protocol of the optional Hangar account, in types.
//!
//! docs/native/cloud.md describes it in words: who calls what, when, and what
//! leaves the PC. The server (`hangar-cloud`) and the desktop (hangar-core's
//! `cloud` module) both build their requests and answers from this crate, so
//! a field renamed on one side does not compile on the other.
//!
//! Nothing here denies unknown fields: inside one `API_VERSION` the protocol
//! only grows, and an older desktop must keep reading a newer server's
//! answers (and the reverse).

mod error;
mod limits;
pub mod paths;
mod stats;

pub use error::{ApiError, ErrorCode};
pub use limits::*;
pub use stats::{
    DeviceStatsRow, PutStatsRequest, PutStatsResponse, StatsQuery, StatsResponse, StatsRow,
    StatsRowError, is_day,
};

use serde::{Deserialize, Serialize};

/// The version of the protocol, which is the `/v1` every path starts with.
/// It changes only when an existing request or answer stops meaning what it
/// meant; a new field or a new route does not change it.
pub const API_VERSION: u32 = 1;

/// Where the account server lives, unless `HANGAR_CLOUD_URL` says otherwise.
pub const DEFAULT_BASE_URL: &str = "https://account.hangar.simnjs.fr";

/// The environment variable the desktop reads to reach another server (a
/// local one, while developing).
pub const BASE_URL_ENV: &str = "HANGAR_CLOUD_URL";

/// What a device token starts with, so that one found in a log or a paste is
/// recognised for what it is.
pub const TOKEN_PREFIX: &str = "hgr_";

/// Who vouches for the person at sign-in: GitHub, the one provider there is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Github,
}

impl Provider {
    pub const ALL: [Provider; 1] = [Provider::Github];

    /// The name the protocol uses, in `?provider=` and in `providers`.
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Github => "github",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.as_str() == name)
    }
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The query of `GET /v1/auth/start`, the page the desktop opens in the
/// system browser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthStartQuery {
    pub provider: Provider,
    /// The loopback port the desktop listens on: the browser comes back to
    /// `http://127.0.0.1:<port>/callback`, and nowhere else.
    pub port: u16,
    /// The desktop's own state, returned untouched with the link code.
    pub state: String,
    /// `base64url(sha256(verifier))`, without padding (PKCE, S256).
    pub challenge: String,
    /// The machine's name, shown on the server's pages so the person sees
    /// what they are linking. The one that is recorded comes with the
    /// exchange.
    pub machine: String,
}

/// The query the browser brings back to the desktop's loopback listener,
/// `GET http://127.0.0.1:<port>/callback`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopbackCallback {
    /// The one-time link code, when sign-in went through.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Why it did not, otherwise: `access_denied` when the person said no at
    /// the provider, `provider_error` for anything else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The desktop's state, as it sent it.
    pub state: String,
}

/// The machine the desktop runs on, as it presents itself at the exchange.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineInfo {
    /// An id the desktop draws once and keeps: signing in again from the same
    /// machine finds its device, and its statistics, again.
    pub id: String,
    pub name: String,
    /// `windows`, `macos` or `linux`.
    pub os: String,
}

/// The body of `POST /v1/auth/exchange`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExchangeRequest {
    pub code: String,
    /// The PKCE verifier whose S256 was the `challenge` of the start.
    pub verifier: String,
    pub machine: MachineInfo,
}

/// The answer to the exchange: the only time a device token is ever sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExchangeResponse {
    pub token: String,
    pub account: Account,
    pub device: Device,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// The providers this account can sign in with.
    pub providers: Vec<Provider>,
}

/// One machine linked to the account. Times are milliseconds since the Unix
/// epoch, as everywhere in Hangar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    /// The server's id for the device, the one in `/v1/devices/{id}`.
    pub id: String,
    pub name: String,
    pub os: String,
    pub created_at: i64,
    pub last_seen_at: i64,
    /// When the device was signed out, if it was: its token is gone, its
    /// statistics are still the account's until the device is forgotten.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<i64>,
}

/// The answer to `GET /v1/me`: the account, and the device asking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeResponse {
    pub account: Account,
    pub device: Device,
}

/// The answer to `GET /v1/devices`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevicesResponse {
    pub devices: Vec<Device>,
}

/// The body of `PATCH /v1/devices/{id}`; the answer is the `Device`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenameDeviceRequest {
    pub name: String,
}

/// The query of `DELETE /v1/devices/{id}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteDeviceQuery {
    /// Without it the device is signed out and its statistics stay. With it
    /// the device and every row it sent are deleted.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub forget: bool,
}

/// The answer to `GET /health`, the one route outside `/v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Health {
    /// Always `ok`: a server that cannot answer does not answer this either.
    pub status: String,
    pub api_version: u32,
    /// The server's own version, the workspace's.
    pub version: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::DeserializeOwned;
    use serde_json::{Value, json};

    /// The value serialises to exactly `wire`, and `wire` reads back as the
    /// value: both directions, so a renamed field fails here.
    pub(crate) fn round_trip<T>(value: &T, wire: Value)
    where
        T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
    {
        assert_eq!(serde_json::to_value(value).unwrap(), wire);
        assert_eq!(&serde_json::from_value::<T>(wire).unwrap(), value);
    }

    fn device() -> Device {
        Device {
            id: "dev_1".into(),
            name: "Atelier".into(),
            os: "windows".into(),
            created_at: 1_794_000_000_000,
            last_seen_at: 1_794_000_600_000,
            revoked_at: None,
        }
    }

    fn account() -> Account {
        Account {
            id: "acc_1".into(),
            display_name: "Ada".into(),
            email: Some("ada@example.org".into()),
            avatar_url: None,
            providers: vec![Provider::Github],
        }
    }

    #[test]
    fn providers_are_their_lowercase_names() {
        for provider in Provider::ALL {
            round_trip(&provider, json!(provider.as_str()));
            assert_eq!(Provider::parse(provider.as_str()), Some(provider));
            assert_eq!(provider.to_string(), provider.as_str());
        }
        assert_eq!(Provider::parse("gitlab"), None);
        assert!(serde_json::from_value::<Provider>(json!("GitHub")).is_err());
    }

    #[test]
    fn the_start_query_round_trips() {
        round_trip(
            &AuthStartQuery {
                provider: Provider::Github,
                port: 49152,
                state: "s".into(),
                challenge: "c".into(),
                machine: "Atelier".into(),
            },
            json!({
                "provider": "github",
                "port": 49152,
                "state": "s",
                "challenge": "c",
                "machine": "Atelier",
            }),
        );
    }

    #[test]
    fn the_loopback_callback_carries_a_code_or_an_error() {
        round_trip(
            &LoopbackCallback {
                code: Some("abc".into()),
                error: None,
                state: "s".into(),
            },
            json!({ "code": "abc", "state": "s" }),
        );
        round_trip(
            &LoopbackCallback {
                code: None,
                error: Some("access_denied".into()),
                state: "s".into(),
            },
            json!({ "error": "access_denied", "state": "s" }),
        );
    }

    #[test]
    fn the_exchange_round_trips() {
        round_trip(
            &ExchangeRequest {
                code: "abc".into(),
                verifier: "v".into(),
                machine: MachineInfo {
                    id: "m1".into(),
                    name: "Atelier".into(),
                    os: "windows".into(),
                },
            },
            json!({
                "code": "abc",
                "verifier": "v",
                "machine": { "id": "m1", "name": "Atelier", "os": "windows" },
            }),
        );
        round_trip(
            &ExchangeResponse {
                token: "hgr_x".into(),
                account: account(),
                device: device(),
            },
            json!({
                "token": "hgr_x",
                "account": {
                    "id": "acc_1",
                    "display_name": "Ada",
                    "email": "ada@example.org",
                    "providers": ["github"],
                },
                "device": {
                    "id": "dev_1",
                    "name": "Atelier",
                    "os": "windows",
                    "created_at": 1_794_000_000_000_i64,
                    "last_seen_at": 1_794_000_600_000_i64,
                },
            }),
        );
    }

    #[test]
    fn an_account_without_email_or_avatar_omits_them_and_reads_without_them() {
        let bare = Account {
            email: None,
            ..account()
        };
        round_trip(
            &bare,
            json!({ "id": "acc_1", "display_name": "Ada", "providers": ["github"] }),
        );
        // A server that writes them as null is read the same way.
        let with_nulls = json!({
            "id": "acc_1",
            "display_name": "Ada",
            "email": null,
            "avatar_url": null,
            "providers": ["github"],
        });
        assert_eq!(serde_json::from_value::<Account>(with_nulls).unwrap(), bare);
    }

    #[test]
    fn me_and_devices_round_trip() {
        let wire_device = json!({
            "id": "dev_1",
            "name": "Atelier",
            "os": "windows",
            "created_at": 1_794_000_000_000_i64,
            "last_seen_at": 1_794_000_600_000_i64,
        });
        round_trip(
            &MeResponse {
                account: Account {
                    email: None,
                    ..account()
                },
                device: device(),
            },
            json!({
                "account": { "id": "acc_1", "display_name": "Ada", "providers": ["github"] },
                "device": wire_device,
            }),
        );
        let revoked = Device {
            revoked_at: Some(1_794_000_700_000),
            ..device()
        };
        let mut wire_revoked = wire_device.clone();
        wire_revoked["revoked_at"] = json!(1_794_000_700_000_i64);
        round_trip(
            &DevicesResponse {
                devices: vec![device(), revoked],
            },
            json!({ "devices": [wire_device, wire_revoked] }),
        );
        round_trip(
            &RenameDeviceRequest {
                name: "Portable".into(),
            },
            json!({ "name": "Portable" }),
        );
    }

    #[test]
    fn deleting_a_device_keeps_its_statistics_unless_asked_to_forget() {
        round_trip(&DeleteDeviceQuery { forget: false }, json!({}));
        round_trip(
            &DeleteDeviceQuery { forget: true },
            json!({ "forget": true }),
        );
    }

    #[test]
    fn health_round_trips() {
        round_trip(
            &Health {
                status: "ok".into(),
                api_version: API_VERSION,
                version: "2.3.0".into(),
            },
            json!({ "status": "ok", "api_version": 1, "version": "2.3.0" }),
        );
    }

    #[test]
    fn a_field_this_version_does_not_know_is_ignored() {
        let newer = json!({
            "id": "dev_1",
            "name": "Atelier",
            "os": "windows",
            "created_at": 1_794_000_000_000_i64,
            "last_seen_at": 1_794_000_600_000_i64,
            "added_later": { "anything": true },
        });
        assert_eq!(serde_json::from_value::<Device>(newer).unwrap(), device());
    }

    #[test]
    fn every_versioned_path_starts_with_the_version() {
        let prefix = format!("/v{API_VERSION}/");
        for path in [
            paths::AUTH_START,
            paths::AUTH_CALLBACK,
            paths::AUTH_EXCHANGE,
            paths::ME,
            paths::DEVICES,
            paths::DEVICE,
            paths::STATS,
            paths::ACCOUNT,
        ] {
            assert!(path.starts_with(&prefix), "{path}");
        }
        assert_eq!(paths::device("dev_1"), "/v1/devices/dev_1");
        assert_eq!(
            paths::auth_callback(Provider::Github),
            "/v1/auth/callback/github"
        );
        assert_eq!(
            paths::DEVICE.replace("{id}", "dev_1"),
            paths::device("dev_1")
        );
    }
}
