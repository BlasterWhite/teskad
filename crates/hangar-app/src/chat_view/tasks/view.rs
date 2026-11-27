//! The tasks panel drawn from its [`Panel`]: nothing is worked out here but
//! the relative times, which follow the clock.

use std::time::Instant;

use iced::widget::{Space, button, column, container, progress_bar, row, rule, scrollable, text};
use iced::{Alignment, Background, Element, Length, Padding};

use hangar_i18n::{Key, args};
use hangar_theme::tokens::{self, font_size, radius, scale, space};
use hangar_theme::{Theme, fonts, style};
use hangar_ui::{Icon, focus, glyph, icon, tip};

use super::model::{Group, Overall, Panel, Row, Waiting};
use super::{Msg, State, msg};
use crate::board::Column;
use crate::board::format::now_ms;
use crate::ctx::Ctx;
use crate::inspector::format::since;
use crate::message::Message;
use crate::navigator::reads;
use crate::needs_you::model::KINDS;
use crate::needs_you::{LandRequest, Msg as NeedsYouMsg};
use crate::state::Workspace;

/// A task's title: 13 on 18.
const TITLE_LINE: f32 = 18.0 / 13.0;
const BAR: f32 = 6.0;
const BAR_RADIUS: f32 = 2.0;
/// The panel folded to its icon.
pub const RAIL_WIDTH: f32 = 44.0;
/// The gap at the sides of the list, and a row's own side padding.
const SIDE: f32 = space::MD;

/// The panel; `fold` when the window is narrow enough that it can go back
/// to its icon.
pub fn panel<'a>(
    st: &'a State,
    ctx: Ctx<'a>,
    ws: &'a Workspace,
    panel: &Panel,
    fold: bool,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let mut body = column![].spacing(space::LG);

    if !panel.waiting.is_empty() {
        body = body.push(waiting(ctx, ws, &panel.waiting));
    }

    if !panel.loaded {
        body = body.push(note(t, ctx.t(Key::BoardLoading).into_owned()));
    } else if panel.groups.is_empty() {
        body = body.push(note(t, ctx.t(Key::BoardEmptyTitle).into_owned()));
    } else {
        let now = now_ms();
        for group in &panel.groups {
            body = body.push(column_group(st, ctx, ws, group, now));
        }
    }

    column![
        head(ctx, panel, fold),
        rule::horizontal(tokens::BORDER_WIDTH).style(style::rule::sep(t)),
        scrollable(
            container(body)
                .padding([space::LG, SIDE])
                .width(Length::Fill)
        )
        .style(style::scrollable::default(t))
        .height(Length::Fill),
    ]
    .into()
}

/// The panel folded: its icon, which opens it, the progress as its badge,
/// and the mark of what waits on the person.
pub fn rail<'a>(ctx: Ctx<'a>, panel: &Panel) -> Element<'a, Message> {
    let t = ctx.theme;
    let mut rail = column![
        hangar_ui::button::icon_button(t, Icon::Board)
            .sized(hangar_ui::button::Side::Sm)
            .tip(ctx.t(Key::ChatViewTasks).into_owned())
            .on_press(msg(Msg::Unfold(true))),
    ]
    .spacing(space::SM)
    .align_x(Alignment::Center);
    if panel.overall.total > 0 {
        rail = rail.push(
            container(
                text(format!("{} %", panel.overall.percent))
                    .size(scale::type_scale::LABEL.size)
                    .font(fonts::mono())
                    .color(t.palette.text_secondary),
            )
            .padding([0.0, space::XXS])
            .style(style::container::chip(t)),
        );
    }
    if !panel.waiting.is_empty() {
        rail = rail.push(glyph::state(
            t,
            reads::activity_glyph(crate::state::AgentActivity::Waiting),
            8.0,
        ));
    }
    container(rail)
        .padding([space::LG, 0.0])
        .center_x(Length::Fill)
        .into()
}

