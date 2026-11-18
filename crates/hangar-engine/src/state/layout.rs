//! The pane tree (`src/lib/layout.ts`, tests in layout.test.ts).
//!
//! The split tree is the one source of truth for the arrangement. Panes are
//! named by id, never by position, so a pane can be closed or moved without
//! touching the identity of the others, and therefore without killing their
//! shells. Rectangles are fractions of the grid, so the grid can place every
//! pane from four numbers and never rebuild a terminal when the arrangement
//! changes.
//!
//! A pane can be pinned to an edge of the grid (`Workspace::pins`). The tree
//! then carves the pinned panes out first, each on its edge across the room
//! the pins before it leave, and the unpinned panes share what is left
//! ([`with_pins`]): whatever lays them out again (a pane added, closed or
//! moved, a preset, the agents' layout) works on that part alone, and the
//! pinned panes keep their place and their size.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use super::types::{DropZone, Pane, PaneId, PanePin, PinEdge, SplitDir, SplitNode, Workspace};

pub use super::types::MAX_PANES;

/// Smallest a pane may be dragged to, in logical pixels (`MIN_PANE_PX`), so it
/// stays usable while dragging.
pub const MIN_PANE_PX: f32 = 150.0;

/// One turn on the way from the root to a split: into its `a` or its `b`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    A,
    B,
}

/// The turns leading to a split node (`Path`).
pub type Path = Vec<Side>;

/// A path as the TS keyed it: `"ab"`, or `"root"` for the root.
pub fn path_key(path: &[Side]) -> String {
    if path.is_empty() {
        return "root".into();
    }
    path.iter()
        .map(|side| match side {
            Side::A => 'a',
            Side::B => 'b',
        })
        .collect()
}

/// Fractions of the grid, 0 to 1 (`Rect`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub const FULL: Rect = Rect {
        left: 0.0,
        top: 0.0,
        width: 1.0,
        height: 1.0,
    };
}

