#!/usr/bin/env python3
"""hangar-rbuild, the build machine's side: run by scripts/remote/hangar_rbuild.py over SSH on the rented box.

  remote.py hello <idle-min>   state of the box (JSON), and a contact for the watchdog
  remote.py run <payload>      takes a lane, waits for the PC's "go" (the sources are synced meanwhile), runs the steps
  remote.py seed-out <lane>    copies the lane's dependency artifacts to /root/hb/seed-out, for the PC's mirror
  remote.py seed-in            installs the mirror the PC pushed to /root/hb/seed-in as /root/hb/seed

Lanes: /root/hb/lanes/<n>/src (a copy of the caller's worktree) and /root/hb/lanes/<n>/target, one run at a time
each (flock on /root/hb/lanes/<n>.lock). A run prefers the lane its worktree used last, then the one used longest
ago: every lane holds a recent build of the same workspace, so a new worktree, or the scratch worktree of a
delivery check, starts warm. A lane without a target starts from /root/hb/seed (the dependencies of the last
mirror) when it was built by the same rustc.
"""
import base64
import fcntl
import hashlib
import json
import os
import shutil
import signal
import subprocess
import sys
import time

HB = "/root/hb"
LANES = f"{HB}/lanes"
STATE = f"{HB}/state"
SEED = f"{HB}/seed"
SEED_OUT = f"{HB}/seed-out"
SEED_IN = f"{HB}/seed-in"
CARGO_HOME = "/root/.cargo"
RUSTUP_HOME = "/root/.rustup"
# Never pruned from a lane's copy of the worktree: target/ holds the kits' wasm build that hangar-kit-host's
# tests make there (tests/tabs.rs), dist-mobile/ is the phone app's stub (an empty folder, as in the CI).
KEEP = {"target", "dist-mobile"}
RUNTIME_DIR = "/tmp/hb-runtime"


def emit(tag, data):
    try:
        sys.stdout.write(f"@@HB-{tag} {json.dumps(data)}\n")
        sys.stdout.flush()
    except OSError:
        pass


def say(text):
    try:
        print(f"[rbuild-box] {text}", file=sys.stderr, flush=True)
    except OSError:
        pass


def touch(path):
    with open(path, "a"):
        pass
    os.utime(path, None)


def contact():
    os.makedirs(STATE, exist_ok=True)
    touch(f"{STATE}/contact")


def read(path, default=""):
    try:
        with open(path, encoding="utf-8") as f:
            return f.read()
    except OSError:
        return default


def write(path, text):
    with open(path + ".tmp", "w", encoding="utf-8") as f:
        f.write(text)
    os.replace(path + ".tmp", path)


def base_env(color="never"):
    """The CI's environment for cargo, from scratch: nothing of the PC's (no HANGAR_PORT_BASE, no CARGO_TARGET_DIR)."""
    return {
        "HOME": "/root", "USER": "root", "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8",
        "PATH": f"{CARGO_HOME}/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
        "CARGO_HOME": CARGO_HOME, "RUSTUP_HOME": RUSTUP_HOME,
        "CARGO_TERM_COLOR": color, "TERM": "xterm-256color" if color == "always" else "dumb",
        # ci.yml: file and line in a backtrace without the full debug info (less to compile, link and keep).
        "CARGO_PROFILE_DEV_DEBUG": "line-tables-only",
        # ci.yml, Tests step: the views' tests draw with iced_test's software renderer.
        "ICED_TEST_BACKEND": "tiny-skia",
        "CI": "true",
        # set on a GitHub runner; without it, GTK and D-Bus clients in the tests print an error each
        "XDG_RUNTIME_DIR": RUNTIME_DIR,
    }


def rustc_version():
    r = subprocess.run([f"{CARGO_HOME}/bin/rustc", "-vV"], capture_output=True, text=True, env=base_env())
    return r.stdout.strip()


def seed_key(rustc, src):
    lock = read(os.path.join(src, "Cargo.lock"))
    return hashlib.sha256((rustc + "\0" + lock).encode()).hexdigest()[:16]


def lane_meta(i):
    try:
        return json.loads(read(f"{LANES}/{i}.json", "{}"))
    except ValueError:
        return {}