/// The title and the global progress, with how many tasks are done.
fn head<'a>(ctx: Ctx<'a>, panel: &Panel, fold: bool) -> Element<'a, Message> {
    let t = ctx.theme;
    let Overall {
        percent,
        done,
        total,
    } = panel.overall;
    let mut title = row![
        text(ctx.t(Key::ChatViewTasks))
            .size(font_size::TITLE)
            .font(fonts::display())
            .style(style::text::primary(t))
            .width(Length::Fill),
    ]
    .align_y(Alignment::Center);
    if fold {
        title = title.push(
            hangar_ui::button::icon_button(t, Icon::SidebarRight)
                .small()
                .tip(ctx.t(Key::ChatViewTasks).into_owned())
                .on_press(msg(Msg::Unfold(false))),
        );
    }
    let mut head = column![title].spacing(space::MD);
    if total > 0 {
        let full = done == total;
        head = head.push(
            row![
                progress_bar(0.0..=100.0, f32::from(percent))
                    .girth(BAR)
                    .style(bar_style(t, full)),
                text(format!("{percent} %"))
                    .size(scale::type_scale::LABEL.size)
                    .font(fonts::mono())
                    .color(t.palette.text_secondary),
            ]
            .spacing(space::MD)
            .align_y(Alignment::Center),
        );
        head = head.push(
            text(format!("{} {done}/{total}", ctx.t(Key::BoardDone)))
                .size(scale::type_scale::CAPTION.size)
                .style(style::text::muted(t)),
        );
    }
    container(head)
        .padding(Padding {
            top: space::XL,
            right: space::XXL,
            bottom: space::LG,
            left: space::XXL,
        })
        .width(Length::Fill)
        .into()
}

/// A bar with the mock-up's squared ends: green once everything is done.
fn bar_style(t: &Theme, full: bool) -> impl Fn(&iced::Theme) -> progress_bar::Style + '_ {
    move |theme| {
        let mut look = if full {
            style::progress_bar::success(t)(theme)
        } else {
            style::progress_bar::accent(t)(theme)
        };
        look.border.radius = BAR_RADIUS.into();
        look
    }
}

/// A group's name in small capitals and how many it holds.
fn heading<'a>(t: &'a Theme, label: String, count: usize) -> Element<'a, Message> {
    container(
        row![
            text(label.to_uppercase())
                .size(scale::type_scale::LABEL.size)
                .font(fonts::mono())
                .color(t.palette.text_muted),
            text(count.to_string())
                .size(scale::type_scale::LABEL.size)
                .font(fonts::mono())
                .color(t.palette.text_muted),
        ]
        .spacing(space::SM),
    )
    .padding([space::XXS, space::MD])
    .into()
}

fn note<'a>(t: &'a Theme, words: String) -> Element<'a, Message> {
    text(words)
        .size(font_size::BODY)
        .style(style::text::muted(t))
        .into()
}

fn caption<'a>(t: &'a Theme, words: String) -> iced::widget::Text<'a> {
    text(words)
        .size(font_size::CAPTION)
        .color(t.palette.text_secondary)
}

