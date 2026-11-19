//! NixOS, where Claude Code comes from nixpkgs.
//!
//! Anthropic's native installer drops a `claude` linked for the loader of a
//! usual Linux (`/lib64/ld-linux-x86-64.so.2`), which NixOS does not have: it
//! does not start there without nix-ld. So on NixOS Claude Code's card shows
//! the Nix way, and keeps the installer behind « Use Anthropic's installer
//! anyway » for the systems that have nix-ld.
//!
//! Codex's installer is not in that case: on Linux it downloads the
//! `x86_64-unknown-linux-musl` or `aarch64-unknown-linux-musl` build, a static
//! binary that needs no loader (scripts/install/install.sh of openai/codex,
//! rust-v0.170.0), so it starts on NixOS like anywhere else and its card is
//! the installer's there too.
//!
//! nixpkgs marks `claude-code` unfree (`meta.license = lib.licenses.unfree`,
//! pkgs/by-name/cl/claude-code/package.nix): a configuration allows it with
//! `nixpkgs.config.allowUnfree = true`, and a flake reference, whose
//! evaluation is pure, only with `NIXPKGS_ALLOW_UNFREE=1` and `--impure`.

use std::path::Path;

use hangar_core::agent::AgentKind;

/// The install for good: a line of the NixOS configuration (`home.packages`
/// with Home Manager), then a rebuild.
pub const SYSTEM_PACKAGES: &str = "environment.systemPackages = [ pkgs.claude-code ];";

/// The install right away, in the user's profile, the unfree package allowed
/// for this one command.
pub const PROFILE_INSTALL: &str =
    "NIXPKGS_ALLOW_UNFREE=1 nix profile install --impure nixpkgs#claude-code";

/// Whether `kind` comes from nixpkgs on NixOS: the agent whose installer's
/// program does not start there, Claude Code.
pub fn from_nixpkgs(kind: AgentKind) -> bool {
    matches!(kind, AgentKind::Claude)
}

/// Whether this machine runs NixOS. Linux only: on a Mac with Nix, the native
/// installer's `claude` starts like any other program.
pub fn here() -> bool {
    cfg!(target_os = "linux") && is_nixos(Path::new("/"))
}

/// Whether the system whose `/` is `root` is NixOS: its marker file
/// `/etc/NIXOS`, or `ID=nixos` in `/etc/os-release`.
pub fn is_nixos(root: &Path) -> bool {
    root.join("etc/NIXOS").exists()
        || std::fs::read_to_string(root.join("etc/os-release"))
            .is_ok_and(|text| os_release_id(&text) == Some("nixos"))
}

/// The `ID` an os-release file gives, without the quotes it may be written
/// in (`ID=nixos`, `ID="nixos"`).
pub(super) fn os_release_id(text: &str) -> Option<&str> {
    text.lines().find_map(|line| {
        let value = line.trim().strip_prefix("ID=")?.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
            .unwrap_or(value);
        Some(value)
    })
}
