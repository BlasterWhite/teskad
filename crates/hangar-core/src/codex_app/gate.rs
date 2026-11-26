//! Which Codex releases Hangar's client speaks to (docs/native/agents.md §4.6, D12).
//!
//! Codex ships about three releases a week. Hangar's protocol types come from one of them
//! ([`CODEX_PINNED`]); the oldest it attaches to is [`CODEX_ATTACH_MIN`]. Older: refused with the
//! version needed, and the pane runs the plain TUI (D6). Newer than the pin: spoken to, decoding
//! leniently. A release users report broken, or whose schema diff (T12) drops something Hangar
//! needs, goes in [`CODEX_BROKEN`] and is refused like an old one, without waiting for a bump.
//!
//! The version comes from `codex --version` before a server is started (`agent::detect`), and from
//! the server itself at `initialize`, whose `userAgent` starts with it ([`version_from_user_agent`]).

use std::fmt;

use semver::Version;

use super::generated::CODEX_VERSION;

/// The release whose schema the protocol types were generated from (scripts/codex-protocol/).
pub const CODEX_PINNED: &str = CODEX_VERSION;

/// The oldest release Hangar attaches to, and speaks to at all.
///
/// Found as D12 says, by generating the schema of each release from 0.157.0 to the pin
/// (`codex app-server generate-json-schema`, no login) and comparing it with the
/// pinned subset (scripts/codex-protocol/schema.subset.json): from 0.157.0 on, every method of the
/// subset is there, and every field the pinned types require of what Codex sends is there and
/// required. The only differences are in audio inputs, which Hangar never sends (`UserInput`'s
/// `audio` until 0.167.0, a tool output's `input_audio` until 0.169.0). 0.157.0 is also the first
/// release whose TUI takes `--remote` (§4.0), and its pane socket is the same WebSocket over a Unix
/// socket, with the same `userAgent` (`app-server-transport/src/transport/unix_socket.rs`,
/// `login/src/auth/default_client.rs` at rust-v0.157.0): nothing older can be attached anyway.
pub const CODEX_ATTACH_MIN: &str = "0.157.0";

/// Releases inside the range that Hangar refuses: reported broken by users, or whose schema diff
/// removes something Hangar needs. Exact versions; empty until one is.
pub const CODEX_BROKEN: &[&str] = &[];

/// What Hangar makes of a Codex it may speak to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// From [`CODEX_ATTACH_MIN`] to [`CODEX_PINNED`]: the protocol Hangar was built for.
    Supported(Version),
    /// Newer than the pin: what it adds is ignored, what it changed is dropped and reported.
    Newer(Version),
    /// No version could be read (an unknown `userAgent` shape, a source build saying `0.0.0`):
    /// spoken to as a newer one.
    Unknown(String),
}

impl Verdict {
    /// The release, when it was read.
    pub fn version(&self) -> Option<&Version> {
        match self {
            Verdict::Supported(version) | Verdict::Newer(version) => Some(version),
            Verdict::Unknown(_) => None,
        }
    }
}

/// Why Hangar will not speak to a Codex: the pane runs the plain TUI instead, with this as the
/// notice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Older than [`CODEX_ATTACH_MIN`].
    TooOld { found: Version, min: Version },
    /// Listed in [`CODEX_BROKEN`].
    Broken { found: Version },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::TooOld { found, min } => write!(
                f,
                "Codex {found} is too old for Hangar to attach to: it needs Codex {min} or newer \
                 (npm install -g @openai/codex)"
            ),
            Refusal::Broken { found } => write!(
                f,
                "Codex {found} is known not to work with Hangar: update Codex, or use another \
                 release"
            ),
        }
    }
}

impl std::error::Error for Refusal {}

/// The verdict on `version`, as `codex --version` gives it (`codex-cli 0.170.0`) or as a bare
/// release number (`0.170.0`).
pub fn check(version: &str) -> Result<Verdict, Refusal> {
    let text = version.split_whitespace().last().unwrap_or("");
    match Version::parse(text.trim_start_matches('v')) {
        Ok(found) => check_version(found),
        Err(_) => Ok(Verdict::Unknown(version.trim().to_string())),
    }
}

/// The verdict on a release number already read.
pub fn check_version(found: Version) -> Result<Verdict, Refusal> {
    // A build from source says 0.0.0 whatever its age: nothing to compare.
    if found == Version::new(0, 0, 0) {
        return Ok(Verdict::Unknown(found.to_string()));
    }
    if CODEX_BROKEN
        .iter()
        .any(|broken| Version::parse(broken).is_ok_and(|broken| broken == found))
    {
        return Err(Refusal::Broken { found });
    }
    let min = Version::parse(CODEX_ATTACH_MIN).expect("CODEX_ATTACH_MIN is a version");
    let pinned = Version::parse(CODEX_PINNED).expect("CODEX_PINNED is a version");
    if found < min {
        Err(Refusal::TooOld { found, min })
    } else if found > pinned {
        Ok(Verdict::Newer(found))
    } else {
        Ok(Verdict::Supported(found))
    }
}

