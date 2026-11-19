//! The `delegate`, `delegation_status`, `delegation_cancel` and
//! `delegation_list` control operations: an agent handing a sub-task to
//! another agent that runs with no terminal, and following it
//! (docs/native/agents.md §5.2, D14).
//!
//! The delegations are core's (`hangar_core::delegate`): its registry starts
//! the child, follows it, holds what it asks the person, and queues an event
//! to the caller's pane when it ends. What is decided here is what the window
//! knows and core does not:
//!
//! - **The parent** is the caller's pane, as core's API stamped it from the
//!   pane's own token, never one the model names: the pane the event goes
//!   to. An agent Hangar did not start has no pane to hear back in.
//! - **The child's agent** is the one asked for, else the caller's own. One
//!   whose capabilities leave no way to run it with no terminal here
//!   ([`takes_a_delegation`]), or to make the images asked for, is refused
//!   with the reason, never replaced by another.
//! - **What it runs under**: the folder of the caller's pane (a worker's
//!   worktree), the workspace's orchestration policy, the account the
//!   workspace runs that agent on, the settings' launch command.
//! - **Who sees what**: a delegation is its workspace's. Any of its panes
//!   lists and reads it; the pane that asked for it, or the workspace's
//!   orchestrator, cancels it.
//!
//! Like the rest of the control ops, what is refused is read by a model:
//! English, saying what to do next, held word for word by the tests.

use std::time::Duration;

use serde_json::{Map, Value, json};

use hangar_core::agent::Capabilities;
use hangar_core::delegate::{self as registry, Delegation, Isolation, Request, Status, WAIT_MAX};

use super::port::Port;
use super::run::{agent_arg, effective_cwd, number_arg, unknown};
use super::types::{Caller, text};
use crate::accounts::{AccountLevels, MAIN_ACCOUNT};
use crate::panes::agents::permission_mode;
use crate::state::roles::{orchestration_of, orchestrator_of};
use crate::state::types::{AgentKind, AppState, Pane, Workspace};

/// The operations this module answers.
pub const OPERATIONS: [&str; 4] = [
    "delegate",
    "delegation_status",
    "delegation_cancel",
    "delegation_list",
];

/// A request that comes from no pane: there is nowhere to send the result.
pub const NO_PANE: &str = "delegate hands the result back to the pane that asked for it, and this call comes from no pane of Hangar's. Call it from an agent Hangar runs in a pane.";

/// A delegation with no brief.
pub const NO_PROMPT: &str =
    "Give prompt: the sub-task's whole brief. The agent that takes it sees nothing else of your work.";

/// A window that cannot run a child: a node's panes, asking the desktop.
pub const NO_HOST: &str = "Delegations cannot be started from this Hangar window.";

/// `delegation_status` or `delegation_cancel` without the delegation.
pub const NO_ID: &str =
    "Give id: the delegation's, as delegate or delegation_list gave it (d-3).";

/// What `delegate` adds to its answer: the child runs, and how it comes back.
pub const STARTED: &str = "It runs with no terminal. When it ends, a message from Hangar brings you its result: carry on with your work, or call delegation_status with wait to wait for it here.";

/// What `delegation_status` adds when the wait ran out.
pub const STILL_RUNNING: &str = "Still running after the wait. Call delegation_status again with wait, or carry on: a message from Hangar reaches you when it ends.";

/// What `delegation_status` adds while its child waits on the person.
pub const NEEDS_YOU: &str = "Its agent waits for the person: an approval or a question, which only the person answers, in Hangar. It carries on once they have.";

/// What `delegation_cancel` adds for a delegation that had ended already.
pub const ALREADY_OVER: &str = "It had already ended: nothing was stopped.";

/// What `delegation_cancel` adds when its child is still stopping.
pub const STILL_STOPPING: &str =
    "It is still stopping: a message from Hangar reaches you once it has.";