def try_lock(i):
    f = open(f"{LANES}/{i}.lock", "a")
    try:
        fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return f
    except BlockingIOError:
        f.close()
        return None


def acquire(count, key):
    os.makedirs(LANES, exist_ok=True)
    metas = [(i, lane_meta(i)) for i in range(count)]
    # its own lane first, then lanes already built (longest unused first), then empty ones
    order = sorted(metas, key=lambda m: (m[1].get("key") != key, not m[1].get("used"), m[1].get("used", 0)))
    waited = None
    while True:
        for i, _ in order:
            f = try_lock(i)
            if f:
                if waited:
                    say(f"lane {i} free after {time.time() - waited:.0f} s")
                return i, f
        if waited is None:
            waited = time.time()
            say(f"all {count} lanes are busy: waiting for one")
        contact()
        time.sleep(2)


def busy_lanes(count, mine):
    busy = 0
    for i in range(count):
        if i == mine:
            continue
        f = try_lock(i)
        if f:
            f.close()
        else:
            busy += 1
    return busy


def prune(src, listfile):
    """Removes what the caller's worktree no longer has (a deleted or renamed file), then the empty folders: a
    leftover crates/<x>/ without its Cargo.toml would break the workspace's `crates/*` glob."""
    keep = {x for x in open(listfile, "rb").read().decode("utf-8", "surrogateescape").split("\0") if x}
    removed = 0
    for root, dirs, files in os.walk(src):
        rel_root = os.path.relpath(root, src)
        if rel_root == ".":
            dirs[:] = [d for d in dirs if d not in KEEP]
        for name in files + [d for d in dirs if os.path.islink(os.path.join(root, d))]:
            rel = name if rel_root == "." else os.path.join(rel_root, name)
            if rel not in keep:
                os.unlink(os.path.join(root, name))
                removed += 1
    for root, dirs, files in os.walk(src, topdown=False):
        rel_root = os.path.relpath(root, src)
        if rel_root != "." and rel_root.split(os.sep, 1)[0] not in KEEP and not os.listdir(root):
            os.rmdir(root)
    return removed


class SeedLock:
    """Seed folders are rewritten by seed-out and seed-in (exclusive) and copied into empty lanes (shared)."""

    def __init__(self, exclusive):
        self.mode = fcntl.LOCK_EX if exclusive else fcntl.LOCK_SH

    def __enter__(self):
        os.makedirs(STATE, exist_ok=True)
        self.f = open(f"{STATE}/seed.lock", "a")
        fcntl.flock(self.f, self.mode)
        return self

    def __exit__(self, *exc):
        self.f.close()
        return False


def lane_target(i, rustc):
    """The lane's target dir. An empty one starts from the newest seed built by this rustc: the PC's mirror pushed
    at the box's creation (seed/), or what a lane of this box built (seed-out/, made for the mirror)."""
    target = f"{LANES}/{i}/target"
    tag = f"{LANES}/{i}/rustc.txt"
    if os.path.isdir(target) and read(tag) != rustc:
        say(f"lane {i}: another rustc than its last build, target cleared")
        shutil.rmtree(target, ignore_errors=True)
    if not os.path.isdir(target):
        with SeedLock(exclusive=False):
            seeds = []
            for folder in (SEED, SEED_OUT):
                try:
                    info = json.loads(read(f"{folder}/hb-seed.json", "{}") or "{}")
                except ValueError:
                    info = {}
                if info.get("rustc") == rustc and os.path.isdir(f"{folder}/target"):
                    seeds.append((info.get("created", 0), folder, info))
            if seeds:
                _, folder, info = max(seeds)
                t0 = time.time()
                subprocess.run(["cp", "-a", f"{folder}/target", target], check=True)
                say(f"lane {i}: dependencies copied from {os.path.basename(folder)} (key {info.get('key')}) in {time.time() - t0:.0f} s")
            else:
                say(f"lane {i}: no seed built by {rustc.splitlines()[0] if rustc else 'this rustc'}: cold build")
        os.makedirs(target, exist_ok=True)
    write(tag, rustc)
    return target


CHILD = {"proc": None}


