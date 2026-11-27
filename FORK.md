# This fork

[BlasterWhite/teskad](https://github.com/BlasterWhite/teskad) is an independent fork of
HangarAI, not affiliated with it (the [README](README.md) opens by saying so). It is based
on the last open-source release, **2.5.1** (MIT, © simnJS), and builds on the earlier fork
[OLIVIER-CG/hangar](https://github.com/OLIVIER-CG/hangar). Codex support is the original
project's own, from 2.5.0.

The original project is closed source and paid from 3.0, so there is nothing more to merge from
upstream: this fork goes its own way, on `main`.

## What differs from upstream 2.5.1

| Change | Where | Why |
|---|---|---|
| Updates from this fork's releases only | `crates/hangar-update/src/lib.rs` (`ENDPOINT`, `PUBKEY`), `platform.rs` (`SELF_UPDATE`) | The app updates itself from the GitHub Releases of `BlasterWhite/teskad`, signed with this fork's own key; it never reads upstream's feed and refuses anything signed by another key. A build of your own (from `target/`) does not update itself: pull and build again. |
| No cloud account | Settings, statistics; the account server's crates are gone | This fork runs no hosted service. |
| No website | the website and its build are gone, as are the rented build machines | Everything builds on your own machine or on GitHub Actions. |
| No link to upstream's services | title bar, Discord Rich Presence, Web Push contact | The app does not send people to a project it is not part of. Rich Presence needs a Discord application ID of your own (Settings › Discord). |

## Your data

Teskad has an identity of its own, shared with nothing of the official app's: the bundle
identifier is `io.github.blasterwhite.teskad` (the official one is `com.simnjs.iabench`), the
publisher is BlasterWhite, the program is `teskad` (not `hangar-ai`) and its home folder is
`~/.teskad` (not `~/.hangar`). It installs **beside** the official HangarAI, and both can run at
the same time.

| What | Teskad | Official HangarAI (never written by Teskad) |
|---|---|---|
| App data: `state.json` (workspaces, panes, settings), `memory.json`, `activity/`, `mcp/`, the statistics cache, `remote.json` | Linux `~/.local/share/io.github.blasterwhite.teskad/`, macOS `~/Library/Application Support/io.github.blasterwhite.teskad/`, Windows `%APPDATA%\io.github.blasterwhite.teskad\` | the same folders under `com.simnjs.iabench` |
| Logs | `…/io.github.blasterwhite.teskad/logs` (macOS `~/Library/Logs/io.github.blasterwhite.teskad`, Windows `%LOCALAPPDATA%\io.github.blasterwhite.teskad\logs`) | the same under `com.simnjs.iabench` |
| Home folder: `endpoint.json`, `mods/` (the Claude Code bridge), `accounts/`, `kits/`, `kit-data/`, `codex-plugins/` | `~/.teskad/` | `~/.hangar/` (and `~/.iabench/endpoint.json`, which Teskad no longer writes) |
| Windows install | `%LOCALAPPDATA%\Teskad`, registry `Software\BlasterWhite\Teskad`, uninstall entry « Teskad » | `%LOCALAPPDATA%\Hangar.AI`, `Software\simnjs\Hangar.AI`, « Hangar.AI » |
| Linux install | package `teskad` (`.deb`), `Teskad` (`.rpm`), `teskad.desktop`, `usr/lib/teskad/` | `hangar-ai`, `Hangar.AI`, `hangar-ai.desktop` |
| Login item | `Teskad` (registry `Run` value, `~/.config/autostart/Teskad.desktop`, `~/Library/LaunchAgents/Teskad.plist`) | `Hangar.AI` |
| Notifications (Windows AUMID) | the identifier above | `com.simnjs.iabench` |

Your project folders are shared, as they are the same projects. Teskad keeps its task board and
uploads in a `.teskad/` folder inside each one; the first time it opens a project that only has the
official app's `.hangar/`, it copies it there (the original is left as it is, for the official app). What agents see is Teskad's own: its MCP
server is named `teskad` (tools `mcp__teskad__*`), its Claude Code mod `teskad-bridge`, its
messages to agents start with `[Teskad]`, and the variables it sets are `TESKAD_*`. The
official app's `hangar` entry in a project's `.mcp.json` is never touched: the two servers sit
side by side, each under its own name (see [Going back](#going-back-to-the-official-app)).

A variable you set yourself (`TESKAD_DATA_ROOT`, `TESKAD_WEBVIEW`, the `TESKAD_DEV_*` and test
ones…) is read under its `TESKAD_` name first, and under its old `HANGAR_` name when the new one
is not set, so a script or a profile written for the official app keeps working. The variables
Teskad sets for its own panes and servers have no such fallback: a Teskad started inside a pane
of the official app never takes that pane's for its own.

### First launch: your data is copied, not moved

The first time Teskad starts and its own folder does not exist yet, it **copies** what the
official app left (`crates/hangar-core/src/migrate.rs`):

- from `com.simnjs.iabench` (data, config and local-data folders): `state.json`, `memory.json`,
  `activity/`, `mcp/`, the statistics cache, `remote.json`, the HQ folder and the rest, but not
  `logs/`, the voice `models/` (download them again in Settings) or the `state.json.corrupt.*`
  rescues;
- from `~/.hangar`: `mods/`, `accounts/`, `kits/`, `kit-data/`, `codex-plugins/`, but not
  `endpoint.json`, the official app's port and token.

The official folders are only read: nothing there is moved, changed or deleted. Each folder is
copied beside its final place (`<folder>.importing`) and renamed into it, with a marker file
`.imported-from-official` inside, so a launch interrupted half way leaves nothing to mistake for
your data and the next one starts over. A folder that exists is never written into, and one that
holds the marker is never copied again, also after you cleared something in it. To import again,
quit Teskad and delete the folder (`~/.teskad`, or the app data folder).

Then Teskad rewrites what named the official program: the `hangar` server of the copied
`mcp/*.json` becomes `teskad` and runs Teskad (the copy only), and in the `.mcp.json` of every
workspace in the copied `state.json` (your own project files) an older `iabench` entry is
pointed at Teskad as `teskad`, after the file is saved as `.mcp.json.hangar-bak`. A `hangar`
entry that runs the official app stays as it is. Nothing is imported or rewritten when
`TESKAD_DATA_ROOT` or `TESKAD_ENDPOINT_FILE` is set (a development build, see
[CONTRIBUTING.md](CONTRIBUTING.md)).

At every launch, a project's `.mcp.json` that still holds Teskad's server under the name
`hangar` (written before 2.6.0) gets it renamed `teskad`, and the Claude Code mod Teskad had
installed as `~/.teskad/mods/hangar-bridge` is replaced by `teskad-bridge`. The official app's
own entries and its `~/.hangar/mods` are not touched. Agent sessions already open keep seeing
`mcp__hangar__*`: restart them to get `mcp__teskad__*`.

Three things do not carry over:

- **Sign-ins.** An account's Claude or Codex login is copied with its folder, but Codex keeps
  its token in the system keyring under the folder's path, and a token a provider rotates is
  valid in one copy only. Sign in again from Settings › Accounts if a copied account asks for it.
- **The Groq key for dictation.** It lives in the system keychain under Teskad's own service (the
  bundle identifier), not the official app's: enter it again in Settings › Voice.
- **Codex plugins marketplace.** Codex still knows `~/.hangar/codex-plugins`; run « Add to Codex »
  again from the kit's settings to register `~/.teskad/codex-plugins`.

### Going back to the official app

Nothing was taken from it: install or start HangarAI and it finds its data as it left it
(changes made in Teskad since the import are not in it). Its `.mcp.json` entry, named `hangar`,
was never touched, so it works as before; Teskad's is the `teskad` one beside it. If a project
had Teskad's server moved from `hangar` to `teskad` and you want the old file back, restore
`.mcp.json.hangar-bak`. To remove Teskad, uninstall it as any app
and delete `~/.teskad` and its data folder (above); that leaves the official app untouched.

Run a development build with `scripts/native-dev.sh`: it runs the app on data of its own under
`target/dev-home/` (see [CONTRIBUTING.md](CONTRIBUTING.md)).

## Build on Linux (Fedora)

With Rust stable (and `rustup target add wasm32-unknown-unknown` for native kit tabs):

```bash
sudo dnf install gcc-c++ cmake clang-devel openssl-devel alsa-lib-devel webkit2gtk4.1-devel
pnpm install && pnpm mobile:build      # optional (Node 22, pnpm 10): the phone app Teskad serves
cargo build --profile dist --locked -p hangar-app
cargo build --profile dist --locked -p hangar-webview --features engine
./target/dist/teskad
```

To make an `.rpm` instead, see the [packaging guide](docs/native/PACKAGING.md).

## Build on Windows

From Git Bash, with Rust, Visual Studio's C++ tools, Node and pnpm installed (cmake and
libclang are found inside Visual Studio):

```bash
scripts/fork/build.sh             # target/dist: the release profile (slow, fat LTO)
scripts/fork/build.sh --release   # target/release: much faster to build, for trying changes
```

### Install

Run `scripts/fork/build.sh`, then the installer from the
[packaging guide](docs/native/PACKAGING.md) (`Teskad_<version>_x64-setup.exe`).
It installs to `%LOCALAPPDATA%\Teskad`, beside the official app if it is there: nothing of the
official install is replaced, backed up or restored. Teskad's own data is in the table above.
