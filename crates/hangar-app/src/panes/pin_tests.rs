//! A pinned pane in the grid as it is drawn (`workspace_terminals::view`):
//! it keeps its edge and its pixels while workers open beside it, its
//! divider dragged sets the pixels it keeps (`StoreAction::ResizeSplit`),
//! and its header shows the pin that lets it go.

use iced::{Point, Size};
use iced_test::Simulator;

use super::*;
use crate::app::Packages;
use crate::navigator::testing::{WINDOW, ctx, model_of, pane, simulate, workspace};
use crate::panes::grid::CARD_GAP;
use crate::panes::header::pin_glyph_id;
use crate::panes::terminal_pane::slot_id;
use crate::state::types::{AgentId, AppState, PinEdge};

const WS: &str = "pin-grid";

/// What the grid divides in a window of [`WINDOW`]'s size, and says it
/// measured: the window less one gap each way.
const TILES: Size = Size::new(WINDOW.width - CARD_GAP, WINDOW.height - CARD_GAP);

/// Two shells side by side, Leo on the left and Term on the right, in a
/// grid that has measured itself at the window's size.
fn two_shells() -> (Model, Packages) {
    let m = model_of(AppState {
        workspaces: vec![workspace(
            WS,
            "Shop",
            None,
            vec![
                pane("leo", "Leo", AgentId::Shell),
                pane("term", "Term", AgentId::Shell),
            ],
        )],
        active_workspace_id: Some(WS.into()),
        ..AppState::default()
    });
    let mut pkg = Packages::default();
    pkg.panes.grid_sizes.insert(WS.into(), TILES);
    pkg.panes
        .mount_for_test(m.workspace(WS).expect("the workspace"));
    (m, pkg)
}

/// The grid of the workspace as it is drawn now, in a window of its size.
fn grid<'a>(m: &'a Model, pkg: &'a Packages) -> Simulator<'a, Message> {
    let ws = m.workspace(WS).expect("the workspace");
    simulate(workspace_terminals::view(&pkg.panes, ctx(m, pkg), ws))
}

/// Pins pane `id` (on), or lets it go, as the pin's messages do.
fn pin_pane(m: &mut Model, pkg: &Packages, id: &str, on: bool) {
    pin(&pkg.panes.grid_sizes, m, WS.into(), id.into(), on);
}

/// A worker opened as `worker_spawn` opens one (`control/writes.rs`): a
/// pane, then the agents' layout over the grid.
fn open_worker(m: &mut Model, pkg: &mut Packages) {
    let added = m
        .apply(StoreAction::AddPane {
            ws: WS.into(),
            agent: Some(AgentId::Shell),
            near: None,
            dir: None,
            cwd: None,
            name: None,
            init: None,
        })
        .pane;
    assert!(added.is_some(), "a pane opens");
    m.apply(StoreAction::ArrangePanes(WS.into()));
    pkg.panes
        .mount_for_test(m.workspace(WS).expect("the workspace"));
}

fn bounds_of(ui: &mut Simulator<'_, Message>, pane: &str) -> iced::Rectangle {
    ui.find(slot_id(pane))
        .unwrap_or_else(|_| panic!("pane {pane} is drawn"))
        .bounds()
}

fn at(ui: &mut Simulator<'_, Message>, x: f32, y: f32) {
    let position = Point::new(x, y);
    ui.point_at(position);
    ui.simulate([iced::Event::Mouse(iced::mouse::Event::CursorMoved {
        position,
    })]);
}

/// A boundary pressed at `from` and let go at `to`, along the middle row.
fn drag(ui: &mut Simulator<'_, Message>, from: f32, to: f32) {
    let y = WINDOW.height / 2.0;
    at(ui, from, y);
    ui.simulate([iced::Event::Mouse(iced::mouse::Event::ButtonPressed(
        iced::mouse::Button::Left,
    ))]);
    at(ui, (from + to) / 2.0, y);
    at(ui, to, y);
    ui.simulate([iced::Event::Mouse(iced::mouse::Event::ButtonReleased(
        iced::mouse::Button::Left,
    ))]);
}

fn panes_said(ui: Simulator<'_, Message>) -> Vec<Msg> {
    ui.into_messages()
        .filter_map(|message| match message {
            Message::Panes(msg) => Some(msg),
            _ => None,
        })
        .collect()
}