/// One of the delegation operations, for the caller in workspace `ws`.
pub async fn control(
    op: &str,
    ws: &Workspace,
    args: &Value,
    caller: &Caller,
    port: &Port,
) -> Result<Value, String> {
    match op {
        "delegate" => {
            let snapshot = port.snapshot();
            let pane = caller_pane(ws, caller)?;
            let request = request(&snapshot.state, ws, pane, args)?;
            let host = port.core().ok_or_else(|| NO_HOST.to_string())?;
            delegate(&host, &snapshot.state, ws, request).await
        }
        "delegation_status" => status(ws, args).await,
        "delegation_cancel" => cancel(ws, args, caller).await,
        "delegation_list" => Ok(list(ws)),
        _ => Err(unknown(op)),
    }
}

/// The caller's own pane in `ws`: the delegation's parent.
fn caller_pane<'a>(ws: &'a Workspace, caller: &Caller) -> Result<&'a Pane, String> {
    let id = caller.pane_id.as_deref().ok_or(NO_PANE)?;
    ws.panes
        .iter()
        .find(|pane| pane.id == id)
        .ok_or_else(|| NO_PANE.to_string())
}

/// What the caller's pane asks core to run, checked: everything but the
/// account, which [`delegate`] reads from the disk.
pub fn request(
    state: &AppState,
    ws: &Workspace,
    pane: &Pane,
    args: &Value,
) -> Result<Request, String> {
    let task = args
        .get("prompt")
        .and_then(Value::as_str)
        .filter(|prompt| !prompt.trim().is_empty())
        .ok_or(NO_PROMPT)?
        .to_string();
    let kind = match agent_arg(args.get("agent"))? {
        Some(agent) => agent.kind().ok_or_else(|| {
            format!(
                "A delegation is run by an agent, not a plain shell. Pass agent as one of: {}.",
                agent_ids()
            )
        })?,
        None => pane.agent.kind().ok_or_else(|| {
            format!(
                "Your pane runs a plain shell, so there is no agent of yours to run it: pass agent as one of {}.",
                agent_ids()
            )
        })?,
    };
    let images = args.get("images").and_then(Value::as_bool).unwrap_or(false);
    let info = kind.info();
    if let Some(refusal) = child_refusal(info.label, &info.caps, images) {
        return Err(refusal);
    }
    Ok(Request {
        parent: pane.id.clone(),
        cwd: effective_cwd(ws, pane),
        task,
        kind,
        model: text(args.get("model")),
        isolation: isolation_arg(args.get("isolation"))?,
        images,
        permission_mode: orchestration_of(ws)
            .permission_mode
            .as_deref()
            .and_then(permission_mode),
        account: None,
        command: state.settings.agent_commands.of(kind).map(str::to_string),
    })
}

/// Whether an agent that can do `caps` takes a delegation here: core runs a
/// child as a thread of a protocol server Hangar is the client of
/// (`Headless::app_server`). A one-shot child (`Headless::one_shot`) waits
/// for its runner (T11b).
pub fn takes_a_delegation(caps: &Capabilities) -> bool {
    caps.headless.app_server
}

/// Why an agent called `label`, that can do `caps`, cannot take this
/// delegation; `None` when it can.
pub fn child_refusal(label: &str, caps: &Capabilities, images: bool) -> Option<String> {
    if !takes_a_delegation(caps) {
        return Some(format!(
            "Hangar cannot run a delegation on {label} yet: it runs one with no terminal, as a thread of the agent's own server, and {label} has none. Agents that take one here: {}. Pass agent as one of them, or give the work to a worker.",
            able(|caps| takes_a_delegation(caps))
        ));
    }
    if images && !caps.images.generates {
        return Some(format!(
            "{label} does not make images, so images: true cannot be given to it. Agents that make them here: {}.",
            able(|caps| takes_a_delegation(caps) && caps.images.generates)
        ));
    }
    None
}

/// The agents whose capabilities pass `can`, as `codex (Codex)`; `none`.
fn able(can: impl Fn(&Capabilities) -> bool) -> String {
    let names: Vec<String> = AgentKind::ALL
        .iter()
        .filter(|kind| can(&kind.info().caps))
        .map(|kind| format!("{} ({})", kind.id(), kind.info().label))
        .collect();
    if names.is_empty() {
        "none".into()
    } else {
        names.join(", ")
    }
}

/// Every agent's id, for a refusal.
fn agent_ids() -> String {
    AgentKind::ALL
        .iter()
        .map(|kind| kind.id())
        .collect::<Vec<_>>()
        .join(", ")
}

