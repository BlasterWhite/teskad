# Remote builds: Rust checks on a rented machine

Since 17 November 2026 nothing in Rust is compiled on the maintainer's PC, and
waiting 10 to 25 minutes for GitHub CI at every iteration is too slow. The
tools in `scripts/remote/` run cargo on a machine rented from
[Vast.ai](https://vast.ai) instead: the sources go there by rsync, cargo runs
there, its output comes back as it is printed, and its exit code is the
tool's.

```bash
scripts/remote/rcargo test -p hangar-app            # any cargo command
scripts/remote/rcargo clippy --workspace --all-targets --locked -- -D warnings
scripts/remote/rbuild check                         # the CI's Linux rust checks, the workspace check
scripts/remote/rbuild status                        # the machine, today's spend, the last builds
```

Both work from any git worktree of Hangar, the scratch worktree of a delivery
check included: they send the worktree the command is run from. They never
compile anything locally: when no machine can be had they exit with code
**75** and say why.

## What a call does

1. **A machine.** If none of ours is running, it rents one (below), installs
   what the CI installs, and seeds it from the local mirror. Otherwise it
   reuses the running one. The instance's Vast label, `hangar-rbuild <last
   use> <boot|busy|idle>`, records the last use by any caller.
2. **A lane.** The machine has 4 lanes (`HANGAR_RBUILD_LANES`), each a copy of
   a worktree and its own `target/`, used by one run at a time. A run takes
   the lane its worktree used last if it is free, else the one unused longest:
   every lane holds a recent build of the same workspace, so a new worktree,
   or a delivery check's scratch worktree, starts warm. Three workers calling
   at once get three lanes and build side by side; there is no lock around a
   build, only the lane's own.
3. **The sources.** The files `git ls-files` lists (tracked, and untracked
   ones git does not ignore) go to the lane by `rsync --checksum`, without
   their times: a file whose content did not change keeps the time of the
   lane's last build, so cargo rebuilds only what changed, even coming from
   another worktree. Never sent: `.git`, `target/`, ignored files, untracked
   files over 16 MB, in a tool's folder (`.claude`, `.hangar`, `.vscode`…) or
   named like a secret (`.env`, `*.pem`, `id_*`, `*token*`…). Files the
   worktree no longer has are removed from the lane.
4. **cargo** runs with the CI's environment: `CARGO_PROFILE_DEV_DEBUG=line-tables-only`,
   `ICED_TEST_BACKEND=tiny-skia`, an empty `dist-mobile/`, no
   `HANGAR_PORT_BASE` (nothing of the PC's environment is sent). Unlike the CI
   it keeps incremental compilation, since the lanes are reused.
5. **After it**, the label goes back to idle, and a reaper waits on the PC.

`rbuild check` runs the steps of the CI's rust job on ubuntu-22.04, in its
order, and stops at the first failure (`--keep-going` runs them all):
`cargo fmt --all --check`, clippy on the workspace, clippy and tests of
`hangar-core` without its default features (the headless node), clippy of
`hangar-webview` with its engine, then `cargo test --workspace --locked
--no-fail-fast`. The cross-check of `hangar-node` for Linux arm64 and the
Windows job stay with GitHub CI.

## The machine

Vast.ai rents the spare capacity of other people's GPU servers; the GPU goes
unused, the build runs on the CPU cores that come with it. An offer is
picked among the verified hosts in Europe with at least 32 threads, 63 GB of
memory, a 70 GB disk, 200/100 Mb/s and 98 % reliability, under 0.32 $/h, by
the cost of a two-hour session plus the seed's upload, divided by its build
speed (threads, and a thread's speed counted twice: a warm build is mostly
serial). `scripts/remote/rbuild offers` shows the ranking. The image is
`ubuntu:22.04`, the CI's release (and glibc); `scripts/remote/vm/provision.sh`
installs the CI's packages, mold 3.0.0 as `ld`, rustup's latest stable with
rustfmt, clippy and the wasm32 target, and Mesa's software Vulkan driver so
the 3D capture tests draw as on a GitHub runner.

The instance is **destroyed** an hour after its last use, never kept stopped:
a host bills its disk by the month (0.04 to 0.60 $/GB), more than the hours
of building it saves. Three things destroy it:

- the PC's reaper, started after each call (`rbuild reap --watch`): it wakes
  when the hour is up, copies the machine's dependency cache home if it
  changed, and destroys it;
- `.github/workflows/rbuild-reaper.yml`, every 15 minutes, when the PC is off:
  it destroys our instances idle for 70 minutes, marked busy for 4 hours (a
  lost run), stuck in creation, or stopped. It reads only the labels
  `hangar-rbuild …`: the aps5 project rents on the same account with its own
  labels (`aps5 …`), and neither tool touches the other's instances. Its key is
  the repository secret `VAST_API_KEY`, a restricted Vast key (instance read
  and write only);
- the box's own watchdog (`scripts/remote/vm/watchdog.sh`), which holds no
  key: after an hour without contact it kills any build and refuses new ones,
  so the next call or a reaper replaces it.

`rbuild stop` destroys it at once (after refreshing the mirror).

## The seed: a new machine starts warm

A new instance is empty, and a cold build of the workspace (980 crates) is
the slow part. So the PC keeps a mirror of the last machine's dependency
artifacts in `~/.cache/hangar-rbuild/seed/`: its built third-party crates
(`target/debug/{deps,build,.fingerprint}` without the workspace's own crates
nor the incremental caches, what `swatinem/rust-cache` keeps in the CI) and
the cargo registry's index and crates. After a `check` whose tests ran, if
`Cargo.lock` or rustc changed since the mirror was taken, it is refreshed in
the background (`~/.local/state/hangar-rbuild/background.log`); the PC's
reaper also refreshes it before destroying a machine. A new machine gets the
mirror right after its setup, and each of its lanes starts from it: only the
workspace's crates are compiled. A new stable Rust makes the mirror useless
until the next check refreshes it (that day's first build is cold, as it is
in the CI).

## Costs and the guard

MEASUREMENTS

Every call checks a budget guard first: today's spend by this tool (its
journal: each instance's hours at its $/h, and the traffic at its host's
price) plus one more hour may not exceed `HANGAR_RBUILD_GUARD` (5 $ a day).
Over it, the call exits with 75. The aps5 project's own guard counts the
whole Vast account, this tool's spend included.

The journal is `~/.local/state/hangar-rbuild/events.jsonl`: every creation,
destruction, transfer and build, with its durations. `rbuild status` sums it
up.

## Settings

| Variable | Default | |
|---|---|---|
| `HANGAR_RBUILD_GUARD` | 5 | $ a day this tool may spend |
| `HANGAR_RBUILD_IDLE_MIN` | 60 | minutes without a call before the machine is destroyed |
| `HANGAR_RBUILD_LANES` | 4 | builds that run side by side |
| `HANGAR_RBUILD_MIN_THREADS`, `HANGAR_RBUILD_MIN_RAM` | 32, 63 | smallest offer (threads, GB) |
| `HANGAR_RBUILD_MAX_DPH` | 0.32 | dearest offer, $/h |
| `HANGAR_RBUILD_MAX_STORAGE` | 0.40 | dearest disk, $/GB-month |
| `HANGAR_RBUILD_DISK` | 70 | GB of disk |
| `HANGAR_RBUILD_SKIP_MACHINES` | | Vast machine ids never to rent |
| `HANGAR_RBUILD_ANYWHERE` | | `1` to rent outside Europe too |

## Troubleshooting

- **Exit 75**: no machine. The message says why: the guard, no offer under
  the price (`rbuild offers`; raise `HANGAR_RBUILD_MAX_DPH` or lower
  `HANGAR_RBUILD_MIN_THREADS`), Vast refusing, SSH not answering, or the
  connection lost during a run (the host reclaimed it). Run it again: the next
  call rents another machine. Nothing was compiled locally.
- **A host that misbehaves** (SSH refused, slow disk): `rbuild stop`, then
  `HANGAR_RBUILD_SKIP_MACHINES=<machine id>` (`rbuild status` shows it).
- **Look around**: `scripts/remote/rbuild ssh` opens a shell on it (the lanes
  are in `/root/hb/lanes/<n>/src`, their builds in `.../target`); `rbuild ssh
  'tail /root/hb/state/watchdog.log'` runs one command.
- **A build that differs from the CI's**: the machine is ubuntu-22.04 with
  the CI's packages and the latest stable Rust; `rbuild ssh 'rustc -V'`
  against the CI log's toolchain line. A lane can be reset with `rbuild ssh
  'rm -rf /root/hb/lanes/<n>'`.
- **The mirror**: `rbuild seed` shows it; delete `~/.cache/hangar-rbuild/seed`
  to start over (the next check refills it).
