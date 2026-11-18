#!/usr/bin/env python3
"""Hangar's Rust builds on a machine rented from Vast.ai: nothing is compiled on this PC.

  scripts/remote/rcargo <cargo arguments…>   cargo on the build machine, in a copy of the current git worktree
  scripts/remote/rbuild check [--keep-going] the CI rust job's Linux checks in one remote call (the workspace check)
  scripts/remote/rbuild up | status | stop | ssh [command] | offers | reap [--watch | --github] | seed [pull]

The machine is one Vast.ai instance (the CPU cores of a GPU host), rented on the first call, labelled
"hangar-rbuild <last use> <boot|busy|idle>", and destroyed 60 min after its last use, by this PC's reaper or by
.github/workflows/rbuild-reaper.yml when the PC is off. A new one starts from the mirror of the last one's
dependencies in ~/.cache/hangar-rbuild/seed (built crates and the cargo registry). Exit codes: cargo's own; 75
when no build machine can be had (budget guard, no offer, Vast refusing, connection lost); 2 for a usage error.
It never falls back to compiling here. How it works, costs, troubleshooting: docs/native/remote-build.md.
"""
import base64
import datetime
import fcntl
import hashlib
import json
import os
import re
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
VM_DIR = os.path.join(HERE, "vm")
STATE = os.path.join(os.environ.get("XDG_STATE_HOME") or os.path.expanduser("~/.local/state"), "hangar-rbuild")
CACHE = os.path.join(os.environ.get("XDG_CACHE_HOME") or os.path.expanduser("~/.cache"), "hangar-rbuild")
SEED = os.path.join(CACHE, "seed")
EVENTS = os.path.join(STATE, "events.jsonl")
KEY = os.path.join(STATE, "ssh", "id_ed25519")
VASTAI = os.environ.get("HANGAR_RBUILD_VASTAI") or shutil.which("vastai") or os.path.expanduser("~/.local/bin/vastai")


def env_float(name, default):
    try:
        return float(os.environ.get(name) or default)
    except ValueError:
        return float(default)


GUARD = env_float("HANGAR_RBUILD_GUARD", 5)               # $ a day, this tool's spend
IDLE_MIN = env_float("HANGAR_RBUILD_IDLE_MIN", 60)        # destroyed after this long without a run
BUSY_MAX_MIN = env_float("HANGAR_RBUILD_BUSY_MAX_MIN", 240)  # a "busy" label older than this is a lost run
DISK_GB = int(env_float("HANGAR_RBUILD_DISK", 70))
MAX_DPH = env_float("HANGAR_RBUILD_MAX_DPH", 0.32)
MAX_STORAGE = env_float("HANGAR_RBUILD_MAX_STORAGE", 0.40)  # $ per GB-month
MIN_THREADS = int(env_float("HANGAR_RBUILD_MIN_THREADS", 32))
MIN_RAM_GB = int(env_float("HANGAR_RBUILD_MIN_RAM", 63))
LANES = int(env_float("HANGAR_RBUILD_LANES", 4))
IMAGE = os.environ.get("HANGAR_RBUILD_IMAGE", "ubuntu:22.04")
SKIP = {int(x) for x in re.split(r"[\s,]+", os.environ.get("HANGAR_RBUILD_SKIP_MACHINES", "")) if x.isdigit()}
UNAVAILABLE = 75
LABEL = "hangar-rbuild"
EUROPE = set("AT BE BG CH CY CZ DE DK EE ES FI FR GB GR HR HU IE IS IT LI LT LU LV MT NL NO PL PT RO RS SE SI SK".split())
ONSTART = ("mkdir -p /root/hb/state; if [ -f /root/hb/bin/watchdog.sh ]; then "
           "(setsid nohup bash /root/hb/bin/watchdog.sh >/dev/null 2>&1 </dev/null &); fi; true")
# Untracked files that never leave the PC, even when git does not ignore them.
DENY_TOP = {".git", ".claude", ".hangar", ".iabench", ".cursor", ".vscode", ".idea", ".ssh", ".aws", ".config", ".gnupg"}
DENY_NAME = re.compile(r"(^\.env|\.env$|\.pem$|\.key$|\.p12$|\.pfx$|^id_(rsa|dsa|ecdsa|ed25519)|credential|secret|token|\.netrc$)", re.I)
UNTRACKED_MAX = 16 << 20

# The CI's rust job on ubuntu-22.04 (.github/workflows/ci.yml), in its order; formatting is checked there on
# Windows only, which makes no difference to the result. The arm64 cross-check of hangar-node is left to the CI.
CHECK_STEPS = [
    {"name": "fmt", "argv": ["cargo", "fmt", "--all", "--check"]},
    {"name": "clippy", "argv": ["cargo", "clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"]},
    {"name": "clippy core headless", "argv": ["cargo", "clippy", "-p", "hangar-core", "--no-default-features", "--all-targets",
                                              "--locked", "--", "-D", "warnings"]},
    {"name": "test core headless", "argv": ["cargo", "test", "-p", "hangar-core", "--no-default-features", "--locked"],
     "unset": ["ICED_TEST_BACKEND"]},
    {"name": "clippy webview engine", "argv": ["cargo", "clippy", "-p", "hangar-webview", "--all-targets", "--features", "engine",
                                               "--locked", "--", "-D", "warnings"]},
    {"name": "test", "argv": ["cargo", "test", "--workspace", "--locked", "--no-fail-fast"]},
]


class Unavailable(Exception):
    """No build machine now: exit 75, never a local build."""


# ------------------------------------------------------------------------------------------------ small things
def say(msg):
    print(f"[rbuild {time.strftime('%H:%M:%S')}] {msg}", file=sys.stderr, flush=True)