/// The boundary between the two sides of a split, which a drag moves.
#[derive(Debug, Clone, PartialEq)]
pub struct Handle {
    pub key: String,
    pub path: Path,
    pub dir: SplitDir,
    /// The line itself: no width for a row split, no height for a column.
    pub rect: Rect,
    /// What the split owns, to turn a pointer position into a ratio.
    pub bounds: Rect,
    /// The panes on either side, left or above, right or below, in reading
    /// order: what the handle's label names.
    pub before: Vec<PaneId>,
    pub after: Vec<PaneId>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GridLayout {
    pub rects: HashMap<PaneId, Rect>,
    pub handles: Vec<Handle>,
    /// Pane ids in reading order: what the numbering and Ctrl+N follow.
    pub order: Vec<PaneId>,
}

pub fn leaf(id: impl Into<PaneId>) -> SplitNode {
    SplitNode::Pane { id: id.into() }
}

/// The pane ids of a tree, left to right and top to bottom (`leafIds`).
pub fn leaf_ids(node: &SplitNode) -> Vec<PaneId> {
    fn walk(node: &SplitNode, out: &mut Vec<PaneId>) {
        match node {
            SplitNode::Pane { id } => out.push(id.clone()),
            SplitNode::Split { a, b, .. } => {
                walk(a, out);
                walk(b, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(node, &mut out);
    out
}

fn split(dir: SplitDir, ratio: f64, a: SplitNode, b: SplitNode) -> SplitNode {
    SplitNode::Split {
        dir,
        ratio,
        a: Box::new(a),
        b: Box::new(b),
    }
}

/// Folds nodes into a balanced binary split with even shares.
fn fold(dir: SplitDir, mut nodes: Vec<SplitNode>) -> SplitNode {
    if nodes.len() == 1 {
        return nodes.remove(0);
    }
    let mid = nodes.len().div_ceil(2);
    let ratio = mid as f64 / nodes.len() as f64;
    let rest = nodes.split_off(mid);
    split(dir, ratio, fold(dir, nodes), fold(dir, rest))
}

/// The even grid for a set of panes: one wide row up to two panes, then as
/// many columns as the rows allow, which gives the familiar 1, 2, 2×2 and 4×2
/// shapes (`presetTree`).
pub fn preset_tree(ids: &[PaneId]) -> SplitNode {
    if ids.len() <= 1 {
        return leaf(ids.first().cloned().unwrap_or_default());
    }
    let rows = ((ids.len() as f64 / 2.0).sqrt().ceil() as usize).max(1);
    let columns = ids.len().div_ceil(rows);
    let bands = ids
        .chunks(columns)
        .map(|band| fold(SplitDir::Row, band.iter().cloned().map(leaf).collect()))
        .collect();
    fold(SplitDir::Col, bands)
}

/// The share of the width the lead pane takes beside `others` panes: the most
/// of any pane whatever their number, less as they grow so theirs stay
/// usable.
fn lead_share(others: usize) -> f64 {
    match others {
        0 | 1 => 0.6,
        2 | 3 => 0.55,
        4..=7 => 0.5,
        _ => 0.4,
    }
}

/// The layout Hangar gives a workspace when an agent opens or closes a pane,
/// instead of halving one pane each time: the `lead` pane (the
/// orchestrator's) on the left, bigger than any other, and the rest in an
/// even grid beside it, in `order`. Without a lead, the even grid
/// ([`preset_tree`]).
///
/// The grid beside the lead has as many columns as keep its panes about as
/// wide for their height as the window is: one column up to four panes, two
/// from five, three from eleven.
pub fn arranged_tree(order: &[PaneId], lead: Option<&str>) -> SplitNode {
    let Some(lead) = lead.filter(|lead| order.iter().any(|id| id == lead)) else {
        return preset_tree(order);
    };
    let others: Vec<PaneId> = order.iter().filter(|id| *id != lead).cloned().collect();
    if others.is_empty() {
        return leaf(lead);
    }
    let share = lead_share(others.len());
    let columns = ((others.len() as f64 * (1.0 - share)).sqrt().round() as usize).max(1);
    // As many rows as the columns need, the panes dealt so that no row has
    // more than one pane over another: 13 in three columns are rows of 3, 3,
    // 3, 2 and 2, not four of 3 and one pane alone across the width.
    let rows = others.len().div_ceil(columns);
    let (base, extra) = (others.len() / rows, others.len() % rows);
    let mut rest = others.as_slice();
    let bands = (0..rows)
        .map(|row| {
            let (band, after) = rest.split_at(base + usize::from(row < extra));
            rest = after;
            fold(SplitDir::Row, band.iter().cloned().map(leaf).collect())
        })
        .collect();
    split(SplitDir::Row, share, leaf(lead), fold(SplitDir::Col, bands))
}

/// The tree with the ratio at `path` replaced (`withRatio`).
pub fn with_ratio(node: &SplitNode, path: &[Side], ratio: f64) -> SplitNode {
    match node {
        SplitNode::Pane { .. } => node.clone(),
        SplitNode::Split {
            dir,
            ratio: own,
            a,
            b,
        } => match path.split_first() {
            None => split(*dir, ratio, (**a).clone(), (**b).clone()),
            Some((Side::A, rest)) => split(*dir, *own, with_ratio(a, rest, ratio), (**b).clone()),
            Some((Side::B, rest)) => split(*dir, *own, (**a).clone(), with_ratio(b, rest, ratio)),
        },
    }
}

/// The ratio at `path`; a leaf answers an even half (`ratioAt`).
pub fn ratio_at(node: &SplitNode, path: &[Side]) -> f64 {
    match node {
        SplitNode::Pane { .. } => 0.5,
        SplitNode::Split { ratio, a, b, .. } => match path.split_first() {
            None => *ratio,
            Some((Side::A, rest)) => ratio_at(a, rest),
            Some((Side::B, rest)) => ratio_at(b, rest),
        },
    }
}

/// The tree without pane `id`, its sibling taking the whole space; `None`
/// once the last pane is gone (`removeLeaf`).
pub fn remove_leaf(node: &SplitNode, id: &str) -> Option<SplitNode> {
    match node {
        SplitNode::Pane { id: own } => (own != id).then(|| node.clone()),
        SplitNode::Split { dir, ratio, a, b } => match (remove_leaf(a, id), remove_leaf(b, id)) {
            (None, rest) | (rest, None) => rest,
            (Some(a), Some(b)) => Some(split(*dir, *ratio, a, b)),
        },
    }
}

/// The same arrangement with pane `from` now called `to`: a respawned pane
/// keeps its place (`renameLeaf`).
pub fn rename_leaf(node: &SplitNode, from: &str, to: &str) -> SplitNode {
    replace_leaf(node, from, &|_| leaf(to))
}

fn replace_leaf(node: &SplitNode, id: &str, make: &dyn Fn(&SplitNode) -> SplitNode) -> SplitNode {
    match node {
        SplitNode::Pane { id: own } if own == id => make(node),
        SplitNode::Pane { .. } => node.clone(),
        SplitNode::Split { dir, ratio, a, b } => split(
            *dir,
            *ratio,
            replace_leaf(a, id, make),
            replace_leaf(b, id, make),
        ),
    }
}

/// Splits `target` in two and puts `new_id` on the side `zone` names
/// (`insertLeaf`). `Center` is not a side; it is read as `Right`.
pub fn insert_leaf(node: &SplitNode, target: &str, new_id: &str, zone: DropZone) -> SplitNode {
    let dir = match zone {
        DropZone::Left | DropZone::Right | DropZone::Center => SplitDir::Row,
        DropZone::Top | DropZone::Bottom => SplitDir::Col,
    };
    let before = matches!(zone, DropZone::Left | DropZone::Top);
    replace_leaf(node, target, &|target| {
        let (a, b) = if before {
            (leaf(new_id), target.clone())
        } else {
            (target.clone(), leaf(new_id))
        };
        split(dir, 0.5, a, b)
    })
}

/// Exchanges two panes without changing the shape of the tree
/// (`swapLeaves`).
pub fn swap_leaves(node: &SplitNode, first: &str, second: &str) -> SplitNode {
    match node {
        SplitNode::Pane { id } if id == first => leaf(second),
        SplitNode::Pane { id } if id == second => leaf(first),
        SplitNode::Pane { .. } => node.clone(),
        SplitNode::Split { dir, ratio, a, b } => split(
            *dir,
            *ratio,
            swap_leaves(a, first, second),
            swap_leaves(b, first, second),
        ),
    }
}

/// A drop in the middle of a pane swaps the two; a drop on an edge splits
/// the target there (`moveLeaf`).
pub fn move_leaf(node: &SplitNode, drag: &str, target: &str, zone: DropZone) -> SplitNode {
    if drag == target {
        return node.clone();
    }
    if zone == DropZone::Center {
        return swap_leaves(node, drag, target);
    }
    match remove_leaf(node, drag) {
        Some(without) => insert_leaf(&without, target, drag, zone),
        None => node.clone(),
    }
}

/// A tree as `state.json` holds it, which may be a legacy one whose leaves
/// stored a pane's index rather than its id: mapped onto the panes' ids
/// (`migrate`). A split with one side unreadable is its other side; a ratio
/// outside (0, 1) is an even half.
pub fn migrate(raw: &Value, ids: &[PaneId]) -> Option<SplitNode> {
    let raw = raw.as_object()?;
    match raw.get("type").and_then(Value::as_str) {
        Some("pane") => {
            if let Some(id) = raw.get("id").and_then(Value::as_str) {
                return Some(leaf(id));
            }
            let index = raw.get("index")?.as_f64()?;
            if index < 0.0 || index.fract() != 0.0 {
                return None;
            }
            ids.get(index as usize).map(|id| leaf(id.clone()))
        }
        Some("split") => {
            let a = raw.get("a").and_then(|a| migrate(a, ids));
            let b = raw.get("b").and_then(|b| migrate(b, ids));
            let (a, b) = match (a, b) {
                (None, rest) | (rest, None) => return rest,
                (Some(a), Some(b)) => (a, b),
            };
            let ratio = raw
                .get("ratio")
                .and_then(Value::as_f64)
                .filter(|ratio| *ratio > 0.0 && *ratio < 1.0)
                .unwrap_or(0.5);
            let dir = match raw.get("dir").and_then(Value::as_str) {
                Some("col") => SplitDir::Col,
                _ => SplitDir::Row,
            };
            Some(split(dir, ratio, a, b))
        }
        _ => None,
    }
}

/// `migrate` for a tree already typed: the ratios the TS would not have kept.
fn sanitized(node: &SplitNode) -> SplitNode {
    match node {
        SplitNode::Pane { .. } => node.clone(),
        SplitNode::Split { dir, ratio, a, b } => {
            let ratio = if *ratio > 0.0 && *ratio < 1.0 {
                *ratio
            } else {
                0.5
            };
            split(*dir, ratio, sanitized(a), sanitized(b))
        }
    }
}

/// The tree, made to cover exactly the current panes (`normalizeTree`). A
/// stored tree that still matches comes back as it was, ratios included;
/// anything else is rebuilt as the even grid, in the order the person had.
///
/// It knows the terminals alone, as 1.x's did: the leaves of the views are
/// unknown to it and dropped. What lays out a workspace's grid reads
/// [`grid_tree`], which keeps them.
pub fn normalize_tree(panes: &[Pane], tree: Option<&SplitNode>) -> SplitNode {
    let ids: Vec<PaneId> = panes.iter().map(|pane| pane.id.clone()).collect();
    normalize_tree_ids(&ids, tree)
}

/// The workspace's grid, made to cover exactly its terminals and its views
/// (`Workspace::leaf_ids`), by the rules of [`normalize_tree`], its pinned
/// panes on their edges ([`pinned_tree`]): what is drawn, moved, resized,
/// numbered and laid out again.
pub fn grid_tree(ws: &Workspace) -> SplitNode {
    pinned_grid(ws).0
}

/// [`grid_tree`], and the pins that hold in it ([`live_pins`]), each with
/// its share of its room: what an arrangement of the unpinned panes is put
/// back into ([`with_pins`]).
pub fn pinned_grid(ws: &Workspace) -> (SplitNode, Vec<PanePin>) {
    let tree = normalize_tree_ids(&ws.leaf_ids(), ws.tree.as_ref());
    if ws.pins.is_empty() {
        return (tree, Vec::new());
    }
    let pins = with_shares(&tree, live_pins(ws));
    let tree = with_pins(&pins, unpinned_tree(&tree, &pins));
    (tree, pins)
}

/// Every pane of the grid, terminals and views, in the order they are read
/// on screen: what Ctrl+1…9, the next and previous pane, the presets and
/// the agents' layout follow.
pub fn grid_order(ws: &Workspace) -> Vec<PaneId> {
    if ws.pane_count() == 0 {
        return Vec::new();
    }
    leaf_ids(&grid_tree(ws))
}

/// [`normalize_tree`] over any set of leaf ids.
pub fn normalize_tree_ids(ids: &[PaneId], tree: Option<&SplitNode>) -> SplitNode {
    if ids.is_empty() {
        return leaf("");
    }
    let Some(tree) = tree.map(sanitized) else {
        return preset_tree(ids);
    };
    let known: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let mut seen = HashSet::new();
    let leaves = leaf_ids(&tree);
    let mut kept = Vec::new();
    for id in &leaves {
        if !known.contains(id.as_str()) || !seen.insert(id.clone()) {
            continue;
        }
        kept.push(id.clone());
    }
    if kept.len() == leaves.len() && kept.len() == ids.len() {
        return tree;
    }
    kept.extend(ids.iter().filter(|id| !seen.contains(*id)).cloned());
    preset_tree(&kept)
}

/// The panes in the order they are read on screen (`readingOrder`): the order
/// a preset keeps them in, so the panes seen first stay.
pub fn reading_order<'a>(panes: &'a [Pane], tree: Option<&SplitNode>) -> Vec<&'a Pane> {
    let by_id: HashMap<&str, &Pane> = panes.iter().map(|pane| (pane.id.as_str(), pane)).collect();
    leaf_ids(&normalize_tree(panes, tree))
        .iter()
        .filter_map(|id| by_id.get(id.as_str()).copied())
        .collect()
}

/// How many panes a preset asking for `count` ends up with (`presetSize`).
pub fn preset_size(count: usize) -> usize {
    count.clamp(1, MAX_PANES)
}

/// The panes a preset of `count` closes, in reading order (`closedByPreset`).
pub fn closed_by_preset<'a>(
    panes: &'a [Pane],
    tree: Option<&SplitNode>,
    count: usize,
) -> Vec<&'a Pane> {
    reading_order(panes, tree)
        .into_iter()
        .skip(preset_size(count))
        .collect()
}

/// How many terminals a preset asking for `count` leaves a workspace that
/// holds `views` view panes: a preset sets the number of terminals and
/// keeps every view, so the terminals have what `MAX_PANES` leaves them,
/// and at least one.
pub fn preset_terminals(count: usize, views: usize) -> usize {
    preset_size(count)
        .min(MAX_PANES.saturating_sub(views))
        .max(1)
}

/// How many unpinned terminals a preset asking for `count` leaves workspace
/// `ws`: a preset sets the number of the terminals that are not pinned, and
/// keeps every pinned pane and every view, so those have the room
/// `MAX_PANES` leaves them ([`preset_terminals`] with no pin), and at least
/// one when no terminal is pinned.
pub fn preset_terminals_in(ws: &Workspace, count: usize) -> usize {
    let pinned = ws
        .panes
        .iter()
        .filter(|pane| ws.pin_of(&pane.id).is_some())
        .count();
    preset_size(count)
        .min(MAX_PANES.saturating_sub(ws.view_panes.len() + pinned))
        .max(usize::from(pinned == 0))
}

/// The terminals a preset of `count` closes in workspace `ws`, in reading
/// order: [`closed_by_preset`] with the room its views and its pinned panes
/// take. A pinned pane is never one of them.
pub fn closed_by_preset_in(ws: &Workspace, count: usize) -> Vec<&Pane> {
    reading_order(&ws.panes, ws.tree.as_ref())
        .into_iter()
        .filter(|pane| ws.pin_of(&pane.id).is_none())
        .skip(preset_terminals_in(ws, count))
        .collect()
}

/// The tree as flat rectangles and the boundaries between them
/// (`computeLayout`). Ratios are clamped to 5–95 % so no pane is crushed.
pub fn compute_layout(tree: &SplitNode) -> GridLayout {
    fn walk(node: &SplitNode, rect: Rect, path: &mut Path, out: &mut GridLayout) {
        let SplitNode::Split { dir, ratio, a, b } = node else {
            if let SplitNode::Pane { id } = node {
                out.rects.insert(id.clone(), rect);
                out.order.push(id.clone());
            }
            return;
        };
        let ratio = ratio.clamp(0.05, 0.95);
        let (first, second, line) = match dir {
            SplitDir::Row => {
                let width = rect.width * ratio;
                (
                    Rect { width, ..rect },
                    Rect {
                        left: rect.left + width,
                        width: rect.width - width,
                        ..rect
                    },
                    Rect {
                        left: rect.left + width,
                        top: rect.top,
                        width: 0.0,
                        height: rect.height,
                    },
                )
            }
            SplitDir::Col => {
                let height = rect.height * ratio;
                (
                    Rect { height, ..rect },
                    Rect {
                        top: rect.top + height,
                        height: rect.height - height,
                        ..rect
                    },
                    Rect {
                        left: rect.left,
                        top: rect.top + height,
                        width: rect.width,
                        height: 0.0,
                    },
                )
            }
        };
        path.push(Side::A);
        walk(a, first, path, out);
        path.pop();
        path.push(Side::B);
        walk(b, second, path, out);
        path.pop();
        out.handles.push(Handle {
            key: path_key(path),
            path: path.clone(),
            dir: *dir,
            rect: line,
            bounds: rect,
            before: leaf_ids(a),
            after: leaf_ids(b),
        });
    }
    let mut out = GridLayout::default();
    walk(tree, Rect::FULL, &mut Vec::new(), &mut out);
    out
}

/// The half of a pane a dropped pane would take, previewed while dragging
/// (`zoneRect`); the middle is the whole pane.
pub fn zone_rect(rect: Rect, zone: DropZone) -> Rect {
    match zone {
        DropZone::Left => Rect {
            width: rect.width / 2.0,
            ..rect
        },
        DropZone::Right => Rect {
            left: rect.left + rect.width / 2.0,
            width: rect.width / 2.0,
            ..rect
        },
        DropZone::Top => Rect {
            height: rect.height / 2.0,
            ..rect
        },
        DropZone::Bottom => Rect {
            top: rect.top + rect.height / 2.0,
            height: rect.height / 2.0,
            ..rect
        },
        DropZone::Center => rect,
    }
}

/// Which side of `rect` (in pixels) the point is closest to; the middle
/// third means a swap (`zoneAt`).
pub fn zone_at(rect: Rect, x: f64, y: f64) -> DropZone {
    let dx = (x - rect.left) / rect.width.max(1.0);
    let dy = (y - rect.top) / rect.height.max(1.0);
    if dx > 0.33 && dx < 0.67 && dy > 0.33 && dy < 0.67 {
        return DropZone::Center;
    }
    let mut zone = DropZone::Left;
    let mut best = dx;
    if 1.0 - dx < best {
        best = 1.0 - dx;
        zone = DropZone::Right;
    }
    if dy < best {
        best = dy;
        zone = DropZone::Top;
    }
    if 1.0 - dy < best {
        zone = DropZone::Bottom;
    }
    zone
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// The pane next to `from` in direction `dir`, or `None` at the edge of the
/// grid (`neighborOf`).
///
/// Read off the rectangles rather than the tree: "the pane on the left" is
/// what is on screen, and two panes can sit side by side without being
/// siblings. A candidate must lie entirely past our edge and face us (overlap
/// on the other axis); the closest wins, then the one facing us the most,
/// then the first in reading order.
pub fn neighbor_of(tree: &SplitNode, from: &str, dir: Direction) -> Option<PaneId> {
    const EPSILON: f64 = 1e-4;
    let layout = compute_layout(tree);
    let own = *layout.rects.get(from)?;
    let horizontal = matches!(dir, Direction::Left | Direction::Right);
    let backwards = matches!(dir, Direction::Left | Direction::Up);

    let near = if horizontal { own.left } else { own.top };
    let far = near + if horizontal { own.width } else { own.height };
    let side_start = if horizontal { own.top } else { own.left };
    let side_end = side_start + if horizontal { own.height } else { own.width };

    let mut best: Option<PaneId> = None;
    let mut best_gap = f64::INFINITY;
    let mut best_overlap = 0.0;
    for id in &layout.order {
        if id == from {
            continue;
        }
        let rect = layout.rects[id];
        let start = if horizontal { rect.left } else { rect.top };
        let end = start + if horizontal { rect.width } else { rect.height };
        let gap = if backwards { near - end } else { start - far };
        if gap < -EPSILON {
            continue;
        }
        let other_start = if horizontal { rect.top } else { rect.left };
        let other_end = other_start + if horizontal { rect.height } else { rect.width };
        let overlap = side_end.min(other_end) - side_start.max(other_start);
        if overlap <= EPSILON {
            continue;
        }
        if gap < best_gap - EPSILON || (gap <= best_gap + EPSILON && overlap > best_overlap) {
            best = Some(id.clone());
            best_gap = gap;
            best_overlap = overlap;
        }
    }
    best
}

/// The direction to split a pane of this size in: along its longer side, so
/// panes stay as square as they can (`preferredDir`). Without a size, a row.
pub fn preferred_dir(size: Option<(f32, f32)>) -> SplitDir {
    match size {
        Some((width, height)) if width < height * 1.15 => SplitDir::Col,
        _ => SplitDir::Row,
    }
}

// ---- Pinned panes

/// The most of its room a pinned pane takes, whatever size it was pinned
/// at: what is left stays usable, and a window that shrinks takes from the
/// pinned pane past that point.
pub const MAX_PIN_SHARE: f64 = 0.7;

/// The least of its room a pinned pane takes, as little as
/// [`compute_layout`] leaves any pane.
pub const MIN_PIN_SHARE: f64 = 0.05;

/// The share of a pin that says none, and whose pane the tree does not show.
const DEFAULT_PIN_SHARE: f64 = 0.3;

fn bounded_share(share: f64) -> f64 {
    if share.is_nan() {
        return DEFAULT_PIN_SHARE;
    }
    share.clamp(MIN_PIN_SHARE, MAX_PIN_SHARE)
}

/// A pin's share of its room, within bounds.
fn share_of(pin: &PanePin) -> f64 {
    pin.share().map_or(DEFAULT_PIN_SHARE, bounded_share)
}

/// The ratio of a pin's split that gives the pinned pane `share` of it.
fn pin_ratio(edge: PinEdge, share: f64) -> f64 {
    if edge.leads() { share } else { 1.0 - share }
}

/// The pinned pane's share of its split at `ratio`: [`pin_ratio`] turned
/// back, which is the same turn.
fn pin_share(edge: PinEdge, ratio: f64) -> f64 {
    pin_ratio(edge, ratio)
}

fn is_pinned(pins: &[PanePin], id: &str) -> bool {
    pins.iter().any(|pin| pin.id == id)
}

/// `rect`'s extent along `dir`: its width for a row, its height for a
/// column.
fn along(rect: &Rect, dir: SplitDir) -> f64 {
    match dir {
        SplitDir::Row => rect.width,
        SplitDir::Col => rect.height,
    }
}

/// `room` without what a pin of `share` takes of it on its edge.
fn room_left(room: Rect, edge: PinEdge, share: f64) -> Rect {
    match edge {
        PinEdge::Left => {
            let taken = room.width * share;
            Rect {
                left: room.left + taken,
                width: room.width - taken,
                ..room
            }
        }
        PinEdge::Right => Rect {
            width: room.width * pin_ratio(edge, share),
            ..room
        },
        PinEdge::Top => {
            let taken = room.height * share;
            Rect {
                top: room.top + taken,
                height: room.height - taken,
                ..room
            }
        }
        PinEdge::Bottom => Rect {
            height: room.height * pin_ratio(edge, share),
            ..room
        },
    }
}

/// The pins of `ws` that hold, in order, the first the outermost: each of a
/// pane of its grid, each pane once (its first pin).
pub fn live_pins(ws: &Workspace) -> Vec<PanePin> {
    let mut seen = HashSet::new();
    ws.pins
        .iter()
        .filter(|pin| ws.has_leaf(&pin.id) && seen.insert(pin.id.as_str()))
        .cloned()
        .collect()
}

/// `pins`, each with a share of its room: its own, within bounds, or
/// failing that the one `tree` gives its pane now, of the room the pins
/// before it leave.
fn with_shares(tree: &SplitNode, mut pins: Vec<PanePin>) -> Vec<PanePin> {
    if pins.iter().all(|pin| pin.share().is_some()) {
        return pins;
    }
    let layout = compute_layout(tree);
    let mut room = Rect::FULL;
    for pin in &mut pins {
        let share = match pin.share() {
            Some(share) => bounded_share(share),
            None => {
                let dir = pin.edge.dir();
                let share = layout.rects.get(&pin.id).map_or(DEFAULT_PIN_SHARE, |rect| {
                    along(rect, dir) / along(&room, dir).max(f64::EPSILON)
                });
                let share = bounded_share(share);
                pin.fraction = Some(share);
                share
            }
        };
        room = room_left(room, pin.edge, share);
    }
    pins
}

/// `tree` without the leaves of `pins`, each sibling taking the room: the
/// unpinned panes, as `tree` arranges them. None when every pane is pinned.
pub fn unpinned_tree(tree: &SplitNode, pins: &[PanePin]) -> Option<SplitNode> {
    pins.iter()
        .try_fold(tree.clone(), |tree, pin| remove_leaf(&tree, &pin.id))
}

/// `inner`, the unpinned panes' tree, with `pins` carved out around it in
/// order, the first the outermost: each pinned pane on its edge, across the
/// whole of the room the pins before it leave, at its share of that room.
/// Two pins on one edge stand side by side, the first against the edge.
/// With no unpinned pane, the last pin takes the room the others leave.
pub fn with_pins(pins: &[PanePin], inner: Option<SplitNode>) -> SplitNode {
    pins.iter()
        .rev()
        .fold(inner, |inner, pin| {
            let pinned = leaf(pin.id.clone());
            Some(match inner {
                None => pinned,
                Some(rest) => {
                    let ratio = pin_ratio(pin.edge, share_of(pin));
                    if pin.edge.leads() {
                        split(pin.edge.dir(), ratio, pinned, rest)
                    } else {
                        split(pin.edge.dir(), ratio, rest, pinned)
                    }
                }
            })
        })
        .unwrap_or_else(|| leaf(""))
}

/// `tree` with `pins` holding ([`with_pins`]), the unpinned panes keeping
/// the arrangement they have in it. A tree that holds them already comes
/// back as it is.
pub fn pinned_tree(tree: &SplitNode, pins: &[PanePin]) -> SplitNode {
    if pins.is_empty() {
        return tree.clone();
    }
    let pins = with_shares(tree, pins.to_vec());
    with_pins(&pins, unpinned_tree(tree, &pins))
}

/// The sides of `node` when it is `pin`'s split as [`with_pins`] makes it:
/// the pinned leaf, and the room it leaves.
fn carved<'a>(node: &'a SplitNode, pin: &PanePin) -> Option<(&'a SplitNode, &'a SplitNode)> {
    let SplitNode::Split { dir, a, b, .. } = node else {
        return None;
    };
    let (pinned, rest) = if pin.edge.leads() {
        (&**a, &**b)
    } else {
        (&**b, &**a)
    };
    let holds = *dir == pin.edge.dir() && matches!(pinned, SplitNode::Pane { id } if *id == pin.id);
    holds.then_some((pinned, rest))
}

/// What a pin takes of a room `span` pixels long, as a share of it: its
/// pixels (its share of the room when it has none), within `MIN_PANE_PX`
/// and [`MAX_PIN_SHARE`] of the room; the most when the room is too small
/// for both.
fn sized_share(pin: &PanePin, span: f64) -> f64 {
    if !(span.is_finite() && span > 0.0) {
        return share_of(pin);
    }
    let wanted = pin.pixels().unwrap_or_else(|| share_of(pin) * span);
    let most = span * MAX_PIN_SHARE;
    let least = f64::from(MIN_PANE_PX).min(most);
    wanted.clamp(least, most) / span
}

/// The grid's tree for a grid of `size` logical pixels: each pin's split
/// gives its pane the pixels it was pinned at ([`sized_share`]) of the room
/// the pins before it leave, so a terminal keeps its columns however the
/// window and the other panes change. The rest of the tree is as it was.
/// `tree` is a pinned tree of these `pins` ([`pinned_tree`]); a split that
/// is not one is left as it is, and those inside it too.
pub fn sized_tree(tree: &SplitNode, pins: &[PanePin], size: (f32, f32)) -> SplitNode {
    fn walk(node: &SplitNode, pins: &[PanePin], room: (f64, f64)) -> SplitNode {
        let Some((pin, inside)) = pins.split_first() else {
            return node.clone();
        };
        let Some((pinned, rest)) = carved(node, pin) else {
            return node.clone();
        };
        let dir = pin.edge.dir();
        let span = match dir {
            SplitDir::Row => room.0,
            SplitDir::Col => room.1,
        };
        let share = sized_share(pin, span);
        let taken = span * share;
        let room = match dir {
            SplitDir::Row => (room.0 - taken, room.1),
            SplitDir::Col => (room.0, room.1 - taken),
        };
        let rest = walk(rest, inside, room);
        let ratio = pin_ratio(pin.edge, share);
        if pin.edge.leads() {
            split(dir, ratio, pinned.clone(), rest)
        } else {
            split(dir, ratio, rest, pinned.clone())
        }
    }
    walk(tree, pins, (f64::from(size.0), f64::from(size.1)))
}

/// The workspace's grid as it is drawn at `size` logical pixels, once the
/// grid knows its size ([`sized_tree`]); its tree ([`grid_tree`]) until
/// then.
pub fn drawn_tree(ws: &Workspace, size: Option<(f32, f32)>) -> SplitNode {
    let (tree, pins) = pinned_grid(ws);
    match size {
        Some(size) if !pins.is_empty() => sized_tree(&tree, &pins, size),
        _ => tree,
    }
}

/// `pins` without pane `id`'s (unpinned, or closed), the pins inside it
/// keeping the size `tree`, a pinned tree of `pins`, gives them now: their
/// share of a room that grew is made smaller.
pub fn pins_without(tree: &SplitNode, pins: &[PanePin], id: &str) -> Vec<PanePin> {
    let Some(at) = pins.iter().position(|pin| pin.id == id) else {
        return pins.to_vec();
    };
    let mut rest = pins.to_vec();
    rest.remove(at);
    for pin in &mut rest[at..] {
        pin.fraction = None;
    }
    with_shares(tree, rest)
}

/// The edge a pane at `rect` is pinned to in `room`, the room the other
/// pins leave: among the edges of the room it lies along, the one it covers
/// the most of, so that pinning it changes it the least; the nearest when
/// it lies along none. [`PinEdge::ALL`] settles a tie: a pane alone in its
/// room goes to the right.
pub fn pin_edge(rect: Rect, room: Rect) -> PinEdge {
    const EPSILON: f64 = 1e-6;
    let gap = |edge: PinEdge| match edge {
        PinEdge::Left => rect.left - room.left,
        PinEdge::Right => (room.left + room.width) - (rect.left + rect.width),
        PinEdge::Top => rect.top - room.top,
        PinEdge::Bottom => (room.top + room.height) - (rect.top + rect.height),
    };
    let cover = |edge: PinEdge| match edge.dir() {
        SplitDir::Row => rect.height / room.height.max(EPSILON),
        SplitDir::Col => rect.width / room.width.max(EPSILON),
    };
    let mut best = PinEdge::ALL[0];
    let mut best_score = f64::NEG_INFINITY;
    let touching = PinEdge::ALL.iter().any(|edge| gap(*edge).abs() < EPSILON);
    for edge in PinEdge::ALL {
        let score = if touching {
            if gap(edge).abs() >= EPSILON {
                continue;
            }
            cover(edge)
        } else {
            -gap(edge)
        };
        if score > best_score + EPSILON {
            best = edge;
            best_score = score;
        }
    }
    best
}

/// The smallest rectangle holding both.
fn bounding(a: Rect, b: Rect) -> Rect {
    let left = a.left.min(b.left);
    let top = a.top.min(b.top);
    Rect {
        left,
        top,
        width: (a.left + a.width).max(b.left + b.width) - left,
        height: (a.top + a.height).max(b.top + b.height) - top,
    }
}

/// The pins of workspace `ws` once pane `id` (a terminal or a view) is
/// pinned to `edge`, or to the edge it lies along when none is given
/// ([`pin_edge`]), last of them, so innermost: its size is its size along
/// that axis as the grid shows it now, in pixels when the grid's `size` is
/// known, and as its share of the room the other pins leave. A pane pinned
/// already is taken as unpinned first: pinned again, it moves inside the
/// others. None when it is no pane of the grid.
pub fn pinned_with(
    ws: &Workspace,
    id: &str,
    edge: Option<PinEdge>,
    size: Option<(f32, f32)>,
) -> Option<Vec<PanePin>> {
    if !ws.has_leaf(id) {
        return None;
    }
    let (tree, pins) = pinned_grid(ws);
    let mut pins = pins_without(&tree, &pins, id);
    let tree = with_pins(&pins, unpinned_tree(&tree, &pins));
    let drawn = match size {
        Some(size) => sized_tree(&tree, &pins, size),
        None => tree,
    };
    let layout = compute_layout(&drawn);
    let rect = *layout.rects.get(id)?;
    // The room the other pins leave is what the unpinned panes cover.
    let room = layout
        .rects
        .iter()
        .filter(|(pane, _)| !is_pinned(&pins, pane))
        .map(|(_, rect)| *rect)
        .reduce(bounding)
        .unwrap_or(rect);
    let edge = edge.unwrap_or_else(|| pin_edge(rect, room));
    let dir = edge.dir();
    let extent = along(&rect, dir);
    let span = |(width, height): (f32, f32)| match dir {
        SplitDir::Row => f64::from(width),
        SplitDir::Col => f64::from(height),
    };
    pins.push(PanePin {
        px: size.map(|size| extent * span(size)),
        fraction: Some(bounded_share(extent / along(&room, dir).max(f64::EPSILON))),
        ..PanePin::new(id, edge)
    });
    Some(pins)
}

/// The pins after the grid's tree went from `before` to `after` (a divider
/// dragged), both pinned trees of `pins`: a pin whose split the drag moved
/// takes its new share, within bounds, its pixels following in proportion
/// (here the grid's size is not known; [`pin_resized`] knows it). The
/// others are as they were.
pub fn pins_dragged(before: &SplitNode, after: &SplitNode, pins: &[PanePin]) -> Vec<PanePin> {
    let mut out = pins.to_vec();
    let (mut before, mut after) = (before, after);
    for pin in &mut out {
        let (Some((_, was)), Some((_, now))) = (carved(before, pin), carved(after, pin)) else {
            break;
        };
        let (SplitNode::Split { ratio: old, .. }, SplitNode::Split { ratio: new, .. }) =
            (before, after)
        else {
            break;
        };
        if old != new {
            let old_share = pin_share(pin.edge, old.clamp(0.05, 0.95));
            let share = bounded_share(pin_share(pin.edge, *new));
            pin.px = pin.pixels().map(|px| px * share / old_share);
            pin.fraction = Some(share);
        }
        before = was;
        after = now;
    }
    out
}

/// `pins` with the one whose split lies at `path` of `tree`, a pinned tree
/// of them, dragged to `ratio` in a grid of `size` logical pixels: it takes
/// that share of its room, within bounds, and the pixels that is when the
/// size is known (in proportion to those it had when not). None when no
/// pin's split lies at `path`.
pub fn pin_resized(
    tree: &SplitNode,
    pins: &[PanePin],
    path: &[Side],
    ratio: f64,
    size: Option<(f32, f32)>,
) -> Option<Vec<PanePin>> {
    let mut node = tree;
    let mut room = size.map(|(width, height)| (f64::from(width), f64::from(height)));
    let mut at: Path = Vec::new();
    for (index, pin) in pins.iter().enumerate() {
        let (_, rest) = carved(node, pin)?;
        let dir = pin.edge.dir();
        let span = room.map(|(width, height)| match dir {
            SplitDir::Row => width,
            SplitDir::Col => height,
        });
        if at == path {
            let share = bounded_share(pin_share(pin.edge, ratio));
            let mut out = pins.to_vec();
            out[index].px = match span {
                Some(span) => Some(share * span),
                None => pin.pixels().map(|px| px * share / share_of(pin)),
            };
            out[index].fraction = Some(share);
            return Some(out);
        }
        if let (Some(room), Some(span)) = (room.as_mut(), span) {
            let taken = span * sized_share(pin, span);
            match dir {
                SplitDir::Row => room.0 -= taken,
                SplitDir::Col => room.1 -= taken,
            }
        }
        at.push(if pin.edge.leads() { Side::B } else { Side::A });
        node = rest;
    }
    None
}

/// `tree`, a pinned tree of `pins`, with a new leaf `id` beside pane `near`
/// on the side `zone` says, among the unpinned panes: a pane added never
/// takes a pinned pane's room. Beside a pinned pane, it lands beside the
/// unpinned pane next to it, towards the middle of the grid; without
/// `near`, or with one that is not in the tree, beside the unpinned pane
/// read last; with every pane pinned, it takes the room they leave.
pub fn add_beside(
    tree: &SplitNode,
    pins: &[PanePin],
    near: Option<&str>,
    id: &str,
    zone: DropZone,
) -> SplitNode {
    let Some(inner) = unpinned_tree(tree, pins) else {
        return with_pins(pins, Some(leaf(id)));
    };
    let target = landing(tree, pins, near)
        .or_else(|| leaf_ids(&inner).pop())
        .unwrap_or_default();
    with_pins(pins, Some(insert_leaf(&inner, &target, id, zone)))
}

/// The unpinned pane a pane added beside `near` goes beside ([`add_beside`]).
fn landing(tree: &SplitNode, pins: &[PanePin], near: Option<&str>) -> Option<PaneId> {
    let near = near?;
    let Some(pin) = pins.iter().find(|pin| pin.id == near) else {
        return leaf_ids(tree).into_iter().find(|id| id == near);
    };
    let inwards = match pin.edge {
        PinEdge::Left => Direction::Right,
        PinEdge::Right => Direction::Left,
        PinEdge::Top => Direction::Down,
        PinEdge::Bottom => Direction::Up,
    };
    // Across the pins stacked on the way.
    let mut from = near.to_string();
    for _ in 0..pins.len() {
        let next = neighbor_of(tree, &from, inwards)?;
        if !is_pinned(pins, &next) {
            return Some(next);
        }
        from = next;
    }
    None
}

#[cfg(test)]
mod tests {
    //! layout.test.ts, case by case.