fn waiting<'a>(ctx: Ctx<'a>, ws: &'a Workspace, items: &[Waiting]) -> Element<'a, Message> {
    let t = ctx.theme;
    let mut list = column![heading(
        t,
        ctx.t(Key::NeedsYouTitle).into_owned(),
        items.len()
    )]
    .spacing(space::XS);
    for kind in KINDS {
        for item in items.iter().filter(|item| item.kind() == kind) {
            list = list.push(waiting_row(ctx, ws, item));
        }
    }
    container(list)
        .padding(space::MD)
        .width(Length::Fill)
        .style(move |_: &iced::Theme| container::Style {
            background: Some(Background::Color(t.palette.hover_overlay)),
            border: iced::Border {
                radius: hangar_theme::tokens::radius::MD.into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

fn waiting_row<'a>(ctx: Ctx<'a>, ws: &'a Workspace, item: &Waiting) -> Element<'a, Message> {
    let t = ctx.theme;
    let tr = ctx.i18n;
    let title = |words: &str| {
        text(words.to_string())
            .size(font_size::UI)
            .font(fonts::ui_medium())
            .color(t.palette.text)
    };
    let (mark, lines, action): (
        Element<'a, Message>,
        Element<'a, Message>,
        Element<'a, Message>,
    ) = match item {
        Waiting::Pane { pane, name } => (
            glyph::state(
                t,
                reads::activity_glyph(crate::state::AgentActivity::Waiting),
                7.0,
            )
            .into(),
            column![
                title(name),
                caption(t, tr.t(Key::NeedsYouWaitingDetail).into_owned())
            ]
            .spacing(1)
            .into(),
            tip::bottom(
                t,
                hangar_ui::button::secondary(t, tr.t(Key::NeedsYouGo).into_owned())
                    .small()
                    .on_press(msg(Msg::GoTo {
                        ws: ws.id.clone(),
                        pane: pane.clone(),
                    })),
                tr.t_with(Key::NeedsYouGoTo, &args!("name" => name.as_str()))
                    .into_owned(),
            ),
        ),
        Waiting::Question {
            title: words,
            note,
            orchestrator,
            ..
        } => {
            let mut lines = column![title(words)].spacing(1);
            if let Some(note) = note {
                lines = lines.push(caption(t, note.clone()));
            }
            let action: Element<'a, Message> = match orchestrator {
                Some((id, name)) => tip::bottom(
                    t,
                    hangar_ui::button::secondary(t, tr.t(Key::NeedsYouAnswer).into_owned())
                        .small()
                        .on_press(msg(Msg::GoTo {
                            ws: ws.id.clone(),
                            pane: id.clone(),
                        })),
                    tr.t_with(Key::NeedsYouAnswerIn, &args!("name" => name.as_str()))
                        .into_owned(),
                ),
                None => Space::new().into(),
            };
            (
                icon(Icon::Flag, 13.0).color(t.colors.state_waiting).into(),
                lines.into(),
                action,
            )
        }
        Waiting::Land {
            base,
            task,
            title: words,
            assignee,
        } => {
            let needs_you = &ctx.pkg.needs_you;
            let mut lines = column![title(words)].spacing(1);
            if let Some(who) = assignee {
                lines = lines.push(caption(t, who.clone()));
            }
            if let Some(error) = needs_you.failure(&base.key) {
                lines = lines.push(
                    text(
                        tr.t_with(Key::NeedsYouLandFailed, &args!("error" => error))
                            .into_owned(),
                    )
                    .size(font_size::CAPTION)
                    .color(t.palette.status.failed),
                );
            }
            let landing = needs_you.is_landing(ctx.model, base, task, Instant::now());
            let land = hangar_ui::button::primary(
                t,
                tr.t(if landing {
                    Key::NeedsYouLanding
                } else {
                    Key::NeedsYouLand
                })
                .into_owned(),
            )
            .icon(Icon::Merge)
            .small()
            .on_press_maybe((!landing).then(|| {
                Message::NeedsYou(NeedsYouMsg::Land(LandRequest {
                    key: base.key.clone(),
                    ws: base.workspace_id.clone(),
                    task: task.clone(),
                }))
            }))
            .tip(tr.t(Key::NeedsYouLandHint).into_owned());
            (
                icon(Icon::Merge, 13.0)
                    .color(t.palette.status.running)
                    .into(),
                lines.into(),
                land.into(),
            )
        }
    };
    row![
        container(mark).width(16).align_x(Alignment::Center),
        container(lines).width(Length::Fill).clip(true),
        action,
    ]
    .spacing(space::MD)
    .align_y(Alignment::Center)
    .padding([space::XS, 0.0])
    .into()
}

fn column_group<'a>(
    st: &'a State,
    ctx: Ctx<'a>,
    ws: &'a Workspace,
    group: &Group,
    now: u64,
) -> Element<'a, Message> {
    let mut list = column![heading(
        ctx.theme,
        ctx.t(group.column.label_key()).into_owned(),
        group.rows.len()
    )]
    .spacing(space::XXS);
    for task in &group.rows {
        list = list.push(task_row(st, ctx, ws, task, now));
    }
    list.into()
}

fn task_row<'a>(
    st: &'a State,
    ctx: Ctx<'a>,
    ws: &'a Workspace,
    task: &Row,
    now: u64,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let tr = ctx.i18n;
    let open = st.is_expanded(&task.id);

    let who = task
        .assignee
        .clone()
        .unwrap_or_else(|| tr.t(Key::BoardUnassigned).into_owned());
    let mut lines = column![
        text(task.title.clone())
            .size(font_size::UI)
            .line_height(text::LineHeight::Relative(TITLE_LINE))
            .color(t.palette.text)
            .width(Length::Fill),
        text(format!("{who} · {}", tr.t(task.column.label_key())))
            .size(scale::type_scale::LABEL.size)
            .color(t.palette.text_muted),
    ]
    .spacing(space::XXS);

    if let Some(reason) = &task.blocked {
        lines = lines.push(
            row![
                glyph::state(t, glyph::GlyphState::Failed, 12.0),
                text(
                    tr.t_with(Key::BoardBlockedFor, &args!("reason" => reason.as_str()))
                        .into_owned()
                )
                .size(scale::type_scale::CAPTION.size)
                .color(t.palette.status.failed)
                .width(Length::Fill),
            ]
            .spacing(space::SM),
        );
    }

    // Its delivery, while Hangar runs it and a few seconds after: beside the
    // progress its worker last reported.
    if let Some(stage) = ctx.model.live.panes_logic.delivery_stage(&task.id, now) {
        lines = lines.push(super::chip::delivery_chip(
            ctx, &ws.cwd, &task.id, stage, now, true, false,
        ));
    }

    // A done task is done: its 100 says nothing the column does not.
    if let Some(progress) = task
        .progress
        .as_ref()
        .filter(|_| task.column != Column::Done)
    {
        lines = lines.push(
            row![
                progress_bar(0.0..=100.0, f32::from(progress.percent))
                    .girth(BAR)
                    .style(bar_style(t, false)),
                text(format!("{} %", progress.percent))
                    .size(font_size::CAPTION)
                    .font(fonts::mono())
                    .color(t.palette.text_secondary),
            ]
            .spacing(space::MD)
            .align_y(Alignment::Center),
        );
        let mut said = Vec::new();
        if !progress.note.is_empty() {
            said.push(progress.note.clone());
        }
        if progress.at > 0 {
            let at = i64::try_from(progress.at).unwrap_or(i64::MAX);
            let now = i64::try_from(now).unwrap_or(i64::MAX);
            said.push(since(at, now, ctx.model.locale()).text(tr));
        }
        if !said.is_empty() {
            lines = lines.push(caption(t, said.join(" · ")));
        }
    }

    if open {
        if !task.description.is_empty() {
            lines = lines.push(
                container(
                    text(task.description.clone())
                        .size(font_size::CAPTION_LG)
                        .color(t.palette.text),
                )
                .padding(Padding {
                    top: space::XS,
                    ..Padding::ZERO
                }),
            );
        }
        if let Some(comment) = &task.last_comment {
            lines = lines.push(
                column![
                    text(comment.author.clone())
                        .size(font_size::LABEL)
                        .font(fonts::ui_semibold())
                        .color(t.palette.text_secondary),
                    text(comment.text.clone())
                        .size(font_size::CAPTION_LG)
                        .color(t.palette.text_secondary),
                ]
                .spacing(1),
            );
        }
    }

    let chevron = icon(
        if open {
            Icon::ChevronDown
        } else {
            Icon::ChevronRight
        },
        11.0,
    )
    .color(t.palette.text_muted);
    // What is being worked on stands out of the list.
    let current = task.column == Column::Doing;
    let press = msg(Msg::Toggle(task.id.clone()));
    let line = button(row![chevron, lines].spacing(space::SM))
        .padding([space::SM, space::MD])
        .width(Length::Fill)
        .style(move |theme, status| {
            let mut look = style::button::row(t, open)(theme, status);
            if current && matches!(status, button::Status::Active) {
                look.background = Some(Background::Color(t.palette.selection_bg));
            }
            look
        })
        .on_press(press.clone());
    focus::ring(t, line)
        .radius(radius::SM)
        .on_activate(press)
        .into()
}