def jlog(event, **fields):
    os.makedirs(STATE, exist_ok=True)
    rec = {"time": datetime.datetime.now().isoformat(timespec="seconds"), "t": round(time.time(), 1), "event": event, **fields}
    with open(EVENTS, "a", encoding="utf-8") as f:
        f.write(json.dumps(rec) + "\n")


def jread():
    out = []
    try:
        with open(EVENTS, encoding="utf-8") as f:
            for line in f:
                try:
                    out.append(json.loads(line))
                except ValueError:
                    pass
    except OSError:
        pass
    return out


class Flock:
    def __init__(self, name, what, blocking=True):
        self.path, self.what, self.blocking = os.path.join(STATE, name), what, blocking

    def __enter__(self):
        os.makedirs(STATE, exist_ok=True)
        self.f = open(self.path, "a")
        try:
            fcntl.flock(self.f, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            if not self.blocking:
                self.f.close()
                return None
            say(f"waiting: {self.what}")
            fcntl.flock(self.f, fcntl.LOCK_EX)
        return self

    def __exit__(self, *exc):
        if not self.f.closed:
            fcntl.flock(self.f, fcntl.LOCK_UN)
            self.f.close()
        return False


def midnight():
    return time.mktime(time.strptime(time.strftime("%Y-%m-%d"), "%Y-%m-%d"))


def gb(n):
    return f"{n / 1e9:.2f} GB"


# ------------------------------------------------------------------------------------------------ Vast
SCRUB = re.compile(r"[0-9A-Za-z_\-]{32,}")
RATE_LIMIT = re.compile(r"too many requests|rate.?limit|(?:error|status|code|http)\D{0,12}\b429\b", re.I)


def vast(*args, parse=True, timeout=120):
    """The vastai CLI (the key comes from ~/.config/vastai or VAST_API_KEY). Its raw output is never printed: the
    creation's answer and some errors carry keys."""
    for attempt in range(6):
        try:
            r = subprocess.run([VASTAI, *args, "--raw"], capture_output=True, text=True, timeout=timeout)
        except (OSError, subprocess.TimeoutExpired) as e:
            raise Unavailable(f"vastai {' '.join(args[:2])}: {e}")
        text = (r.stdout or "") + (r.stderr or "")
        limited = RATE_LIMIT.search(text) and not _is_json(r.stdout)
        if not limited or attempt == 5:
            break
        time.sleep(2 * (attempt + 1))
    if r.returncode != 0:
        raise Unavailable(f"vastai {' '.join(args[:2])}: exit {r.returncode}: " + SCRUB.sub("<hidden>", text.strip())[-400:])
    if not parse:
        return r.stdout
    try:
        return json.loads(r.stdout)
    except ValueError:
        raise Unavailable(f"vastai {' '.join(args[:2])}: unreadable answer: " + SCRUB.sub("<hidden>", r.stdout.strip())[:300])


def _is_json(text):
    try:
        json.loads(text)
        return True
    except (ValueError, TypeError):
        return False


def make_label(state):
    return f"{LABEL} {int(time.time())} {state}"


def parse_label(text):
    m = re.match(rf"^{LABEL} (\d+) (boot|busy|idle)\b", (text or "").strip())
    return {"epoch": int(m.group(1)), "state": m.group(2)} if m else None


def ours():
    return [i for i in vast("show", "instances") if (i.get("label") or "").startswith(LABEL + " ") or (i.get("label") or "") == LABEL]


def set_label(iid, state):
    try:
        vast("label", "instance", str(iid), make_label(state), parse=False)
    except Unavailable as e:
        say(f"label of {iid} not refreshed ({e}): the reapers judge it by the previous one")


def country(geo):
    m = re.search(r"\b([A-Z]{2})\s*$", geo or "")
    return m.group(1) if m else "?"


def machine(inst):
    ports = (inst.get("ports") or {}).get("22/tcp") or []
    public = (inst.get("public_ipaddr") or "").strip()
    if ports and public:
        host, port = public, int(ports[0].get("HostPort") or 0)
    else:
        host, port = inst.get("ssh_host"), int(inst.get("ssh_port") or 0)
    return {"id": inst["id"], "host": host, "port": port, "dph": float(inst.get("dph_total") or 0),
            "down_cost": float(inst.get("inet_down_cost") or 0), "up_cost": float(inst.get("inet_up_cost") or 0)}


def cpu_factor(name):
    """Throughput of one of the offer's threads for a Rust build, 1.0 for a recent core."""
    n = (name or "").replace("®", "").replace("™", "")
    m = re.search(r"Ryzen (?:Threadripper (?:PRO )?)?(?:\d )?(\d)\d{3}", n)
    if m:
        return 1.0 if int(m.group(1)) >= 5 else 0.8
    m = re.search(r"EPYC \w{3}(\d)", n)
    if m:
        return 1.0 if int(m.group(1)) >= 3 else 0.8
    if "Core Ultra" in n:
        return 1.0
    m = re.search(r"i\d-(\d{4,5})", n)
    if m:
        gen = int(m.group(1)[:2]) if len(m.group(1)) == 5 else int(m.group(1)[0])
        return 1.0 if gen >= 12 else 0.85 if gen >= 10 else 0.65
    if re.search(r"Xeon.*(Gold|Platinum|Silver) \d[3-9]\d\d", n):
        return 0.8
    return 0.6


def search_offers(seed_gb=None):
    if seed_gb is None:
        seed_gb = seed_size() / 1e9
    q = (f"verified=true rentable=true reliability>0.98 cpu_cores_effective>={MIN_THREADS} cpu_ram>={MIN_RAM_GB} "
         f"disk_space>={DISK_GB} inet_down>=200 inet_up>=100 cpu_arch=amd64 dph<={MAX_DPH} disk_bw>=400")
    offers = vast("search", "offers", q, "-o", "dph", "--storage", str(DISK_GB), "--limit", "400", timeout=180)
    keep = []
    for o in offers:
        if o.get("machine_id") in SKIP or float(o.get("storage_cost") or 9) > MAX_STORAGE:
            continue
        o["_country"] = country(o.get("geolocation"))
        if o["_country"] not in EUROPE and os.environ.get("HANGAR_RBUILD_ANYWHERE") != "1":
            continue
        # A warm build is mostly serial (one crate, then the links): a thread's speed counts twice, their number once.
        threads = min(48.0, float(o.get("cpu_cores_effective") or 1))
        o["_speed"] = threads * cpu_factor(o.get("cpu_name")) ** 2
        # $ for a two-hour session (machine and disk, dph_total) plus the seed's upload, per unit of build speed
        o["_score"] = (float(o["dph_total"]) * 2 + seed_gb * float(o.get("inet_down_cost") or 0)) / o["_speed"]
        keep.append(o)
    keep.sort(key=lambda o: o["_score"])
    return keep


def offer_text(o):
    return (f"offer {o['id']} (machine {o.get('machine_id')}, {o.get('geolocation')}): {(o.get('cpu_name') or '').strip()}, "
            f"{o.get('cpu_cores_effective', 0):.0f} threads, {o.get('cpu_ram', 0) / 1024:.0f} GB, {o['dph_total']:.3f} $/h "
            f"(storage {o.get('storage_cost', 0):.2f} $/GB-month), traffic {o.get('inet_down_cost', 0):.4f}/{o.get('inet_up_cost', 0):.4f} $/GB "
            f"down/up, {o.get('inet_down', 0):.0f}/{o.get('inet_up', 0):.0f} Mb/s, reliability {o.get('reliability2', o.get('reliability', 0)):.3f}")


# ------------------------------------------------------------------------------------------------ cost and guard
def spend_today(live=None):
    """This tool's spend today from its journal: each instance's hours today at its $/h (machine and disk), plus the
    traffic of the seeds and the source syncs at the host's prices."""
    day0, now = midnight(), time.time()
    insts, traffic = {}, 0.0
    for e in jread():
        if e["event"] == "create":
            insts[e["id"]] = {"dph": e.get("dph", 0.0), "start": e["t"], "end": None}
        elif e["event"] == "destroy" and e.get("id") in insts:
            insts[e["id"]]["end"] = e.get("at", e["t"])
        elif e["event"] in ("transfer", "build") and e["t"] >= day0:
            traffic += e.get("traffic_usd", 0.0)
    hours = usd = 0.0
    for i in insts.values():
        h = max(0.0, (i["end"] or now) - max(i["start"], day0)) / 3600
        hours += h
        usd += h * i["dph"]
    return usd + traffic, hours, traffic


def guard(dph):
    usd, hours, _ = spend_today()
    if usd + dph > GUARD:
        raise Unavailable(f"budget guard: {usd:.2f} $ spent today ({hours:.1f} h of build machine) + {dph:.2f} $ for one more hour "
                          f"> {GUARD:.2f} $ (HANGAR_RBUILD_GUARD)")


def reconcile(live):
    """Instances of the journal that are gone (destroyed by the GitHub reaper, or by Vast): their end is noted."""
    ids = {i["id"] for i in live}
    ended = {e["id"] for e in jread() if e["event"] == "destroy"}
    last_use = {}
    for e in jread():
        if e.get("id") is not None:
            last_use[e["id"]] = e["t"]
    for e in jread():
        if e["event"] == "create" and e["id"] not in ended and e["id"] not in ids:
            at = min(time.time(), last_use.get(e["id"], e["t"]) + (IDLE_MIN + 15) * 60)
            jlog("destroy", id=e["id"], at=round(at, 1), reason="gone: destroyed elsewhere (GitHub reaper, Vast or by hand)")


# ------------------------------------------------------------------------------------------------ ssh and rsync
def ensure_key():
    if not os.path.exists(KEY):
        os.makedirs(os.path.dirname(KEY), mode=0o700, exist_ok=True)
        subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "hangar-rbuild", "-f", KEY], check=True)
    return open(KEY + ".pub").read().strip()