/// The boundaries the grid committed, by path and ratio.
fn ratios(ui: Simulator<'_, Message>) -> Vec<(layout::Path, f64)> {
    panes_said(ui)
        .into_iter()
        .filter_map(|msg| match msg {
            Msg::SetRatio(_, path, ratio) => Some((path, ratio)),
            _ => None,
        })
        .collect()
}

fn pinned_px(m: &Model, id: &str) -> f64 {
    m.workspace(WS)
        .and_then(|ws| ws.pin_of(id))
        .and_then(|pin| pin.px)
        .unwrap_or_else(|| panic!("{id} is pinned, in pixels"))
}

#[test]
fn a_pinned_pane_keeps_its_width_while_one_to_six_workers_open_beside_it() {
    let (mut m, mut pkg) = two_shells();
    pin_pane(&mut m, &pkg, "term", true);
    let kept = m.workspace(WS).unwrap().pin_of("term").cloned().unwrap();
    // On the edge it lay along, at the pixels it was drawn at: half of what
    // the grid divides.
    assert_eq!(kept.edge, PinEdge::Right);
    assert!((pinned_px(&m, "term") - 546.0).abs() < 0.5, "{kept:?}");

    let mut widths = Vec::new();
    for workers in 1..=6 {
        open_worker(&mut m, &mut pkg);
        let ws = m.workspace(WS).unwrap();
        assert_eq!(ws.panes.len(), 2 + workers);
        let mut ui = grid(&m, &pkg);
        let term = bounds_of(&mut ui, "term");
        // A gap in from the right edge, its 546 px less the gap its card
        // gives up.
        assert_eq!(
            term.x + term.width,
            WINDOW.width - CARD_GAP,
            "{workers} workers"
        );
        assert!(
            (term.width - 538.0).abs() <= 1.0,
            "{workers} workers: {term:?}"
        );
        // Every other pane shares the room it leaves.
        for other in ws.panes.iter().filter(|pane| pane.id != "term") {
            let bounds = bounds_of(&mut ui, &other.id);
            assert!(
                bounds.x + bounds.width <= term.x,
                "{workers} workers: {} at {bounds:?} reaches into {term:?}",
                other.id
            );
        }
        widths.push(term.width);
    }
    assert!(
        widths.windows(2).all(|pair| pair[0] == pair[1]),
        "the same width throughout: {widths:?}"
    );
    // The pixels it keeps never moved.
    assert!((pinned_px(&m, "term") - 546.0).abs() < 0.5);
}

#[test]
fn dragging_a_pinned_panes_divider_sets_the_pixels_it_keeps() {
    let (mut m, mut pkg) = two_shells();
    pin_pane(&mut m, &pkg, "term", true);
    // Its divider, at the middle of the window, dragged 150 px right.
    let mut ui = grid(&m, &pkg);
    drag(&mut ui, 550.0, 700.0);
    let said = ratios(ui);
    assert_eq!(said.len(), 1, "one write, on release: {said:?}");
    let (path, ratio) = said[0].clone();
    assert!(path.is_empty(), "the pin's split is the grid's first");
    assert!((ratio - 696.0 / 1092.0).abs() < 0.005, "{ratio}");

    // The engine keeps the pixels that is (`ResizeSplit`), not a share.
    set_ratio(&pkg.panes.grid_sizes, &mut m, WS.into(), path, ratio);
    assert!(
        (pinned_px(&m, "term") - 396.0).abs() < 2.0,
        "{}",
        pinned_px(&m, "term")
    );
    // Drawn so, and kept so as a worker opens.
    open_worker(&mut m, &mut pkg);
    let mut ui = grid(&m, &pkg);
    let term = bounds_of(&mut ui, "term");
    assert!((term.width - 388.0).abs() <= 2.0, "{term:?}");
    assert_eq!(term.x + term.width, WINDOW.width - CARD_GAP);
}