/// `isolation` as given: absent is shared.
fn isolation_arg(value: Option<&Value>) -> Result<Isolation, String> {
    match text(value).as_deref() {
        None | Some("shared") => Ok(Isolation::Shared),
        Some("worktree") => Ok(Isolation::Worktree),
        Some(other) => Err(format!(
            "Unknown isolation \"{other}\". One of: shared, worktree."
        )),
    }
}

/// Starts `request` on core's registry, on the account the workspace runs
/// its agent on.
async fn delegate(
    host: &hangar_core::Host,
    state: &AppState,
    ws: &Workspace,
    mut request: Request,
) -> Result<Value, String> {
    request.account = account(state, ws, request.kind).await?;
    let started = registry::start(host, request)
        .await
        .map_err(|why| format!("Nothing was started: {}", sentence(&why)))?;
    let mut answer = view(ws, &started, true);
    let mut note = STARTED.to_string();
    if let Some(branch) = &started.branch {
        note.push_str(&format!(
            " It works in {}, on branch {branch}, made from your last commit: what you had not committed is not there.",
            started.cwd
        ));
    }
    answer["note"] = json!(note);
    Ok(answer)
}

/// The account of `kind` the workspace runs on (workspace, folder, default);
/// `None` for main.
async fn account(
    state: &AppState,
    ws: &Workspace,
    kind: AgentKind,
) -> Result<Option<String>, String> {
    let levels = AccountLevels::of_kind(state, ws, None, kind);
    let resolved = tokio::task::spawn_blocking(move || levels.launch_of(kind))
        .await
        .map_err(|error| format!("Hangar could not read the accounts: {error}"))?;
    Ok((resolved.id != MAIN_ACCOUNT).then_some(resolved.id))
}

/// `delegation_status`: where it is, after waiting for it to end when asked.
async fn status(ws: &Workspace, args: &Value) -> Result<Value, String> {
    let id = id_arg(args)?;
    here(ws, &id)?;
    let wait = number_arg(args.get("wait"))
        .filter(|seconds| *seconds > 0.0)
        .map(|seconds| Duration::from_secs_f64(seconds.min(WAIT_MAX.as_secs_f64())));
    let polled = registry::status(&id, wait)
        .await
        .ok_or_else(|| not_here(ws, &id))?;
    let delegation = polled.delegation;
    let mut answer = view(ws, &delegation, true);
    if polled.waited_out {
        answer["waitedOut"] = json!(true);
        answer["note"] = json!(STILL_RUNNING);
    } else if delegation.status == Status::NeedsYou {
        answer["note"] = json!(NEEDS_YOU);
    }
    Ok(answer)
}

/// `delegation_cancel`: by the pane that asked for it, or the workspace's
/// orchestrator.
async fn cancel(ws: &Workspace, args: &Value, caller: &Caller) -> Result<Value, String> {
    let id = id_arg(args)?;
    let delegation = here(ws, &id)?;
    let me = caller.pane_id.as_deref();
    let parent = me == Some(delegation.parent.as_str());
    let orchestrator = me.is_some() && orchestrator_of(ws).map(|pane| pane.id.as_str()) == me;
    if !parent && !orchestrator {
        let owner = parent_name(ws, &delegation.parent);
        return Err(format!(
            "Delegation {id} is {owner}'s: only {owner}, or the workspace's orchestrator, can cancel it."
        ));
    }
    let was_over = delegation.status.is_over();
    let ended = registry::cancel(&id).await.map_err(|why| sentence(&why))?;
    let mut answer = view(ws, &ended, true);
    if was_over {
        answer["note"] = json!(ALREADY_OVER);
    } else if !ended.status.is_over() {
        answer["note"] = json!(STILL_STOPPING);
    } else if let Some(branch) = ended.branch.as_deref().filter(|_| ended.status == Status::Cancelled)
    {
        answer["note"] = json!(format!(
            "Its worktree holds commits, so it was kept: branch {branch}, in {}.",
            ended.cwd
        ));
    }
    Ok(answer)
}