def ssh_opts(m):
    return ["-i", KEY, "-o", "IdentitiesOnly=yes", "-o", f"UserKnownHostsFile={STATE}/ssh/known_hosts-{m['id']}",
            "-o", "StrictHostKeyChecking=accept-new", "-o", "BatchMode=yes", "-o", "ConnectTimeout=15",
            "-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=8", "-o", "LogLevel=ERROR",
            "-o", "ControlMaster=auto", "-o", f"ControlPath={STATE}/ssh/cm-%C", "-o", "ControlPersist=120", "-p", str(m["port"])]


def ssh(m, command, timeout=None, data=None):
    try:
        return subprocess.run(["ssh", *ssh_opts(m), f"root@{m['host']}", command], input=data, capture_output=True, text=True,
                              stdin=None if data is not None else subprocess.DEVNULL, timeout=timeout)
    except subprocess.TimeoutExpired:
        return subprocess.CompletedProcess([], 124, "", f"no answer in {timeout} s")


def rsync(m, args, what):
    e = "ssh " + " ".join(shlex.quote(a) for a in ssh_opts(m))
    r = subprocess.run(["rsync", "-e", e, "--stats", *args], capture_output=True, text=True, stdin=subprocess.DEVNULL)
    if r.returncode not in (0, 24):
        raise Unavailable(f"rsync ({what}) exit {r.returncode}: {(r.stderr or r.stdout).strip()[-400:]}")
    sent = re.search(r"Total bytes sent: ([\d,.]+)", r.stdout)
    recv = re.search(r"Total bytes received: ([\d,.]+)", r.stdout)
    num = lambda x: int(re.sub(r"[,.]", "", x.group(1))) if x else 0  # noqa: E731
    return num(sent), num(recv)


