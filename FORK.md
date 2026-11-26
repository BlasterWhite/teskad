# This fork

[BlasterWhite/hangar](https://github.com/BlasterWhite/hangar) is an independent fork of
HangarAI, not affiliated with it (the [README](README.md) opens by saying so). It is based
on the last open-source release, **2.5.1** (MIT, © simnJS), and builds on the earlier fork
[OLIVIER-CG/hangar](https://github.com/OLIVIER-CG/hangar). Codex support is the original
project's own, from 2.5.0.

The original project is closed source and paid from 3.0, so there is nothing more to merge from
upstream: this fork goes its own way, on `main`.

## What differs from upstream 2.5.1

| Change | Where | Why |
|---|---|---|
| No upstream self-update | `crates/hangar-update/src/platform.rs` (`SELF_UPDATE`) | The update feed and its signing key are upstream's: an update would replace this build with theirs. For a new version, pull and build again. |
| No cloud account | Settings, statistics; the account server's crates are gone | This fork runs no hosted service. |
| No website | the website and its build are gone, as are the rented build machines | Everything builds on your own machine or on GitHub Actions. |
| No link to upstream's services | title bar, Discord Rich Presence, Web Push contact | The app does not send people to a project it is not part of. Rich Presence needs a Discord application ID of your own (Settings › Discord). |

## Your data

The fork keeps the original's app identifier (`com.simnjs.iabench`), so it uses the same folders as
the original app and needs no migration:

- the app's data folder (`~/.local/share/com.simnjs.iabench/` on Linux,
  `%APPDATA%\com.simnjs.iabench\` on Windows): `state.json` (workspaces, panes, settings),
  the memory, the statistics cache;
- `~/.hangar/`: the endpoint file, the Claude Code bridge mod, kits;
- your project folders, wherever they are.

Run only one Teskad at a time: two would share `state.json` and the endpoint. To try a
change without touching your real data, `scripts/native-dev.sh` runs the app on data of its
own under `target/dev-home/` (see [CONTRIBUTING.md](CONTRIBUTING.md)).

## Build on Linux (Fedora)

With Rust stable (and `rustup target add wasm32-unknown-unknown` for native kit tabs):

```bash
sudo dnf install gcc-c++ cmake clang-devel openssl-devel alsa-lib-devel webkit2gtk4.1-devel
pnpm install && pnpm mobile:build      # optional (Node 22, pnpm 10): the phone app Teskad serves
cargo build --profile dist --locked -p hangar-app
cargo build --profile dist --locked -p hangar-webview --features engine
./target/dist/hangar-ai
```

To make an `.rpm` instead, see the [packaging guide](docs/native/PACKAGING.md).

## Build on Windows

From Git Bash, with Rust, Visual Studio's C++ tools, Node and pnpm installed (cmake and
libclang are found inside Visual Studio):

```bash
scripts/fork/build.sh             # target/dist: the release profile (slow, fat LTO)
scripts/fork/build.sh --release   # target/release: much faster to build, for trying changes
```

### Install over the original app, and back

Close Teskad, then from PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\fork\install.ps1                # -BuildProfile release for a --release build
powershell -ExecutionPolicy Bypass -File scripts\fork\restore-official.ps1       # back to the original exe
```

`install.ps1` backs up the data folder to `backups\<time>\` and the original executables to
`backups\official\` first. `backups\` is not committed (`.git/info/exclude`); `git clean -x`
would delete it.
