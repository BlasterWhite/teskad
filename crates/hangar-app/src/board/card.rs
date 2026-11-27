//! A task's card, its priority and its assignee (`BoardView.tsx`'s
//! `renderCard`, `BoardPriority.tsx`, `BoardAssignee.tsx`).

use hangar_i18n::{Key, args};
use hangar_theme::tokens::scale::{radius, type_scale};
use hangar_theme::tokens::{BORDER_WIDTH, space};
use hangar_theme::{Theme, fonts};
use hangar_ui::button as ui_button;
use hangar_ui::clamp::clamp;
use hangar_ui::glyph::{self, GlyphState as Shown};
use hangar_ui::icon::{Icon, icon};
use hangar_ui::{focus, text_width, tip};
use iced::widget::{Space, button, column, container, mouse_area, row, rule, text};
use iced::{Alignment, Element, Length, Padding, mouse};

use crate::message::Message;
use crate::state::GlyphState;

use super::format::{
    DateStyle, elapsed_label, priority_key, priority_level, short_task_id, time_today, when,
};
use super::hit::{Look, hit};
use super::look::{self, CardLook};
use super::model::{Blocker, Column, Task, blockers_of};
use super::view::View;
use super::{Msg, card_id};

/// A card's title: its size and its line, three of them at most.
const TITLE_LINE: f32 = 18.0;
/// The line of the 12 px sentences (what blocks it).
const SMALL_LINE: f32 = 16.0;
/// A card's padding, over and under its parts.
const CARD_PADDING_Y: f32 = 10.0;
/// The side of the state glyph beside an agent's name.
const GLYPH: f32 = 10.0;
/// The gap between the parts of a card's foot.
const FOOT_GAP: f32 = 10.0;
/// A count of the foot: its icon and the gap after it.
const STAT_ICON: f32 = 12.0;
const STAT_GAP: f32 = 3.0;

