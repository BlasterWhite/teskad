# This fork

[OLIVIER-CG/hangar](https://github.com/OLIVIER-CG/hangar), forked from
[hangar-ai/hangar](https://github.com/hangar-ai/hangar) at **v2.4.0**, with upstream's
**2.5.1** merged in (Codex support is upstream's own, from 2.5.0). Our changes live on the
`custom` branch; `main` mirrors upstream. The `codex-panes` branch holds our earlier Codex
work, kept for reference only: it is not merged.

## What differs from upstream

| Change | Where | Why |
|---|---|---|
| Never updates itself | `crates/hangar-update/src/platform.rs` (`SELF_UPDATE`) | Upstream's updater would replace this build with the official release. Take upstream's changes by merging them instead (below). |

Add a row for every feature we change, so a merge from upstream knows what to keep.

## Your data

The fork keeps Hangar's app identifier (`com.simnjs.iabench`), so it uses the same folders as
the official app and needs no migration:

- `%APPDATA%\com.simnjs.iabench\`: `state.json` (workspaces, panes, settings), `memory.json`,
  `activity\`, `mcp\` (the agents' MCP configs), the stats cache;
- `%USERPROFILE%\.hangar\`: the endpoint file, the Claude Code bridge mod, kits;
- your project folders, wherever they are.

Run only one Hangar at a time: two would share `state.json` and the endpoint.

## Build

From Git Bash, with Rust, Visual Studio's C++ tools, Node and pnpm installed (cmake and
libclang are found inside Visual Studio):

```bash
scripts/fork/build.sh             # target/dist: the release profile upstream ships (slow, fat LTO)
scripts/fork/build.sh --release   # target/release: much faster to build, for trying changes
```

To try a change without touching your real Hangar, `scripts/native-dev.sh` runs it on data of
its own under `target/dev-home/` (see CONTRIBUTING.md).

## Install over the official app, and back

Close Hangar, then from PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\fork\install.ps1                # -BuildProfile release for a --release build
powershell -ExecutionPolicy Bypass -File scripts\fork\restore-official.ps1       # back to the official exe
```

`install.ps1` backs up the data folder to `backups\<time>\` and the official executables to
`backups\official\` first. `backups\` is not committed (`.git/info/exclude`); `git clean -x`
would delete it.

## Taking upstream's changes

```bash
git fetch upstream --tags
git merge upstream/main   # or a release tag, e.g. v2.4.2
scripts/fork/build.sh
```

then close Hangar and run `install.ps1` again.
