//! The optional Hangar account ("Compte Hangar"), seen from the desktop:
//! signing in with GitHub or Google, the machines linked to the account, and
//! the usage statistics they share (docs/native/cloud.md).
//!
//! Not `accounts`: that module is the Claude accounts a pane runs under, and
//! has nothing to do with this one.
//!
//! All of it is opt-in. A Hangar nobody signed in with never calls the account
//! server, and nothing else in the app waits on it or behaves differently
//! without it. Signed in, statistics are sent only while their sync is on,
//! and what is sent is totals per day, model and project name: never a path,
//! never a line of a transcript. The types say so (`StatsRow` has nowhere to
//! put either), and so does the server, which refuses a project that looks
//! like a path.
//!
//! The protocol's types are hangar-cloud-api's, the crate the server builds
//! its answers from; they are re-exported here so the rest of the app has one
//! place to name them. The device token lives in the operating system's
//! keychain, as the Groq key does (voice/key.rs), and nowhere else.
//!
//! This is the skeleton: where the server is. Sign-in, the devices and the
//! sync each come as a module of this folder.

pub use hangar_cloud_api as api;
pub use hangar_cloud_api::{
    Account, ApiError, Device, DeviceStatsRow, DevicesResponse, ErrorCode, ExchangeRequest,
    ExchangeResponse, MachineInfo, MeResponse, Provider, StatsResponse, StatsRow, API_VERSION,
};

/// The account server's address, without a trailing slash: the public one,
/// unless `HANGAR_CLOUD_URL` names another (a local server, while developing).
pub fn base_url() -> String {
    resolve_base_url(std::env::var(api::BASE_URL_ENV).ok())
}

fn resolve_base_url(from_env: Option<String>) -> String {
    from_env
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .unwrap_or(api::DEFAULT_BASE_URL)
        .trim_end_matches('/')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_server_is_the_public_one_unless_the_environment_names_another() {
        assert_eq!(resolve_base_url(None), "https://account.hangar.simnjs.fr");
        assert_eq!(
            resolve_base_url(Some("  ".into())),
            "https://account.hangar.simnjs.fr"
        );
        // Paths are appended as they are, so the slash goes.
        assert_eq!(
            resolve_base_url(Some("http://127.0.0.1:21120/".into())),
            "http://127.0.0.1:21120"
        );
    }
}