/// A task's card, `width` wide.
pub(super) fn card<'a>(v: &View<'a>, task: &'a Task, width: f32) -> Element<'a, Message> {
    let t = v.t();
    let tr = v.ctx.i18n;
    let done = task.column == "done";
    let dragging =
        v.st.press
            .as_ref()
            .is_some_and(|press| press.moved && press.task == task.id);
    let hovered =
        v.st.hovered
            .as_ref()
            .is_some_and(|(cwd, id)| cwd == v.cwd && *id == task.id);
    let selected = v.selected == Some(task.id.as_str());
    let blockers = if done {
        Vec::new()
    } else {
        blockers_of(task, &v.by_id)
    };
    let blocker_names: Vec<String> = blockers
        .iter()
        .map(|blocker| match blocker {
            Blocker::Missing(_) => tr.t(Key::BoardMissingTask).into_owned(),
            Blocker::Task(dep) => format!("#{} · {}", short_task_id(&dep.id), dep.title),
        })
        .collect();

    let id = mono_muted(t, format!("#{}", short_task_id(&task.id)));
    // Under the pointer, the priority gives its place to the button that
    // blocks the task, or lifts its block. The task's sheet has both for the
    // keyboard.
    let corner: Element<'a, Message> = match hovered.then(|| block_action(v, task)).flatten() {
        Some(action) => action,
        None => priority(v, task.priority),
    };
    let top = row![
        tip::over(t, id, task.id.clone()),
        Space::new().width(Length::Fill),
        // As tall either way, so that nothing moves under the pointer.
        container(corner).center_y(CORNER),
    ]
    .spacing(space::MD)
    .align_y(Alignment::Center);

    let title = clamp(task.title.clone(), 3)
        .size(type_scale::SMALL.size)
        .line_height(TITLE_LINE)
        .font(fonts::ui_medium())
        .color(t.palette.text);

    let mut body = column![top, Element::from(title)].spacing(space::MD);

    if !task.labels.is_empty() {
        body = body.push(
            row(task.labels.iter().map(|label| chip(t, label)))
                .spacing(space::XS)
                .wrap(),
        );
    }

    if !blockers.is_empty() {
        let words = tr
            .t_with(
                Key::BoardBlockedBy,
                &args!("tasks" => blocker_names.join(", ").as_str()),
            )
            .into_owned();
        let line = row![
            icon(Icon::Lock, STAT_ICON).color(t.palette.status.failed),
            Element::from(
                clamp(words, 2)
                    .size(type_scale::CAPTION.size)
                    .line_height(SMALL_LINE)
                    .color(t.palette.status.failed)
            ),
        ]
        .spacing(space::SM);
        body = body.push(tip::over(t, line, blocker_names.join("\n")));
    }

    if task.blocked.is_some() {
        body = body.push(blocked_inset(v, task));
    }

    let inner = width - 2.0 * (look::CARD_PADDING + BORDER_WIDTH);
    body = body.push(
        column![
            rule::horizontal(1).style(rule_of(t)),
            footer(v, task, done, blockers.is_empty(), inner)
        ]
        .spacing(space::MD),
    );

    let framed = container(body.padding(Padding {
        top: CARD_PADDING_Y,
        right: look::CARD_PADDING + BORDER_WIDTH,
        bottom: CARD_PADDING_Y,
        left: look::CARD_PADDING + BORDER_WIDTH,
    }))
    .width(Length::Fill)
    .style(look::card(
        t,
        CardLook {
            hovered,
            selected,
            dragging,
        },
    ));

    // The card says which board it is on: the window that draws it is not
    // always the main window's active workspace.
    let (pressed, opened) = (task.id.clone(), task.id.clone());
    let (pressed_on, opened_on) = (v.cwd.to_owned(), v.cwd.to_owned());
    let pressable = hit(framed)
        .id(card_id(&task.id))
        .on_press(move |at| Message::Board(Msg::Press(pressed_on.clone(), pressed.clone(), at)))
        .on_activate(Message::Board(Msg::OpenTask(opened_on, opened)))
        .interaction(mouse::Interaction::Pointer)
        .look(Look {
            hover: None,
            ring: Some(t.palette.focus_ring),
            ring_offset: hangar_theme::tokens::scale::FOCUS_OFFSET,
            radius: radius::MD,
            inset_x: 0.0,
        });
    mouse_area(pressable)
        .on_enter(Message::Board(Msg::Hover(
            v.cwd.to_owned(),
            task.id.clone(),
        )))
        .on_exit(Message::Board(Msg::Unhover(
            v.cwd.to_owned(),
            task.id.clone(),
        )))
        .into()
}

/// The height of a card's first line.
const CORNER: f32 = 22.0;

/// "Block" or "Unblock", for a task that can be: one on hold can be let go,
/// and one that waits or is under way can be put on hold (the board refuses
/// it of work in review or done).
fn block_action<'a>(v: &View<'a>, task: &'a Task) -> Option<Element<'a, Message>> {
    let t = v.t();
    let tr = v.ctx.i18n;
    let (words, glyph, message) = if task.blocked.is_some() {
        (
            Key::BoardUnblock,
            Icon::Unlock,
            Msg::UnblockTask(v.cwd.to_owned(), task.id.clone()),
        )
    } else if matches!(Column::of(task), Some(Column::Todo | Column::Doing)) {
        (
            Key::BoardBlock,
            Icon::Lock,
            Msg::BlockTask(v.cwd.to_owned(), task.id.clone()),
        )
    } else {
        return None;
    };
    Some(
        ui_button::secondary(t, tr.t(words))
            .icon(glyph)
            .small()
            .on_press(Message::Board(message))
            .into(),
    )
}

/// A label: mono, 20 px, corners of 4.
pub(super) fn chip<'a>(t: &'a Theme, label: &str) -> Element<'a, Message> {
    container(
        text(label.to_owned())
            .size(type_scale::LABEL.size)
            .font(fonts::mono())
            .wrapping(text::Wrapping::None),
    )
    .height(20.0)
    .padding([0.0, space::SM])
    .align_y(Alignment::Center)
    .style(look::chip(t))
    .into()
}