def remote_tagged(out, tag):
    for line in out.splitlines():
        k = line.find(f"@@HB-{tag} ")
        if k >= 0:
            return json.loads(line[k + len(tag) + 6:])
    return None


# ------------------------------------------------------------------------------------------------ the instance
def show(iid):
    try:
        r = vast("show", "instance", str(iid))
        return r[0] if isinstance(r, list) else r
    except Unavailable:
        return None


def destroy(inst_or_id, reason):
    iid = inst_or_id["id"] if isinstance(inst_or_id, dict) else inst_or_id
    say(f"destroying instance {iid}: {reason}")
    try:
        out = vast("destroy", "instance", str(iid), "-y", parse=False)
    except Unavailable as e:
        if "not found" not in str(e).lower() and "404" not in str(e):
            raise
        out = ""
    if "Aborted" in out:
        raise Unavailable(f"vastai refused to destroy {iid}")
    jlog("destroy", id=iid, reason=reason)


def wait_running(iid, seconds=600):
    t0, last = time.time(), None
    while time.time() - t0 < seconds:
        inst = show(iid)
        st = (inst or {}).get("actual_status")
        msg = ((inst or {}).get("status_msg") or "").strip()[:120]
        if (st, msg) != last:
            say(f"instance {iid}: {st or 'scheduling'}{' — ' + msg if msg else ''}")
            last = (st, msg)
        if inst and st == "running" and machine(inst)["port"]:
            return inst
        if inst and st in ("exited", "offline") and time.time() - t0 > 60:
            raise Unavailable(f"instance {iid} is {st}: {msg}")
        time.sleep(5)
    raise Unavailable(f"instance {iid} not running after {seconds} s")


def wait_ssh(m, seconds=240):
    t0, info = time.time(), ""
    while time.time() - t0 < seconds:
        r = ssh(m, "echo ok", timeout=30)
        if r.returncode == 0 and "ok" in r.stdout:
            return
        info = (r.stderr or "").strip()[-200:]
        time.sleep(5)
    raise Unavailable(f"SSH to {m['host']}:{m['port']} not answering after {seconds} s: {info}")


def vm_hash():
    h = hashlib.sha256()
    for name in sorted(os.listdir(VM_DIR)):
        h.update(name.encode() + b"\0" + open(os.path.join(VM_DIR, name), "rb").read())
    return h.hexdigest()[:16]


def provision_hash():
    return hashlib.sha256(open(os.path.join(VM_DIR, "provision.sh"), "rb").read()).hexdigest()[:16]


