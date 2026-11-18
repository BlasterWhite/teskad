//! Whether the agent of a pane has a job of its own running between two
//! turns.
//!
//! A worker that started a long build, a benchmark or a watch on its CI in
//! the background ends its turn and waits: Claude Code wakes it when the
//! command reports ("1 shell, 1 monitor still running" under its prompt).
//! Seen from outside it is a worker that stopped without delivering; the
//! orchestration engine asks here before it says so.
//!
//! What counts is a command the agent ran: a shell started by the agent —
//! the program the pane's shell runs, Claude Code or another — directly, or
//! through a sandbox or a wrapper (`bwrap`, `sandbox-exec`, `env`…). Claude
//! Code runs every Bash and Monitor command, in the foreground or in the
//! background, in a shell of its own (`bash -c …`), as other agents do.
//! What does not count:
//!
//! - the pane's shell, and a shell started from a shell (the person typed
//!   `bash`, a `claude.cmd` shim): they lead to the agent;
//! - every program the agent started that is not a shell: its MCP servers
//!   (`hangar-ai --mcp`, `npx …`, `uvx …`), its language servers, its
//!   helpers — and whatever runs below them;
//! - a shell that wraps an MCP server (`sh -c "npx some-mcp"`): its command
//!   line names MCP and is not one of Claude Code's own commands; and `cmd`,
//!   which Windows' MCP setups wrap their servers in (`cmd /c npx …`) and no
//!   agent runs its commands with.
//!
//! A pane this manager does not run, or a process table that cannot be read,
//! is `Unknown`: the caller then does what it did before it could ask.
//!
//! The table is read the platform's way: `ps` on Linux and macOS, with each
//! process's command line; a ToolHelp snapshot on Windows, with each
//! executable's name only. Walking it is plain work over rows, and is what
//! the tests below exercise.

use std::collections::{HashMap, HashSet};

use super::PtyManager;
use crate::procs::Proc;

/// What the agent of a pane has running of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneJob {
    /// Nobody can tell: a pane this manager does not run, or a process table
    /// that cannot be read.
    Unknown,
    /// Nothing it started runs.
    Nothing,
    /// A command it started runs: what it is, shortened.
    Running(String),
}

/// One row of the process table, with what the process runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcCommand {
    pub pid: u32,
    pub ppid: u32,
    /// The program and its arguments, as `ps` prints them; on Windows, the
    /// executable's name alone.
    pub command: String,
}

/// What a job is shown as, at most, in characters.
const MAX_SHOWN: usize = 100;

/// The shells, by program name. `cmd` leads to an agent (its npm shim) but
/// never counts as a job.
const SHELLS: [&str; 14] = [
    "sh",
    "bash",
    "zsh",
    "dash",
    "ash",
    "ksh",
    "mksh",
    "fish",
    "tcsh",
    "csh",
    "nu",
    "pwsh",
    "powershell",
    "cmd",
];

/// What the shell of pane `id` has below it that its agent started.
pub fn pane_job(manager: &PtyManager, id: &str) -> PaneJob {
    let Some(shell) = super::shell_pid(manager, id) else {
        return PaneJob::Unknown;
    };
    match table() {
        Some(table) => job_below(&table, shell, crate::procs::started),
        None => PaneJob::Unknown,
    }
}

/// What an agent may start its commands through: a sandbox (Claude Code's,
/// on Linux and macOS) or a plain wrapper. Their children are the agent's.
const WRAPPERS: [&str; 7] = [
    "bwrap",
    "sandbox-exec",
    "env",
    "nice",
    "nohup",
    "setsid",
    "timeout",
];

/// A program's name: without its folder, its `.exe` and the `-` of a login
/// shell, in lower case.
fn program(command: &str) -> String {
    let first = command.split_whitespace().next().unwrap_or("");
    let base = first.rsplit(['/', '\\']).next().unwrap_or(first);
    let base = base.trim_start_matches('-').to_ascii_lowercase();
    base.strip_suffix(".exe").unwrap_or(&base).to_string()
}

fn is_shell(command: &str) -> bool {
    SHELLS.contains(&program(command).as_str())
}

/// Claude Code's own commands start by sourcing its snapshot of the
/// person's shell.
fn claude_code_command(command: &str) -> bool {
    command.contains("shell-snapshots")
}

/// A shell started by the agent that runs one of the agent's commands, not
/// an MCP server it was configured with.
fn runs_a_command(command: &str) -> bool {
    if !is_shell(command) || program(command) == "cmd" {
        return false;
    }
    let lower = command.to_ascii_lowercase();
    claude_code_command(command)
        || !(lower.contains("mcp") || lower.contains("modelcontextprotocol"))
}

