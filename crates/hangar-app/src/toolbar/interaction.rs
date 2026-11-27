//! The toolbar driven with real pointer events: what its controls and its
//! menus say to the app.
#![allow(clippy::field_reassign_with_default)]

use hangar_i18n::{Key, Locale, Translator, args};
use hangar_ui::menu as ui_menu;

use super::*;
use crate::app::Packages;
use crate::navigator::testing::{Drive, ctx, model_of, pane, simulate, workspace};
use crate::state::types::{AgentId, AppState, PaneRole};

fn model() -> Model {
    let mut ws = workspace(
        "w",
        "hangar",
        None,
        vec![
            pane("p1", "Ava", AgentId::Claude),
            pane("p2", "Ben", AgentId::Claude),
            pane("p3", "Dan", AgentId::Shell),
        ],
    );
    ws.panes[0].role = Some(PaneRole::Orchestrator);
    ws.panes[1].role = Some(PaneRole::Worker);
    let mut model = model_of(AppState {
        workspaces: vec![ws],
        active_workspace_id: Some("w".into()),
        ..AppState::default()
    });
    model.ui.view = View::Terminals;
    model
}

fn run(
    st: &State,
    model: &Model,
    drive: impl FnOnce(&mut iced_test::Simulator<'_, Message>),
) -> Vec<Message> {
    let pkg = Packages::default();
    let ctx = ctx(model, &pkg);
    let mut ui = simulate(iced::widget::container(view(st, ctx)).height(44));
    drive(&mut ui);
    ui.into_messages().collect()
}

fn sent(messages: &[Message], wanted: impl Fn(&Intent) -> bool) -> bool {
    messages.iter().any(|message| match message {
        Message::Intent(intent) => wanted(intent),
        _ => false,
    })
}

fn toolbar(messages: &[Message], wanted: impl Fn(&Msg) -> bool) -> bool {
    messages.iter().any(|message| match message {
        Message::Toolbar(msg) => wanted(msg),
        _ => false,
    })
}

const EN: Translator = Translator::new(Locale::En);

#[test]
fn a_view_button_switches_the_view() {
    let messages = run(&State::default(), &model(), |ui| {
        ui.click("Board").expect("the view is offered");
    });
    assert!(
        sent(&messages, |i| matches!(i, Intent::SetView(View::Board))),
        "{messages:?}"
    );
}

#[test]
fn each_of_the_four_views_is_offered_by_name() {
    let model = model();
    let pkg = Packages::default();
    let st = State::default();
    let mut ui = simulate(view(&st, ctx(&model, &pkg)));
    for name in ["Terminals", "Board", "Memory", "Statistics"] {
        assert!(ui.find(name).is_ok(), "{name} is missing");
    }
}

#[test]
fn the_toolbar_names_the_workspace() {
    let model = model();
    let pkg = Packages::default();
    let st = State::default();
    let mut ui = simulate(view(&st, ctx(&model, &pkg)));
    assert!(ui.find("hangar").is_ok());
}

#[test]
fn the_layouts_are_one_button_whose_menu_offers_the_four() {
    let mut model = model();
    let pkg = Packages::default();
    let window = model.main_window().expect("the main window");
    let mut st = State::default();
    drop(step(&mut st, &mut model, Msg::ToggleLayouts(window)));
    assert_eq!(st.layouts_open, Some(window));

    // Its lines, one per even layout; the third is four panes. Five pixels
    // of padding, then 26 per line.
    let ctx = ctx(&model, &pkg);
    let ws = model.active_workspace().unwrap();
    let entries = layout_entries(ctx, ws);
    assert_eq!(entries.len(), LAYOUTS.len());
    let mut ui = simulate(ui_menu::menu(ctx.theme, entries));
    ui.move_to(40.0, 5.0 + 26.0 * 2.0 + 13.0);
    ui.press(iced::mouse::Button::Left);
    ui.release(iced::mouse::Button::Left);
    let picked: Vec<Message> = ui.into_messages().collect();
    assert!(
        toolbar(
            &picked,
            |msg| matches!(msg, Msg::Preset(ws, 4) if ws == "w")
        ),
        "{picked:?}"
    );

    // Picking closes it, and so does the button again.
    drop(step(&mut st, &mut model, Msg::Preset("w".into(), 4)));
    assert_eq!(st.layouts_open, None);
    drop(step(&mut st, &mut model, Msg::ToggleLayouts(window)));
    drop(step(&mut st, &mut model, Msg::ToggleLayouts(window)));
    assert_eq!(st.layouts_open, None);
}

/// The « … » menu on its own at the window's corner, which a click picks a
/// line of by its height: five pixels of padding, then each line 26 pixels
/// tall and each separator 9.
fn pick_line(model: &Model, y: f32) -> Vec<Message> {
    let pkg = Packages::default();
    let ctx = ctx(model, &pkg);
    let ws = model.active_workspace().unwrap();
    let entries = menus::more_entries(ctx, ws, true, false, true, false);
    let mut ui = simulate(ui_menu::menu(ctx.theme, entries));
    ui.move_to(40.0, y);
    ui.press(iced::mouse::Button::Left);
    ui.release(iced::mouse::Button::Left);
    ui.into_messages().collect()
}

/// The intents a pick of a line of the menu hands to the toolbar.
fn picked(messages: &[Message]) -> Vec<String> {
    messages
        .iter()
        .filter_map(|message| match message {
            Message::Toolbar(Msg::Pick(list)) => Some(format!("{list:?}")),
            _ => None,
        })
        .collect()
}

// Open in a window 6..36, —, Command palette 49..79, the board beside the
// terminals 79..109, —, the agents' heading 122..146, Orchestration 146..176,
// Stop the workers 176..206, —, Reset agents… 219..249, padding to 255.

#[test]
fn the_more_menu_opens_the_workspace_in_a_window() {
    let picked = picked(&pick_line(&model(), 20.0));
    assert_eq!(picked, ["[Detach(\"w\")]"]);
}

#[test]
fn the_more_menu_opens_the_command_palette() {
    let picked = picked(&pick_line(&model(), 59.0));
    assert_eq!(picked, ["[TogglePalette]"]);
}

/// What lets the shell leave a drawer alone while a press lands on a line of
/// the menu hanging over it.
#[test]
fn the_toolbar_says_when_a_windows_more_menu_is_open() {
    let mut model = model();
    let window = model.main_window().expect("the main window");
    let mut st = State::default();
    assert!(!st.more_menu_open(window));
    drop(step(&mut st, &mut model, Msg::ToggleMore(window)));
    assert!(st.more_menu_open(window));
    assert!(!st.more_menu_open(iced::window::Id::unique()));
    drop(step(
        &mut st,
        &mut model,
        Msg::Pick(vec![Intent::ToggleDock]),
    ));
    assert!(!st.more_menu_open(window));
}

#[test]
fn the_more_menu_shows_the_board_beside_the_terminals() {
    let picked = picked(&pick_line(&model(), 89.0));
    assert_eq!(picked, ["[ToggleDock]"]);
}

#[test]
fn an_orchestrated_workspace_has_its_settings_in_the_more_menu() {
    let picked = picked(&pick_line(&model(), 152.0));
    assert_eq!(picked, ["[OpenOrchestration(\"w\")]"]);
}

#[test]
fn the_more_menu_says_which_agent_orchestrates_and_which_the_workers_run() {
    let mut model = model();
    // The workers run Codex here; the orchestrator, Claude Code, its pane's.
    let ws = model.workspace("w").unwrap().clone();
    model.apply(crate::state::StoreAction::UpdateWorkspace(
        ws.id.clone(),
        crate::state::Patch::new(|ws: &mut Workspace| {
            ws.orchestration = Some(crate::state::types::OrchestrationSettings {
                worker_agent: Some(crate::state::types::AgentKind::Codex),
                ..Default::default()
            });
        }),
    ));
    let pkg = Packages::default();
    let ctx = ctx(&model, &pkg);
    let entries = menus::more_entries(
        ctx,
        model.active_workspace().unwrap(),
        true,
        false,
        true,
        false,
    );
    let headings: Vec<&str> = entries
        .iter()
        .filter_map(|entry| match entry {
            ui_menu::Entry::Heading(heading) => Some(heading.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        headings,
        ["Orchestrator: Claude Code · workers: Codex (beta)"]
    );
    // A workspace that does not orchestrate says nothing of it.
    let mut plain = model;
    plain.apply(crate::state::StoreAction::UpdateWorkspace(
        ws.id.clone(),
        crate::state::Patch::new(|ws: &mut Workspace| {
            for pane in &mut ws.panes {
                pane.role = None;
            }
        }),
    ));
    let ctx = crate::navigator::testing::ctx(&plain, &pkg);
    let entries = menus::more_entries(
        ctx,
        plain.active_workspace().unwrap(),
        true,
        false,
        true,
        false,
    );
    assert!(
        !entries
            .iter()
            .any(|entry| matches!(entry, ui_menu::Entry::Heading(_)))
    );
}

#[test]
fn stopping_the_workers_takes_over_from_a_reset_that_was_asking() {
    let picked = picked(&pick_line(&model(), 182.0));
    assert_eq!(picked, ["[ArmResetAgents(false), ArmStopWorkers(true)]"]);
}

#[test]
fn resetting_the_agents_takes_over_from_a_stop_that_was_asking() {
    let picked = picked(&pick_line(&model(), 221.0));
    assert_eq!(picked, ["[ArmStopWorkers(false), ArmResetAgents(true)]"]);
}

#[test]
fn a_menu_line_that_is_not_there_picks_nothing() {
    // Below the last line, in the padding.
    assert!(picked(&pick_line(&model(), 252.0)).is_empty());
}

#[test]
fn an_armed_reset_asks_how_many_agents_and_cancels_or_confirms() {
    let mut model = model();
    model.ui.confirm_reset = true;
    // Two agents run in the workspace; the third pane is a shell.
    let title = EN
        .t_with(Key::ToolbarResetTitle, &args!("n" => 2))
        .into_owned();
    let confirm = EN
        .t_with(Key::TopbarResetAllConfirm, &args!("n" => 2))
        .into_owned();

    let pkg = Packages::default();
    let st = State::default();
    let mut ui = simulate(view(&st, ctx(&model, &pkg)));
    assert!(ui.find(title.as_str()).is_ok(), "{title}");
    drop(ui);

    let cancelled = run(&State::default(), &model, |ui| {
        ui.click("Cancel").expect("the safe answer is shown");
    });
    assert!(
        sent(&cancelled, |i| matches!(i, Intent::ArmResetAgents(false))),
        "{cancelled:?}"
    );

    let confirmed = run(&State::default(), &model, |ui| {
        ui.click(confirm.as_str())
            .expect("the confirmation is shown");
    });
    assert!(
        sent(
            &confirmed,
            |i| matches!(i, Intent::ResetAgents(ws) if ws == "w")
        ),
        "{confirmed:?}"
    );
}

#[test]
fn an_armed_stop_asks_about_the_workers() {
    let mut model = model();
    model.ui.confirm_stop_workers = true;
    let confirm = EN
        .t_with(Key::OrchStopConfirm, &args!("n" => 1))
        .into_owned();
    let confirmed = run(&State::default(), &model, |ui| {
        ui.click(confirm.as_str())
            .expect("the confirmation is shown");
    });
    assert!(
        sent(
            &confirmed,
            |i| matches!(i, Intent::StopWorkers(ws) if ws == "w")
        ),
        "{confirmed:?}"
    );
}

#[test]
fn a_preset_that_would_stop_a_busy_agent_names_it_and_asks() {
    let mut st = State::default();
    st.preset = Some(PresetAsk {
        ws: "w".into(),
        view: View::Terminals,
        size: 2,
        closing: vec![("p3".into(), "Dan".into())],
        busy: vec![("Dan".into(), AgentActivity::Working)],
    });
    // Two panes of three stay at a preset of 2: it is the 2 button that asks.
    let model = model();
    let title = EN
        .t_with(Key::ToolbarPresetTitle, &args!("names" => "Dan"))
        .into_owned();
    let pkg = Packages::default();
    let mut ui = simulate(view(&st, ctx(&model, &pkg)));
    assert!(ui.find(title.as_str()).is_ok(), "{title}");
    assert!(ui.find("Working").is_ok());
    drop(ui);

    let cancelled = run(&st, &model, |ui| {
        ui.click("Cancel").unwrap();
    });
    assert!(
        toolbar(&cancelled, |msg| matches!(msg, Msg::PresetCancel)),
        "{cancelled:?}"
    );
    let confirmed = run(&st, &model, |ui| {
        ui.click("Close 1 pane").unwrap();
    });
    assert!(
        toolbar(&confirmed, |msg| matches!(msg, Msg::PresetConfirm)),
        "{confirmed:?}"
    );
}

#[test]
fn a_question_about_another_workspace_or_view_is_not_asked_here() {
    let mut st = State::default();
    st.preset = Some(PresetAsk {
        ws: "elsewhere".into(),
        view: View::Terminals,
        size: 2,
        closing: vec![("p3".into(), "Dan".into())],
        busy: vec![("Dan".into(), AgentActivity::Working)],
    });
    let model = model();
    let pkg = Packages::default();
    let mut ui = simulate(view(&st, ctx(&model, &pkg)));
    assert!(ui.find("Close Dan?").is_err());
}

/// The same, as drawn: in Teskad Light and Teskad Dark, the pixels inside the
/// chosen view's segment are `border`, and those inside another's are the
/// track's `surface`.
#[test]
fn the_chosen_view_is_drawn_apart_from_the_others_in_teskad_light_and_dark() {
    for id in ["teskad-light", "teskad-dark"] {
        let mut model = model();
        let mut state = model.state().clone();
        state.settings.theme_id = id.into();
        model.hydrate(state);
        model.ui.view = View::Terminals;
        let pkg = Packages::default();
        let st = State::default();
        let ctx = ctx(&model, &pkg);
        let t = ctx.theme;
        assert_eq!(t.id, id);

        // Where each label starts: its segment's padding is left of it.
        let mut ui = simulate(iced::widget::container(view(&st, ctx)).height(44));
        let beside = |ui: &mut iced_test::Simulator<'_, Message>, label: &str| {
            let bounds = ui.find(label).expect("the view").bounds();
            (bounds.x - 6.0, bounds.center_y())
        };
        let chosen = beside(&mut ui, "Terminals");
        let other = beside(&mut ui, "Board");
        drop(ui);

        let backend = std::env::var("ICED_TEST_BACKEND").ok();
        let shot = crate::shell::dev_screenshot::render(
            iced::widget::container(view(&st, ctx)).height(44).into(),
            crate::navigator::testing::WINDOW,
            1.0,
            &hangar_theme::to_iced(t),
            backend.as_deref(),
        )
        .expect("drawn");
        let pixel = |(x, y): (f32, f32)| {
            let at = (y as usize * shot.size.width as usize + x as usize) * 4;
            [shot.rgba[at], shot.rgba[at + 1], shot.rgba[at + 2]]
        };
        let near = |seen: [u8; 3], wanted: iced::Color| {
            let wanted = wanted.into_rgba8();
            seen.iter()
                .zip(wanted)
                .all(|(seen, wanted)| seen.abs_diff(wanted) <= 2)
        };
        assert!(
            near(pixel(chosen), t.palette.border),
            "{id}: the chosen segment is {:?}",
            pixel(chosen)
        );
        assert!(
            near(pixel(other), t.palette.surface),
            "{id}: another segment is {:?}",
            pixel(other)
        );
        if let Some(folder) = hangar_core::env::var_os("TESKAD_TOOLBAR_SNAPSHOTS") {
            let path = std::path::PathBuf::from(folder).join(format!("toolbar-{id}.png"));
            crate::shell::dev_screenshot::write(&path, &shot).expect("written");
        }
    }
}
