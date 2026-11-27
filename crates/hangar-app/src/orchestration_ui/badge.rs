//! What a pane is in its workspace's orchestration, in its header, right after
//! its name (`OrchestrationBadge.tsx`): « Orchestrateur », or « Worker », or
//! « Chef de projet » on the HQ's manager, in the brand pack's role pill.
//! Nothing at all on every other pane. The title of a worker's task stands
//! apart on the header's line ([`worker_task`]).
//!
//! A worker's task is read from the workspace's board, the one its worktree
//! shares, so a title edited on the board shows here too. The title is read
//! from the boards cache (`board::Live`) at every draw: the cache holds the
//! board of every workspace's folder and reads it again when it changes, so
//! a title edited from the worktree, or by an agent, follows at once.

use iced::Element;
use iced::widget::container;

use hangar_i18n::{Key, args};
use hangar_theme::style;
use hangar_ui::badge::{self, Role};
use hangar_ui::{Icon, icon, tip};

use crate::ctx::Ctx;
use crate::message::Message;
use crate::state::Model;
use crate::state::types::{Pane, PaneRole, Workspace};
use crate::state::{PaneId, WsId};

/// The header widths at which the badge gives way, in pixels of its content:
/// the orchestrator keeps its mark alone, then every badge goes, before the
/// pane's name has to give way to it.
const NO_ROLE_WORD: f32 = 400.0;
const NONE: f32 = 186.0;

fn pane_of<'a>(ctx: Ctx<'a>, ws: &WsId, pane: &PaneId) -> Option<(&'a Workspace, &'a Pane)> {
    let workspace = ctx.model.workspace(ws)?;
    Some((workspace, workspace.panes.iter().find(|p| &p.id == pane)?))
}

/// The badge of a pane in a header `bar_width` wide.
pub fn pane_badge_in<'a>(
    ctx: Ctx<'a>,
    ws: &'a WsId,
    pane: &'a PaneId,
    bar_width: f32,
) -> Element<'a, Message> {
    let Some((workspace, pane)) = pane_of(ctx, ws, pane) else {
        return nothing();
    };
    if bar_width < NONE {
        return nothing();
    }
    match pane.role {
        Some(PaneRole::Orchestrator) => lead(
            ctx,
            Key::OrchBadgeOrchestrator,
            Key::OrchBadgeOrchestratorHint,
            bar_width,
        ),
        Some(PaneRole::Manager) => lead(
            ctx,
            Key::OrchBadgeManager,
            Key::OrchBadgeManagerHint,
            bar_width,
        ),
        Some(PaneRole::Worker) => tip::bottom(
            ctx.theme,
            badge::role(ctx.theme, Role::Worker, ctx.t(Key::OrchBadgeWorker)),
            worker_hint(ctx, workspace, pane),
        ),
        None => nothing(),
    }
}

/// The task a worker is on, as its board names it, and the sentence that
/// says so: `None` for any other pane, and for a worker whose task the board
/// does not hold.
pub fn worker_task<'a>(ctx: Ctx<'a>, ws: &WsId, pane: &PaneId) -> Option<(&'a str, String)> {
    let (workspace, pane) = pane_of(ctx, ws, pane)?;
    if pane.role != Some(PaneRole::Worker) {
        return None;
    }
    let title = task_title(ctx.model, workspace, pane.task_id.as_deref()?)?;
    Some((title, worker_hint(ctx, workspace, pane)))
}

fn worker_hint(ctx: Ctx<'_>, workspace: &Workspace, pane: &Pane) -> String {
    let title = pane
        .task_id
        .as_deref()
        .and_then(|task| task_title(ctx.model, workspace, task));
    match title {
        Some(title) => ctx
            .i18n
            .t_with(Key::OrchBadgeWorkerHint, &args!("title" => title)),
        None => ctx.t(Key::OrchBadgeWorkerNoTask),
    }
    .into_owned()
}

/// Nothing, and no room for it.
fn nothing<'a>() -> Element<'a, Message> {
    iced::widget::space().width(0).height(0).into()
}

/// The orchestrator's and the manager's: the filled pill, or in a narrow
/// header its mark alone.
fn lead<'a>(ctx: Ctx<'a>, role: Key, hint: Key, bar_width: f32) -> Element<'a, Message> {
    let t = ctx.theme;
    let pill: Element<'a, Message> = if bar_width < NO_ROLE_WORD {
        container(icon(Icon::Orchestrator, 11.0).color(t.palette.status.active))
            .padding([4.0, 5.0])
            .style(style::container::role_pill(t, true))
            .into()
    } else {
        badge::role(t, Role::Orchestrator, ctx.t(role))
    };
    tip::bottom(t, pill, ctx.t(hint))
}

/// The title of a task of the workspace's board, as the boards cache holds it
/// now (`board::Live`).
fn task_title<'a>(m: &'a Model, workspace: &Workspace, task: &str) -> Option<&'a str> {
    m.live
        .board
        .tasks(&workspace.cwd)?
        .iter()
        .find(|candidate| candidate.id == task)
        .map(|candidate| candidate.title.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::model::test_task;

    fn workspace(cwd: &str) -> Workspace {
        Workspace {
            id: "wt".into(),
            cwd: cwd.into(),
            ..Workspace::default()
        }
    }

    fn titled(id: &str, title: &str) -> crate::board::model::Task {
        let mut task = test_task(id);
        task.title = title.into();
        task
    }

    #[test]
    fn a_workers_badge_follows_a_title_edited_from_its_worktree() {
        // The worker's workspace is a worktree of the project, on a folder of
        // its own; the board it shares is read by the boards cache.
        let ws = workspace("/work/project-wt");
        let mut m = Model::default();
        m.live
            .board
            .hold("/work/project-wt", vec![titled("t1", "Draft the page")]);
        assert_eq!(task_title(&m, &ws, "t1"), Some("Draft the page"));

        // The title is edited (the dialog, a worktree, an agent): the cache
        // reads the board again, and the next draw says so, with no state of
        // this package to refresh.
        m.live.board.hold(
            "/work/project-wt",
            vec![titled("t1", "Draft the page, twice")],
        );
        assert_eq!(task_title(&m, &ws, "t1"), Some("Draft the page, twice"));
    }

    #[test]
    fn a_task_the_board_does_not_have_has_no_title() {
        let ws = workspace("/work/project-wt");
        let mut m = Model::default();
        // Not read yet.
        assert_eq!(task_title(&m, &ws, "t1"), None);
        m.live
            .board
            .hold("/work/project-wt", vec![titled("t2", "Another")]);
        assert_eq!(task_title(&m, &ws, "t1"), None);
        // Another folder's board is not this workspace's.
        assert_eq!(task_title(&m, &workspace("/elsewhere"), "t2"), None);
    }
}
