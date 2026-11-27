//! The brand in the toolbar. Where no navigator draws it (the navigator is
//! hidden, or the window is a workspace's own) the toolbar's lead carries the
//! name and the version; it gives up the version as the bar narrows, and the
//! whole brand when not even the name fits.
//!
//! Only where Hangar draws the title bar, which is Linux and Windows.
#![cfg(any(target_os = "linux", target_os = "windows"))]

use std::borrow::Cow;

use iced::{Rectangle, Settings, Size};
use iced_test::Simulator;
use iced_test::selector::Candidate;

use super::*;
use crate::app::Packages;
use crate::navigator::testing::{ctx_in, detach, model_of, pane, workspace};
use crate::shell::title_bar::WORDMARK;
use crate::state::types::{AgentId, AppState};
use crate::workspace_dialog::testing::account;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Where the toolbar is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Where {
    /// The main window, its navigator hidden.
    Hidden,
    /// The main window, its navigator open.
    Open,
    /// A window of the workspace's own.
    Own,
}

/// Workspace `a`, called `name`, drawn `at`. Claude's accounts are loaded, as
/// they are for whoever uses Claude: the toolbar's trail is as wide as it
/// usually is.
fn setup(name: &str, at: Where) -> (Model, window::Id) {
    let mut model = model_of(AppState {
        workspaces: vec![workspace(
            "a",
            name,
            None,
            vec![pane("a1", "Ava", AgentId::Claude)],
        )],
        active_workspace_id: Some("a".into()),
        ..AppState::default()
    });
    let mut state = model.state().clone();
    state.settings.navigator_open = at == Where::Open;
    model.hydrate(state);
    model.ui.view = View::Terminals;
    model.live.accounts.accounts = Some(vec![
        account("main", "Principal", true),
        account("perso", "Perso", true),
    ]);
    let window = match at {
        Where::Own => detach(&mut model, "a"),
        _ => model.main_window().unwrap(),
    };
    (model, window)
}

/// The toolbar of `window`, in a window whose room for the bar itself (what
/// the title bar's buttons leave) is `net` px wide.
fn bar<'a>(
    st: &'a State,
    model: &'a Model,
    pkg: &'a Packages,
    window: window::Id,
    net: f32,
) -> Simulator<'a, Message> {
    let ctx = ctx_in(model, pkg, window);
    let width = net + title_bar::toolbar_room(ctx);
    let settings = Settings {
        fonts: vec![Cow::Borrowed(hangar_theme::fonts::INTER_VARIABLE)],
        default_font: hangar_theme::fonts::ui(),
        ..Settings::default()
    };
    Simulator::with_size(settings, Size::new(width, 44.0), view(st, ctx))
}

/// Every rectangle the bar draws.
fn rects(ui: &mut Simulator<'_, Message>) -> Vec<Rectangle> {
    let mut seen = Vec::new();
    // Nothing is selected: the selector only looks at what is there.
    let _ = ui.find(|candidate: Candidate<'_>| -> Option<()> {
        seen.push(candidate.bounds());
        None
    });
    seen
}

/// What of the brand a toolbar shows: the name, the version.
fn shown(ui: &mut Simulator<'_, Message>) -> (bool, bool) {
    (ui.find(WORDMARK).is_ok(), ui.find(VERSION).is_ok())
}

#[test]
fn the_brand_shows_in_the_toolbar_of_a_main_window_with_the_navigator_hidden() {
    let (model, window) = setup("alpha", Where::Hidden);
    let (pkg, st) = (Packages::default(), State::default());
    let mut ui = bar(&st, &model, &pkg, window, 1400.0);
    assert_eq!(shown(&mut ui), (true, true));
    assert!(ui.find("alpha").is_ok(), "the workspace's name stays");
}

#[test]
fn the_brand_shows_in_the_toolbar_of_a_workspaces_own_window() {
    let (model, window) = setup("alpha", Where::Own);
    let (pkg, st) = (Packages::default(), State::default());
    let mut ui = bar(&st, &model, &pkg, window, 1400.0);
    assert_eq!(shown(&mut ui), (true, true));
    assert!(ui.find("alpha").is_ok(), "the workspace's name stays");
}

#[test]
fn with_the_navigator_open_its_top_holds_the_brand_and_the_toolbar_does_not() {
    let (model, window) = setup("alpha", Where::Open);
    let (pkg, st) = (Packages::default(), State::default());
    let mut ui = bar(&st, &model, &pkg, window, 1400.0);
    assert_eq!(shown(&mut ui), (false, false));
    assert!(ui.find("alpha").is_ok());
}