    use super::*;
    use serde_json::json;

    fn split_of(dir: SplitDir, a: SplitNode, b: SplitNode, ratio: f64) -> SplitNode {
        split(dir, ratio, a, b)
    }
    fn row(a: SplitNode, b: SplitNode) -> SplitNode {
        split(SplitDir::Row, 0.5, a, b)
    }
    fn col(a: SplitNode, b: SplitNode) -> SplitNode {
        split(SplitDir::Col, 0.5, a, b)
    }
    fn panes(ids: &[&str]) -> Vec<Pane> {
        ids.iter()
            .map(|id| Pane {
                id: (*id).into(),
                ..Pane::default()
            })
            .collect()
    }
    fn ids(list: &[&str]) -> Vec<PaneId> {
        list.iter().map(|id| (*id).to_string()).collect()
    }
    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }
    fn of<'a>(panes: &[&'a Pane]) -> Vec<&'a str> {
        panes.iter().map(|pane| pane.id.as_str()).collect()
    }

    // presetTree

    #[test]
    fn is_a_single_leaf_for_one_pane() {
        assert_eq!(preset_tree(&ids(&["a"])), leaf("a"));
    }

    #[test]
    fn puts_two_panes_side_by_side() {
        assert_eq!(preset_tree(&ids(&["a", "b"])), row(leaf("a"), leaf("b")));
    }

    #[test]
    fn lays_four_panes_out_as_an_even_2x2() {
        let layout = compute_layout(&preset_tree(&ids(&["a", "b", "c", "d"])));
        assert_eq!(layout.order, ids(&["a", "b", "c", "d"]));
        assert_eq!(
            layout.rects["a"],
            Rect {
                left: 0.0,
                top: 0.0,
                width: 0.5,
                height: 0.5
            }
        );
        assert_eq!(
            layout.rects["d"],
            Rect {
                left: 0.5,
                top: 0.5,
                width: 0.5,
                height: 0.5
            }
        );
    }

    #[test]
    fn lays_eight_panes_out_as_4x2() {
        let list = ids(&["1", "2", "3", "4", "5", "6", "7", "8"]);
        let layout = compute_layout(&preset_tree(&list));
        for id in &list {
            assert!(close(layout.rects[id].width, 0.25));
            assert!(close(layout.rects[id].height, 0.5));
        }
        assert!(close(layout.rects["5"].top, 0.5));
    }

    #[test]
    fn lets_the_odd_pane_out_take_the_whole_last_row() {
        let layout = compute_layout(&preset_tree(&ids(&["a", "b", "c"])));
        assert_eq!(
            layout.rects["c"],
            Rect {
                left: 0.0,
                top: 0.5,
                width: 1.0,
                height: 0.5
            }
        );
    }

    #[test]
    fn tidies_a_lopsided_tree_into_an_even_grid_in_reading_order() {
        let lopsided = split_of(
            SplitDir::Row,
            leaf("a"),
            split_of(
                SplitDir::Col,
                leaf("b"),
                split_of(
                    SplitDir::Row,
                    leaf("c"),
                    split_of(SplitDir::Col, leaf("d"), leaf("e"), 0.2),
                    0.8,
                ),
                0.3,
            ),
            0.15,
        );
        let tidy = preset_tree(&leaf_ids(&normalize_tree(
            &panes(&["a", "b", "c", "d", "e"]),
            Some(&lopsided),
        )));
        assert_eq!(leaf_ids(&tidy), ids(&["a", "b", "c", "d", "e"]));
        let layout = compute_layout(&tidy);
        for id in ["a", "b", "c"] {
            assert!(close(layout.rects[id].width, 1.0 / 3.0));
            assert!(close(layout.rects[id].height, 0.5));
        }
        for id in ["d", "e"] {
            assert!(close(layout.rects[id].width, 0.5));
            assert!(close(layout.rects[id].top, 0.5));
        }
    }

    // splitting and closing

    #[test]
    fn puts_the_new_pane_on_the_side_it_was_dropped_on() {
        let a = leaf("a");
        assert_eq!(
            insert_leaf(&a, "a", "n", DropZone::Right),
            row(leaf("a"), leaf("n"))
        );
        assert_eq!(
            insert_leaf(&a, "a", "n", DropZone::Left),
            row(leaf("n"), leaf("a"))
        );
        assert_eq!(
            insert_leaf(&a, "a", "n", DropZone::Top),
            col(leaf("n"), leaf("a"))
        );
        assert_eq!(
            insert_leaf(&a, "a", "n", DropZone::Bottom),
            col(leaf("a"), leaf("n"))
        );
    }

    #[test]
    fn splits_only_the_target_deep_in_the_tree() {
        let tree = row(leaf("a"), col(leaf("b"), leaf("c")));
        let next = insert_leaf(&tree, "c", "n", DropZone::Right);
        assert_eq!(leaf_ids(&next), ids(&["a", "b", "c", "n"]));
        // The untouched branch is the same (the TS shared it; here it is an
        // equal copy).
        let (SplitNode::Split { a: next_a, .. }, SplitNode::Split { a: tree_a, .. }) =
            (&next, &tree)
        else {
            panic!("splits expected");
        };
        assert_eq!(next_a, tree_a);
    }

    #[test]
    fn gives_a_closed_pane_s_space_to_its_sibling() {
        let tree = split_of(
            SplitDir::Row,
            leaf("a"),
            split_of(SplitDir::Col, leaf("b"), leaf("c"), 0.3),
            0.7,
        );
        assert_eq!(
            remove_leaf(&tree, "b"),
            Some(split_of(SplitDir::Row, leaf("a"), leaf("c"), 0.7))
        );
        assert_eq!(
            remove_leaf(&row(leaf("a"), leaf("b")), "a"),
            Some(leaf("b"))
        );
    }

    #[test]
    fn returns_none_once_the_last_pane_is_gone() {
        assert_eq!(remove_leaf(&leaf("a"), "a"), None);
    }

    #[test]
    fn returns_the_same_tree_when_the_pane_is_not_in_it() {
        let tree = row(leaf("a"), leaf("b"));
        assert_eq!(remove_leaf(&tree, "zz"), Some(tree));
    }

    #[test]
    fn keeps_the_arrangement_when_a_pane_is_renamed() {
        let tree = row(
            leaf("a"),
            split_of(SplitDir::Col, leaf("b"), leaf("c"), 0.3),
        );
        assert_eq!(
            rename_leaf(&tree, "b", "b2"),
            row(
                leaf("a"),
                split_of(SplitDir::Col, leaf("b2"), leaf("c"), 0.3)
            )
        );
    }

    // ratios

    fn ratio_tree() -> SplitNode {
        row(leaf("a"), col(leaf("b"), leaf("c")))
    }

    #[test]
    fn reads_and_writes_the_ratio_at_a_path() {
        let tree = ratio_tree();
        let next = with_ratio(&tree, &[Side::B], 0.25);
        assert_eq!(ratio_at(&next, &[Side::B]), 0.25);
        assert_eq!(ratio_at(&next, &[]), 0.5);
        assert_eq!(ratio_at(&tree, &[Side::B]), 0.5);
    }

    #[test]
    fn follows_the_ratio_in_the_rectangles() {
        let layout = compute_layout(&with_ratio(&ratio_tree(), &[], 0.25));
        assert!(close(layout.rects["a"].width, 0.25));
        assert!(close(layout.rects["b"].left, 0.25));
        assert!(close(layout.rects["b"].width, 0.75));
    }

    #[test]
    fn clamps_a_ratio_that_would_crush_a_pane() {
        let layout = compute_layout(&split_of(SplitDir::Row, leaf("a"), leaf("b"), 0.0));
        assert!(close(layout.rects["a"].width, 0.05));
        let layout = compute_layout(&split_of(SplitDir::Row, leaf("a"), leaf("b"), 1.0));
        assert!(close(layout.rects["b"].width, 0.05));
    }

    #[test]
    fn has_one_handle_per_split_keyed_by_its_path() {
        let layout = compute_layout(&ratio_tree());
        let mut keys: Vec<&str> = layout.handles.iter().map(|h| h.key.as_str()).collect();
        keys.sort();
        assert_eq!(keys, ["b", "root"]);
        let root = layout.handles.iter().find(|h| h.key == "root").unwrap();
        assert_eq!(root.dir, SplitDir::Row);
        assert_eq!(
            root.rect,
            Rect {
                left: 0.5,
                top: 0.0,
                width: 0.0,
                height: 1.0
            }
        );
    }

    #[test]
    fn names_the_panes_on_either_side_of_each_handle_for_its_label() {
        let layout = compute_layout(&ratio_tree());
        let by_key: HashMap<&str, (Vec<PaneId>, Vec<PaneId>)> = layout
            .handles
            .iter()
            .map(|h| (h.key.as_str(), (h.before.clone(), h.after.clone())))
            .collect();
        assert_eq!(by_key["root"], (ids(&["a"]), ids(&["b", "c"])));
        assert_eq!(by_key["b"], (ids(&["b"]), ids(&["c"])));
        assert_eq!(by_key.len(), 2);
    }

    // moving panes

    fn moving_tree() -> SplitNode {
        row(
            leaf("a"),
            split_of(SplitDir::Col, leaf("b"), leaf("c"), 0.3),
        )
    }

    #[test]
    fn swaps_two_panes_on_a_drop_in_the_middle_keeping_the_shape() {
        let tree = moving_tree();
        assert_eq!(
            move_leaf(&tree, "a", "c", DropZone::Center),
            row(
                leaf("c"),
                split_of(SplitDir::Col, leaf("b"), leaf("a"), 0.3)
            )
        );
        assert_eq!(
            swap_leaves(&tree, "a", "c"),
            move_leaf(&tree, "a", "c", DropZone::Center)
        );
    }

    #[test]
    fn re_splits_the_target_on_a_drop_on_an_edge() {
        let next = move_leaf(&moving_tree(), "a", "c", DropZone::Bottom);
        assert_eq!(leaf_ids(&next), ids(&["b", "c", "a"]));
        let layout = compute_layout(&next);
        assert!(layout.rects["a"].top > layout.rects["c"].top);
    }

    #[test]
    fn does_nothing_when_a_pane_is_dropped_on_itself() {
        let tree = moving_tree();
        assert_eq!(move_leaf(&tree, "b", "b", DropZone::Left), tree);
    }

    // normalizeTree

    #[test]
    fn returns_a_tree_that_still_matches_untouched_ratios_included() {
        let tree = split_of(SplitDir::Row, leaf("a"), leaf("b"), 0.8);
        assert_eq!(normalize_tree(&panes(&["a", "b"]), Some(&tree)), tree);
    }

    #[test]
    fn builds_the_preset_when_there_is_no_tree() {
        assert_eq!(
            normalize_tree(&panes(&["a", "b"]), None),
            preset_tree(&ids(&["a", "b"]))
        );
    }

    #[test]
    fn maps_a_legacy_tree_of_pane_indexes_onto_ids() {
        let legacy = json!({
            "type": "split", "dir": "col", "ratio": 0.6,
            "a": {"type": "pane", "index": 1},
            "b": {"type": "pane", "index": 0}
        });
        let list = panes(&["a", "b"]);
        let ids: Vec<PaneId> = list.iter().map(|p| p.id.clone()).collect();
        let migrated = migrate(&legacy, &ids);
        assert_eq!(
            normalize_tree(&list, migrated.as_ref()),
            split_of(SplitDir::Col, leaf("b"), leaf("a"), 0.6)
        );
    }

    #[test]
    fn drops_panes_that_are_gone_and_appends_new_ones_in_the_user_s_order() {
        let tree = row(leaf("c"), row(leaf("gone"), leaf("a")));
        assert_eq!(
            leaf_ids(&normalize_tree(&panes(&["a", "b", "c"]), Some(&tree))),
            ids(&["c", "a", "b"])
        );
    }

    #[test]
    fn never_lists_a_pane_twice() {
        let tree = row(leaf("a"), leaf("a"));
        assert_eq!(
            leaf_ids(&normalize_tree(&panes(&["a", "b"]), Some(&tree))),
            ids(&["a", "b"])
        );
    }

    #[test]
    fn is_an_empty_leaf_when_there_are_no_panes() {
        assert_eq!(normalize_tree(&[], None), leaf(""));
    }

    #[test]
    fn evens_out_a_ratio_the_ts_would_not_have_kept() {
        let tree = split_of(SplitDir::Row, leaf("a"), leaf("b"), 1.5);
        assert_eq!(
            normalize_tree(&panes(&["a", "b"]), Some(&tree)),
            row(leaf("a"), leaf("b"))
        );
    }

    // neighborOf

    fn grid() -> SplitNode {
        preset_tree(&ids(&["a", "b", "c", "d"]))
    }

    #[test]
    fn finds_the_pane_on_each_side() {
        let grid = grid();
        assert_eq!(
            neighbor_of(&grid, "a", Direction::Right).as_deref(),
            Some("b")
        );
        assert_eq!(
            neighbor_of(&grid, "a", Direction::Down).as_deref(),
            Some("c")
        );
        assert_eq!(
            neighbor_of(&grid, "d", Direction::Left).as_deref(),
            Some("c")
        );
        assert_eq!(neighbor_of(&grid, "d", Direction::Up).as_deref(), Some("b"));
    }

    #[test]
    fn is_none_at_the_edge_of_the_grid() {
        assert_eq!(neighbor_of(&grid(), "a", Direction::Left), None);
        assert_eq!(neighbor_of(&grid(), "a", Direction::Up), None);
    }

    #[test]
    fn reads_the_screen_not_the_tree() {
        let tree = row(
            leaf("a"),
            split_of(SplitDir::Col, leaf("b"), leaf("c"), 0.3),
        );
        assert_eq!(
            neighbor_of(&tree, "a", Direction::Right).as_deref(),
            Some("c")
        );
        assert_eq!(
            neighbor_of(&tree, "c", Direction::Left).as_deref(),
            Some("a")
        );
        assert_eq!(neighbor_of(&tree, "c", Direction::Up).as_deref(), Some("b"));
    }

    #[test]
    fn is_none_for_a_pane_that_is_not_in_the_tree() {
        assert_eq!(neighbor_of(&grid(), "zz", Direction::Left), None);
    }

    // drop zones

    #[test]
    fn swaps_in_the_middle_and_splits_towards_the_nearest_edge() {
        let bx = Rect {
            left: 0.0,
            top: 0.0,
            width: 100.0,
            height: 100.0,
        };
        assert_eq!(zone_at(bx, 50.0, 50.0), DropZone::Center);
        assert_eq!(zone_at(bx, 5.0, 50.0), DropZone::Left);
        assert_eq!(zone_at(bx, 95.0, 50.0), DropZone::Right);
        assert_eq!(zone_at(bx, 50.0, 5.0), DropZone::Top);
        assert_eq!(zone_at(bx, 50.0, 95.0), DropZone::Bottom);
    }

    #[test]
    fn previews_the_half_a_dropped_pane_would_take() {
        let rect = Rect::FULL;
        assert_eq!(
            zone_rect(rect, DropZone::Right),
            Rect {
                left: 0.5,
                top: 0.0,
                width: 0.5,
                height: 1.0
            }
        );
        assert_eq!(
            zone_rect(rect, DropZone::Bottom),
            Rect {
                left: 0.0,
                top: 0.5,
                width: 1.0,
                height: 0.5
            }
        );
        assert_eq!(zone_rect(rect, DropZone::Center), rect);
    }

    // presets

    fn four() -> Vec<Pane> {
        panes(&["a", "b", "c", "d"])
    }
    fn moved() -> SplitNode {
        row(col(leaf("d"), leaf("a")), col(leaf("b"), leaf("c")))
    }

    #[test]
    fn reads_the_panes_in_their_order_on_screen_not_their_order_of_creation() {
        let four = four();
        assert_eq!(
            of(&reading_order(&four, Some(&moved()))),
            ["d", "a", "b", "c"]
        );
    }

    #[test]
    fn closes_the_panes_read_last_which_are_the_ones_it_names_before_doing_it() {
        let four = four();
        assert_eq!(of(&closed_by_preset(&four, Some(&moved()), 2)), ["b", "c"]);
        assert_eq!(
            of(&closed_by_preset(&four, Some(&moved()), 1)),
            ["a", "b", "c"]
        );
    }

    #[test]
    fn closes_nothing_when_it_keeps_or_grows_the_grid() {
        let four = four();
        assert!(closed_by_preset(&four, Some(&moved()), 4).is_empty());
        assert!(closed_by_preset(&four, Some(&moved()), 8).is_empty());
    }

    #[test]
    fn keeps_at_least_one_pane_and_at_most_the_grid_s_maximum() {
        assert_eq!(preset_size(0), 1);
        assert_eq!(preset_size(99), 16);
        let four = four();
        assert_eq!(of(&closed_by_preset(&four, None, 0)), ["b", "c", "d"]);
    }

    #[test]
    fn splits_along_the_longer_side() {
        assert_eq!(preferred_dir(None), SplitDir::Row);
        assert_eq!(preferred_dir(Some((800.0, 400.0))), SplitDir::Row);
        assert_eq!(preferred_dir(Some((400.0, 400.0))), SplitDir::Col);
    }

    // ---- Hangar's own layout for agents' panes

    fn numbered(n: usize) -> Vec<PaneId> {
        (0..n).map(|i| format!("p{i}")).collect()
    }

    fn area(rect: &Rect) -> f64 {
        rect.width * rect.height
    }

    #[test]
    fn the_lead_is_the_biggest_pane_whatever_the_number_of_others() {
        for n in 2..=MAX_PANES {
            let order = numbered(n);
            let tree = arranged_tree(&order, Some("p0"));
            let mut placed = leaf_ids(&tree);
            placed.sort();
            let mut wanted = order.clone();
            wanted.sort();
            assert_eq!(placed, wanted, "{n} panes, each once");
            let grid = compute_layout(&tree);
            let lead = area(&grid.rects["p0"]);
            for id in order.iter().skip(1) {
                assert!(
                    lead > area(&grid.rects[id]),
                    "{n} panes: p0 is not bigger than {id}"
                );
            }
            // On the left, the whole height.
            let rect = grid.rects["p0"];
            assert_eq!((rect.left, rect.top, rect.height), (0.0, 0.0, 1.0));
        }
    }

    #[test]
    fn the_others_share_what_is_left_evenly() {
        for n in 2..=MAX_PANES {
            let order = numbered(n);
            let grid = compute_layout(&arranged_tree(&order, Some("p0")));
            let areas: Vec<f64> = order
                .iter()
                .skip(1)
                .map(|id| area(&grid.rects[id]))
                .collect();
            let (min, max) = areas
                .iter()
                .fold((f64::MAX, 0.0_f64), |(lo, hi), a| (lo.min(*a), hi.max(*a)));
            // Rows differ by one pane at most: two panes beside one at worst.
            assert!(max <= min * 2.0 + 1e-9, "{n} panes: {areas:?}");
        }
    }

    #[test]
    fn the_lead_moves_to_the_left_and_the_others_keep_their_order() {
        let order = numbered(4);
        let tree = arranged_tree(&order, Some("p2"));
        assert_eq!(leaf_ids(&tree), ["p2", "p0", "p1", "p3"]);
    }

    #[test]
    fn without_a_lead_it_is_the_even_grid() {
        let order = numbered(5);
        assert_eq!(arranged_tree(&order, None), preset_tree(&order));
        assert_eq!(arranged_tree(&order, Some("gone")), preset_tree(&order));
        assert_eq!(arranged_tree(&numbered(1), Some("p0")), leaf("p0"));
    }
}