fn rule_of(t: &Theme) -> impl Fn(&iced::Theme) -> rule::Style + '_ {
    move |theme| rule::Style {
        color: t.palette.border,
        ..hangar_theme::style::rule::hairline(t)(theme)
    }
}

/// Why a task is on hold, and the way to let it go: the reason, the day it is
/// looked at again, that only a person can lift it when so, and "Unblock".
fn blocked_inset<'a>(v: &View<'a>, task: &'a Task) -> Element<'a, Message> {
    let t = v.t();
    let tr = v.ctx.i18n;
    let Some(blocked) = &task.blocked else {
        return Space::new().into();
    };
    let reason = tr
        .t_with(
            Key::BoardBlockedFor,
            &args!("reason" => blocked.reason.as_str()),
        )
        .into_owned();
    let mut parts = column![tip::over(
        t,
        row![
            glyph::state(t, Shown::Failed, STAT_ICON),
            Element::from(
                clamp(reason, 2)
                    .size(type_scale::CAPTION.size)
                    .line_height(SMALL_LINE)
                    .color(t.palette.status.failed)
            ),
        ]
        .spacing(space::SM),
        blocked.reason.clone(),
    )]
    .spacing(space::MD);
    let note = |glyph: Icon, words: String| {
        row![
            icon(glyph, STAT_ICON).color(t.palette.text_muted),
            text(words)
                .size(type_scale::CAPTION.size)
                .color(t.palette.text_muted)
                .width(Length::Fill),
        ]
        .spacing(space::SM)
        .align_y(Alignment::Center)
    };
    if let Some(day) = &blocked.recheck {
        let words = tr
            .t_with(Key::BoardBlockedRecheck, &args!("date" => day.as_str()))
            .into_owned();
        parts = parts.push(note(Icon::Clock, words));
    }
    if blocked.human_only {
        parts = parts.push(note(
            Icon::Lock,
            tr.t(Key::BoardBlockHumanOnly).into_owned(),
        ));
    }
    parts = parts.push(Element::from(
        ui_button::secondary(t, tr.t(Key::BoardUnblock))
            .icon(Icon::Unlock)
            .on_press(Message::Board(Msg::UnblockTask(
                v.cwd.to_owned(),
                task.id.clone(),
            ))),
    ));
    container(parts)
        .padding([space::MD, space::LG])
        .width(Length::Fill)
        .style(look::blocked_inset(t))
        .into()
}

/// What the foot of a card `room` wide has room for: the agent's program
/// goes first, then the words of the time elapsed, which stay in its
/// tooltip. `held` is the width of the agent's name and its glyph, `program`
/// that of "· Claude Code", `counts` the other counts', and `clock` that of
/// the time with its words.
fn foot_fits(room: f32, held: f32, program: f32, counts: f32, clock: Option<f32>) -> (bool, bool) {
    let time = clock.map_or(0.0, |width| width + FOOT_GAP);
    if held + program + counts + time <= room {
        (true, true)
    } else if held + counts + time <= room {
        (false, true)
    } else {
        (false, false)
    }
}

