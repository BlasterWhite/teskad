//! The agents' setup: when it shows and when it is satisfied (by either
//! agent), what « shells only » does, the installers it runs, what follows an
//! install, signing in, the sheet drawn, and NixOS, where Claude Code's card
//! offers nixpkgs instead.
//!
//! `TESKAD_P8_SNAPSHOTS=<folder>` writes the sheet's pictures there, as the
//! new-workspace dialog's tests do (workspace_dialog/testing.rs).

use std::path::PathBuf;

use hangar_i18n::Locale;
use iced::futures::StreamExt;

use super::install::{self, Installer, Os};
use super::nixos;
use super::*;
use crate::message::Message;
use crate::shell::Action;
use crate::workspace_dialog::testing::Harness;

/// A window whose detection found neither agent, on a system other than
/// NixOS (whatever the machine running the tests is).
fn missing(locale: Locale) -> (Harness, State) {
    let mut h = Harness::new(locale, "graphite");
    h.model.ui.claude_installed = false;
    h.model.ui.codex_installed = false;
    let st = State {
        detected: true,
        nixos: false,
        ..State::default()
    };
    (h, st)
}

/// The same window, on NixOS.
fn missing_on_nixos(locale: Locale) -> (Harness, State) {
    let (h, mut st) = missing(locale);
    st.nixos = true;
    (h, st)
}

/// A window with Codex alone, found by a look that says its version, its
/// account not signed in yet.
fn codex_only(locale: Locale) -> (Harness, State) {
    let (mut h, mut st) = missing(locale);
    h.model.ui.codex_installed = true;
    st.codex.found = Some(found("/home/me/.local/bin/codex", "0.170.0"));
    st.codex_signed_in = Some(false);
    (h, st)
}

/// What a look finds of an agent on PATH that answered `--version`.
fn found(path: &str, version: &str) -> Detected {
    Detected {
        path: PathBuf::from(path),
        on_path: true,
        version_line: Some(format!("cli {version}")),
        version: Some(version.to_string()),
    }
}

/// A look that found these agents, Codex signed in or not.
fn look(claude: bool, codex: bool, codex_signed_in: bool) -> Look {
    Look {
        claude: claude.then(|| found("/home/me/.local/bin/claude", "2.1.3")),
        codex: codex.then(|| found("/home/me/.local/bin/codex", "0.170.0")),
        codex_signed_in,
    }
}

fn shells_only_on(h: &mut Harness) {
    h.model.apply(StoreAction::UpdateSettings(Patch::new(|s| {
        crate::settings::set_shells_only(s, true)
    })));
}

fn sent(messages: &[Message], wanted: impl Fn(&Msg) -> bool) -> bool {
    messages.iter().any(|m| match m {
        Message::Shell(Action::Setup(msg)) => wanted(msg),
        _ => false,
    })
}

// ── when it shows ─────────────────────────────────────────────────────

#[test]
fn no_agent_at_all_shows_the_setup_at_start() {
    let (h, st) = missing(Locale::En);
    assert!(needed(&st, &h.model));
    assert!(matches!(
        offer_at_start(&st, &h.model),
        Some(Intent::OpenOverlay(Overlay::AgentSetup))
    ));
}

#[test]
fn claude_code_alone_shows_no_setup() {
    let (mut h, st) = missing(Locale::En);
    h.model.ui.claude_installed = true;
    assert!(!needed(&st, &h.model));
    assert!(offer_at_start(&st, &h.model).is_none());
}

/// D17: Hangar is usable with Codex alone. Nothing stands in the way at
/// start, nor in front of a new workspace.
#[test]
fn codex_alone_satisfies_the_setup() {
    let (h, mut st) = codex_only(Locale::En);
    assert!(!h.model.ui.claude_installed);
    assert!(any_installed(&h.model));
    assert!(!needed(&st, &h.model));
    assert!(offer_at_start(&st, &h.model).is_none());
    let (intent, _) = redirect(
        &mut st,
        &h.model,
        Intent::OpenOverlay(Overlay::WorkspaceDialog),
    );
    assert!(matches!(
        intent,
        Intent::OpenOverlay(Overlay::WorkspaceDialog)
    ));
    assert!(!st.then_create);
}

/// Before the detection answers, every machine reads as one without an
/// agent: the sheet waits for the answer.
#[test]
fn nothing_shows_before_the_detection_answers() {
    let (h, mut st) = missing(Locale::En);
    st.detected = false;
    assert!(offer_at_start(&st, &h.model).is_none());
    let (intent, _) = redirect(
        &mut st,
        &h.model,
        Intent::OpenOverlay(Overlay::WorkspaceDialog),
    );
    assert!(matches!(
        intent,
        Intent::OpenOverlay(Overlay::WorkspaceDialog)
    ));
}

#[test]
fn the_setup_is_not_stacked_twice() {
    let (mut h, st) = missing(Locale::En);
    h.model.ui.overlays.push(Overlay::AgentSetup);
    assert!(offer_at_start(&st, &h.model).is_none());
    assert!(is_open(&h.model));
}