#[cfg(test)]
mod view_tests {
    //! The views' leaves: kept by every function that lays out a grid, and
    //! dropped by those that read the terminals alone, as 1.x does.

    use super::*;
    use crate::state::types::ViewPane;

    fn workspace(terminals: &[&str], views: &[&str], tree: Option<SplitNode>) -> Workspace {
        Workspace {
            panes: terminals
                .iter()
                .map(|id| Pane {
                    id: (*id).into(),
                    ..Pane::default()
                })
                .collect(),
            view_panes: views.iter().map(|id| ViewPane::new(*id)).collect(),
            tree,
            ..Workspace::default()
        }
    }

    fn row(a: SplitNode, b: SplitNode) -> SplitNode {
        split(SplitDir::Row, 0.3, a, b)
    }

    #[test]
    fn the_grid_keeps_a_view_s_leaf_with_its_ratio() {
        let tree = row(leaf("a"), split(SplitDir::Col, 0.7, leaf("k"), leaf("b")));
        let ws = workspace(&["a", "b"], &["k"], Some(tree.clone()));
        assert_eq!(grid_tree(&ws), tree);
        assert_eq!(grid_order(&ws), ["a", "k", "b"]);
        // What reads the terminals alone drops it, as 1.x's normalizeTree.
        assert_eq!(
            leaf_ids(&normalize_tree(&ws.panes, ws.tree.as_ref())),
            ["a", "b"]
        );
        let terminals: Vec<&str> = reading_order(&ws.panes, ws.tree.as_ref())
            .iter()
            .map(|pane| pane.id.as_str())
            .collect();
        assert_eq!(terminals, ["a", "b"]);
    }