#[test]
fn a_pinned_panes_divider_stops_where_the_engine_keeps_it() {
    let (mut m, pkg) = two_shells();
    pin_pane(&mut m, &pkg, "term", true);
    // Dragged nearly to the left edge: the pinned pane would take 1000 px of
    // 1092; it takes the most the engine lets it, 70% of the grid, while it
    // is dragged and once let go.
    let mut ui = grid(&m, &pkg);
    drag(&mut ui, 550.0, 100.0);
    let said = ratios(ui);
    let (path, ratio) = said.last().cloned().expect("a ratio");
    assert!((ratio - 0.3).abs() < 1e-6, "{ratio}");
    set_ratio(&pkg.panes.grid_sizes, &mut m, WS.into(), path, ratio);
    assert!((pinned_px(&m, "term") - 764.4).abs() < 0.5);
    let mut ui = grid(&m, &pkg);
    let term = bounds_of(&mut ui, "term");
    assert!((term.width - 756.4).abs() <= 1.0, "{term:?}");
}

#[test]
fn an_unpinned_divider_still_sets_a_share_of_the_grid() {
    let (mut m, pkg) = two_shells();
    let mut ui = grid(&m, &pkg);
    drag(&mut ui, 550.0, 700.0);
    let (path, ratio) = ratios(ui).pop().expect("a ratio");
    set_ratio(&pkg.panes.grid_sizes, &mut m, WS.into(), path, ratio);
    let ws = m.workspace(WS).unwrap();
    assert!(ws.pins.is_empty());
    assert!(
        matches!(&ws.tree, Some(SplitNode::Split { ratio: r, .. }) if (r - ratio).abs() < 1e-9),
        "{:?}",
        ws.tree
    );
}

#[test]
fn a_pinned_pane_shows_a_pin_in_its_header_that_lets_it_go() {
    let (mut m, pkg) = two_shells();
    {
        let mut ui = grid(&m, &pkg);
        assert!(ui.find(pin_glyph_id("term")).is_err(), "nothing pinned yet");
    }
    pin_pane(&mut m, &pkg, "term", true);
    let said = {
        let mut ui = grid(&m, &pkg);
        assert!(
            ui.find(pin_glyph_id("leo")).is_err(),
            "only the pinned pane has one"
        );
        ui.click(pin_glyph_id("term"))
            .expect("the pin in Term's header");
        panes_said(ui)
    };
    let unpin = said.into_iter().find_map(|msg| match msg {
        Msg::Pin(ws, pane, on) => Some((ws, pane, on)),
        _ => None,
    });
    assert_eq!(unpin, Some((WS.to_string(), "term".to_string(), false)));

    // Handled, it lets the pane go: no pin, nothing in the header.
    pin_pane(&mut m, &pkg, "term", false);
    assert!(m.workspace(WS).unwrap().pins.is_empty());
    let mut ui = grid(&m, &pkg);
    assert!(ui.find(pin_glyph_id("term")).is_err());
}

#[test]
fn a_view_pane_pins_from_its_own_header_and_shows_the_pin() {
    let (mut m, pkg) = two_shells();
    m.apply(StoreAction::AddView {
        ws: WS.into(),
        id: "notes".into(),
        near: None,
        dir: None,
    });
    // Focused, its header shows its actions, the pin among them: a view has
    // no « … » menu to pin it from.
    m.ui.focus_by_workspace.insert(WS.into(), "notes".into());
    let said = {
        let mut ui = grid(&m, &pkg);
        assert!(ui.find(pin_glyph_id("notes")).is_err());
        ui.click(view_pane::pin_toggle_id("notes"))
            .expect("the view's pin button");
        panes_said(ui)
    };
    assert!(
        said.iter()
            .any(|msg| matches!(msg, Msg::Pin(_, pane, true) if pane == "notes")),
        "{said:?}"
    );
    pin_pane(&mut m, &pkg, "notes", true);
    assert!(m.workspace(WS).unwrap().pin_of("notes").is_some());
    let said = {
        let mut ui = grid(&m, &pkg);
        assert!(
            ui.find(pin_glyph_id("notes")).is_ok(),
            "pinned: the pin shows"
        );
        ui.click(view_pane::pin_toggle_id("notes"))
            .expect("the view's pin button");
        panes_said(ui)
    };
    assert!(
        said.iter()
            .any(|msg| matches!(msg, Msg::Pin(_, pane, false) if pane == "notes")),
        "pressed in, the button lets it go: {said:?}"
    );
}