def ensure_tools(m):
    """Provisioning (once per instance) and the box's scripts (whenever they change here)."""
    r = ssh(m, "cat /root/hb/state/provisioned 2>/dev/null; echo; cat /root/hb/bin/.hash 2>/dev/null; true", timeout=60)
    if r.returncode != 0:
        raise Unavailable(f"SSH: {r.stderr.strip()[-300:]}")
    lines = (r.stdout.split("\n") + ["", ""])[:2]
    if lines[0].strip() != provision_hash():
        t0 = time.time()
        say("provisioning the box (apt packages, mold, rustup stable): ~2-4 min, once per instance")
        p = subprocess.run(["ssh", *ssh_opts(m), f"root@{m['host']}", "bash -s"], stdin=open(os.path.join(VM_DIR, "provision.sh"), "rb"),
                           stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        for line in p.stdout.splitlines():
            if line.startswith("[rbuild-box]"):
                say(line[13:])
        if p.returncode != 0:
            print(p.stdout[-3000:], file=sys.stderr)
            raise Unavailable(f"provisioning failed (exit {p.returncode})")
        ssh(m, f"echo {provision_hash()} > /root/hb/state/provisioned", timeout=30)
        jlog("provision", id=m["id"], s=round(time.time() - t0, 1))
    if lines[1].strip() != vm_hash():
        rsync(m, ["-r", "--chmod=F755", VM_DIR + "/", f"root@{m['host']}:/root/hb/bin/"], "box scripts")
        ssh(m, f"echo {vm_hash()} > /root/hb/bin/.hash", timeout=30)
    ssh(m, "(setsid nohup bash /root/hb/bin/watchdog.sh >/dev/null 2>&1 </dev/null &) ; true", timeout=30)


def hello(m):
    r = ssh(m, f"python3 /root/hb/bin/remote.py hello {int(IDLE_MIN)}", timeout=60)
    info = remote_tagged(r.stdout, "HELLO") if r.returncode == 0 else None
    if info is None:
        raise Unavailable(f"the box does not answer: {(r.stderr or r.stdout).strip()[-300:]}")
    return info


def create_fresh():
    offers = search_offers()
    if not offers:
        raise Unavailable(f"no Vast offer in Europe with {MIN_THREADS}+ threads, {MIN_RAM_GB}+ GB, {DISK_GB} GB of disk under "
                          f"{MAX_DPH} $/h (HANGAR_RBUILD_MAX_DPH, HANGAR_RBUILD_MIN_THREADS…)")
    guard(float(offers[0]["dph_total"]))
    pub = ensure_key()
    for o in offers[:3]:
        say("renting " + offer_text(o))
        t0 = time.time()
        boot = make_label("boot")
        try:
            r = vast("create", "instance", str(o["id"]), "--image", IMAGE, "--disk", str(DISK_GB), "--ssh", "--direct",
                     "--label", boot, "--onstart-cmd", ONSTART, "--cancel-unavail", timeout=180)
        except Unavailable as e:
            r = {"error": str(e)}
        iid = r.get("new_contract") if isinstance(r, dict) else None
        if not iid:
            # an unreadable answer may still have rented one: it would carry our label
            ghost = next((i for i in ours() if (i.get("label") or "") == boot), None)
            if ghost is None:
                say(f"refused: {r.get('error', 'no instance id in the answer') if isinstance(r, dict) else r}")
                continue
            iid = ghost["id"]
        jlog("create", id=iid, offer=o["id"], machine_id=o.get("machine_id"), dph=float(o["dph_total"]),
             storage_cost=o.get("storage_cost"), down_cost=o.get("inet_down_cost"), up_cost=o.get("inet_up_cost"),
             cpu=(o.get("cpu_name") or "").strip(), threads=o.get("cpu_cores_effective"), ram_gb=round(o.get("cpu_ram", 0) / 1024),
             disk_gb=DISK_GB, geo=o.get("geolocation"))
        try:
            vast("attach", "ssh", str(iid), pub, parse=False)
            inst = wait_running(iid)
            m = machine(inst)
            wait_ssh(m)
            jlog("running", id=iid, s=round(time.time() - t0, 1))
            say(f"instance {iid} up in {time.time() - t0:.0f} s ({m['host']}:{m['port']})")
            ensure_tools(m)
            info = hello(m)
            seed_push(m, info)
            return m
        except Unavailable as e:
            say(f"instance {iid} unusable: {e}")
            try:
                destroy(iid, f"unusable at creation: {str(e)[:120]}")
            except Unavailable as e2:
                say(f"destroy of {iid} failed, the reapers will retry: {e2}")
    raise Unavailable("no offer could be rented and reached (three tried)")


def ensure_instance(state="busy"):
    """A running, reachable, provisioned box, its label set to `state` (and its last use to now) before the lock
    that keeps the reaper out is released; Unavailable otherwise."""
    with Flock("instance.lock", "another call is starting the build machine"):
        live = ours()
        reconcile(live)
        good = None
        for inst in sorted(live, key=lambda i: i.get("start_date") or 0, reverse=True):
            st = inst.get("actual_status")
            age = time.time() - (inst.get("start_date") or time.time())
            if good is None and st == "running":
                m = machine(inst)
                ok = m["port"] and ssh(m, "true", timeout=40).returncode == 0
                if not ok and m["port"]:
                    # rented by a caller with another key (another HOME): ours is added, then tried again
                    try:
                        vast("attach", "ssh", str(inst["id"]), ensure_key(), parse=False)
                        time.sleep(3)
                        ok = ssh(m, "true", timeout=40).returncode == 0
                    except Unavailable:
                        pass
                if ok:
                    good = m
                    continue
                if age > 600:
                    destroy(inst, "running but unreachable over SSH")
            elif st in ("exited", "offline", "stopped") or inst.get("intended_status") == "stopped":
                destroy(inst, f"{st}: never kept stopped")
            elif st != "running" and age > 900:
                destroy(inst, f"{st or 'scheduling'} for {age / 60:.0f} min")
        if good:
            guard(good["dph"])
            ensure_tools(good)
            info = hello(good)
            if info.get("retired"):
                destroy(good, "its watchdog stopped the build service (no contact for an hour)")
            else:
                set_label(good["id"], state)
                return good
        m = create_fresh()
        set_label(m["id"], state)
        return m


# ------------------------------------------------------------------------------------------------ seed mirror
def seed_info():
    try:
        return json.load(open(os.path.join(SEED, "seed.json")))
    except (OSError, ValueError):
        return None


def seed_size():
    info = seed_info()
    return (info or {}).get("bytes", 0)


def seed_push(m, box):
    info = seed_info()
    if not info:
        say("no seed mirror yet in ~/.cache/hangar-rbuild/seed: this box builds cold, and fills it afterwards")
        return
    rustc = (box.get("rustc") or [""])[0]
    if info.get("rustc", "").splitlines()[:1] != [rustc]:
        say(f"seed built by {info.get('rustc', '?').splitlines()[0]}, the box has {rustc}: cold build (a new stable Rust)")
        return
    t0 = time.time()
    say(f"seeding the box from ~/.cache/hangar-rbuild/seed ({gb(info.get('bytes', 0))} on disk)…")
    s1, _ = rsync(m, ["-a", "--mkpath", "--compress", "--compress-choice=zstd", SEED + "/registry/", f"root@{m['host']}:/root/.cargo/registry/"], "seed registry")
    s2, _ = rsync(m, ["-a", "--mkpath", "--compress", "--compress-choice=zstd", SEED + "/target/", f"root@{m['host']}:/root/hb/seed-in/target/"], "seed target")
    s3, _ = rsync(m, [os.path.join(SEED, "hb-seed.json"), f"root@{m['host']}:/root/hb/seed-in/hb-seed.json"], "seed tag")
    r = ssh(m, "python3 /root/hb/bin/remote.py seed-in", timeout=300)
    if r.returncode != 0:
        say(f"seed not installed: {(r.stderr or r.stdout).strip()[-300:]}")
    sent = s1 + s2 + s3
    usd = sent / 1e9 * m["down_cost"]
    jlog("transfer", id=m["id"], what="seed push", bytes=sent, s=round(time.time() - t0, 1), traffic_usd=round(usd, 5))
    say(f"seed pushed: {gb(sent)} over the network in {time.time() - t0:.0f} s ({usd:.3f} $ of traffic)")


def seed_pull(m, lane):
    """The mirror follows the box: the dependency artifacts of a lane that ran the whole check, and the registry."""
    with Flock("seed.lock", "another seed pull"):
        t0 = time.time()
        r = ssh(m, f"python3 /root/hb/bin/remote.py seed-out {int(lane)}", timeout=1800)
        out = remote_tagged(r.stdout, "SEED") if r.returncode == 0 else None
        if out is None:
            raise Unavailable(f"seed-out: {(r.stderr or r.stdout).strip()[-300:]}")
        os.makedirs(SEED, exist_ok=True)
        tag = os.path.join(SEED, "seed.json")
        if os.path.exists(tag):
            os.remove(tag)  # a half-updated mirror is never pushed
        _, r1 = rsync(m, ["-a", "--delete", "--mkpath", "--compress", "--compress-choice=zstd", f"root@{m['host']}:/root/hb/seed-out/target/", SEED + "/target/"], "seed target")
        _, r2 = rsync(m, ["-a", "--delete", "--mkpath", "--compress", "--compress-choice=zstd", f"root@{m['host']}:/root/.cargo/registry/index/", SEED + "/registry/index/"], "seed index")
        _, r3 = rsync(m, ["-a", "--delete", "--mkpath", f"root@{m['host']}:/root/.cargo/registry/cache/", SEED + "/registry/cache/"], "seed crates")
        _, r4 = rsync(m, [f"root@{m['host']}:/root/hb/seed-out/hb-seed.json", os.path.join(SEED, "hb-seed.json")], "seed tag")
        size = int(subprocess.run(["du", "-sb", SEED], capture_output=True, text=True).stdout.split()[0])
        got = r1 + r2 + r3 + r4
        usd = got / 1e9 * m["up_cost"]
        with open(tag + ".tmp", "w") as f:
            json.dump({"key": out["key"], "rustc": out["rustc"], "bytes": size, "pulled": time.time(), "from": m["id"],
                       "worktree": out.get("worktree", "")}, f)
        os.replace(tag + ".tmp", tag)
        jlog("transfer", id=m["id"], what="seed pull", bytes=got, s=round(time.time() - t0, 1), traffic_usd=round(usd, 5),
             seed_bytes=size, key=out["key"])
        say(f"seed mirror refreshed: {gb(size)} on disk, {gb(got)} over the network in {time.time() - t0:.0f} s ({usd:.3f} $)")


# ------------------------------------------------------------------------------------------------ a run
def toplevel():
    r = subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True)
    if r.returncode != 0:
        print("rbuild: run it from inside a git worktree of Hangar", file=sys.stderr)
        sys.exit(2)
    top = r.stdout.strip()
    if not os.path.isfile(os.path.join(top, "Cargo.toml")):
        print(f"rbuild: {top} has no Cargo.toml", file=sys.stderr)
        sys.exit(2)
    return top


