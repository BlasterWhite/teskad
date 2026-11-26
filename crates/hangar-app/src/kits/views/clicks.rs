//! The activation dialog clicked as the window shows it (`shell::frame`):
//! over the dialog it was opened from, a workspace's Kits dialog or the
//! settings, each an overlay of the window's stack.
//!
//! iced hands an event to every overlay, the lowest first. The dialog under
//! the activation used to answer a click meant for it: a workspace's Kits
//! dialog, shorter than the activation, took a press on « Accept and enable »
//! (below its own card) as one on its veil and asked to close, which closed the
//! activation instead, with nothing stored. Only the overlay in front takes
//! input now (`layer::inert`): these clicks reach the activation, and only it.
//!
//! The kit is shaped like `ci-status`, the one the bug was met with: a hook
//! that runs by itself, three declared commands and a tab that asks for them.

use iced::mouse;

use hangar_core::kits::StepStatus;

use super::activation_model::StepRun;
use crate::app::Packages;
use crate::kits::testing::{
    OTHER_COMMANDS, OTHER_HOOKS, ci_inspection, ci_status, entry, inspection, kit, tab,
};
use crate::kits::{Activation, Target};
use crate::navigator::testing::{WINDOW, ctx, model_of, simulate, workspace};
use crate::shell::{Overlay, SettingsCategory};
use crate::state::Model;
use crate::state::types::{AppState, WorkspaceKit};

const ACCEPT: &str = "Accept and enable";

/// The window on workspace `w1`, ci-status installed, `kits` being the
/// workspace's own, with these overlays open, the activation last.
fn model(overlays: Vec<Overlay>, kits: Option<Vec<WorkspaceKit>>) -> Model {
    let mut ws = workspace("w1", "Studio", None, Vec::new());
    ws.kits = kits;
    let mut m = model_of(AppState {
        workspaces: vec![ws],
        active_workspace_id: Some("w1".into()),
        ..AppState::default()
    });
    m.live.kits.installed = Some(vec![ci_status()]);
    m.ui.overlays = overlays;
    m
}

/// The activation's overlay, as `start_activation` opens it.
fn activation_overlay(kit: &str) -> Overlay {
    Overlay::KitActivation {
        kit: kit.into(),
        ws: Some("w1".into()),
    }
}

/// The packages with the activation of `kit` open for `target`, read: what
/// the person is shown.
fn opened(kit: &hangar_core::kits::KitInfo, target: Target, read: Activation) -> Packages {
    let mut pkg = Packages::default();
    pkg.kits.activation = Some(Activation {
        kit: kit.id.clone(),
        ws: Some("w1".into()),
        target,
        seen: (
            kit.hooks_permission.clone(),
            kit.commands_permission.clone(),
        ),
        ..read
    });
    pkg
}

/// What the activation has read of ci-status, nothing run yet.
fn ci_read() -> Activation {
    Activation {
        kit: String::new(),
        target: Target::Draft,
        ws: None,
        inspection: Some(ci_inspection()),
        steps: Some(Vec::new()),
        running: None,
        ran: None,
        error: None,
        seen: (None, None),
    }
}

/// The messages of the dialogs among what the window said: those of the
/// packages that draw the overlays, and the intents (closing one).
fn of_dialogs(said: impl Iterator<Item = String>) -> Vec<String> {
    said.filter(|message| {
        ["Kits(", "Settings(", "WorkspaceDialog(", "Intent("]
            .iter()
            .any(|package| message.starts_with(package))
    })
    .collect()
}

/// What a click on the button that says `label` says, in the window.
fn click(m: &Model, pkg: &Packages, label: &str) -> Vec<String> {
    let mut ui = simulate(crate::shell::frame::main(ctx(m, pkg)));
    ui.click(label).expect("the button is on screen");
    of_dialogs(ui.into_messages().map(|message| format!("{message:?}")))
}

#[test]
fn accept_over_a_workspace_s_kits_dialog_is_the_activation_s() {
    let m = model(
        vec![
            Overlay::KitsDialog("w1".into()),
            activation_overlay("ci-status"),
        ],
        None,
    );
    let pkg = opened(&ci_status(), Target::Workspace("w1".into()), ci_read());
    // Not the Kits dialog's veil asking to close: that closed the activation
    // in front of it, and nothing was stored.
    assert_eq!(click(&m, &pkg, ACCEPT), ["Kits(Accept)"]);
}

#[test]
fn accepting_what_an_update_changed_over_the_kits_dialog_is_the_activation_s() {
    // Enabled before an update changed its hook and its commands: « Review ».
    let before = entry(
        "ci-status",
        &[
            "notify",
            "command:ci-list",
            "command:ci-log",
            "command:ci-run",
            OTHER_HOOKS,
            OTHER_COMMANDS,
        ],
    );
    let m = model(
        vec![
            Overlay::KitsDialog("w1".into()),
            activation_overlay("ci-status"),
        ],
        Some(vec![before]),
    );
    let pkg = opened(&ci_status(), Target::Workspace("w1".into()), ci_read());
    assert_eq!(click(&m, &pkg, "Accept what changed"), ["Kits(Accept)"]);
}

#[test]
fn accept_over_the_settings_is_the_activation_s() {
    // « Enable on all workspaces »: the activation over Settings › Kits,
    // whose own buttons and switches lie under it.
    let m = model(
        vec![
            Overlay::Settings(SettingsCategory::Kits),
            activation_overlay("ci-status"),
        ],
        None,
    );
    let pkg = opened(&ci_status(), Target::Everywhere, ci_read());
    assert_eq!(click(&m, &pkg, ACCEPT), ["Kits(Accept)"]);
}