/// What runs below the shell `shell`, read off `table`, that its agent
/// started (see the module's text). `started` gives a process's creation
/// time where the platform tells it (`procs::descendants`).
pub fn job_below(
    table: &[ProcCommand],
    shell: u32,
    started: impl Fn(u32) -> Option<u64>,
) -> PaneJob {
    let by_pid: HashMap<u32, &ProcCommand> = table.iter().map(|row| (row.pid, row)).collect();
    let Some(root) = by_pid.get(&shell) else {
        // Not in the table: it ended, or the table is not what it seems.
        return PaneJob::Unknown;
    };
    let rows: Vec<Proc> = table
        .iter()
        .map(|row| Proc {
            pid: row.pid,
            ppid: row.ppid,
        })
        .collect();
    let below = crate::procs::descendants(&rows, shell, started);
    // The shells that lead to the agent, and the agent: a program that is
    // not a shell with only shells between it and the pane. A pane started
    // on its program directly has that program for its agent.
    let mut leading: HashSet<u32> = HashSet::new();
    let mut agents: HashSet<u32> = HashSet::new();
    if is_shell(&root.command) {
        leading.insert(shell);
    } else {
        agents.insert(shell);
    }
    for pid in &below {
        let Some(row) = by_pid.get(pid) else {
            continue;
        };
        if leading.contains(&row.ppid) {
            if is_shell(&row.command) {
                leading.insert(*pid);
            } else {
                agents.insert(*pid);
            }
        } else if agents.contains(&row.ppid) {
            if runs_a_command(&row.command) {
                return PaneJob::Running(shown(&rows, &by_pid, *pid));
            }
            // What a wrapper starts, the agent started.
            if WRAPPERS.contains(&program(&row.command).as_str()) {
                agents.insert(*pid);
            }
        }
    }
    PaneJob::Nothing
}

/// A job as it is shown: the first program it runs that is not a shell,
/// else its own command line, without Claude Code's wrapping.
fn shown(rows: &[Proc], by_pid: &HashMap<u32, &ProcCommand>, job: u32) -> String {
    let inner = crate::procs::descendants(rows, job, |_| None)
        .into_iter()
        .filter_map(|pid| by_pid.get(&pid))
        .find(|row| !is_shell(&row.command));
    match (inner, by_pid.get(&job)) {
        (Some(row), _) => short(&row.command),
        (None, Some(row)) => short(&unwrapped(&row.command)),
        (None, None) => String::new(),
    }
}

/// The command Claude Code wraps as `… && eval '<command>' && pwd -P >| …`
/// (`… eval '<command>' \< /dev/null && …` before 2.1), with its quotes
/// undone; any other command line as it is.
fn unwrapped(command: &str) -> String {
    let Some((_, rest)) = command.split_once("eval '") else {
        return command.to_string();
    };
    let end = ["' \\< /dev/null", "' < /dev/null", "' && pwd -P"]
        .iter()
        .find_map(|tail| rest.rfind(tail))
        .unwrap_or(rest.len());
    rest[..end].replace("'\"'\"'", "'").replace("'\\''", "'")
}

/// On one line, cut to `MAX_SHOWN` characters.
fn short(command: &str) -> String {
    let flat = command.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= MAX_SHOWN {
        return flat;
    }
    let cut: String = flat.chars().take(MAX_SHOWN).collect();
    format!("{}…", cut.trim_end())
}

/// The rows of `ps -A -ww -o pid= -o ppid= -o args=`, which Linux and macOS
/// both print: two numbers, then the command line.
#[cfg(any(unix, test))]
fn parse_ps(output: &str) -> Vec<ProcCommand> {
    output
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let (pid, rest) = line.split_once(char::is_whitespace)?;
            let rest = rest.trim_start();
            let (ppid, command) = match rest.split_once(char::is_whitespace) {
                Some((ppid, command)) => (ppid, command.trim()),
                None => (rest, ""),
            };
            Some(ProcCommand {
                pid: pid.parse().ok()?,
                ppid: ppid.trim().parse().ok()?,
                command: command.to_string(),
            })
        })
        .collect()
}

/// The process table with each command line, or `None` when it cannot be
/// read — which is not an empty one.
#[cfg(unix)]
fn table() -> Option<Vec<ProcCommand>> {
    let out = std::process::Command::new("ps")
        .args(["-A", "-ww", "-o", "pid=", "-o", "ppid=", "-o", "args="])
        .output()
        .ok()?;
    let rows = parse_ps(&String::from_utf8_lossy(&out.stdout));
    (out.status.success() && !rows.is_empty()).then_some(rows)
}