def file_list(top):
    """Tracked files, and untracked ones git does not ignore: what `git add -A` would see, minus anything that looks
    like a secret or lives in a tool's folder. Never .git, target/ or ignored files."""
    def ls(*a):
        r = subprocess.run(["git", "-C", top, "ls-files", "-z", *a], capture_output=True)
        if r.returncode != 0:
            raise SystemExit(f"git ls-files: {r.stderr.decode(errors='replace').strip()}")
        return [x.decode("utf-8", "surrogateescape") for x in r.stdout.split(b"\0") if x]
    files, skipped = [], []
    for rel in ls("-c"):
        p = os.path.join(top, rel)
        if os.path.islink(p) or os.path.isfile(p):
            files.append(rel)
    for rel in ls("-o", "--exclude-standard"):
        p = os.path.join(top, rel)
        first = rel.split("/", 1)[0]
        ok = (os.path.islink(p) or (os.path.isfile(p) and os.path.getsize(p) <= UNTRACKED_MAX)) and first not in DENY_TOP \
            and not any(DENY_NAME.search(part) for part in rel.split("/"))
        (files if ok else skipped).append(rel)
    return files, skipped


def worktree_key(top):
    real = os.path.realpath(top)
    return f"{os.path.basename(real)[:40]}-{hashlib.sha1(real.encode()).hexdigest()[:8]}"


def spawn(*args):
    os.makedirs(STATE, exist_ok=True)
    log = open(os.path.join(STATE, "background.log"), "a")
    subprocess.Popen([sys.executable, os.path.abspath(__file__), *args], stdin=subprocess.DEVNULL, stdout=log, stderr=log,
                     start_new_session=True, close_fds=True)