def stop_child(signum, _frame):
    p = CHILD["proc"]
    if p and p.poll() is None:
        try:
            os.killpg(p.pid, signal.SIGTERM)
            for _ in range(50):
                if p.poll() is not None:
                    break
                time.sleep(0.2)
            else:
                os.killpg(p.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
    os._exit(128 + signum)


def run(payload):
    p = json.loads(base64.b64decode(payload))
    contact()
    if os.path.exists(f"{STATE}/retired"):
        emit("RETIRED", {"why": read(f"{STATE}/watchdog.log").strip().splitlines()[-1:]})
        return 75
    for s in (signal.SIGHUP, signal.SIGTERM, signal.SIGINT):
        signal.signal(s, stop_child)
    t0 = time.time()
    i, _lock = acquire(int(p["lanes"]), p["key"])
    src = f"{LANES}/{i}/src"
    os.makedirs(src, exist_ok=True)
    emit("LANE", {"lane": i, "src": src, "list": f"{LANES}/{i}.files", "wait_s": round(time.time() - t0, 1)})
    if sys.stdin.readline().strip() != "go":
        return 0
    t1 = time.time()
    removed = prune(src, f"{LANES}/{i}.files")
    os.makedirs(f"{src}/dist-mobile", exist_ok=True)
    rustc = rustc_version()
    target = lane_target(i, rustc)
    prep_s = round(time.time() - t1, 1)
    os.makedirs(RUNTIME_DIR, mode=0o700, exist_ok=True)
    env = base_env(p.get("color", "never"))
    env["CARGO_TARGET_DIR"] = target
    steps, code = [], 0
    for step in p["steps"]:
        say(f"{step['name']}: {' '.join(step['argv'])}")
        contact()
        s0 = time.time()
        e = dict(env)
        e.update(step.get("env", {}))
        for name in step.get("unset", []):
            e.pop(name, None)
        proc = subprocess.Popen(step["argv"], cwd=src, env=e, stdin=subprocess.DEVNULL, start_new_session=True)
        CHILD["proc"] = proc
        rc = proc.wait()
        CHILD["proc"] = None
        steps.append({"name": step["name"], "code": rc, "s": round(time.time() - s0, 1)})
        if rc != 0:
            code = code or rc
            if not p.get("keep_going"):
                break
    contact()
    meta = {"key": p["key"], "worktree": p.get("worktree", ""), "used": time.time(), "kind": p.get("kind", ""),
            "tested": any(s["name"] == "test" for s in steps), "seed_key": seed_key(rustc, src),
            "rustc": rustc.splitlines()[0] if rustc else ""}
    write(f"{LANES}/{i}.json", json.dumps(meta))
    emit("EXIT", {"code": code, "lane": i, "removed": removed, "prep_s": prep_s, "steps": steps,
                  "busy": busy_lanes(int(p["lanes"]), i), "seed_key": meta["seed_key"], "rustc": meta["rustc"]})
    return code


def members(src, env):
    r = subprocess.run(["cargo", "metadata", "--no-deps", "--format-version", "1", "--offline"], cwd=src, env=env,
                       capture_output=True, text=True)
    if r.returncode != 0:
        raise SystemExit(f"cargo metadata: {r.stderr.strip()[-400:]}")
    data = json.loads(r.stdout)
    pkgs = {pkg["name"] for pkg in data["packages"]}
    targets = {t["name"].replace("-", "_") for pkg in data["packages"] for t in pkg["targets"]}
    return pkgs, targets


def seed_out(lane):
    """The lane's dependency artifacts, without the workspace's own crates (they change with every commit) nor the
    incremental caches: what swatinem/rust-cache keeps in the CI. rsync --delete keeps seed-out in step."""
    i = int(lane)
    contact()
    f = try_lock(i)
    while f is None:
        time.sleep(2)
        contact()
        f = try_lock(i)
    src, target = f"{LANES}/{i}/src", f"{LANES}/{i}/target"
    env = base_env()
    pkgs, targets = members(src, env)
    # cargo's build-dir layout since Rust 1.100: debug/build/<package>/<hash>/{fingerprint,out,run} holds a unit's
    # artifacts, fingerprint and build-script run; the older one (deps/, .fingerprint/, build/<package>-<hash>/)
    # is filtered the same way in case a toolchain still writes it.
    rules = ["- /debug/incremental/", "- /debug/examples/"]
    rules += [f"- /debug/build/{n}/" for n in sorted(pkgs)] + [f"- /debug/build/{n}-*" for n in sorted(pkgs)]
    rules += [f"- /debug/.fingerprint/{n}-*" for n in sorted(pkgs)]
    rules += [f"- /debug/deps/{t}-*" for t in sorted(targets)] + [f"- /debug/deps/lib{t}-*" for t in sorted(targets)]
    rules += ["- /debug/build/*/*/incremental/", "+ /debug/", "+ /debug/deps/***", "+ /debug/build/***",
              "+ /debug/.fingerprint/***", "+ /.rustc_info.json", "+ /CACHEDIR.TAG", "- *"]
    with open(f"{STATE}/seed-rules", "w") as rf:
        rf.write("\n".join(rules) + "\n")
    t0 = time.time()
    rustc = rustc_version()
    info = {"key": seed_key(rustc, src), "rustc": rustc, "lane_target": target, "created": time.time(),
            "worktree": lane_meta(i).get("worktree", "")}
    with SeedLock(exclusive=True):
        os.makedirs(f"{SEED_OUT}/target", exist_ok=True)
        if os.path.exists(f"{SEED_OUT}/hb-seed.json"):
            os.remove(f"{SEED_OUT}/hb-seed.json")
        subprocess.run(["rsync", "-a", "--delete", "--delete-excluded", f"--filter=merge {STATE}/seed-rules",
                        target + "/", f"{SEED_OUT}/target/"], check=True)
        write(f"{SEED_OUT}/hb-seed.json", json.dumps(info))
    f.close()
    du = subprocess.run(["du", "-sb", f"{SEED_OUT}/target", f"{CARGO_HOME}/registry/cache", f"{CARGO_HOME}/registry/index"],
                        capture_output=True, text=True).stdout.split()
    info.update(copy_s=round(time.time() - t0, 1), target_bytes=int(du[0]) if du else 0,
                registry_bytes=sum(int(x) for x in du[2::2]) if du else 0)
    emit("SEED", info)
    return 0


def seed_in():
    """The PC pushed its mirror to /root/hb/seed-in (target/ and hb-seed.json; the registry went straight to
    /root/.cargo/registry): it becomes /root/hb/seed, what empty lanes start from."""
    if not os.path.isfile(f"{SEED_IN}/hb-seed.json"):
        raise SystemExit("seed-in: no hb-seed.json")
    with SeedLock(exclusive=True):
        shutil.rmtree(SEED, ignore_errors=True)
        os.rename(SEED_IN, SEED)
    info = json.loads(read(f"{SEED}/hb-seed.json"))
    emit("SEEDED", {"key": info.get("key"), "same_rustc": info.get("rustc") == rustc_version()})
    return 0


def hello(idle_min):
    os.makedirs(STATE, exist_ok=True)
    write(f"{STATE}/idle-min", str(int(idle_min)))
    contact()
    st = os.statvfs("/root")
    mem = 0
    for line in open("/proc/meminfo"):
        if line.startswith("MemTotal:"):
            mem = int(line.split()[1]) // 1048576
    lanes = {}
    for name in sorted(os.listdir(LANES)) if os.path.isdir(LANES) else []:
        if name.endswith(".json"):
            lanes[name[:-5]] = lane_meta(name[:-5])
    emit("HELLO", {"retired": os.path.exists(f"{STATE}/retired"), "nproc": os.cpu_count(), "mem_gb": mem,
                   "free_gb": round(st.f_bavail * st.f_frsize / 2 ** 30, 1), "rustc": rustc_version().splitlines()[:1],
                   "lanes": lanes, "seed": json.loads(read(f"{SEED}/hb-seed.json", "{}") or "{}").get("key"),
                   "watchdog": subprocess.run(["pgrep", "-f", "/root/hb/bin/watchdog.sh"], capture_output=True).returncode == 0})
    return 0


def main():
    cmd, args = sys.argv[1], sys.argv[2:]
    if cmd == "run":
        return run(args[0])
    if cmd == "hello":
        return hello(args[0] if args else 60)
    if cmd == "seed-out":
        return seed_out(args[0])
    if cmd == "seed-in":
        return seed_in()
    raise SystemExit(f"remote.py: unknown command {cmd}")


if __name__ == "__main__":
    sys.exit(main())