    #[test]
    fn a_view_the_tree_lost_comes_back_last_and_a_gone_one_goes() {
        // A tree 1.x rewrote without the view, and a leaf whose view closed.
        let ws = workspace(&["a", "b"], &["k"], Some(row(leaf("a"), leaf("b"))));
        assert_eq!(grid_order(&ws), ["a", "b", "k"]);
        let ws = workspace(&["a"], &[], Some(row(leaf("a"), leaf("k"))));
        assert_eq!(grid_tree(&ws), leaf("a"));
    }

    #[test]
    fn moving_between_panes_reaches_a_view() {
        let ws = workspace(
            &["a", "b"],
            &["k"],
            Some(row(leaf("a"), row(leaf("k"), leaf("b")))),
        );
        let tree = grid_tree(&ws);
        assert_eq!(
            neighbor_of(&tree, "a", Direction::Right).as_deref(),
            Some("k")
        );
        assert_eq!(
            neighbor_of(&tree, "k", Direction::Right).as_deref(),
            Some("b")
        );
        assert_eq!(
            neighbor_of(&tree, "b", Direction::Left).as_deref(),
            Some("k")
        );
    }

    #[test]
    fn the_agents_layout_and_the_presets_keep_the_views() {
        let ws = workspace(&["a", "b", "c"], &["k"], None);
        let order = grid_order(&ws);
        assert_eq!(order, ["a", "b", "c", "k"]);
        let arranged = arranged_tree(&order, Some("c"));
        assert_eq!(leaf_ids(&arranged), ["c", "a", "b", "k"]);
        assert_eq!(leaf_ids(&preset_tree(&order)), order);
    }