/// The foot of a card: who holds it, and its clock and counts. Never on two
/// lines.
fn footer<'a>(
    v: &View<'a>,
    task: &'a Task,
    done: bool,
    dependencies_met: bool,
    room: f32,
) -> Element<'a, Message> {
    let t = v.t();
    let tr = v.ctx.i18n;
    let small = |words: &str| text_width(words, type_scale::LABEL.size, fonts::mono());
    let stat_width = |words: &str| FOOT_GAP + STAT_ICON + STAT_GAP + small(words);

    // The link shows only once everything the task waited on is done.
    let links = (!task.depends_on.is_empty() && dependencies_met)
        .then(|| task.depends_on.len().to_string());
    let comments = (!task.comments.is_empty()).then(|| task.comments.len().to_string());
    // The clock of work under way.
    let since = task.doing_since.filter(|_| task.column == "doing");
    let elapsed = since.map(|since| elapsed_label(v.now - since as i64, tr));

    let (mut program, mut clock_words) = (true, true);
    if let Some(assignee) = &task.assignee {
        let pane = v.pane(assignee);
        let name = pane.map_or(assignee.as_str(), |pane| pane.name.as_str());
        let held =
            GLYPH + space::SM + text_width(name, type_scale::CAPTION.size, fonts::ui_medium());
        let agent = pane.map_or(0.0, |pane| {
            space::SM
                + text_width(
                    &format!("· {}", pane.agent),
                    type_scale::CAPTION.size,
                    fonts::ui(),
                )
        });
        let counts = links
            .iter()
            .chain(comments.iter())
            .map(|n| stat_width(n))
            .sum();
        (program, clock_words) = foot_fits(
            room,
            held,
            agent,
            counts,
            elapsed
                .as_deref()
                .map(|words| STAT_ICON + STAT_GAP + small(words)),
        );
    }

    let who: Element<'a, Message> = match &task.assignee {
        Some(assignee) => assignee_view(v, assignee, done, program, false),
        None => text(tr.t(Key::BoardNobody))
            .size(type_scale::CAPTION.size)
            .color(t.palette.text_muted)
            .wrapping(text::Wrapping::None)
            .into(),
    };

    let stat = |glyph: Icon, count: Option<String>, words: String| -> Element<'a, Message> {
        let mut line = row![icon(glyph, STAT_ICON).color(t.palette.text_muted)]
            .spacing(STAT_GAP)
            .align_y(Alignment::Center);
        if let Some(count) = count {
            line = line.push(mono_muted(t, count));
        }
        tip::over(t, line, words)
    };
    let mut meta = row![].spacing(FOOT_GAP).align_y(Alignment::Center);

    if let (Some(since), Some(elapsed)) = (since, elapsed) {
        let started = since_words(v, since);
        // With no room for the time, it joins the tooltip.
        let (shown, hint) = if clock_words {
            (Some(elapsed), started)
        } else {
            (None, format!("{started} · {elapsed}"))
        };
        meta = meta.push(stat(Icon::Clock, shown, hint));
    }
    if let Some(links) = links {
        meta = meta.push(stat(
            Icon::Link,
            Some(links),
            tr.t(Key::BoardDependsOn).into_owned(),
        ));
    }
    if let Some(comments) = comments {
        meta = meta.push(stat(
            Icon::Bubble,
            Some(comments),
            tr.tn(Key::BoardComments, task.comments.len(), &[])
                .into_owned(),
        ));
    }

    row![container(who).width(Length::Fill).clip(true), meta]
        .spacing(FOOT_GAP)
        .align_y(Alignment::Center)
        .into()
}

/// "In progress since 12:24", with the date when it began another day.
fn since_words(v: &View<'_>, since: u64) -> String {
    let tr = v.ctx.i18n;
    match time_today(since, v.now as u64, tr.locale()) {
        Some(time) => tr
            .t_with(Key::BoardDoingSinceTime, &args!("time" => time.as_str()))
            .into_owned(),
        None => tr
            .t_with(
                Key::BoardDoingSince,
                &args!("date" => when(since, tr.locale(), DateStyle::Full).as_str()),
            )
            .into_owned(),
    }
}

fn mono_muted<'a>(t: &'a Theme, content: String) -> iced::widget::Text<'a> {
    text(content)
        .size(type_scale::LABEL.size)
        .font(fonts::mono())
        .wrapping(text::Wrapping::None)
        .color(t.palette.text_muted)
}