/// The close button, Escape and the veil are held back while there is no
/// agent and Hangar is not set to shells only, and while an installer runs.
#[test]
fn the_setup_closes_only_with_an_agent_and_nothing_running() {
    let (mut h, mut st) = missing(Locale::En);
    assert!(!closable(&st, &h.model));

    h.model.ui.codex_installed = true;
    assert!(closable(&st, &h.model));
    h.model.ui.codex_installed = false;
    h.model.ui.claude_installed = true;
    assert!(closable(&st, &h.model));

    h.model.ui.claude_installed = false;
    shells_only_on(&mut h);
    assert!(closable(&st, &h.model));

    // An installer running holds it open, whatever is installed.
    h.model.ui.codex_installed = true;
    let _ = installing(&mut h, &mut st, AgentKind::Claude);
    assert!(!closable(&st, &h.model));
}

/// Escape asks the sheet to close its own way: through « Done », which also
/// opens the dialog it stood in front of.
#[test]
fn escape_on_the_setup_is_its_close_button() {
    let (mut h, mut st) = missing(Locale::En);
    st.then_create = true;
    // In the way: Escape does nothing, and « Done » sent anyway neither.
    let _ = update(&mut st, &mut h.model, Msg::Done);
    assert!(st.then_create);

    h.model.ui.codex_installed = true;
    let _ = update(&mut st, &mut h.model, Msg::Done);
    assert!(!st.then_create, "closed, the dialog on its way");
}

// ── a new workspace ───────────────────────────────────────────────────

#[test]
fn a_new_workspace_without_any_agent_opens_the_setup_then_the_dialog() {
    let (h, mut st) = missing(Locale::En);
    let (intent, _) = redirect(
        &mut st,
        &h.model,
        Intent::OpenOverlay(Overlay::WorkspaceDialog),
    );
    assert!(matches!(intent, Intent::OpenOverlay(Overlay::AgentSetup)));
    assert!(st.then_create);

    // Done: the sheet closes, and the dialog it stood in front of opens.
    let intents = closing(&mut st);
    assert!(matches!(
        intents.as_slice(),
        [
            Intent::CloseOverlay,
            Intent::OpenOverlay(Overlay::WorkspaceDialog)
        ]
    ));
    assert!(!st.then_create);
}

#[test]
fn other_overlays_pass_untouched() {
    let (h, mut st) = missing(Locale::En);
    let (intent, _) = redirect(&mut st, &h.model, Intent::OpenOverlay(Overlay::Palette));
    assert!(matches!(intent, Intent::OpenOverlay(Overlay::Palette)));
    assert!(!st.then_create);
    assert_eq!(st.look, 0, "no look for another overlay");
}

#[test]
fn closing_a_setup_shown_at_start_opens_nothing_else() {
    let (_, mut st) = missing(Locale::En);
    assert!(matches!(
        closing(&mut st).as_slice(),
        [Intent::CloseOverlay]
    ));
}

// ── the look as it opens ──────────────────────────────────────────────

/// Opened, from the start or from a link, the sheet looks at both agents
/// again: their versions, Codex's login, and the window's own flags.
#[test]
fn opening_the_setup_looks_at_the_agents_again() {
    let (mut h, mut st) = missing(Locale::En);
    let (intent, _) = redirect(&mut st, &h.model, Intent::OpenOverlay(Overlay::AgentSetup));
    assert!(matches!(intent, Intent::OpenOverlay(Overlay::AgentSetup)));
    let first = st.look;
    assert_ne!(first, 0);

    // Already open: no second look.
    h.model.ui.overlays.push(Overlay::AgentSetup);
    let _ = redirect(&mut st, &h.model, Intent::OpenOverlay(Overlay::AgentSetup));
    assert_eq!(st.look, first);
    h.model.ui.overlays.clear();

    // Opened again: a newer look, whose answer is the one kept.
    let _ = redirect(&mut st, &h.model, Intent::OpenOverlay(Overlay::AgentSetup));
    let second = st.look;
    assert_ne!(second, first);
    let _ = update(
        &mut st,
        &mut h.model,
        Msg::Looked(first, look(true, true, true)),
    );
    assert!(!h.model.ui.codex_installed, "an earlier look is ignored");
    assert_eq!(st.codex_signed_in, None);

    let _ = update(
        &mut st,
        &mut h.model,
        Msg::Looked(second, look(false, true, true)),
    );
    assert!(h.model.ui.codex_installed);
    assert!(!h.model.ui.claude_installed);
    assert_eq!(st.codex_signed_in, Some(true));
    assert_eq!(
        st.codex.found.as_ref().and_then(|f| f.version.as_deref()),
        Some("0.170.0")
    );
    assert!(!needed(&st, &h.model));
}

// ── shells only ───────────────────────────────────────────────────────

#[test]
fn shells_only_is_kept_in_the_settings_and_honoured() {
    let (mut h, mut st) = missing(Locale::En);
    st.then_create = true;
    let _ = update(&mut st, &mut h.model, Msg::ShellsOnly);
    assert!(crate::settings::shells_only(h.model.settings()));
    assert!(!st.then_create, "the dialog it stood in front of opens");

    // Saved: the settings file says so, and reads back the same.
    let json = serde_json::to_value(h.model.settings()).expect("settings serialise");
    assert_eq!(json["shellsOnly"], serde_json::Value::Bool(true));
    let back: crate::state::types::Settings =
        serde_json::from_value(json).expect("settings read back");
    assert!(crate::settings::shells_only(&back));

    // Honoured: no sheet at start, and a new workspace is the dialog, whose
    // start falls back to a shell.
    assert!(!needed(&st, &h.model));
    assert!(offer_at_start(&st, &h.model).is_none());
    let (intent, _) = redirect(
        &mut st,
        &h.model,
        Intent::OpenOverlay(Overlay::WorkspaceDialog),
    );
    assert!(matches!(
        intent,
        Intent::OpenOverlay(Overlay::WorkspaceDialog)
    ));
    assert_eq!(
        crate::workspace_dialog::folder::start_of(false, h.model.ui.claude_installed),
        crate::workspace_dialog::folder::Start::Shell
    );
}