    #[test]
    fn a_preset_leaves_the_terminals_what_the_views_leave_them() {
        assert_eq!(preset_terminals(4, 0), 4);
        assert_eq!(preset_terminals(16, 3), 13);
        assert_eq!(preset_terminals(8, 16), 1, "at least one terminal");
        let views: Vec<String> = (0..12).map(|n| format!("v{n}")).collect();
        let views: Vec<&str> = views.iter().map(String::as_str).collect();
        let ws = workspace(&["a", "b", "c", "d", "e", "f"], &views, None);
        // Eight asked for, four fit beside twelve views: the two read last go.
        let closed: Vec<&str> = closed_by_preset_in(&ws, 8)
            .iter()
            .map(|pane| pane.id.as_str())
            .collect();
        assert_eq!(closed, ["e", "f"]);
    }
}

#[cfg(test)]
mod pin_tests {
    //! Pinned panes: carved out first, each on its edge at its size, the
    //! others laid out in the room they leave.

    use super::*;

    const GRID: (f32, f32) = (1600.0, 900.0);

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn ids(list: &[&str]) -> Vec<PaneId> {
        list.iter().map(|id| (*id).to_string()).collect()
    }

    fn row(a: SplitNode, b: SplitNode) -> SplitNode {
        split(SplitDir::Row, 0.5, a, b)
    }