/// The process table with each executable's name: a ToolHelp snapshot does
/// not carry command lines, and reading each one out of its process would
/// cost what this question is not worth.
#[cfg(windows)]
fn table() -> Option<Vec<ProcCommand>> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut rows = Vec::new();
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while more {
        let name = &entry.szExeFile;
        let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
        rows.push(ProcCommand {
            pid: entry.th32ProcessID,
            ppid: entry.th32ParentProcessID,
            command: String::from_utf16_lossy(&name[..len]),
        });
        more = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    unsafe {
        CloseHandle(snapshot);
    }
    (!rows.is_empty()).then_some(rows)
}

#[cfg(not(any(unix, windows)))]
fn table() -> Option<Vec<ProcCommand>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNKNOWN: fn(u32) -> Option<u64> = |_| None;

    fn job(table: &[ProcCommand], shell: u32) -> PaneJob {
        job_below(table, shell, UNKNOWN)
    }

    fn table(rows: &[(u32, u32, &str)]) -> Vec<ProcCommand> {
        rows.iter()
            .map(|&(pid, ppid, command)| ProcCommand {
                pid,
                ppid,
                command: command.to_string(),
            })
            .collect()
    }

    /// A Bash command as Claude Code 2.1 starts it.
    fn claude_code(command: &str) -> String {
        format!(
            "/bin/bash -c source /home/ada/.claude/shell-snapshots/snapshot-bash-1-x.sh 2>/dev/null || true && eval '{command}' && pwd -P >| /tmp/claude-19ce-cwd"
        )
    }

    /// 10: the pane's shell, 11: Claude Code, with Hangar's MCP server (12)
    /// and another one through npx (13, 14). 1: the system, 20: another pane.
    fn at_rest() -> Vec<(u32, u32, String)> {
        vec![
            (1, 0, "/sbin/init".into()),
            (10, 1, "/bin/bash".into()),
            (
                11,
                10,
                "claude --name Ada --mcp-config /tmp/ada.json".into(),
            ),
            (12, 11, "/usr/bin/hangar-ai --mcp".into()),
            (13, 11, "npm exec @acme/tickets".into()),
            (14, 13, "sh -c tickets".into()),
            (
                15,
                14,
                "node /home/ada/.npm/_npx/1/node_modules/.bin/tickets".into(),
            ),
            (20, 1, "/bin/zsh".into()),
            (21, 20, "claude".into()),
            (22, 21, claude_code("cargo bench")),
        ]
    }

    fn rows(rows: &[(u32, u32, String)]) -> Vec<ProcCommand> {
        table(
            &rows
                .iter()
                .map(|(pid, ppid, command)| (*pid, *ppid, command.as_str()))
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn an_agent_with_only_its_mcp_servers_has_nothing_running() {
        assert_eq!(job(&rows(&at_rest()), 10), PaneJob::Nothing);
    }

    #[test]
    fn a_command_the_agent_runs_in_the_background_is_its_job() {
        let mut busy = at_rest();
        busy.push((
            30,
            11,
            claude_code("gh pr checks 14 --watch 2>&1 | tail -5"),
        ));
        busy.push((31, 30, "gh pr checks 14 --watch".into()));
        busy.push((32, 30, "tail -5".into()));
        assert_eq!(
            job(&rows(&busy), 10),
            PaneJob::Running("gh pr checks 14 --watch".into())
        );
        // The other pane's agent waits on its benchmark: shown unwrapped
        // when nothing but the shell runs it.
        assert_eq!(
            job(&rows(&busy), 20),
            PaneJob::Running("cargo bench".into())
        );
    }

    #[test]
    fn a_claude_code_command_that_names_mcp_is_still_a_job() {
        let mut busy = at_rest();
        busy.push((30, 11, claude_code("cargo test -p hangar-core mcp_scope")));
        assert!(matches!(
            job(&rows(&busy), 10),
            PaneJob::Running(shown) if shown == "cargo test -p hangar-core mcp_scope"
        ));
    }

    #[test]
    fn mcp_servers_and_whatever_runs_below_them_do_not_count() {
        let mut quiet = at_rest();
        // A server wrapped in a shell, Windows' `cmd /c`, and a server that
        // runs shells of its own.
        quiet.push((
            40,
            11,
            "sh -c npx -y @modelcontextprotocol/server-github".into(),
        ));
        quiet.push((41, 11, "cmd /c npx -y tickets-server".into()));
        quiet.push((42, 11, "uvx git-server".into()));
        quiet.push((43, 42, "/bin/sh -c git log -1".into()));
        // A language server and a helper of the agent.
        quiet.push((44, 11, "rust-analyzer".into()));
        quiet.push((45, 11, "caffeinate -i -w 11".into()));
        assert_eq!(job(&rows(&quiet), 10), PaneJob::Nothing);
    }

    #[test]
    fn a_shell_the_person_typed_leads_to_the_agent() {
        // pwsh → cmd (claude.cmd) → node (Claude Code) → bash: on Windows,
        // with executables' names only.
        let windows = table(&[
            (100, 4, "pwsh.exe"),
            (101, 100, "cmd.exe"),
            (102, 101, "node.exe"),
            (103, 102, "hangar-ai.exe"),
            (104, 102, "cmd.exe"),
            (105, 104, "node.exe"),
        ]);
        assert_eq!(job(&windows, 100), PaneJob::Nothing);
        let mut busy = windows.clone();
        busy.extend(table(&[(106, 102, "bash.exe"), (107, 106, "cargo.exe")]));
        assert_eq!(job(&busy, 100), PaneJob::Running("cargo.exe".into()));
        // A shell typed in the pane, then the agent in it.
        let typed = table(&[
            (10, 1, "-zsh"),
            (11, 10, "bash"),
            (12, 11, "claude"),
            (13, 12, "/bin/zsh -c make bench"),
        ]);
        assert_eq!(
            job(&typed, 10),
            PaneJob::Running("/bin/zsh -c make bench".into())
        );
    }

    #[test]
    fn a_command_run_in_the_agents_sandbox_is_its_job() {
        let mut busy = at_rest();
        busy.push((
            30,
            11,
            "bwrap --ro-bind / / --dev /dev -- /bin/bash -c make bench".into(),
        ));
        busy.push((31, 30, claude_code("make bench")));
        busy.push((32, 31, "make bench".into()));
        assert_eq!(job(&rows(&busy), 10), PaneJob::Running("make bench".into()));
        // A wrapped MCP server is still a server.
        let mut quiet = at_rest();
        quiet.push((30, 11, "env NODE_ENV=production npx -y tickets-mcp".into()));
        quiet.push((31, 30, "sh -c tickets-mcp".into()));
        assert_eq!(job(&rows(&quiet), 10), PaneJob::Nothing);
    }

    #[test]
    fn a_pane_whose_shell_is_not_in_the_table_is_unknown() {
        assert_eq!(job(&rows(&at_rest()), 99), PaneJob::Unknown);
    }

    #[test]
    fn a_pane_started_on_its_agent_has_that_agent() {
        let direct = table(&[(10, 1, "codex"), (11, 10, "bash -lc pytest -x")]);
        assert_eq!(
            job(&direct, 10),
            PaneJob::Running("bash -lc pytest -x".into())
        );
    }

    #[test]
    fn unwraps_claude_codes_commands_and_cuts_long_ones() {
        assert_eq!(
            unwrapped(&claude_code("echo '\"'\"'hi'\"'\"' && sleep 600")),
            "echo 'hi' && sleep 600"
        );
        assert_eq!(
            unwrapped("bash -c -l eval 'npm run e2e' \\< /dev/null && pwd -P >| /tmp/x"),
            "npm run e2e"
        );
        assert_eq!(unwrapped("make -j8"), "make -j8");
        let long = short(&"x".repeat(300));
        assert_eq!(long.chars().count(), MAX_SHOWN + 1);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn reads_ps_output_with_command_lines() {
        let rows = parse_ps(
            "    1     0 /sbin/init splash\n  412     1 /bin/bash -c sleep 5 && echo  done\n 9000   412\n\n garbage\n",
        );
        assert_eq!(
            rows,
            table(&[
                (1, 0, "/sbin/init splash"),
                (412, 1, "/bin/bash -c sleep 5 && echo  done"),
                (9000, 412, ""),
            ])
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_live_table_has_this_process_with_its_command_line() {
        let table = super::table().expect("ps answers");
        let me = std::process::id();
        let row = table
            .iter()
            .find(|row| row.pid == me)
            .expect("this process");
        assert!(!row.command.is_empty());
    }

    #[test]
    fn a_pane_this_manager_does_not_run_is_unknown() {
        let manager = PtyManager::default();
        assert_eq!(pane_job(&manager, "nobody"), PaneJob::Unknown);
    }
}