#[test]
fn shells_only_is_undone_in_the_settings() {
    let (mut h, st) = missing(Locale::En);
    shells_only_on(&mut h);
    assert!(!needed(&st, &h.model));
    h.model.apply(StoreAction::UpdateSettings(Patch::new(|s| {
        crate::settings::set_shells_only(s, false)
    })));
    assert!(needed(&st, &h.model));
    // Off is no key at all: the file is written back as it was before.
    let json = serde_json::to_value(h.model.settings()).expect("settings serialise");
    assert!(json.get("shellsOnly").is_none());
}

// ── the installers ────────────────────────────────────────────────────

/// The commands of code.claude.com/docs/en/setup and
/// learn.chatgpt.com/docs/codex/cli, word for word, for the user alone.
#[test]
fn the_official_installer_of_each_agent_and_system() {
    assert_eq!(
        install::official(Os::Unix, AgentKind::Claude),
        Installer {
            program: "bash".into(),
            args: vec![
                "-o".into(),
                "pipefail".into(),
                "-c".into(),
                "curl -fsSL https://claude.ai/install.sh | bash".into(),
            ],
            env: vec![],
            shown: "curl -fsSL https://claude.ai/install.sh | bash".into(),
        }
    );
    assert_eq!(
        install::official(Os::Unix, AgentKind::Codex),
        Installer {
            program: "bash".into(),
            args: vec![
                "-o".into(),
                "pipefail".into(),
                "-c".into(),
                "curl -fsSL https://chatgpt.com/codex/install.sh | sh".into(),
            ],
            // Its own questions skipped: no terminal to answer them on.
            env: vec![("CODEX_NON_INTERACTIVE".into(), "1".into())],
            shown: "curl -fsSL https://chatgpt.com/codex/install.sh | sh".into(),
        }
    );
    for (kind, line) in [
        (AgentKind::Claude, "irm https://claude.ai/install.ps1 | iex"),
        (
            AgentKind::Codex,
            "irm https://chatgpt.com/codex/install.ps1 | iex",
        ),
    ] {
        let windows = install::official(Os::Windows, kind);
        assert_eq!(windows.program, "powershell");
        assert_eq!(windows.args.last().map(String::as_str), Some(line));
        assert_eq!(windows.shown, line);
    }
    for kind in AgentKind::ALL {
        for os in [Os::Unix, Os::Windows] {
            let installer = install::official(os, kind);
            assert!(
                !installer.args.iter().any(|arg| arg.contains("sudo")),
                "{installer:?}"
            );
        }
    }
    assert_eq!(
        install::docs_url(AgentKind::Codex),
        "https://learn.chatgpt.com/docs/codex/cli"
    );
}

#[test]
fn installer_lines_are_shown_plain() {
    assert_eq!(
        install::clean("\u{1b}[1;32m✔ Installed\u{1b}[0m\n"),
        "✔ Installed"
    );
    assert_eq!(install::clean("10%\r50%\r100%\r\n"), "100%");
    assert_eq!(install::clean("a\tb"), "a\tb");
}

