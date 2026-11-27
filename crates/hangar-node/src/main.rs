//! `teskad-node`: the headless Hangar a node runs (docs/native/nodes.md,
//! sections 2 and 3).
//!
//! - `teskad-node version` prints `<version> <target triple>`;
//! - `teskad-node serve` runs the node in the foreground (for systemd), and
//!   `serve --detach` starts it in a session of its own, its output in
//!   `~/.local/state/teskad-node/node.log`, and returns once it answers;
//!   `serve --detach --if-version <v>` does so only when this binary is
//!   version `<v>`, and exits 3 otherwise: the desktop's one SSH call that
//!   probes the node also starts it when nothing is to be installed;
//! - `teskad-node --mcp` is the MCP stdio proxy the agents of the node's
//!   panes start, as `hangar-ai --mcp` is on the desktop;
//! - `teskad-node workspaces`, `workspace add <folder>`, `workspace remove
//!   <id>` and `send <pane> <text>` show and change the workspaces the node
//!   runs itself (own.rs, cli.rs), through the running daemon;
//! - `teskad-node discord setup --guild <id>`, `discord status` and
//!   `discord off` set up the Discord connector the daemon runs beside them
//!   (discord.rs); `teskad-node service install` makes the daemon a systemd
//!   user service, for a node that runs with no desktop (service.rs).
//!
//! Linux only: built for another system, the binary says so and exits.

mod api;
mod cli;
mod control;
mod daemon;
mod discord;
mod engine;
mod events;
mod files;
mod own;
mod panes;
mod proxy;
mod service;
mod state;
mod transcripts;

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("teskad-node runs on Linux only");
    std::process::exit(1);
}

#[cfg(target_os = "linux")]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(run(&args));
}

/// The version and target `version` prints.
#[cfg(target_os = "linux")]
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
#[cfg(target_os = "linux")]
pub const TARGET: &str = env!("TESKAD_NODE_TARGET");

#[cfg(target_os = "linux")]
const USAGE: &str =
    "usage: teskad-node version | serve [--detach [--if-version <v>] | --supervised] | --mcp
       teskad-node workspaces | workspace add <folder> [--name <name>] [--no-orchestrator]
       teskad-node workspace remove <id> | send <pane> <text…>
       teskad-node discord setup --guild <id> | discord status | discord off
       teskad-node service install";

/// What `serve --detach --if-version` exits with when this is another
/// version: nothing was started.
#[cfg(target_os = "linux")]
const OTHER_VERSION: i32 = 3;

#[cfg(target_os = "linux")]
fn run(args: &[String]) -> i32 {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["version"] | ["--version"] | ["-V"] => {
            println!("{VERSION} {TARGET}");
            0
        }
        ["--mcp", ..] => {
            mcp();
            0
        }
        ["serve"] => report(daemon::serve(false)),
        ["serve", "--supervised"] => report(daemon::serve(true)),
        ["serve", "--detach"] | ["--detach", "serve"] => report(daemon::detach()),
        ["serve", "--detach", "--if-version", wanted] => {
            if *wanted == VERSION {
                report(daemon::detach())
            } else {
                eprintln!("teskad-node: this is {VERSION}, not {wanted}: not started");
                OTHER_VERSION
            }
        }
        ["workspaces"] => report(cli::workspaces()),
        ["workspace", "add", rest @ ..] => report(cli::add(rest)),
        ["workspace", "remove", id] => report(cli::remove(id)),
        ["send", pane, words @ ..] if !words.is_empty() => report(cli::send(pane, words)),
        ["discord", "setup", rest @ ..] => report(discord::setup(rest)),
        ["discord", "status"] => report(discord::print_status()),
        ["discord", "off"] => report(discord::off()),
        ["service", "install"] => report(service::install()),
        ["help"] | ["--help"] | ["-h"] => {
            println!("{USAGE}");
            0
        }
        _ => {
            eprintln!("{USAGE}");
            2
        }
    }
}

#[cfg(target_os = "linux")]
fn report(outcome: Result<(), String>) -> i32 {
    match outcome {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("teskad-node: {err}");
            1
        }
    }
}

/// The node publishes its agents' API to a file of its own
/// (`~/.local/state/teskad-node/endpoint.json`), never the desktop's
/// `~/.teskad/endpoint.json`, so a desktop Teskad on the same account keeps
/// its agents. The proxy finds it through `TESKAD_ENDPOINT_FILE`, unless the
/// caller already names one.
#[cfg(target_os = "linux")]
fn mcp() {
    let named = std::env::var_os("TESKAD_ENDPOINT_FILE").is_some_and(|v| !v.is_empty());
    if !named {
        if let Ok(dir) = state::dir() {
            // Still single-threaded: nothing else reads the environment yet.
            std::env::set_var("TESKAD_ENDPOINT_FILE", state::endpoint_file(&dir));
        }
    }
    hangar_core::run_mcp();
}
