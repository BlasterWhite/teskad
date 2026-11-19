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
scripts/remote/rbuild check --affected              # the same, for the crates a branch changes and their dependents
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
3. **Room on the disk.** Before anything is sent, the box makes sure it has
   20 GB or 15 % of its disk free, whichever is more, by pruning idle lanes
   if it must ([below](#the-disk)). If it cannot, the call exits with 75 and
   says what holds the disk.
4. **The sources.** The files `git ls-files` lists (tracked, and untracked
   ones git does not ignore) go to the lane by `rsync --checksum`, without
   their times: a file whose content did not change keeps the time of the
   lane's last build, so cargo rebuilds only what changed, even coming from
   another worktree. Never sent: `.git`, `target/`, ignored files, untracked
   files over 16 MB, in a tool's folder (`.claude`, `.hangar`, `.vscode`…) or
   named like a secret (`.env`, `*.pem`, `id_*`, `*token*`…). Files the
   worktree no longer has are removed from the lane.
5. **cargo** runs with the CI's environment: `CARGO_PROFILE_DEV_DEBUG=line-tables-only`,
   `ICED_TEST_BACKEND=tiny-skia`, an empty `dist-mobile/`, no
   `HANGAR_PORT_BASE` (nothing of the PC's environment is sent). Unlike the CI
   (`CARGO_INCREMENTAL=0`) it keeps rustc's incremental caches, since the
   lanes are reused: a check after a one-line change takes 109 s with them,
   150 s without ([measured](#the-disk)), for 5 GB a lane that the disk guard
   frees first.
6. **After it**, the label goes back to idle, and a reaper waits on the PC.

`rbuild check` runs the steps of the CI's rust job on ubuntu-22.04, in its
order, and stops at the first failure (`--keep-going` runs them all):
`cargo fmt --all --check`, clippy on the workspace, clippy and tests of
`hangar-core` without its default features (the headless node), clippy of
`hangar-webview` with its engine, then `cargo test --workspace --locked
--no-fail-fast`. The cross-check of `hangar-node` for Linux arm64 and the
Windows job stay with GitHub CI.

## `check --affected`: only what a change reaches

`rbuild check` is the whole workspace, a few minutes even for a one-line
change, and it stays what a release is judged by. `rbuild check --affected`
runs the same checks for the crates a change reaches and no others:

1. **The files that changed.** The PC (git is only there: the lanes have no
   `.git`) lists the files that differ from the merge-base of `HEAD` and
   `origin/main` as the worktree is now: committed, staged, unstaged and
   untracked ones (not git's ignored ones), a deletion, and both sides of a
   rename. Nothing is fetched. `main` is the base when there is no
   `origin/main`; `--base REF` (or `HANGAR_RBUILD_BASE`) names another, say
   `HEAD~1`, or `HEAD` to check what is not committed yet. A stale
   `origin/main`, or a local `main` ahead of it that the checkout is built on,
   only widens the list.
2. **The crates.** On the box, `cargo metadata` gives the workspace's crates,
   their folders and their dependencies. A file belongs to every crate whose
   folder holds it (a crate nested in another, `hangar-kit-host`'s probe,
   takes its parent along), and to the crates that read it from outside
   their folder: an `include_str!`, `include_bytes!`, `include!` or
   `#[path]` that leaves the crate (`hangar-theme` embeds
   `hangar-core/kit-sdk/tokens.json`), or a test's `CARGO_MANIFEST_DIR`
   joined with a `../…` path (an asset, the example kit), found by reading
   the crates' sources. Then come the crates that depend on those, directly
   or through others (normal, dev and build dependencies), and the crates
   whose tests run cargo on another (`"-p", "hangar-kit-probe"`).
3. **The steps** are the full check's, for those crates: `cargo fmt --all
   --check`; `cargo clippy -p <each> --all-targets --locked -- -D warnings`;
   the headless `hangar-core` clippy and tests only if `hangar-core` is
   affected; the webview engine clippy only if `hangar-webview` is; `cargo
   test -p <each> --locked --no-fail-fast`.

It says what it chose and why before cargo starts, and again at the end:

```
[rbuild-box] affected: 1 file differs from origin/main (merge-base 38e5a712)
[rbuild-box] 4 of 19 crates: hangar-app, hangar-kit-host, hangar-theme, hangar-ui
[rbuild-box]   hangar-app: depends on hangar-kit-host, hangar-theme, hangar-ui
[rbuild-box]   hangar-kit-host: depends on hangar-theme, hangar-ui
[rbuild-box]   hangar-theme: 1 file changed (crates/hangar-theme/src/lib.rs)
[rbuild-box]   hangar-ui: depends on hangar-theme
[rbuild-box] steps for hangar-core, hangar-webview skipped: not affected
```

**It runs the full check instead** (the same command, the reason in the
output) when it cannot be sure what a change reaches: a `Cargo.toml` (any
crate's: features and dependencies are how crates are unified), `Cargo.lock`,
a `rust-toolchain` file, rustfmt's or clippy's settings, anything in
`.cargo/`, a `build.rs` (or any build script cargo lists) or the remote
build's own `scripts/remote/` changed; a file no crate owns or reads and that
is not plainly documentation (anything outside `docs/`, `site/`, `mobile/`,
`.github/`, `scripts/` (not `scripts/remote/`), `nix/` and the root's
Markdown, licence, lockfile and packaging files, so a new top-level folder
gets the full check until `IGNORED_DIRS` in `vm/affected.py` is told); every crate is affected;
nothing differs from the base; or git or cargo metadata cannot be read.
**Documentation alone** (`docs/`, `site/`, `mobile/`…) is `cargo fmt --all
--check` and nothing else: exit 0 in about 8 seconds on a running machine.

What it can miss: cargo unifies features over the crates it builds, so `-p`
builds a crate with the features its selected dependents ask for, not the
whole workspace's (the first affected check in a lane compiles those
variants once: 40 s more, and the next ones find them); a test that reads a repository
file through a path built at run time, not by one of the forms above, is not
seen to depend on it; and the arm64 and Windows checks stay with the CI, as
for the full check. The full check is what a release, and anything that
touches a manifest, goes through. Its seed refresh does not follow a partial
run: only a lane that tested the whole workspace updates the mirror.

Measured on 19 November 2026 on the shared machine (an AMD EPYC 7B13, 64
threads; other workers' checks ran in its other lanes meanwhile, so a time
varies by 10 to 20 s), in one lane: a one-line comment added to a source file
of the crate, `check` and `check --affected` run in turn, each on a new
version of the line (`--base HEAD`, the change not committed):

| The change | `check --affected` | `check` (full) |
|---|---|---|
| `docs/` only | 8 s: fmt 4 s, exit 0 | about 100 s (nothing to recompile) |
| `hangar-app`: 1 of 19 crates | 81, 85 and 87 s: clippy 19 s, tests 55 to 60 s | 158 and 158 s: clippy 21 s, headless core tests 18 s, tests 111 s |
| `hangar-theme`: 4 crates, with `hangar-ui`, `hangar-kit-host` and `hangar-app` | 91, 95, 101 and 102 s (139 and 141 s for the first in a lane): clippy 20 to 24 s, tests 62 to 70 s | 158 and 179 s; 230 and 251 s when the check before was on `hangar-core`, whose headless steps then run again |
| `hangar-core`: 7 crates, and the headless core and webview engine steps | 234 and 237 s: clippy 38 s, headless core 13 s and 41 s, webview engine 12 s, tests 125 s | 247 and 252 s |
| nothing changed since the last run (`hangar-theme`) | 34 s: clippy 1 s, tests 26 s | 100 to 104 s ([above](#the-seed-a-new-machine-starts-warm)) |

So it halves the wait for a change in `hangar-app` (the crate most changes
are in, and that nothing depends on) and cuts it by about 40 % for one in
`hangar-theme`. For `hangar-core`, which `hangar-app`, `hangar-engine`,
`hangar-discord`, `hangar-node`, `hangar-term` and `hangar-webview` all lean
on, it saves 10 to 18 s: the tests of the twelve other crates, the big ones
being affected anyway; and the headless core and webview engine steps run
again either way.

The planning is `scripts/remote/vm/affected.py` and its tests are
`python3 -B -m unittest discover -s scripts/remote/tests` (no machine, no
cargo; `-B`, so that no `__pycache__` lands in `vm/`, which is shipped to
the box).

## The machine

Vast.ai rents the spare capacity of other people's GPU servers; the GPU goes
unused, the build runs on the CPU cores that come with it. An offer is
picked among the verified hosts in Europe with at least 32 threads, 63 GB of
memory, a 150 GB disk, 200/100 Mb/s and 98 % reliability, under 0.32 $/h, by
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
the cargo registry's index and crates. After a `check` whose tests ran, the
mirror is refreshed in the background (`~/.local/state/hangar-rbuild/background.log`)
when rustc changed, or when the dependencies changed and it is over 12 hours
old; the PC's reaper does the same before destroying a machine. A new
machine receives the mirror while it installs its packages, and each of its
lanes starts from it: only the workspace's crates are compiled. To refresh
the mirror, the box first copies a lane's dependencies to
`/root/hb/seed-out` (6 GB); once the PC has pulled it, that copy becomes the
box's seed in place of the older one, so the disk holds one seed, never two.
An empty lane starts from the newer of the two built by the box's rustc. A
new stable Rust makes the mirror useless until the next check refreshes it
(that day's first build is cold, as it is in the CI).

Measured on 19 November 2026, on a 32-thread AMD EPYC 7763 (Italy) and 7J13
(Norway) with a cgroup quota of about 30 CPUs, which cargo and the test
harness honour (`-j30`):

| Run | Time |
|---|---|
| `rbuild check`, cold (empty box, no seed) | 410 s: fmt 5, clippy 98, headless core clippy 20 and tests 48, webview engine clippy 46, tests 189 |
| `rbuild check` again, nothing changed | 100 to 104 s: each clippy about 1 s, tests 74 s (the tests running) |
| `rbuild check` after a one-line change in `hangar-app` | 177 s: clippy 28 s, tests 123 s |
| `rbuild check` in a new lane started from the seed | 213 s alone (clippy 33 s, tests 130 s); no third-party crate rebuilt |
| three `rbuild check` at once, from three worktrees | 100 s (a warm lane), 255 s and 270 s (two new seeded lanes) |
| `rcargo test -p hangar-theme`, then again | 33 s, then 6 s |
| `rcargo test -p hangar-app` after a change | 51 to 59 s |
| overhead of a call | 2 to 7 s to find the machine, about 1 s to compare and send the 1,270 files |
| a new machine | rented in 21 to 27 s, set up in 47 to 82 s |
| the seed | 6.0 to 6.6 GB on disk, 1.43 GB once compressed: 165 s at 8.6 MB/s to 287 s at 5 MB/s, depending on what else uses the PC's upload; 0.013 $ of traffic |
| refreshing the mirror | 0 to 0.7 GB in 12 to 19 s once it exists (rsync sends only what changed), up to 0.02 $ |

The PC's upload is the seed's limit: on a 30-CPU box it takes about as long
as compiling those dependencies, so the first check on a new machine is not
faster for it (536 s from nothing, against about 520 s cold). Seeding pays
when several workers arrive together: each new lane starts with the
dependencies instead of compiling them alongside the others.

## The disk

The box has 150 GB (`HANGAR_RBUILD_DISK`). A lane's `target/` grows with
every build: on 19 November 2026 four lanes of 8 to 20 GB and a 6 GB
seed-out filled a 70 GB disk, and every call failed in rsync with "No space
left on device". So before each run, once it holds its lane and before a
file is sent, the box checks its free space against the larger of 20 GB
(`HANGAR_RBUILD_MIN_FREE`) and 15 % of its disk
(`HANGAR_RBUILD_MIN_FREE_PCT`): 24.2 GB, Vast's 150 GB being 150 GiB
(161.1 GB). A lane started from the seed grows by about that much in its
first check. Short of it, the box frees what
can be rebuilt, least useful first, and stops as soon as it has enough:

1. a seed-out the PC never took (made over 30 minutes ago: a pull that
   failed or was cut);
2. the incremental caches (`debug/incremental`, and the per-unit ones of
   cargo's build-dir layout, `debug/build/<package>/<hash>/incremental`) and
   `debug/examples` of the idle lanes, least recently used first;
3. the whole `target/` of the idle lanes, least recently used first: the next
   run there starts from the seed;
4. the run's own lane: its caches, then its `target/`.

A lane is idle when its lock is free, and the pruning holds that lock while
it deletes; a lane another run holds is never touched. If the disk is still
short, the run is refused before a file is sent: exit 75, with the free
space, what was pruned and which lanes are busy. Run it again when they
finish, or `rbuild stop` for a new, empty machine. The copy made for the
mirror (seed-out) passes the same guard first, keeping its own lane's
`target/`, which is what it copies. And a step that fails while less than
2 GB is left (several lanes growing at once after passing the guard) makes
the call exit with 75 rather than cargo's code: a full disk says nothing
about the code.

Each pruning is a `prune` event in the journal, and each build records the
free space at its start and end. `rbuild status` shows the box's free disk,
the threshold, each lane's size (and how much of it is incremental caches
and examples), the seed and seed-out, and the last prunings.

Tried on 19 November 2026 on a trial box (`HANGAR_RBUILD_LABEL`; EPYC 7J13,
Norway; threshold 24.2 GB). Lanes 1 to 3 were copies of a built lane of 21 GB (5.0 GB of
`debug/incremental`, 1.2 GB of `debug/examples`, the rest `debug/build`),
lane 3 the least recently used but held by another run, and a `fallocate`d
file set the free space before each call:

| Free before the call | What the guard pruned | Result |
|---|---|---|
| 20.0 GB | lane 3 skipped (busy); lane 1's caches and examples (5.2 GB) | the call ran, 25.2 GB free |
| 10.0 GB | lane 2's caches and examples (5.2 GB), then lane 1's `target/` (17.0 GB) | ran, 31.9 GB free |
| 1.0 GB | lane 2's `target/` (17.0 GB), then the call's own lane: caches (5.2 GB) and `target/` (17.0 GB) | ran from the seed (copied in 4 s), 40.2 GB free |
| 5.0 GB, every idle lane already empty | the own lane's `target/` (7.4 GB) | refused in 4 s, nothing synced: exit 75, "the build machine's disk is short: 12.4 GB free of 161.1 GB, and a run starts with 24.2 GB free (…); lane 3 busy, left alone" |
| 1.5 GB, the guard off (`HANGAR_RBUILD_MIN_FREE=0 HANGAR_RBUILD_MIN_FREE_PCT=0`) | nothing; the seed's copy failed (cold build), cargo stopped on "No space left on device" | exit 75 (cargo's 101 in the journal): "the box's disk filled up during this run" |

Lane 3 was never touched. Once a seed pull had taken `/root/hb/seed-out`,
it replaced the box's seed: 6.5 GB freed.

The incremental caches, measured the same day on that box in one lane, with
the same one-line change in `hangar-app` made, then undone:

| `rbuild check` | Incremental caches (cargo's default) | `CARGO_INCREMENTAL=0` (the CI's) |
|---|---|---|
| after the change, then after undoing it | 109 s and 109 s: clippy 7 s, tests 78 s | 150 s and 149 s: clippy 19 s, tests 104 s |
| nothing changed | 94 s | 96 s |
| disk | 5.0 GB of `debug/incremental` in a 21 GB lane | none written |

The box keeps them: 40 s saved on every check that follows a change, for
5 GB a lane that the guard drops first when the disk is short. Switching
from one to the other recompiles the workspace's crates once in each lane
(the first check after it took 228 s).

## Costs and the guard

The machine costs 0.28 to 0.32 $/h, its disk included; traffic is billed by
the host at 0.003 to 0.04 $/GB. It runs while builds come in and an hour
after the last one. For about 3 hours of builds a day, the machine is up 4 to
6 hours: **1.2 to 1.9 $ a day, about 27 to 40 $ a month** over 22 working
days (the seed and the mirror's refreshes add a few cents a day). Setting it
up, measuring it and three workers' first hour of use cost 0.56 $. A
16-thread machine (`HANGAR_RBUILD_MIN_THREADS=16 HANGAR_RBUILD_MIN_RAM=31
HANGAR_RBUILD_MAX_DPH=0.2`) costs about half, builds more slowly, and serves
fewer builds at once.

Every call checks a budget guard first: today's spend by this tool (its
journal: each instance's hours at its $/h, and the traffic at its host's
price) plus one more hour may not exceed `HANGAR_RBUILD_GUARD` (5 $ a day).
Over it, the call exits with 75. The aps5 project's own guard counts the
whole Vast account, this tool's spend included.

The journal is `~/.local/state/hangar-rbuild/events.jsonl`: every creation,
destruction, transfer, build and pruning of the box's disk, with its
durations. `rbuild status` sums it up.

## Settings

| Variable | Default | |
|---|---|---|
| `HANGAR_RBUILD_GUARD` | 5 | $ a day this tool may spend |
| `HANGAR_RBUILD_IDLE_MIN` | 60 | minutes without a call before the machine is destroyed |
| `HANGAR_RBUILD_LANES` | 4 | builds that run side by side |
| `HANGAR_RBUILD_MIN_THREADS`, `HANGAR_RBUILD_MIN_RAM` | 32, 63 | smallest offer (threads, GB) |
| `HANGAR_RBUILD_MAX_DPH` | 0.32 | dearest offer, $/h |
| `HANGAR_RBUILD_MAX_STORAGE` | 0.40 | dearest disk, $/GB-month |
| `HANGAR_RBUILD_DISK` | 150 | GB of disk |
| `HANGAR_RBUILD_MIN_FREE`, `HANGAR_RBUILD_MIN_FREE_PCT` | 20, 15 | the box keeps the larger of this many GB and this % of its disk free before a run, pruning idle lanes ([the disk](#the-disk)) |
| `HANGAR_RBUILD_INCREMENTAL` | unset (cargo's default: on) | `0` or `1`: `CARGO_INCREMENTAL` for the box's cargo; a change recompiles the workspace's crates once in each lane |
| `HANGAR_RBUILD_BASE` | `origin/main`, else `main` | the ref `check --affected` takes its merge-base with (`--base REF` on the command line) |
| `HANGAR_RBUILD_SKIP_MACHINES` | | Vast machine ids never to rent |
| `HANGAR_RBUILD_ANYWHERE` | | `1` to rent outside Europe too |
| `HANGAR_RBUILD_LABEL`, `HANGAR_RBUILD_STATE` | `hangar-rbuild`, `~/.local/state/hangar-rbuild` | a trial machine beside the shared one: another label (the GitHub reaper ignores it, so `rbuild stop` it) and another journal |

## Troubleshooting

- **Exit 75**: no machine. The message says why: the guard, no offer under
  the price (`rbuild offers`; raise `HANGAR_RBUILD_MAX_DPH` or lower
  `HANGAR_RBUILD_MIN_THREADS`), Vast refusing, SSH not answering, the
  connection lost during a run (the host reclaimed it), or the box's disk
  short even after pruning its idle lanes (the message names the busy ones;
  `rbuild status` shows what holds the disk). Run it again: the next call
  rents another machine, or finds the lanes free. Nothing was compiled
  locally.
- **`check --affected` ran everything**: the box says why in a `full check:`
  line before cargo starts (a manifest, the lockfile or `scripts/remote/`
  changed, a file nothing reads, every crate affected, no merge-base…), and
  the journal keeps it (`plan` in the build's event). A local `main` far
  ahead of `origin/main` widens the list of changed files: `--base` names a
  nearer ref (`--base HEAD~1`, or `--base HEAD` for what is not committed).
  While two worktrees have different versions of `scripts/remote/vm/`, each
  call ships its own to the box, so a call can meet the older `remote.py`
  that another one just put there: it runs the whole check, without the
  `affected:` lines. Run it again.
- **A host that misbehaves** (SSH refused, slow disk): `rbuild stop`, then
  `HANGAR_RBUILD_SKIP_MACHINES=<machine id>` (`rbuild status` shows it).
- **Look around**: `scripts/remote/rbuild ssh` opens a shell on it (the lanes
  are in `/root/hb/lanes/<n>/src`, their builds in `.../target`); `rbuild ssh
  'tail /root/hb/state/watchdog.log'` runs one command.
- **A build that differs from the CI's**: the machine is ubuntu-22.04 with
  the CI's packages and the latest stable Rust; `rbuild ssh 'rustc -V'`
  against the CI log's toolchain line. A lane can be reset with `rbuild ssh
  'rm -rf /root/hb/lanes/<n>'`.
- **A test that fails here and passes in the CI**: the machine runs 30 test
  threads (a GitHub runner, 4) and several lanes share its ports, so a test
  that waits a fixed time or expects a port to stay closed fails now and then.
  Seen on 19 November 2026: `a_worker_s_thread_is_opened_once_even_after_a_restart`
  and `a_lightning_bolt_sends_a_waiting_message_now` (`hangar-discord/tests/discord.rs`),
  `a_sign_in_can_be_cancelled_and_does_not_wait_forever` (`hangar-core`'s
  cloud tests). Run it again before suspecting a change.
- **The mirror**: `rbuild seed` shows it; delete `~/.cache/hangar-rbuild/seed`
  to start over (the next check refills it). It follows the box when rustc
  changes, or at most every 12 hours when the dependencies do, so that
  branches with different `Cargo.lock` files do not pull it back and forth.
- **Vast says "too frequent"**: the account is shared with aps5 and every
  worker; calls wait and retry for up to two minutes.