    fn pin(id: &str, edge: PinEdge, fraction: f64) -> PanePin {
        PanePin {
            fraction: Some(fraction),
            ..PanePin::new(id, edge)
        }
    }

    fn pin_px(id: &str, edge: PinEdge, px: f64, fraction: f64) -> PanePin {
        PanePin {
            px: Some(px),
            ..pin(id, edge, fraction)
        }
    }

    fn rect_of(tree: &SplitNode, id: &str) -> Rect {
        compute_layout(tree).rects[id]
    }

    fn assert_rect(rect: Rect, left: f64, top: f64, width: f64, height: f64) {
        assert!(
            close(rect.left, left)
                && close(rect.top, top)
                && close(rect.width, width)
                && close(rect.height, height),
            "{rect:?} is not ({left}, {top}, {width}, {height})"
        );
    }

    fn workspace(terminals: &[&str], pins: Vec<PanePin>, tree: Option<SplitNode>) -> Workspace {
        Workspace {
            panes: terminals
                .iter()
                .map(|id| Pane {
                    id: (*id).into(),
                    ..Pane::default()
                })
                .collect(),
            pins,
            tree,
            ..Workspace::default()
        }
    }

    #[test]
    fn a_pin_carves_its_pane_out_on_its_edge_across_the_whole_room() {
        let inner = Some(row(leaf("a"), leaf("b")));
        for (edge, expected) in [
            (PinEdge::Left, (0.0, 0.0, 0.3, 1.0)),
            (PinEdge::Right, (0.7, 0.0, 0.3, 1.0)),
            (PinEdge::Top, (0.0, 0.0, 1.0, 0.3)),
            (PinEdge::Bottom, (0.0, 0.7, 1.0, 0.3)),
        ] {
            let tree = with_pins(&[pin("t", edge, 0.3)], inner.clone());
            let (left, top, width, height) = expected;
            assert_rect(rect_of(&tree, "t"), left, top, width, height);
        }
        // Alone, it has the grid; with nothing at all, the empty leaf.
        assert_eq!(with_pins(&[pin("t", PinEdge::Right, 0.3)], None), leaf("t"));
        assert_eq!(with_pins(&[], None), leaf(""));
    }

    #[test]
    fn the_first_pin_is_the_outermost_and_two_on_one_edge_stand_side_by_side() {
        // The left one takes the whole height, the top one what it leaves.
        let tree = with_pins(
            &[pin("l", PinEdge::Left, 0.2), pin("t", PinEdge::Top, 0.25)],
            Some(leaf("a")),
        );
        assert_rect(rect_of(&tree, "l"), 0.0, 0.0, 0.2, 1.0);
        assert_rect(rect_of(&tree, "t"), 0.2, 0.0, 0.8, 0.25);
        assert_rect(rect_of(&tree, "a"), 0.2, 0.25, 0.8, 0.75);
        // Two on the left: the first against the edge, the second beside it,
        // at its share of what the first leaves.
        let tree = with_pins(
            &[pin("x", PinEdge::Left, 0.2), pin("y", PinEdge::Left, 0.25)],
            Some(leaf("a")),
        );
        assert_rect(rect_of(&tree, "x"), 0.0, 0.0, 0.2, 1.0);
        assert_rect(rect_of(&tree, "y"), 0.2, 0.0, 0.2, 1.0);
        assert_rect(rect_of(&tree, "a"), 0.4, 0.0, 0.6, 1.0);
        assert_eq!(leaf_ids(&tree), ["x", "y", "a"]);
    }

    #[test]
    fn a_tree_is_carved_once_and_one_that_holds_its_pins_comes_back_as_it_is() {
        // An even grid with the pinned pane in it, as an older Hangar wrote it.
        let tree = preset_tree(&ids(&["a", "t", "b"]));
        let pins = [pin("t", PinEdge::Right, 0.4)];
        let carved = pinned_tree(&tree, &pins);
        assert_eq!(
            carved,
            with_pins(&pins, Some(split(SplitDir::Col, 0.5, leaf("a"), leaf("b"))))
        );
        assert_eq!(pinned_tree(&carved, &pins), carved, "the same tree");
        // A pin with no share takes the one its pane has on screen.
        let no_size = [PanePin::new("t", PinEdge::Right)];
        assert!(close(
            rect_of(&pinned_tree(&tree, &no_size), "t").width,
            0.5
        ));
        assert_eq!(pinned_tree(&tree, &[]), tree);
    }

    #[test]
    fn a_pinned_pane_keeps_its_rect_however_many_panes_open_beside_it() {
        let pins = [pin_px("t", PinEdge::Right, 480.0, 0.3)];
        for n in 0..=8 {
            let mut order = ids(&["lead"]);
            order.extend((0..n).map(|i| format!("w{i}")));
            for tree in [
                with_pins(&pins, Some(arranged_tree(&order, Some("lead")))),
                with_pins(&pins, Some(preset_tree(&order))),
            ] {
                // Laid out where the size is unknown: its share.
                let layout = compute_layout(&tree);
                assert_rect(layout.rects["t"], 0.7, 0.0, 0.3, 1.0);
                for id in &order {
                    let rect = layout.rects[id];
                    assert!(rect.left + rect.width <= 0.7 + 1e-9, "{n}: {id} {rect:?}");
                }
                // Drawn: its pixels.
                let drawn = rect_of(&sized_tree(&tree, &pins, GRID), "t");
                assert!(close(drawn.width * 1600.0, 480.0), "{n}: {drawn:?}");
                assert!(close(drawn.left + drawn.width, 1.0));
                assert_eq!((drawn.top, drawn.height), (0.0, 1.0));
            }
        }
    }

    #[test]
    fn a_window_that_shrinks_takes_from_a_pinned_pane_past_70_percent_only() {
        let pins = [pin_px("t", PinEdge::Right, 800.0, 0.5)];
        let tree = with_pins(&pins, Some(row(leaf("a"), leaf("b"))));
        let width_at = |width: f32| rect_of(&sized_tree(&tree, &pins, (width, 900.0)), "t").width;
        assert!(close(width_at(1600.0) * 1600.0, 800.0));
        // 70 % of 1000 is 700: what is left stays usable.
        assert!(close(width_at(1000.0), MAX_PIN_SHARE));
        assert!(close(width_at(400.0), MAX_PIN_SHARE));
        // The window grows back: the pane has its 800 pixels again.
        assert!(close(width_at(3200.0) * 3200.0, 800.0));
        // Never under `MIN_PANE_PX`, while the window has the room.
        let small = [pin_px("t", PinEdge::Right, 50.0, 0.1)];
        let tree = with_pins(&small, Some(leaf("a")));
        let drawn = rect_of(&sized_tree(&tree, &small, GRID), "t");
        assert!(close(drawn.width * 1600.0, f64::from(MIN_PANE_PX)));
        // A height for a pane pinned at the top; a share where no pixels.
        let top = [pin_px("t", PinEdge::Top, 300.0, 0.2)];
        let tree = with_pins(&top, Some(leaf("a")));
        let drawn = rect_of(&sized_tree(&tree, &top, GRID), "t");
        assert!(close(drawn.height * 900.0, 300.0));
        let share = [pin("t", PinEdge::Right, 0.3)];
        let tree = with_pins(&share, Some(leaf("a")));
        let drawn = rect_of(&sized_tree(&tree, &share, (1000.0, 900.0)), "t");
        assert!(close(drawn.width, 0.3));
    }