def remote_run(kind, steps, keep_going=False, retry=True):
    top = toplevel()
    files, skipped = file_list(top)
    key = worktree_key(top)
    t0 = time.time()
    m = ensure_instance("busy")
    vm_s = time.time() - t0
    color = os.environ.get("CARGO_TERM_COLOR") or ("always" if sys.stdout.isatty() else "never")
    payload = base64.b64encode(json.dumps({"key": key, "worktree": top, "lanes": LANES, "steps": steps, "color": color,
                                           "keep_going": keep_going, "kind": kind}).encode()).decode()
    proc = subprocess.Popen(["ssh", *ssh_opts(m), f"root@{m['host']}", f"python3 /root/hb/bin/remote.py run {payload}"],
                            stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    lane = exit_info = None
    sync = {}
    retired = False
    out = sys.stdout.buffer
    try:
        for raw in proc.stdout:
            k = raw.find(b"@@HB-")
            if k < 0:
                out.write(raw)
                out.flush()
                continue
            if k > 0:
                out.write(raw[:k] + b"\n")
            tag, _, data = raw[k + 5:].decode("utf-8", "replace").strip().partition(" ")
            if tag == "RETIRED":
                retired = True
            elif tag == "LANE":
                lane = json.loads(data)
                t1 = time.time()
                with tempfile.NamedTemporaryFile(prefix="files-", dir=STATE, delete=False) as lst:
                    lst.write("\0".join(files).encode("utf-8", "surrogateescape"))
                try:
                    rsync(m, [lst.name, f"root@{m['host']}:{lane['list']}"], "file list")
                    sent, _ = rsync(m, ["-lpc", "--from0", f"--files-from={lst.name}", "--mkpath", "--compress", "--compress-choice=zstd",
                                        top + "/", f"root@{m['host']}:{lane['src']}/"], "sources")
                except Unavailable:
                    proc.stdin.close()
                    raise
                finally:
                    os.unlink(lst.name)
                sync = {"sync_s": round(time.time() - t1, 1), "sent": sent}
                say(f"lane {lane['lane']}: {len(files)} files compared, {gb(sent)} sent in {sync['sync_s']} s"
                    + (f" (waited {lane['wait_s']} s for a lane)" if lane["wait_s"] > 1 else "")
                    + (f"; {len(skipped)} untracked left on the PC" if skipped else ""))
                proc.stdin.write(b"go\n")
                proc.stdin.close()
            elif tag == "EXIT":
                exit_info = json.loads(data)
        proc.wait()
    except KeyboardInterrupt:
        proc.terminate()
        say("interrupted: the run is stopped on the box")
        return 130
    except Unavailable:
        proc.terminate()
        raise
    total = time.time() - t0
    if retired and retry:
        with Flock("instance.lock", "instance"):
            destroy(m, "its watchdog stopped the build service")
        return remote_run(kind, steps, keep_going, retry=False)
    if exit_info is None:
        jlog("build", id=m["id"], kind=kind, worktree=top, code=UNAVAILABLE, total_s=round(total, 1), lost=True)
        raise Unavailable(f"connection to the build machine lost (ssh exit {proc.returncode}); the instance may have been reclaimed")
    set_label(m["id"], "busy" if exit_info.get("busy") else "idle")
    traffic = sync.get("sent", 0) / 1e9 * m["down_cost"]
    jlog("build", id=m["id"], kind=kind, worktree=top, lane=exit_info["lane"], code=exit_info["code"], vm_s=round(vm_s, 1),
         wait_s=(lane or {}).get("wait_s"), sync_s=sync.get("sync_s"), prep_s=exit_info.get("prep_s"), total_s=round(total, 1),
         steps=exit_info["steps"], sent=sync.get("sent", 0), traffic_usd=round(traffic, 6),
         argv=steps[0]["argv"] if kind == "cargo" else None)
    summary = ", ".join(f"{s['name']} {'ok' if s['code'] == 0 else 'FAILED'} {s['s']:.0f} s" for s in exit_info["steps"])
    say(f"{kind} {'ok' if exit_info['code'] == 0 else 'failed (exit ' + str(exit_info['code']) + ')'} in {total:.0f} s on instance {m['id']} "
        f"(machine {vm_s:.0f} s, sync {sync.get('sync_s', 0):.0f} s, lane prep {exit_info.get('prep_s', 0):.0f} s): {summary}")
    if kind == "check" and any(s["name"] == "test" for s in exit_info["steps"]):
        info = seed_info()
        if not info or info.get("key") != exit_info.get("seed_key"):
            say("the seed mirror is out of date: refreshing it in the background (~/.local/state/hangar-rbuild/background.log)")
            spawn("seed", "pull", "--id", str(m["id"]), "--lane", str(exit_info["lane"]))
    spawn("reap", "--watch")
    return exit_info["code"]


# ------------------------------------------------------------------------------------------------ reaping
def reap(grace_min=0.0, refresh_seed=True, quiet=False):
    """Destroys this tool's instances unused for IDLE_MIN (+ grace) minutes, stopped or broken ones, and those whose
    "busy" label is too old to be a live run. Never touches another label (aps5's "aps5 …") or an unlabelled one."""
    kept = []
    for inst in ours():
        lab = parse_label(inst.get("label"))
        now = time.time()
        st = inst.get("actual_status")
        epoch = lab["epoch"] if lab else (inst.get("start_date") or now)
        age = (now - epoch) / 60
        reason = None
        if st in ("exited", "offline", "stopped") or inst.get("intended_status") == "stopped":
            reason = f"{st or inst.get('intended_status')}: never kept stopped"
        elif lab is None or lab["state"] == "boot":
            if age >= 45:
                reason = f"creation started {age:.0f} min ago and never finished"
        elif lab["state"] == "idle" and age >= IDLE_MIN + grace_min:
            reason = f"unused for {age:.0f} min"
        elif lab["state"] == "busy" and age >= BUSY_MAX_MIN:
            reason = f"marked busy {age:.0f} min ago: a lost run"
        if not reason:
            kept.append(inst)
            if not quiet:
                say(f"instance {inst['id']} kept: {lab['state'] if lab else '?'} for {age:.0f} min ({st})")
            continue
        if refresh_seed and st == "running":
            try_refresh_seed(machine(inst))
        destroy(inst, reason)
    return kept


def try_refresh_seed(m):
    """Before a box goes: the mirror takes the dependencies of the lane that last ran the whole check, if they are
    not the ones it already has."""
    try:
        lanes = hello(m).get("lanes") or {}
        checked = sorted((v.get("used", 0), int(k), v) for k, v in lanes.items() if v.get("kind") == "check" and v.get("tested"))
        if not checked:
            return
        _, lane, meta = checked[-1]
        if meta.get("seed_key") and meta["seed_key"] == (seed_info() or {}).get("key"):
            return
        seed_pull(m, lane)
    except (Unavailable, ValueError) as e:
        say(f"seed not refreshed before destroying: {e}")


def reap_watch():
    """Background reaper on the PC: wakes when the instance's hour is up, refreshes the mirror, destroys it."""
    with Flock("reaper.lock", "reaper", blocking=False) as held:
        if held is None:
            return 0
        while True:
            with Flock("instance.lock", "instance"):
                try:
                    kept = reap(quiet=True)
                except Unavailable as e:
                    say(f"reaper: {e}")
                    kept = [None]
            if not kept:
                return 0
            nxt = 300.0
            for inst in kept:
                lab = parse_label((inst or {}).get("label"))
                if lab and lab["state"] == "idle":
                    nxt = min(nxt, max(30.0, lab["epoch"] + IDLE_MIN * 60 - time.time() + 5))
            time.sleep(nxt)


# ------------------------------------------------------------------------------------------------ commands
def cmd_status():
    live = ours()
    reconcile(live)
    usd, hours, traffic = spend_today()
    if not live:
        print("build machine: none (the next rcargo or check rents one)")
    for inst in live:
        lab = parse_label(inst.get("label"))
        m = machine(inst)
        up = (time.time() - (inst.get("start_date") or time.time())) / 60
        when = ""
        if lab and lab["state"] == "idle":
            left = IDLE_MIN - (time.time() - lab["epoch"]) / 60
            when = f", idle for {(time.time() - lab['epoch']) / 60:.0f} min: destroyed in {max(0, left):.0f} min"
        elif lab:
            when = f", {lab['state']} since {time.strftime('%H:%M', time.localtime(lab['epoch']))}"
        print(f"build machine: instance {inst['id']} {inst.get('actual_status')} for {up:.0f} min{when}")
        print(f"  {inst.get('geolocation')}, {(inst.get('cpu_name') or '').strip()}, {inst.get('cpu_cores_effective', 0):.0f} threads, "
              f"{inst.get('cpu_ram', 0) / 1024:.0f} GB, disk {inst.get('disk_space', 0):.0f} GB, {m['dph']:.3f} $/h, "
              f"traffic {m['down_cost']:.4f}/{m['up_cost']:.4f} $/GB down/up, ssh root@{m['host']} -p {m['port']}")
    print(f"today: {usd:.2f} $ ({hours:.1f} h of build machine, traffic {traffic:.3f} $), guard {GUARD:.2f} $ (HANGAR_RBUILD_GUARD)")
    info = seed_info()
    if info:
        print(f"seed mirror: {SEED}, {gb(info.get('bytes', 0))}, key {info.get('key')}, "
              f"{info.get('rustc', '').splitlines()[0] if info.get('rustc') else '?'}, pulled {time.strftime('%Y-%m-%d %H:%M', time.localtime(info.get('pulled', 0)))}")
    else:
        print(f"seed mirror: none yet ({SEED})")
    builds = [e for e in jread() if e["event"] == "build"][-5:]
    for b in builds:
        steps = " ".join(f"{s['name']}={s['s']:.0f}s" for s in b.get("steps") or [])
        print(f"  {b['time']} {b.get('kind')} exit {b.get('code')} in {b.get('total_s')} s, lane {b.get('lane')}, "
              f"{os.path.basename(b.get('worktree', ''))} {steps}")
    return 0


def main(argv):
    if not argv or argv[0] in ("-h", "--help", "help"):
        print(__doc__)
        return 0
    cmd, rest = argv[0], argv[1:]
    try:
        if cmd == "cargo":
            if not rest:
                print("usage: scripts/remote/rcargo <cargo arguments…>", file=sys.stderr)
                return 2
            return remote_run("cargo", [{"name": "cargo " + rest[0], "argv": ["cargo", *rest]}])
        if cmd == "check":
            return remote_run("check", CHECK_STEPS, keep_going="--keep-going" in rest)
        if cmd == "status":
            return cmd_status()
        if cmd == "up":
            ensure_instance("idle")
            spawn("reap", "--watch")
            return cmd_status()
        if cmd == "ssh":
            m = ensure_instance("idle")
            spawn("reap", "--watch")
            os.execvp("ssh", ["ssh", *ssh_opts(m), "-t", f"root@{m['host']}", *rest])
        if cmd == "stop":
            with Flock("instance.lock", "instance"):
                live = ours()
                for inst in live:
                    if inst.get("actual_status") == "running" and "--no-seed" not in rest:
                        try_refresh_seed(machine(inst))
                    destroy(inst, "rbuild stop")
                if not live:
                    say("no build machine to stop")
            return 0
        if cmd == "offers":
            for o in search_offers()[:12]:
                print(f"{o['_score'] * 1000:6.2f}  {offer_text(o)}")
            return 0
        if cmd == "reap":
            if "--watch" in rest:
                return reap_watch()
            if "--github" in rest:  # .github/workflows/rbuild-reaper.yml: no seed refresh, 10 min more for the PC's own
                kept = reap(grace_min=10, refresh_seed=False)
                print(f"kept {len(kept)} hangar-rbuild instance(s)")
                return 0
            with Flock("instance.lock", "instance"):
                reap()
            return 0
        if cmd == "seed":
            if rest[:1] == ["pull"]:
                iid = int(rest[rest.index("--id") + 1]) if "--id" in rest else None
                lane = int(rest[rest.index("--lane") + 1]) if "--lane" in rest else 0
                live = [i for i in ours() if i.get("actual_status") == "running" and (iid is None or i["id"] == iid)]
                if not live:
                    say("no running build machine to pull the seed from")
                    return UNAVAILABLE
                seed_pull(machine(live[0]), lane)
                return 0
            info = seed_info()
            print(json.dumps(info, indent=1) if info else "no seed mirror")
            return 0
    except Unavailable as e:
        say(f"no build machine: {e}")
        say("nothing was compiled locally (exit 75)")
        return UNAVAILABLE
    print(f"rbuild: unknown command {cmd!r} (see --help)", file=sys.stderr)
    return 2


if __name__ == "__main__":
    signal.signal(signal.SIGPIPE, signal.SIG_DFL)
    sys.exit(main(sys.argv[1:]))