#[test]
fn a_setup_step_s_answer_in_the_docked_card_keeps_the_activation_open() {
    // A kit that asks for a setup step first; the step ran, and the
    // activation has stepped aside into the window's corner to ask how it
    // went, over the Kits dialog it was opened from.
    let setup_kit = kit("1.0.0", vec![tab(&["notify"])]);
    let mut m = model(
        vec![
            Overlay::KitsDialog("w1".into()),
            activation_overlay(&setup_kit.id),
        ],
        None,
    );
    m.live.kits.installed = Some(vec![setup_kit.clone()]);
    let pkg = opened(
        &setup_kit,
        Target::Workspace("w1".into()),
        Activation {
            inspection: Some(inspection(vec![tab(&["notify"])])),
            steps: Some(vec![StepStatus {
                id: "login".into(),
                title: "Sign in".into(),
                description: None,
                command: "gh auth login".into(),
                sha256: "0".repeat(64),
                done: false,
            }]),
            ran: Some(StepRun {
                step: "login".into(),
                pane_name: "Shell".into(),
            }),
            ..ci_read()
        },
    );

    let mut ui = simulate(crate::shell::frame::main(ctx(&m, &pkg)));
    // The card's body scrolls: down to its foot, where the step's question is.
    ui.point_at(iced::Point::new(
        WINDOW.width - 250.0,
        WINDOW.height - 200.0,
    ));
    ui.simulate([iced::Event::Mouse(mouse::Event::WheelScrolled {
        delta: mouse::ScrollDelta::Lines { x: 0.0, y: -100.0 },
    })]);
    ui.click("It worked, mark it done")
        .expect("the step's question is on screen");
    let said = of_dialogs(ui.into_messages().map(|message| format!("{message:?}")));
    // Not also the Kits dialog's veil asking to close, which closed the
    // activation before it could be accepted.
    assert_eq!(said, [r#"Kits(Confirm("login"))"#]);
}

/// What a click on the button that says `label` says, the dialog's body
/// scrolled down until it is on screen: what reaches Codex sits below what
/// the kit brings.
fn click_below(m: &Model, pkg: &Packages, label: &str) -> Vec<String> {
    let mut ui = simulate(crate::shell::frame::main(ctx(m, pkg)));
    ui.point_at(iced::Point::new(WINDOW.width / 2.0, WINDOW.height / 2.0));
    for _ in 0..20 {
        if ui.click(label).is_ok() {
            return of_dialogs(ui.into_messages().map(|message| format!("{message:?}")));
        }
        ui.simulate([iced::Event::Mouse(mouse::Event::WheelScrolled {
            delta: mouse::ScrollDelta::Pixels { x: 0.0, y: -60.0 },
        })]);
    }
    panic!("« {label} » never came on screen");
}

/// What reaches Codex of ci-status: one skill, once added.
fn codex_read() -> Activation {
    use hangar_core::agent::{AgentKind, KitPart};
    use hangar_core::kits::{AgentParts, Inspection, Part, PluginInfo};

    fn part<T>(how: KitPart, loads: Vec<T>) -> Part<T> {
        Part {
            how,
            loads,
            left_out: Vec::new(),
        }
    }
    Activation {
        inspection: Some(Inspection {
            plugin: Some(PluginInfo {
                name: "ci-status".into(),
                dir: "C:/k".into(),
            }),
            by_agent: vec![AgentParts {
                agent: AgentKind::Codex,
                meant_for: true,
                mcp_servers: part(KitPart::PerLaunch, Vec::new()),
                skills: part(KitPart::Installed, vec!["ci".into()]),
                hooks: part(KitPart::Installed, Vec::new()),
                commands: part(KitPart::None, Vec::new()),
                agents: part(KitPart::None, Vec::new()),
                output_styles: part(KitPart::None, Vec::new()),
            }],
            ..ci_inspection()
        }),
        ..ci_read()
    }
}

#[test]
fn add_to_codex_asks_first_and_the_confirmation_runs_it() {
    use hangar_core::kits::{CodexPlugin, CodexState};

    use crate::kits::codex::{CodexEntry, CodexOp};

    let m = model(
        vec![
            Overlay::KitsDialog("w1".into()),
            activation_overlay("ci-status"),
        ],
        None,
    );
    let mut pkg = opened(&ci_status(), Target::Workspace("w1".into()), codex_read());
    pkg.kits.codex.insert(
        "ci-status".into(),
        CodexEntry {
            status: Some(CodexPlugin {
                state: CodexState::NotAdded,
                plugin: "ci-status@hangar-kits".into(),
                skills: vec!["ci".into()],
                hooks: Vec::new(),
                codex: true,
                add: vec!["codex plugin add ci-status@hangar-kits".into()],
                remove: vec!["codex plugin remove ci-status@hangar-kits".into()],
            }),
            ..CodexEntry::default()
        },
    );
    assert_eq!(
        click_below(&m, &pkg, "Add to Codex"),
        [r#"Kits(CodexAsk { kit: "ci-status", op: Some(Add) })"#]
    );

    pkg.kits.codex.get_mut("ci-status").unwrap().asking = Some(CodexOp::Add);
    assert_eq!(
        click_below(&m, &pkg, "Add to Codex"),
        [r#"Kits(CodexRun { kit: "ci-status", op: Add })"#]
    );
    assert_eq!(
        click_below(&m, &pkg, "Cancel"),
        [r#"Kits(CodexAsk { kit: "ci-status", op: None })"#]
    );
}