/// The runner, on a stand-in installer: every line it prints, both streams,
/// then its exit code, all tagged with the agent and the run.
#[cfg(unix)]
#[test]
fn the_runner_streams_the_output_then_the_exit() {
    let fake = Installer {
        program: "sh".into(),
        args: vec![
            "-c".into(),
            "echo downloading; echo 'not found' >&2; echo \"asked: $CODEX_NON_INTERACTIVE\"; exit 3"
                .into(),
        ],
        env: vec![("CODEX_NON_INTERACTIVE".into(), "1".into())],
        shown: "fake".into(),
    };
    let runtime = tokio::runtime::Runtime::new().expect("a runtime");
    let messages: Vec<Message> =
        runtime.block_on(install::run(fake, AgentKind::Codex, 7).collect());
    let mut lines = Vec::new();
    let mut exit = None;
    for message in messages {
        match message {
            Message::Shell(Action::Setup(Msg::Output(AgentKind::Codex, 7, line))) => {
                lines.push(line)
            }
            Message::Shell(Action::Setup(Msg::Exited(AgentKind::Codex, 7, ended))) => {
                exit = Some(ended)
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    lines.sort();
    assert_eq!(lines, vec!["asked: 1", "downloading", "not found"]);
    assert_eq!(exit, Some(Ok(Some(3))));
}

#[test]
fn an_installer_that_cannot_start_says_why() {
    let gone = Installer {
        program: "/nonexistent/hangar-no-such-installer".into(),
        args: vec![],
        env: vec![],
        shown: "gone".into(),
    };
    let runtime = tokio::runtime::Runtime::new().expect("a runtime");
    let messages: Vec<Message> =
        runtime.block_on(install::run(gone, AgentKind::Claude, 1).collect());
    assert!(matches!(
        messages.as_slice(),
        [Message::Shell(Action::Setup(Msg::Exited(
            AgentKind::Claude,
            1,
            Err(_)
        )))]
    ));
}

/// `TESKAD_DEV_CLAUDE_INSTALLER` and `TESKAD_DEV_CODEX_INSTALLER` stand in
/// for the official commands in a debug build (the tests' own), and an empty
/// one does not.
#[test]
fn a_debug_build_takes_a_stand_in_installer() {
    assert_eq!(
        install::fake(Some("echo fake".into())),
        cfg!(debug_assertions).then(|| "echo fake".to_string())
    );
    assert_eq!(install::fake(Some("  ".into())), None);
    assert_eq!(install::fake(None), None);
}

// ── after an install ──────────────────────────────────────────────────

fn installing(h: &mut Harness, st: &mut State, kind: AgentKind) -> u64 {
    let _ = update(st, &mut h.model, Msg::Install(kind));
    assert_eq!(st.card(kind).step, Step::Installing);
    st.card(kind).run
}

#[test]
fn a_successful_codex_install_looks_again_and_satisfies_the_setup() {
    let (mut h, mut st) = missing(Locale::En);
    let run = installing(&mut h, &mut st, AgentKind::Codex);
    let _ = update(
        &mut st,
        &mut h.model,
        Msg::Output(AgentKind::Codex, run, "==> Installed Codex".into()),
    );
    assert_eq!(st.codex.output, vec!["==> Installed Codex"]);
    // Exit 0 asks the PATH again: still installing until it answers.
    let _ = update(
        &mut st,
        &mut h.model,
        Msg::Exited(AgentKind::Codex, run, Ok(Some(0))),
    );
    assert_eq!(st.codex.step, Step::Installing);
    let _ = update(
        &mut st,
        &mut h.model,
        Msg::Found(AgentKind::Codex, run, look(false, true, false)),
    );
    assert_eq!(st.codex.step, Step::Idle);
    assert!(h.model.ui.codex_installed);
    assert!(!h.model.ui.claude_installed);
    assert_eq!(st.codex_signed_in, Some(false));
    assert!(!needed(&st, &h.model));
}

#[test]
fn a_successful_claude_code_install_looks_again() {
    let (mut h, mut st) = missing(Locale::En);
    let run = installing(&mut h, &mut st, AgentKind::Claude);
    let _ = update(
        &mut st,
        &mut h.model,
        Msg::Exited(AgentKind::Claude, run, Ok(Some(0))),
    );
    let _ = update(
        &mut st,
        &mut h.model,
        Msg::Found(AgentKind::Claude, run, look(true, false, false)),
    );
    assert_eq!(st.claude.step, Step::Idle);
    assert!(h.model.ui.claude_installed);
    assert!(!needed(&st, &h.model));
}

#[test]
fn an_install_that_leaves_no_agent_says_so() {
    let (mut h, mut st) = missing(Locale::En);
    let run = installing(&mut h, &mut st, AgentKind::Codex);
    let _ = update(
        &mut st,
        &mut h.model,
        Msg::Exited(AgentKind::Codex, run, Ok(Some(0))),
    );
    let _ = update(
        &mut st,
        &mut h.model,
        Msg::Found(AgentKind::Codex, run, look(false, false, false)),
    );
    assert_eq!(st.codex.step, Step::Failed(Failure::NotFound));
    assert!(!h.model.ui.codex_installed);
    assert!(needed(&st, &h.model));
}

#[test]
fn a_failed_install_keeps_its_output_and_says_its_code() {
    let (mut h, mut st) = missing(Locale::En);
    let run = installing(&mut h, &mut st, AgentKind::Claude);
    let _ = update(
        &mut st,
        &mut h.model,
        Msg::Output(AgentKind::Claude, run, "curl: (22) 403".into()),
    );
    let _ = update(
        &mut st,
        &mut h.model,
        Msg::Exited(AgentKind::Claude, run, Ok(Some(22))),
    );
    assert_eq!(st.claude.step, Step::Failed(Failure::Exit(22)));
    assert_eq!(st.claude.output, vec!["curl: (22) 403"]);

    // Try again: a run of its own, the old one's late lines ignored.
    let again = installing(&mut h, &mut st, AgentKind::Claude);
    assert!(st.claude.output.is_empty());
    let _ = update(
        &mut st,
        &mut h.model,
        Msg::Output(AgentKind::Claude, run, "late".into()),
    );
    let _ = update(
        &mut st,
        &mut h.model,
        Msg::Found(AgentKind::Claude, run, look(true, false, false)),
    );
    assert!(st.claude.output.is_empty());
    assert_eq!(st.claude.step, Step::Installing);
    // A line tagged for the other agent is not this card's.
    let _ = update(
        &mut st,
        &mut h.model,
        Msg::Output(AgentKind::Codex, again, "other".into()),
    );
    assert!(st.claude.output.is_empty());
    let _ = update(
        &mut st,
        &mut h.model,
        Msg::Exited(AgentKind::Claude, again, Ok(None)),
    );
    assert_eq!(st.claude.step, Step::Failed(Failure::Stopped));
}

/// Both installers write the same shell profiles: one runs at a time, and
/// the sheet does not close under it.
#[test]
fn one_install_at_a_time_and_no_close_while_it_runs() {
    let (mut h, mut st) = codex_only(Locale::En);
    let _ = installing(&mut h, &mut st, AgentKind::Claude);
    let _ = update(&mut st, &mut h.model, Msg::Install(AgentKind::Codex));
    assert_eq!(st.codex.step, Step::Idle);
    let _ = update(&mut st, &mut h.model, Msg::Done);
    assert_eq!(
        st.claude.step,
        Step::Installing,
        "still open, still running"
    );
}

#[test]
fn signing_in_opens_the_agent_in_a_pane_of_the_workspace_on_screen() {
    for (kind, agent) in [
        (AgentKind::Claude, AgentId::Claude),
        (AgentKind::Codex, AgentId::Codex),
    ] {
        let (mut h, mut st) = missing(Locale::En);
        h.model.ui.claude_installed = true;
        h.model.ui.codex_installed = true;
        let before = h.model.workspace("w1").expect("w1").panes.len();
        let _ = update(&mut st, &mut h.model, Msg::SignIn(kind));
        let panes = &h.model.workspace("w1").expect("w1").panes;
        assert_eq!(panes.len(), before + 1, "{kind:?}");
        assert_eq!(panes.last().map(|p| p.agent), Some(agent));
        assert_eq!(st.card(kind).step, Step::Idle);
    }
}

#[test]
fn signing_in_on_the_way_to_a_new_workspace_opens_its_dialog() {
    let (mut h, mut st) = codex_only(Locale::En);
    st.then_create = true;
    let before = h.model.workspace("w1").expect("w1").panes.len();
    let mut probe = State {
        then_create: true,
        ..State::default()
    };
    assert!(matches!(
        closing(&mut probe).as_slice(),
        [_, Intent::OpenOverlay(Overlay::WorkspaceDialog)]
    ));
    let _ = update(&mut st, &mut h.model, Msg::SignIn(AgentKind::Codex));
    assert_eq!(h.model.workspace("w1").expect("w1").panes.len(), before);
    assert!(!st.then_create);
}

/// Claude Code's login is the accounts package's read of its main account;
/// Codex's, the sheet's last look.
#[test]
fn each_agents_main_login_is_what_signed_in_reads() {
    let (mut h, mut st) = missing(Locale::En);
    assert_eq!(signed_in(&st, &h.model, AgentKind::Claude), Some(true));
    h.model.live.accounts.accounts = Some(vec![crate::workspace_dialog::testing::account(
        "main",
        "Principal",
        false,
    )]);
    assert_eq!(signed_in(&st, &h.model, AgentKind::Claude), Some(false));
    h.model.live.accounts.accounts = None;
    assert_eq!(signed_in(&st, &h.model, AgentKind::Claude), None);

    assert_eq!(signed_in(&st, &h.model, AgentKind::Codex), None);
    let _ = redirect(&mut st, &h.model, Intent::OpenOverlay(Overlay::AgentSetup));
    let run = st.look;
    let _ = update(
        &mut st,
        &mut h.model,
        Msg::Looked(run, look(false, true, true)),
    );
    assert_eq!(signed_in(&st, &h.model, AgentKind::Codex), Some(true));
}

// ── drawn ─────────────────────────────────────────────────────────────

#[test]
fn the_offer_shows_both_agents_codex_in_beta_and_the_way_out() {
    for (locale, texts, name) in [
        (
            Locale::En,
            [
                "Claude Code",
                "Codex",
                "beta",
                "Not installed",
                "Install Claude Code",
                "Install Codex",
                "Use Teskad with shells only",
            ],
            "setup-offer-en-graphite",
        ),
        (
            Locale::Fr,
            [
                "Claude Code",
                "Codex",
                "bêta",
                "Pas installé",
                "Installer Claude Code",
                "Installer Codex",
                "Utiliser Teskad avec des shells uniquement",
            ],
            "setup-offer-fr-graphite",
        ),
    ] {
        let (h, st) = missing(locale);
        let mut ui = h.show(view(&st, h.ctx()));
        for text in texts {
            assert!(ui.find(text).is_ok(), "{text}");
        }
        if !cfg!(windows) {
            for line in [
                "curl -fsSL https://claude.ai/install.sh | bash",
                "curl -fsSL https://chatgpt.com/codex/install.sh | sh",
            ] {
                assert!(ui.find(line).is_ok(), "{line}");
            }
        }
        // Nothing to be done with: no « Done » until an agent is there.
        assert!(ui.find("Done").is_err());
        assert!(ui.find("Terminé").is_err());
        h.shot(&mut ui, name);
    }
}

#[test]
fn each_agent_installs_on_a_click() {
    let (h, st) = missing(Locale::En);
    for (label, kind) in [
        ("Install Claude Code", AgentKind::Claude),
        ("Install Codex", AgentKind::Codex),
    ] {
        let mut ui = h.show(view(&st, h.ctx()));
        assert!(ui.click(label).is_ok(), "{label}");
        let messages = crate::workspace_dialog::testing::messages(ui);
        assert!(
            sent(&messages, |m| matches!(m, Msg::Install(k) if *k == kind)),
            "{messages:?}"
        );
    }
}

#[test]
fn a_failure_shows_the_output_and_the_guide() {
    let (h, mut st) = missing(Locale::En);
    st.codex.step = Step::Failed(Failure::Exit(22));
    st.codex.output = vec!["curl: (22) The requested URL returned error: 403".into()];
    let mut ui = h.show(view(&st, h.ctx()));
    for text in [
        "The installation failed (exit code 22).",
        "curl: (22) The requested URL returned error: 403",
        "https://learn.chatgpt.com/docs/codex/cli",
        "Open the install guide",
        "Try again",
        "Install Claude Code",
        "Use Teskad with shells only",
    ] {
        assert!(ui.find(text).is_ok(), "{text}");
    }
    h.shot(&mut ui, "setup-failed-en-graphite");
    assert!(ui.click("Open the install guide").is_ok());
    let messages = crate::workspace_dialog::testing::messages(ui);
    assert!(
        sent(&messages, |m| matches!(m, Msg::OpenDocs(AgentKind::Codex))),
        "{messages:?}"
    );
}

#[test]
fn an_install_that_found_nothing_names_the_command() {
    let (h, mut st) = missing(Locale::En);
    st.codex.step = Step::Failed(Failure::NotFound);
    let mut ui = h.show(view(&st, h.ctx()));
    assert!(
        ui.find("The installer finished, but Teskad still cannot find codex on this machine.")
            .is_ok()
    );
}

/// With Codex alone the sheet is satisfied: it says Codex's version, offers
/// to sign in to it and to install Claude Code, and closes.
#[test]
fn with_codex_alone_the_sheet_closes_and_offers_claude_code() {
    for (locale, texts, name) in [
        (
            Locale::En,
            [
                "Installed, version 0.170.0",
                "Sign in",
                "Install Claude Code",
                "Done",
            ],
            "setup-codex-only-en-graphite",
        ),
        (
            Locale::Fr,
            [
                "Installé, version 0.170.0",
                "Se connecter",
                "Installer Claude Code",
                "Terminé",
            ],
            "setup-codex-only-fr-graphite",
        ),
    ] {
        let (h, st) = codex_only(locale);
        let mut ui = h.show(view(&st, h.ctx()));
        for text in texts {
            assert!(ui.find(text).is_ok(), "{text}");
        }
        assert!(ui.find("Install Codex").is_err());
        assert!(ui.find("Use Teskad with shells only").is_err());
        h.shot(&mut ui, name);
    }

    let (h, st) = codex_only(Locale::En);
    let clicked = |label: &str| {
        let mut ui = h.show(view(&st, h.ctx()));
        assert!(ui.click(label).is_ok(), "{label}");
        crate::workspace_dialog::testing::messages(ui)
    };
    let messages = clicked("Sign in");
    assert!(
        sent(&messages, |m| matches!(m, Msg::SignIn(AgentKind::Codex))),
        "{messages:?}"
    );
    let messages = clicked("Done");
    assert!(sent(&messages, |m| matches!(m, Msg::Done)), "{messages:?}");
}

#[test]
fn both_installed_and_signed_in_says_so() {
    let (mut h, mut st) = codex_only(Locale::En);
    h.model.ui.claude_installed = true;
    st.claude.found = Some(found("/home/me/.local/bin/claude", "2.1.3"));
    st.codex_signed_in = Some(true);
    let mut ui = h.show(view(&st, h.ctx()));
    for text in [
        "Installed, version 2.1.3",
        "Installed, version 0.170.0",
        "Signed in",
        "Done",
    ] {
        assert!(ui.find(text).is_ok(), "{text}");
    }
    for text in ["Sign in", "Install Claude Code", "Install Codex"] {
        assert!(ui.find(text).is_err(), "{text}");
    }
}

/// A Claude Code whose main account is not logged in is offered the sign in
/// beside Codex's.
#[test]
fn each_agent_not_signed_in_has_its_own_sign_in() {
    let (mut h, st) = codex_only(Locale::En);
    h.model.ui.claude_installed = true;
    h.model.live.accounts.accounts = Some(vec![crate::workspace_dialog::testing::account(
        "main",
        "Principal",
        false,
    )]);
    let mut ui = h.show(view(&st, h.ctx()));
    for text in [
        "Sign in with your Claude account: Claude Code asks for it in a pane the first time it starts.",
        "Sign in with ChatGPT or an OpenAI API key: Codex asks for it in a pane the first time it starts.",
    ] {
        assert!(ui.find(text).is_ok(), "{text}");
    }
    h.shot(&mut ui, "setup-sign-in-en-graphite");
}

#[test]
fn an_agent_found_off_the_path_asks_for_a_restart() {
    let (h, mut st) = codex_only(Locale::En);
    if let Some(found) = st.codex.found.as_mut() {
        found.on_path = false;
    }
    let mut ui = h.show(view(&st, h.ctx()));
    assert!(
        ui.find("Found in /home/me/.local/bin/codex: restart Teskad so that its panes find it.")
            .is_ok()
    );
}

#[test]
fn while_an_installer_runs_there_is_no_way_out_and_no_second_install() {
    let (h, mut st) = codex_only(Locale::En);
    st.claude.step = Step::Installing;
    st.claude.output = vec!["downloading".into()];
    let mut ui = h.show(view(&st, h.ctx()));
    for text in ["Installing Claude Code…", "downloading"] {
        assert!(ui.find(text).is_ok(), "{text}");
    }
    assert!(ui.find("Done").is_err());
    let _ = ui.click("Sign in");
    let messages = crate::workspace_dialog::testing::messages(ui);
    assert!(
        !sent(&messages, |m| matches!(m, Msg::SignIn(_))),
        "{messages:?}"
    );
}

// ── NixOS ─────────────────────────────────────────────────────────────

/// A folder standing for `/`, with these files in it.
fn root_with(files: &[(&str, &str)]) -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("a folder");
    for (path, text) in files {
        let path = root.path().join(path);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("its folder");
        std::fs::write(path, text).expect("the file written");
    }
    root
}

#[test]
fn nixos_is_known_by_its_marker_or_its_os_release() {
    let marker = root_with(&[("etc/NIXOS", "")]);
    assert!(nixos::is_nixos(marker.path()));
    let release = root_with(&[(
        "etc/os-release",
        "BUILD_ID=\"25.11\"\nNAME=NixOS\nID=nixos\nVERSION_ID=\"25.11\"\n",
    )]);
    assert!(nixos::is_nixos(release.path()));
    let quoted = root_with(&[("etc/os-release", "ID=\"nixos\"\n")]);
    assert!(nixos::is_nixos(quoted.path()));

    let ubuntu = root_with(&[(
        "etc/os-release",
        "NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\n",
    )]);
    assert!(!nixos::is_nixos(ubuntu.path()));
    // Only the ID counts, not another key that names NixOS.
    let variant = root_with(&[("etc/os-release", "ID=debian\nVARIANT_ID=nixos\n")]);
    assert!(!nixos::is_nixos(variant.path()));
    let bare = root_with(&[]);
    assert!(!nixos::is_nixos(bare.path()));
}

#[test]
fn the_os_release_id_is_read_without_its_quotes() {
    assert_eq!(nixos::os_release_id("ID=nixos"), Some("nixos"));
    assert_eq!(nixos::os_release_id("ID='nixos'"), Some("nixos"));
    assert_eq!(
        nixos::os_release_id("NAME=x\n  ID=\"arch\"\n"),
        Some("arch")
    );
    assert_eq!(nixos::os_release_id("ID_LIKE=nixos"), None);
}

/// Claude Code's installer drops a program NixOS cannot start; Codex's, a
/// static one that starts there.
#[test]
fn only_claude_code_comes_from_nixpkgs() {
    assert!(nixos::from_nixpkgs(AgentKind::Claude));
    assert!(!nixos::from_nixpkgs(AgentKind::Codex));
    let (_, st) = missing_on_nixos(Locale::En);
    assert!(st.offers_nixpkgs(AgentKind::Claude));
    assert!(!st.offers_nixpkgs(AgentKind::Codex));
}

#[test]
fn on_nixos_claude_code_comes_from_nixpkgs_and_codex_from_its_installer() {
    for (locale, texts, name) in [
        (
            Locale::En,
            [
                "Check again",
                "Use Anthropic's installer anyway",
                "Install Codex",
                "Use Teskad with shells only",
            ],
            "setup-nixos-en-graphite",
        ),
        (
            Locale::Fr,
            [
                "Vérifier à nouveau",
                "Utiliser quand même l'installeur d'Anthropic",
                "Installer Codex",
                "Utiliser Teskad avec des shells uniquement",
            ],
            "setup-nixos-fr-graphite",
        ),
    ] {
        let (h, st) = missing_on_nixos(locale);
        let mut ui = h.show(view(&st, h.ctx()));
        for text in texts
            .into_iter()
            .chain([nixos::SYSTEM_PACKAGES, nixos::PROFILE_INSTALL])
        {
            assert!(ui.find(text).is_ok(), "{text}");
        }
        assert!(
            ui.find("curl -fsSL https://claude.ai/install.sh | bash")
                .is_err()
        );
        if !cfg!(windows) {
            assert!(
                ui.find("curl -fsSL https://chatgpt.com/codex/install.sh | sh")
                    .is_ok()
            );
        }
        if locale == Locale::En {
            assert!(ui.find("Install Claude Code").is_err());
        }
        h.shot(&mut ui, name);
    }
}

#[test]
fn on_nixos_check_again_and_shells_only_are_clicks() {
    let (h, st) = missing_on_nixos(Locale::En);
    let clicked = |label: &str| {
        let mut ui = h.show(view(&st, h.ctx()));
        assert!(ui.click(label).is_ok(), "{label}");
        crate::workspace_dialog::testing::messages(ui)
    };
    let messages = clicked("Check again");
    assert!(
        sent(&messages, |m| matches!(m, Msg::Recheck(AgentKind::Claude))),
        "{messages:?}"
    );
    let messages = clicked("Use Teskad with shells only");
    assert!(
        sent(&messages, |m| matches!(m, Msg::ShellsOnly)),
        "{messages:?}"
    );
}

#[test]
fn off_nixos_the_offer_is_the_installers_as_before() {
    let (h, st) = missing(Locale::En);
    assert!(!st.offers_nixpkgs(AgentKind::Claude));
    let mut ui = h.show(view(&st, h.ctx()));
    assert!(ui.find("Install Claude Code").is_ok());
    for text in [
        nixos::SYSTEM_PACKAGES,
        nixos::PROFILE_INSTALL,
        "Check again",
        "Use Anthropic's installer anyway",
        "Install from nixpkgs instead",
    ] {
        assert!(ui.find(text).is_err(), "{text}");
    }
}

#[test]
fn on_nixos_the_installer_is_still_there_behind_a_link() {
    let (mut h, mut st) = missing_on_nixos(Locale::En);
    {
        let mut ui = h.show(view(&st, h.ctx()));
        assert!(ui.click("Use Anthropic's installer anyway").is_ok());
        let messages = crate::workspace_dialog::testing::messages(ui);
        assert!(
            sent(&messages, |m| matches!(m, Msg::InstallerAnyway)),
            "{messages:?}"
        );
    }

    // Chosen: the installer's card, as anywhere else, with the way back.
    let _ = update(&mut st, &mut h.model, Msg::InstallerAnyway);
    assert!(!st.offers_nixpkgs(AgentKind::Claude));
    assert_eq!(st.claude.step, Step::Idle);
    {
        let mut ui = h.show(view(&st, h.ctx()));
        assert!(ui.find("Install from nixpkgs instead").is_ok());
        assert!(ui.find("Check again").is_err());
        assert!(ui.click("Install Claude Code").is_ok());
        let messages = crate::workspace_dialog::testing::messages(ui);
        assert!(
            sent(&messages, |m| matches!(m, Msg::Install(AgentKind::Claude))),
            "{messages:?}"
        );
    }

    // It runs the installer, whose failure says so and keeps the way back.
    let run = installing(&mut h, &mut st, AgentKind::Claude);
    let _ = update(
        &mut st,
        &mut h.model,
        Msg::Exited(AgentKind::Claude, run, Ok(Some(1))),
    );
    assert_eq!(st.claude.step, Step::Failed(Failure::Exit(1)));
    {
        let mut ui = h.show(view(&st, h.ctx()));
        assert!(ui.find("Install from nixpkgs instead").is_ok());
        assert!(ui.find("Try again").is_ok());
    }

    // Back to nixpkgs, from the start.
    let _ = update(&mut st, &mut h.model, Msg::Nixpkgs);
    assert!(st.offers_nixpkgs(AgentKind::Claude));
    assert_eq!(st.claude.step, Step::Idle);

    // The sheet closed starts again from nixpkgs.
    let _ = update(&mut st, &mut h.model, Msg::InstallerAnyway);
    let _ = closing(&mut st);
    assert!(st.offers_nixpkgs(AgentKind::Claude));
}

#[test]
fn check_again_looks_for_claude_and_moves_on_once_found() {
    let (mut h, mut st) = missing_on_nixos(Locale::En);
    let _ = update(&mut st, &mut h.model, Msg::Recheck(AgentKind::Claude));
    assert_eq!(st.claude.step, Step::Checking);
    let first = st.claude.run;

    // Not there yet: the card says so, and stays.
    let _ = update(
        &mut st,
        &mut h.model,
        Msg::Found(AgentKind::Claude, first, look(false, false, false)),
    );
    assert_eq!(st.claude.step, Step::Idle);
    assert!(st.claude.still_missing);
    assert!(st.offers_nixpkgs(AgentKind::Claude));
    assert!(!h.model.ui.claude_installed);
    assert!(needed(&st, &h.model));
    {
        let mut ui = h.show(view(&st, h.ctx()));
        assert!(
            ui.find(
                "Teskad still cannot find claude on this machine. Once the rebuild or the install is done, check again."
            )
            .is_ok()
        );
    }

    // Again: while it looks, the sheet waits, with no way out.
    let _ = update(&mut st, &mut h.model, Msg::Recheck(AgentKind::Claude));
    let again = st.claude.run;
    assert_ne!(again, first);
    {
        let mut ui = h.show(view(&st, h.ctx()));
        assert!(ui.find("Checking…").is_ok());
        assert!(ui.find("Use Teskad with shells only").is_err());
        assert!(ui.find("Use Anthropic's installer anyway").is_err());
    }
    // A second click while it looks, an install of the other agent, and an
    // earlier look's late answer, do nothing.
    let _ = update(&mut st, &mut h.model, Msg::Recheck(AgentKind::Claude));
    let _ = update(&mut st, &mut h.model, Msg::InstallerAnyway);
    let _ = update(&mut st, &mut h.model, Msg::Install(AgentKind::Codex));
    assert_eq!(st.claude.run, again);
    assert_eq!(st.codex.step, Step::Idle);
    assert!(st.offers_nixpkgs(AgentKind::Claude));
    let _ = update(
        &mut st,
        &mut h.model,
        Msg::Found(AgentKind::Claude, first, look(true, false, false)),
    );
    assert_eq!(st.claude.step, Step::Checking);

    // Installed by now: the card says so, and the setup is satisfied.
    let _ = update(
        &mut st,
        &mut h.model,
        Msg::Found(AgentKind::Claude, again, look(true, false, false)),
    );
    assert_eq!(st.claude.step, Step::Idle);
    assert!(!st.claude.still_missing);
    assert!(h.model.ui.claude_installed);
    assert!(!needed(&st, &h.model));
}

/// An answer that comes when nothing is looked for any more (the sheet
/// closed meanwhile) changes nothing.
#[test]
fn a_late_answer_after_the_sheet_closed_is_ignored() {
    let (mut h, mut st) = missing_on_nixos(Locale::En);
    let _ = update(&mut st, &mut h.model, Msg::Recheck(AgentKind::Claude));
    let run = st.claude.run;
    let _ = closing(&mut st);
    let _ = update(
        &mut st,
        &mut h.model,
        Msg::Found(AgentKind::Claude, run, look(true, false, false)),
    );
    assert_eq!(st.claude.step, Step::Idle);
    assert!(!h.model.ui.claude_installed);
}