#[test]
fn with_the_systems_title_bar_the_toolbar_has_no_brand() {
    // The window's own title carries the brand then, and no band is Hangar's.
    let (mut model, window) = setup("alpha", Where::Hidden);
    let mut state = model.state().clone();
    state.settings.system_title_bar = true;
    model.hydrate(state);
    let (pkg, st) = (Packages::default(), State::default());
    let mut ui = bar(&st, &model, &pkg, window, 1400.0);
    assert_eq!(shown(&mut ui), (false, false));
    assert!(ui.find("alpha").is_ok());
}

#[test]
fn the_brand_comes_before_the_workspaces_name_and_the_views() {
    for at in [Where::Hidden, Where::Own] {
        let (model, window) = setup("alpha", at);
        let (pkg, st) = (Packages::default(), State::default());
        let mut ui = bar(&st, &model, &pkg, window, 1400.0);
        let x = |ui: &mut Simulator<'_, Message>, text: &str| ui.find(text).unwrap().bounds().x;
        let (name, version) = (x(&mut ui, WORDMARK), x(&mut ui, VERSION));
        let (title, views) = (x(&mut ui, "alpha"), x(&mut ui, "Terminals"));
        assert!(name < version, "{at:?}");
        assert!(version < title && title < views, "{at:?}");
    }
}

#[test]
fn as_the_bar_narrows_the_version_goes_first_and_then_the_name() {
    // (where, the bar's room, what shows). The bar's steps are the toolbar's:
    // the workspace's name from 960, labels on the views from 768, the trail
    // folded under 552.
    let cases = [
        // The name and the version, wherever there is room.
        (Where::Own, 1400.0, (true, true)),
        (Where::Hidden, 1400.0, (true, true)),
        // The version goes first...
        (Where::Own, 620.0, (true, false)),
        // ...and the name is the last: it goes with the whole brand.
        (Where::Hidden, 800.0, (false, false)),
    ];
    for (at, net, expected) in cases {
        let (model, window) = setup("alpha", at);
        let (pkg, st) = (Packages::default(), State::default());
        let mut ui = bar(&st, &model, &pkg, window, net);
        assert_eq!(shown(&mut ui), expected, "{at:?} at {net}");
    }
}

#[test]
fn a_long_workspace_name_leaves_the_brand_less_room() {
    // At the same width, the bar of a workspace with a long name keeps the
    // name alone where the one of a short name has room for the version too.
    let long = "a-really-quite-long-workspace-name-for-the-bar";
    let net = 1040.0;
    let (model, window) = setup("alpha", Where::Hidden);
    let (pkg, st) = (Packages::default(), State::default());
    let mut short = bar(&st, &model, &pkg, window, net);
    assert!(short.find(VERSION).is_ok());
    drop(short);
    let (model, window) = setup(long, Where::Hidden);
    let mut name = bar(&st, &model, &pkg, window, net);
    assert!(name.find(WORDMARK).is_ok());
    assert!(name.find(VERSION).is_err());
}

#[test]
fn the_brand_never_pushes_the_trail_out_of_the_window() {
    // Whatever the width, what the bar draws ends where it did before the
    // brand: the trail's last button, a padding from the window's edge.
    for (at, name) in [
        (Where::Hidden, "alpha"),
        (
            Where::Hidden,
            "a-really-quite-long-workspace-name-for-the-bar",
        ),
        (Where::Own, "alpha"),
        (Where::Own, "a-really-quite-long-workspace-name-for-the-bar"),
    ] {
        let (model, window) = setup(name, at);
        let (pkg, st) = (Packages::default(), State::default());
        let ctx = ctx_in(&model, &pkg, window);
        let padding = if title_bar::flush_end(ctx) { 0.0 } else { 8.0 };
        // From the narrowest the windows go (the main window's 900 px, the
        // workspace's 640, less the buttons) to a wide screen.
        for net in (544..=1900).step_by(12) {
            let net = net as f32;
            let mut ui = bar(&st, &model, &pkg, window, net);
            let width = net + title_bar::toolbar_room(ctx);
            let right = rects(&mut ui)
                .iter()
                .filter(|r| r.x >= 1.0)
                .map(|r| r.x + r.width)
                .fold(0.0, f32::max);
            assert!(
                right <= width - padding + 0.5,
                "{at:?} {name}: the bar reaches {right} in a window {width} wide"
            );
        }
    }
}
