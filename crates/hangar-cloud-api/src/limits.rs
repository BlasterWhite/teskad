//! The protocol's limits, which the server enforces and the desktop respects
//! before it sends anything. Lengths are counted in characters (Unicode
//! scalar values), not bytes: a project named in Japanese gets as many as one
//! named in English.

/// The most rows one `PUT /v1/stats` may carry. The desktop sends more in
/// several calls.
pub const MAX_STATS_ROWS: usize = 5000;

/// The longest project name. The desktop cuts a longer folder name to this.
pub const MAX_PROJECT_CHARS: usize = 100;

/// The longest model id.
pub const MAX_MODEL_CHARS: usize = 100;

/// The longest name of a machine, at sign-in or after a rename.
pub const MAX_DEVICE_NAME_CHARS: usize = 100;

/// The longest machine id and the longest `os`.
pub const MAX_MACHINE_ID_CHARS: usize = 100;

/// The longest state the desktop may send with the start.
pub const MAX_STATE_CHARS: usize = 200;

/// A PKCE verifier's length, as RFC 7636 bounds it.
pub const MIN_VERIFIER_CHARS: usize = 43;
pub const MAX_VERIFIER_CHARS: usize = 128;

/// An S256 challenge is 32 bytes in base64url without padding.
pub const CHALLENGE_CHARS: usize = 43;

/// The loopback ports a sign-in may come back to: never a privileged one.
pub const MIN_LOOPBACK_PORT: u16 = 1024;

/// How long a link code stays valid, counted from the provider's callback
/// that draws it (the sign-in itself may take ten minutes before that).
pub const LINK_CODE_TTL_SECS: u64 = 300;

/// The largest counter a statistics row may carry: what SQLite's signed
/// 64-bit integers hold. No real total comes near it.
pub const MAX_COUNTER: u64 = i64::MAX as u64;

/// The largest request body the server reads, in bytes: room for
/// `MAX_STATS_ROWS` rows at their longest names.
pub const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