/// A task's priority as three rising bars and a word (Low, Medium, High): the
/// way a signal is read, how many bars are lit is the urgency. The number
/// agents set stays in the tooltip.
pub(super) fn priority<'a>(v: &View<'a>, priority: u8) -> Element<'a, Message> {
    let t = v.t();
    let tr = v.ctx.i18n;
    let value = f64::from(priority);
    let level = priority_level(value);
    let word = tr.t(priority_key(value)).into_owned();
    // The bars rise to the right: 44 %, 72 % and the whole of the tallest.
    let bars = row((0..3).map(|n| {
        let rise = [0.44, 0.72, 1.0][n];
        container(Space::new())
            .width(look::PRIO_BAR_WIDTH)
            .height(look::PRIO_BAR_HEIGHT * rise)
            .style(look::prio_bar(t, n < level.bars()))
            .into()
    }))
    .spacing(1.5)
    .align_y(Alignment::End);
    let words = tr
        .t_with(
            Key::BoardPriorityTitle,
            &args!("n" => priority, "level" => word.as_str()),
        )
        .into_owned();
    tip::over(
        t,
        row![
            container(bars).height(look::PRIO_BAR_HEIGHT),
            text(word)
                .size(type_scale::LABEL.size)
                .wrapping(text::Wrapping::None)
                .color(t.palette.text_muted)
        ]
        .spacing(5.0)
        .align_y(Alignment::Center),
        words,
    )
}

/// How the glyph of a pane's state is drawn.
pub(super) fn glyph_of(state: GlyphState) -> Shown {
    match state {
        GlyphState::Starting => Shown::Starting,
        GlyphState::Exited => Shown::Exited,
        GlyphState::Working => Shown::Working,
        GlyphState::Waiting => Shown::Waiting,
        GlyphState::Yours => Shown::Yours,
        GlyphState::Idle => Shown::Idle,
    }
}

/// Who holds a task, with the live state of their pane in front of the name:
/// a card in progress whose agent is waiting for an answer reads as such
/// without leaving the board. An assignee no pane answers to is drawn idle. A
/// finished card has the mark of work that is done: what its agent is doing
/// now is no longer about it. The name leads to the agent's pane when one
/// answers to it, and `program` says whether its program is named too.
pub(super) fn assignee_view<'a>(
    v: &View<'a>,
    assignee: &'a str,
    done: bool,
    program: bool,
    in_dialog: bool,
) -> Element<'a, Message> {
    let t = v.t();
    let tr = v.ctx.i18n;
    let pane = v.pane(assignee);
    let name = pane.map_or_else(|| assignee.to_owned(), |pane| pane.name.clone());

    let state = if done {
        Shown::Completed
    } else {
        glyph_of(pane.map_or(GlyphState::Idle, |pane| pane.state))
    };
    let named = text(name.clone())
        .size(type_scale::CAPTION.size)
        .font(fonts::ui_medium())
        .color(t.palette.text)
        .wrapping(text::Wrapping::None);
    let named: Element<'a, Message> = match pane {
        // A button to the pane: its click is its own, and opens no task.
        Some(pane) => {
            let go = Message::Board(Msg::FocusAgent {
                ws: pane.workspace.clone(),
                pane: pane.id.clone(),
                from_dialog: in_dialog,
            });
            let link = button(named)
                .padding(0)
                .style(look::name_link(t))
                .on_press(go.clone());
            tip::over(
                t,
                focus::ring(t, link).radius(radius::XS).on_activate(go),
                tr.t_with(Key::BoardFocusAgent, &args!("name" => name.as_str()))
                    .into_owned(),
            )
        }
        None => tip::over(
            t,
            named,
            tr.t_with(Key::BoardAssigneeTitle, &args!("name" => assignee))
                .into_owned(),
        ),
    };

    let mut line = row![glyph::state(t, state, GLYPH), named]
        .spacing(space::SM)
        .align_y(Alignment::Center);
    if let Some(pane) = pane.filter(|_| program) {
        line = line.push(
            text(format!("· {}", pane.agent))
                .size(type_scale::CAPTION.size)
                .color(t.palette.text_muted)
                .wrapping(text::Wrapping::None),
        );
    }
    line.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_narrow_foot_drops_the_program_first_then_the_time() {
        let fits = |room| foot_fits(room, 40.0, 70.0, 30.0, Some(50.0));
        assert_eq!(fits(200.0), (true, true));
        assert_eq!(fits(199.0), (false, true));
        assert_eq!(fits(130.0), (false, true));
        assert_eq!(fits(129.0), (false, false));
        // With no clock there is only the program to drop.
        assert_eq!(foot_fits(100.0, 40.0, 70.0, 0.0, None), (false, true));
    }
}