/// `delegation_list`: the workspace's, oldest first.
fn list(ws: &Workspace) -> Value {
    let delegations: Vec<Value> = registry::list()
        .iter()
        .filter(|delegation| of_workspace(ws, delegation))
        .map(|delegation| view(ws, delegation, false))
        .collect();
    json!({ "delegations": delegations })
}

/// The delegation an operation names: `d-3`, or `3`.
fn id_arg(args: &Value) -> Result<String, String> {
    let id = text(args.get("id")).ok_or(NO_ID)?;
    Ok(if id.parse::<u64>().is_ok() {
        format!("d-{id}")
    } else {
        id
    })
}

/// Delegation `id`, when it is one of `ws`'s.
fn here(ws: &Workspace, id: &str) -> Result<Delegation, String> {
    registry::list()
        .into_iter()
        .find(|delegation| delegation.id == id && of_workspace(ws, delegation))
        .ok_or_else(|| not_here(ws, id))
}

fn not_here(ws: &Workspace, id: &str) -> String {
    format!(
        "No delegation {id} in \"{}\". delegation_list lists the workspace's.",
        ws.name
    )
}

/// Whether a pane of `ws` asked for it.
fn of_workspace(ws: &Workspace, delegation: &Delegation) -> bool {
    ws.panes.iter().any(|pane| pane.id == delegation.parent)
}

/// The parent's pane name, as the model knows panes; its id once it is gone.
fn parent_name(ws: &Workspace, parent: &str) -> String {
    ws.panes
        .iter()
        .find(|pane| pane.id == parent)
        .map_or_else(|| parent.to_string(), |pane| pane.name.clone())
}

/// A delegation as the model reads it: what it has, nothing empty. `whole`
/// adds what it did (its result, files, images, tokens), which a listing
/// leaves to `delegation_status`.
fn view(ws: &Workspace, delegation: &Delegation, whole: bool) -> Value {
    let mut view = Map::new();
    let mut put = |key: &str, value: Value| {
        view.insert(key.to_string(), value);
    };
    put("id", json!(delegation.id));
    put("status", json!(delegation.status));
    put("agent", json!(delegation.kind.id()));
    if let Some(model) = &delegation.model {
        put("model", json!(model));
    }
    put("parent", json!(parent_name(ws, &delegation.parent)));
    put("title", json!(delegation.title));
    put("isolation", json!(delegation.isolation));
    if let Some(branch) = &delegation.branch {
        put("branch", json!(branch));
    }
    put("started", json!(delegation.started));
    if let Some(ended) = delegation.ended {
        put("ended", json!(ended));
    }
    if let Some(error) = &delegation.error {
        put("error", json!(error));
    }
    if whole {
        put("cwd", json!(delegation.cwd));
        if let Some(tokens) = &delegation.tokens {
            put("tokens", json!(tokens));
        }
        if let Some(result) = &delegation.result {
            put("result", json!(result));
        }
        if !delegation.files.is_empty() {
            put("files", json!(delegation.files));
        }
        if !delegation.images.is_empty() {
            put("images", json!(delegation.images));
        }
    }
    Value::Object(view)
}