    #[test]
    fn two_pins_keep_their_pixels_each_out_of_the_room_the_first_leaves() {
        let pins = [
            pin_px("l", PinEdge::Left, 300.0, 0.2),
            pin_px("r", PinEdge::Right, 400.0, 0.3),
        ];
        let tree = with_pins(&pins, Some(preset_tree(&ids(&["a", "b", "c"]))));
        let drawn = compute_layout(&sized_tree(&tree, &pins, GRID));
        assert!(close(drawn.rects["l"].width * 1600.0, 300.0));
        assert!(close(drawn.rects["r"].width * 1600.0, 400.0));
        assert!(close(drawn.rects["r"].left + drawn.rects["r"].width, 1.0));
        // At 800: the left one fits (under 560), the right one gets 70 % of
        // the 500 left, and the others the rest.
        let drawn = compute_layout(&sized_tree(&tree, &pins, (800.0, 900.0)));
        assert!(close(drawn.rects["l"].width * 800.0, 300.0));
        assert!(close(drawn.rects["r"].width * 800.0, 350.0));
        // What is not a pin's split is left as it is.
        assert_eq!(sized_tree(&leaf("a"), &pins, GRID), leaf("a"));
    }

    #[test]
    fn a_pane_is_pinned_to_the_edge_it_covers_the_most_of() {
        let room = Rect::FULL;
        let at = |left, top, width, height| Rect {
            left,
            top,
            width,
            height,
        };
        // The column on the right: the right.
        assert_eq!(pin_edge(at(0.6, 0.0, 0.4, 1.0), room), PinEdge::Right);
        // A corner: the edge it covers more of.
        assert_eq!(pin_edge(at(0.6, 0.5, 0.4, 0.5), room), PinEdge::Right);
        assert_eq!(pin_edge(at(0.0, 0.5, 1.0, 0.5), room), PinEdge::Bottom);
        assert_eq!(pin_edge(at(0.0, 0.0, 0.3, 0.2), room), PinEdge::Top);
        // Alone in its room: the right.
        assert_eq!(pin_edge(room, room), PinEdge::Right);
        // Along none: the nearest.
        assert_eq!(pin_edge(at(0.1, 0.3, 0.3, 0.3), room), PinEdge::Left);
        assert_eq!(pin_edge(at(0.4, 0.6, 0.2, 0.35), room), PinEdge::Bottom);
        // Within the room the other pins leave.
        let left = at(0.3, 0.0, 0.7, 1.0);
        assert_eq!(pin_edge(at(0.3, 0.0, 0.3, 1.0), left), PinEdge::Left);
    }

    #[test]
    fn a_pane_pinned_keeps_its_size_and_says_it_in_pixels_when_the_grid_knows_them() {
        let tree = arranged_tree(&ids(&["lead", "t"]), Some("lead"));
        let ws = workspace(&["lead", "t"], Vec::new(), Some(tree));
        let pins = pinned_with(&ws, "t", None, Some(GRID)).unwrap();
        assert_eq!(pins.len(), 1);
        assert_eq!(pins[0].edge, PinEdge::Right);
        assert!(close(pins[0].fraction.unwrap(), 0.4));
        assert!(close(pins[0].px.unwrap(), 640.0));
        let no_size = pinned_with(&ws, "t", Some(PinEdge::Bottom), None).unwrap();
        assert_eq!(no_size[0].edge, PinEdge::Bottom);
        assert_eq!(no_size[0].px, None);
        assert!(
            close(no_size[0].fraction.unwrap(), MAX_PIN_SHARE),
            "its whole height, bounded"
        );
        assert_eq!(pinned_with(&ws, "gone", None, None), None);
    }

    #[test]
    fn a_pane_added_never_takes_a_pinned_pane_s_room() {
        let pins = [pin("t", PinEdge::Right, 0.3)];
        let tree = with_pins(&pins, Some(row(leaf("a"), leaf("b"))));
        for near in [Some("t"), None, Some("b"), Some("gone")] {
            let next = add_beside(&tree, &pins, near, "n", DropZone::Right);
            let layout = compute_layout(&next);
            assert_rect(layout.rects["t"], 0.7, 0.0, 0.3, 1.0);
            // Beside the pane next to the pinned one, or the last one read.
            let n = layout.rects["n"];
            assert!(close(n.left + n.width, 0.7), "{near:?}: {n:?}");
        }
        // Beside a pane named that is not pinned, as ever.
        let next = add_beside(&tree, &pins, Some("a"), "n", DropZone::Bottom);
        assert_eq!(leaf_ids(&next), ["a", "n", "b", "t"]);
        // Every pane pinned: the new one takes the room they leave.
        let alone = add_beside(&leaf("t"), &pins, Some("t"), "n", DropZone::Right);
        assert_eq!(alone, with_pins(&pins, Some(leaf("n"))));
    }

    #[test]
    fn dragging_a_pinned_pane_s_divider_sets_the_size_it_keeps() {
        let pins = [pin_px("t", PinEdge::Right, 300.0, 0.3)];
        let before = with_pins(&pins, Some(row(leaf("a"), leaf("b"))));
        // Dragged to 40 %: its pixels in proportion, the grid's size unknown.
        let after = with_ratio(&before, &[], 0.6);
        let dragged = pins_dragged(&before, &after, &pins);
        assert!(close(dragged[0].fraction.unwrap(), 0.4));
        assert!(close(dragged[0].px.unwrap(), 400.0));
        // A divider between the others moves no pin.
        let inner = with_ratio(&before, &[Side::A], 0.3);
        assert_eq!(pins_dragged(&before, &inner, &pins), pins);
        // With the grid's size, the pixels themselves.
        let resized = pin_resized(&before, &pins, &[], 0.6, Some(GRID)).unwrap();
        assert!(close(resized[0].px.unwrap(), 640.0));
        assert!(close(resized[0].fraction.unwrap(), 0.4));
        assert_eq!(
            pin_resized(&before, &pins, &[Side::A], 0.3, Some(GRID)),
            None
        );
        // Past 70 %, it stops there.
        let wide = pin_resized(&before, &pins, &[], 0.1, None).unwrap();
        assert!(close(wide[0].fraction.unwrap(), MAX_PIN_SHARE));
        // The second pin's split, out of the room the first leaves.
        let two = [
            pin_px("l", PinEdge::Left, 300.0, 0.2),
            pin_px("r", PinEdge::Right, 400.0, 0.3),
        ];
        let tree = with_pins(&two, Some(leaf("a")));
        let resized = pin_resized(&tree, &two, &[Side::B], 0.6, Some(GRID)).unwrap();
        assert_eq!(resized[0], two[0]);
        assert!(close(resized[1].px.unwrap(), 0.4 * 1300.0));
    }

    #[test]
    fn a_pin_that_goes_leaves_those_inside_it_their_size() {
        let pins = [pin("l", PinEdge::Left, 0.2), pin("r", PinEdge::Right, 0.25)];
        let tree = with_pins(&pins, Some(leaf("a")));
        assert!(close(rect_of(&tree, "r").width, 0.2));
        let rest = pins_without(&tree, &pins, "l");
        assert_eq!(rest.len(), 1);
        assert!(close(rest[0].fraction.unwrap(), 0.2), "{rest:?}");
        // The inner one going leaves the outer one as it was.
        assert_eq!(pins_without(&tree, &pins, "r"), [pins[0].clone()]);
        assert_eq!(pins_without(&tree, &pins, "a"), pins);
    }

    #[test]
    fn only_the_pins_of_panes_of_the_grid_hold_each_pane_once() {
        let ws = workspace(
            &["a", "b"],
            vec![
                pin("gone", PinEdge::Left, 0.2),
                pin("b", PinEdge::Right, 0.3),
                pin("b", PinEdge::Top, 0.3),
            ],
            None,
        );
        assert_eq!(live_pins(&ws), [pin("b", PinEdge::Right, 0.3)]);
        assert_eq!(ws.pin_of("b").map(|pin| pin.edge), Some(PinEdge::Right));
        assert_eq!(ws.pin_of("gone"), None);
        // The grid holds them, whatever tree the file had.
        assert_eq!(grid_tree(&ws), with_pins(&live_pins(&ws), Some(leaf("a"))));
        assert_eq!(grid_order(&ws), ["a", "b"]);
    }

    #[test]
    fn a_preset_counts_the_unpinned_terminals_and_never_closes_a_pinned_one() {
        let pins = vec![pin("b", PinEdge::Right, 0.3)];
        let tree = with_pins(&pins, Some(preset_tree(&ids(&["a", "c", "d"]))));
        let ws = workspace(&["a", "b", "c", "d"], pins, Some(tree));
        assert_eq!(preset_terminals_in(&ws, 2), 2);
        let closed: Vec<&str> = closed_by_preset_in(&ws, 2)
            .iter()
            .map(|pane| pane.id.as_str())
            .collect();
        assert_eq!(closed, ["d"]);
        let closed: Vec<&str> = closed_by_preset_in(&ws, 1)
            .iter()
            .map(|pane| pane.id.as_str())
            .collect();
        assert_eq!(closed, ["c", "d"]);
        // The pinned terminal takes its room among the 16.
        assert_eq!(preset_terminals_in(&ws, 16), 15);
        let unpinned = workspace(&["a"], Vec::new(), None);
        assert_eq!(
            preset_terminals_in(&unpinned, 0),
            preset_terminals(0, 0),
            "without a pin, as before"
        );
    }

    #[test]
    fn the_grid_is_drawn_at_its_pixels_once_its_size_is_known() {
        let pins = vec![pin_px("b", PinEdge::Right, 500.0, 0.3)];
        let ws = workspace(&["a", "b"], pins.clone(), None);
        assert_eq!(drawn_tree(&ws, None), grid_tree(&ws));
        let drawn = drawn_tree(&ws, Some(GRID));
        assert!(close(rect_of(&drawn, "b").width * 1600.0, 500.0));
        let plain = workspace(&["a", "b"], Vec::new(), None);
        assert_eq!(drawn_tree(&plain, Some(GRID)), grid_tree(&plain));
    }
}