/// The release of the server that answered `initialize`, from its `userAgent`:
/// `<originator>/<version> (<os> <os version>; <arch>) <terminal> (<client>; <client version>)`
/// (`login/src/auth/default_client.rs`, `get_codex_user_agent`). The originator is the first
/// client's `clientInfo.name` (`codex-tui`, `Hangar`…) and may hold a space (`Codex Desktop`), so
/// the version is what follows the last `/` before the first ` (`.
pub fn version_from_user_agent(user_agent: &str) -> Option<Version> {
    let head = user_agent.split(" (").next()?;
    let (_, version) = head.rsplit_once('/')?;
    Version::parse(version.trim()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn the_range_runs_from_the_attach_minimum_to_the_pin() {
        assert_eq!(CODEX_PINNED, "0.170.0");
        assert_eq!(check("0.157.0"), Ok(Verdict::Supported(v("0.157.0"))));
        assert_eq!(check("0.170.0"), Ok(Verdict::Supported(v("0.170.0"))));
        assert_eq!(
            check("codex-cli 0.163.1"),
            Ok(Verdict::Supported(v("0.163.1")))
        );
        // An alpha of the pinned release comes before it; one of the next comes after.
        assert_eq!(
            check("0.170.0-alpha.4"),
            Ok(Verdict::Supported(v("0.170.0-alpha.4")))
        );
        assert_eq!(
            check("0.171.0-alpha.1"),
            Ok(Verdict::Newer(v("0.171.0-alpha.1")))
        );
        assert_eq!(check("codex-cli 0.182.3"), Ok(Verdict::Newer(v("0.182.3"))));
    }

    #[test]
    fn an_older_codex_is_refused_with_the_version_it_needs() {
        let refusal = check("codex-cli 0.156.1").unwrap_err();
        assert_eq!(
            refusal,
            Refusal::TooOld {
                found: v("0.156.1"),
                min: v("0.157.0")
            }
        );
        let notice = refusal.to_string();
        assert!(notice.contains("0.156.1"), "{notice}");
        assert!(notice.contains("0.157.0 or newer"), "{notice}");
        // The attached mode's first release, as an alpha, is still before it.
        assert!(check("0.157.0-alpha.2").is_err());
    }

    #[test]
    fn an_unreadable_version_is_spoken_to_as_a_newer_one() {
        assert_eq!(
            check("codex-cli dev"),
            Ok(Verdict::Unknown("codex-cli dev".into()))
        );
        assert_eq!(check(""), Ok(Verdict::Unknown(String::new())));
        assert_eq!(check("0.0.0"), Ok(Verdict::Unknown("0.0.0".into())));
    }

    #[test]
    fn the_listed_releases_parse() {
        for broken in CODEX_BROKEN {
            Version::parse(broken).expect("CODEX_BROKEN holds exact versions");
        }
        Version::parse(CODEX_ATTACH_MIN).unwrap();
        assert!(v(CODEX_ATTACH_MIN) <= v(CODEX_PINNED));
    }

    // The shapes `get_codex_user_agent` writes at rust-v0.170.0: originator, build version, OS,
    // terminal, then the latest originating client's name and version.
    #[test]
    fn the_server_version_is_read_from_its_user_agent() {
        let cases = [
            (
                "Hangar/0.170.0 (Ubuntu 24.4.0; x86_64) xterm-256color (Hangar; 2.4.4)",
                Some("0.170.0"),
            ),
            (
                "codex-tui/0.171.0-alpha.3 (Mac OS 15.5.0; arm64) iTerm.app/3.5.14 (codex-tui; 0.171.0-alpha.3)",
                Some("0.171.0-alpha.3"),
            ),
            (
                "Codex Desktop/0.168.0 (Windows 11 Pro 10.0.26100; x86_64) unknown (Codex Desktop; 1.4.0)",
                Some("0.168.0"),
            ),
            ("codex_cli_rs/0.170.0 (Linux; x86_64) dumb", Some("0.170.0")),
            ("codex_cli_rs/0.170.0", Some("0.170.0")),
            ("Hangar", None),
            ("", None),
            ("codex/next (Linux; x86_64)", None),
        ];
        for (user_agent, expected) in cases {
            assert_eq!(
                version_from_user_agent(user_agent),
                expected.map(v),
                "{user_agent}"
            );
        }
    }
}
