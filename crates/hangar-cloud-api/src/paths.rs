//! Every route of the protocol. The ones with a `{…}` segment are written as
//! axum 0.8 routes them, with a function beside them that fills the segment
//! in for the caller.

use crate::Provider;

/// Outside `/v1`: what the reverse proxy and the deployment ask.
pub const HEALTH: &str = "/health";

/// The page the desktop opens in the system browser.
pub const AUTH_START: &str = "/v1/auth/start";
/// Where the provider sends the browser back: the redirect URI registered
/// with GitHub and with Google. Never called by the desktop.
pub const AUTH_CALLBACK: &str = "/v1/auth/callback/{provider}";
pub const AUTH_EXCHANGE: &str = "/v1/auth/exchange";
pub const ME: &str = "/v1/me";
pub const DEVICES: &str = "/v1/devices";
pub const DEVICE: &str = "/v1/devices/{id}";
pub const STATS: &str = "/v1/stats";
pub const ACCOUNT: &str = "/v1/account";

/// The path of the desktop's loopback listener, on `http://127.0.0.1:<port>`.
pub const LOOPBACK_CALLBACK: &str = "/callback";

/// `/v1/devices/<id>`. Device ids are the server's and need no escaping.
pub fn device(id: &str) -> String {
    format!("{DEVICES}/{id}")
}

/// `/v1/auth/callback/<provider>`.
pub fn auth_callback(provider: Provider) -> String {
    format!("/v1/auth/callback/{provider}")
}