/// `why` as a sentence: its first letter as it is, a full stop at its end.
fn sentence(why: &str) -> String {
    let why = why.trim();
    if why.ends_with(['.', '!', '?']) {
        why.to_string()
    } else {
        format!("{why}.")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::control::fake::{Fake, pane, workspace};
    use crate::control::run::{Gate, run};
    use crate::state::types::{AgentId, OrchestrationSettings, PaneRole};
    use hangar_core::agent::PermissionMode;

    /// "Lab": Ava (Codex) in a worktree of her own, Ivy (Claude Code) its
    /// orchestrator, Max (Codex) a worker beside her. "Other": Zoe (Codex).
    fn lab(cwd: &str) -> AppState {
        let mut ivy = pane("t17-ivy", "Ivy", AgentId::Claude, None);
        ivy.role = Some(PaneRole::Orchestrator);
        let mut state = AppState {
            workspaces: vec![
                workspace(
                    "t17-lab",
                    "Lab",
                    cwd,
                    vec![
                        pane("t17-ava", "Ava", AgentId::Codex, Some(cwd)),
                        ivy,
                        pane("t17-max", "Max", AgentId::Codex, None),
                    ],
                ),
                workspace(
                    "t17-other",
                    "Other",
                    cwd,
                    vec![pane("t17-zoe", "Zoe", AgentId::Codex, None)],
                ),
            ],
            active_workspace_id: Some("t17-lab".into()),
            ..AppState::default()
        };
        state.workspaces[0].orchestration = Some(OrchestrationSettings {
            permission_mode: Some("acceptEdits".into()),
            ..OrchestrationSettings::default()
        });
        state
    }

    fn from(pane: &str) -> Value {
        json!({ "caller": { "paneId": pane } })
    }

    fn with(base: Value, extra: Value) -> Value {
        let (Value::Object(mut base), Value::Object(extra)) = (base, extra) else {
            panic!("objects only");
        };
        base.extend(extra);
        Value::Object(base)
    }

    async fn op(name: &str, args: Value, fake: &Arc<Fake>) -> Result<Value, String> {
        run(name, args, &fake.port(), Gate::Agent).await
    }

    fn lab_ws(state: &AppState) -> &Workspace {
        &state.workspaces[0]
    }

    fn pane_of<'a>(state: &'a AppState, id: &str) -> &'a Pane {
        lab_ws(state)
            .panes
            .iter()
            .find(|pane| pane.id == id)
            .unwrap()
    }

    #[test]
    fn a_delegation_reports_to_the_caller_s_pane_and_works_in_its_folder() {
        let mut state = lab("/srv/lab");
        state.workspaces[0].panes[0].cwd = Some("/srv/lab-wt/ava".into());
        state.settings.agent_commands.codex = Some("codex --oss".into());
        let ava = pane_of(&state, "t17-ava");
        let request = request(
            &state,
            lab_ws(&state),
            ava,
            &json!({
                "prompt": "Generate the five icons.\nSee docs/icons.md.",
                "model": " gpt-5.5 ",
                "isolation": "worktree",
                "images": true,
            }),
        )
        .unwrap();
        assert_eq!(
            request,
            Request {
                parent: "t17-ava".into(),
                cwd: "/srv/lab-wt/ava".into(),
                task: "Generate the five icons.\nSee docs/icons.md.".into(),
                kind: AgentKind::Codex,
                model: Some("gpt-5.5".into()),
                isolation: Isolation::Worktree,
                images: true,
                permission_mode: Some(PermissionMode::AcceptEdits),
                account: None,
                command: Some("codex --oss".into()),
            }
        );
    }

    #[test]
    fn the_child_runs_the_caller_s_agent_unless_another_is_named() {
        let state = lab("/srv/lab");
        let max = pane_of(&state, "t17-max");
        let mine = request(&state, lab_ws(&state), max, &json!({ "prompt": "x" })).unwrap();
        assert_eq!(mine.kind, AgentKind::Codex);
        assert_eq!(mine.isolation, Isolation::Shared);
        assert!(!mine.images);
        // Max has no folder of his own: the workspace's.
        assert_eq!(mine.cwd, "/srv/lab");
        let ivy = pane_of(&state, "t17-ivy");
        let named = request(
            &state,
            lab_ws(&state),
            ivy,
            &json!({ "prompt": "x", "agent": "codex", "model": "" }),
        )
        .unwrap();
        assert_eq!(named.kind, AgentKind::Codex);
        assert_eq!(named.parent, "t17-ivy");
        assert_eq!(named.model, None);
    }

    /// By what each agent can do, never by its name: every agent without a
    /// way to run with no terminal here is refused, every other one taken.
    #[test]
    fn an_agent_that_cannot_run_without_a_terminal_here_is_refused_with_the_reason() {
        let state = lab("/srv/lab");
        let max = pane_of(&state, "t17-max");
        for kind in AgentKind::ALL {
            let asked = request(
                &state,
                lab_ws(&state),
                max,
                &json!({ "prompt": "x", "agent": kind.id() }),
            );
            if takes_a_delegation(&kind.info().caps) {
                assert_eq!(asked.unwrap().kind, kind);
            } else {
                let refusal = asked.unwrap_err();
                assert!(
                    refusal.starts_with(&format!(
                        "Hangar cannot run a delegation on {} yet:",
                        kind.info().label
                    )),
                    "{refusal}"
                );
            }
        }
    }

    /// Claude Code's children wait for their runner (T11b): an orchestrator
    /// of that agent asking with no agent is told so, and what it can ask.
    #[test]
    fn a_caller_whose_own_agent_cannot_take_one_is_told_which_can() {
        let state = lab("/srv/lab");
        let ivy = pane_of(&state, "t17-ivy");
        assert_eq!(
            request(&state, lab_ws(&state), ivy, &json!({ "prompt": "x" })).unwrap_err(),
            "Hangar cannot run a delegation on Claude Code yet: it runs one with no terminal, as a thread of the agent's own server, and Claude Code has none. Agents that take one here: codex (Codex). Pass agent as one of them, or give the work to a worker."
        );
    }

    #[test]
    fn images_are_asked_of_an_agent_that_makes_them() {
        let codex = AgentKind::Codex.info();
        assert_eq!(child_refusal(codex.label, &codex.caps, true), None);
        let mut caps = codex.caps;
        caps.images.generates = false;
        assert_eq!(
            child_refusal("Painter", &caps, true).unwrap(),
            "Painter does not make images, so images: true cannot be given to it. Agents that make them here: codex (Codex)."
        );
        assert_eq!(child_refusal("Painter", &caps, false), None);
    }

    #[test]
    fn what_a_delegation_cannot_be_is_refused_before_anything_runs() {
        let state = lab("/srv/lab");
        let (ws, max) = (lab_ws(&state), pane_of(&state, "t17-max"));
        let refused = |args: Value| request(&state, ws, max, &args).unwrap_err();
        assert_eq!(refused(json!({})), NO_PROMPT);
        assert_eq!(refused(json!({ "prompt": "  \n " })), NO_PROMPT);
        assert_eq!(
            refused(json!({ "prompt": "x", "agent": "shell" })),
            "A delegation is run by an agent, not a plain shell. Pass agent as one of: claude, codex."
        );
        assert_eq!(
            refused(json!({ "prompt": "x", "agent": "gemini" })),
            "Unknown agent \"gemini\". One of: claude, codex, shell."
        );
        assert_eq!(
            refused(json!({ "prompt": "x", "isolation": "container" })),
            "Unknown isolation \"container\". One of: shared, worktree."
        );
        let mut shell = max.clone();
        shell.agent = AgentId::Shell;
        assert_eq!(
            request(&state, ws, &shell, &json!({ "prompt": "x" })).unwrap_err(),
            "Your pane runs a plain shell, so there is no agent of yours to run it: pass agent as one of claude, codex."
        );
    }

    #[tokio::test]
    async fn only_a_pane_of_hangar_s_can_delegate_and_only_where_a_child_can_run() {
        let fake = Fake::with_state(lab("/srv/lab"));
        let outside = json!({ "caller": { "cwd": "/srv/lab" }, "prompt": "x" });
        assert_eq!(op("delegate", outside, &fake).await.unwrap_err(), NO_PANE);
        // Checked, then refused: the fake window runs no child.
        let asked = with(from("t17-max"), json!({ "prompt": "x" }));
        assert_eq!(op("delegate", asked, &fake).await.unwrap_err(), NO_HOST);
    }

    #[tokio::test]
    async fn status_and_cancel_name_a_delegation_of_the_workspace() {
        let fake = Fake::with_state(lab("/srv/lab"));
        assert_eq!(
            op("delegation_status", from("t17-max"), &fake)
                .await
                .unwrap_err(),
            NO_ID
        );
        assert_eq!(
            op(
                "delegation_cancel",
                with(from("t17-max"), json!({ "id": "d-999999" })),
                &fake
            )
            .await
            .unwrap_err(),
            "No delegation d-999999 in \"Lab\". delegation_list lists the workspace's."
        );
        assert_eq!(
            op(
                "delegation_status",
                with(from("t17-max"), json!({ "id": 999998 })),
                &fake
            )
            .await
            .unwrap_err(),
            NO_ID
        );
    }

    #[test]
    fn an_id_may_be_given_as_its_number() {
        assert_eq!(id_arg(&json!({ "id": " 12 " })).unwrap(), "d-12");
        assert_eq!(id_arg(&json!({ "id": "d-12" })).unwrap(), "d-12");
        assert_eq!(id_arg(&json!({ "id": "" })).unwrap_err(), NO_ID);
    }

    #[test]
    fn a_reason_reads_as_a_sentence() {
        assert_eq!(sentence(" a delegation needs a task "), "a delegation needs a task.");
        assert_eq!(sentence("It stopped."), "It stopped.");
    }

    /// Through core's registry for real: a child whose agent cannot be
    /// started fails, the caller's pane hears of it, and the workspace sees
    /// it where another does not.
    #[tokio::test]
    async fn a_delegation_runs_on_core_s_registry_and_its_end_reaches_the_caller() {
        let root = tempfile::tempdir().unwrap();
        let cwd = root.path().to_string_lossy().into_owned();
        let mut state = lab(&cwd);
        let missing = root.path().join("no-such-agent").join("codex");
        state.settings.agent_commands.codex = Some(missing.to_string_lossy().into_owned());
        let host = hangar_core::Host::with_paths(hangar_core::paths::Paths::rooted(root.path()));

        let ws = lab_ws(&state).clone();
        let ava = pane_of(&state, "t17-ava").clone();
        let asked = request(
            &state,
            &ws,
            &ava,
            &json!({ "prompt": "Answer a question about the repo." }),
        )
        .unwrap();
        let started = delegate(&host, &state, &ws, asked).await.unwrap();
        let id = started["id"].as_str().unwrap().to_string();
        assert!(id.starts_with("d-"), "{started}");
        assert_eq!(started["agent"], "codex");
        assert_eq!(started["parent"], "Ava");
        assert_eq!(started["title"], "Answer a question about the repo.");
        assert_eq!(started["isolation"], "shared");
        assert_eq!(started["cwd"], json!(cwd));
        assert_eq!(started["note"], STARTED);

        let fake = Fake::with_state(state.clone());
        let polled = op(
            "delegation_status",
            with(from("t17-max"), json!({ "id": id, "wait": 30 })),
            &fake,
        )
        .await
        .unwrap();
        assert_eq!(polled["status"], "failed", "{polled}");
        assert!(polled["error"].as_str().is_some_and(|error| !error.is_empty()));
        assert!(polled.get("waitedOut").is_none(), "{polled}");

        let heard = hangar_core::bridge::bridge_queue("t17-ava".into());
        let event = heard
            .iter()
            .find(|queued| queued.text.starts_with(&format!("Delegation {id} (codex) failed:")))
            .unwrap_or_else(|| panic!("nothing about {id} in {heard:?}"));
        assert_eq!(event.from.as_deref(), Some("hangar"));

        let listed = op("delegation_list", from("t17-ivy"), &fake).await.unwrap();
        let mine = listed["delegations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|delegation| delegation["id"] == json!(id))
            .unwrap_or_else(|| panic!("{id} is not in {listed}"));
        assert_eq!(mine["status"], "failed");
        assert_eq!(mine["parent"], "Ava");
        // A listing leaves what it did to delegation_status.
        assert!(mine.get("cwd").is_none(), "{mine}");
        let elsewhere = op("delegation_list", from("t17-zoe"), &fake).await.unwrap();
        assert!(
            elsewhere["delegations"]
                .as_array()
                .unwrap()
                .iter()
                .all(|delegation| delegation["id"] != json!(id)),
            "{elsewhere}"
        );
        assert_eq!(
            op(
                "delegation_status",
                with(from("t17-zoe"), json!({ "id": id })),
                &fake
            )
            .await
            .unwrap_err(),
            format!("No delegation {id} in \"Other\". delegation_list lists the workspace's.")
        );

        let by = |pane: &str| with(from(pane), json!({ "id": id }));
        assert_eq!(
            op("delegation_cancel", by("t17-max"), &fake)
                .await
                .unwrap_err(),
            format!("Delegation {id} is Ava's: only Ava, or the workspace's orchestrator, can cancel it.")
        );
        for pane in ["t17-ava", "t17-ivy"] {
            let cancelled = op("delegation_cancel", by(pane), &fake).await.unwrap();
            assert_eq!(cancelled["status"], "failed", "{cancelled}");
            assert_eq!(cancelled["note"], ALREADY_OVER);
        }
    }
}
