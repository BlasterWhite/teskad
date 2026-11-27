//! The board's two surfaces (`BoardView.tsx`): the full view, four columns
//! side by side that stack when the room is narrow, and the dock, a panel of
//! sections that fold.

use std::collections::HashMap;

use hangar_i18n::{Key, args};
use hangar_theme::tokens::scale::type_scale;
use hangar_theme::tokens::{BORDER_WIDTH, font_size, size, space};
use hangar_theme::{Theme, fonts, style};
use hangar_ui::button;
use hangar_ui::dashed::dashed;
use hangar_ui::icon::{Icon, icon};
use hangar_ui::tab_toolbar;
use iced::alignment::{Horizontal, Vertical};
use iced::widget::{
    Space, column, container, mouse_area, responsive, row, rule, scrollable, stack, text,
    text_input,
};
use iced::{Alignment, Element, Length, Padding, Radians};

use crate::ctx::Ctx;
use crate::message::Message;
use crate::panes_logic::agent_pane::Finding;
use crate::panes_logic::with_finder;
use crate::state::GlyphState;

use super::card;
use super::dock;
use super::filter;
use super::format::now_ms;
use super::look;
use super::model::{Column, Task, index_by_id};
use super::select;
use super::{Msg, State};

/// The pane an assignee leads to, as a card names it (`AssigneePane`).
#[derive(Debug, Clone)]
pub struct AssigneePane {
    pub workspace: String,
    pub id: String,
    pub name: String,
    /// The agent running there, "Claude Code"; dropped when the card is narrow.
    pub agent: &'static str,
    pub state: GlyphState,
}

/// What a frame of the board is drawn from: the model, the cache and the
/// work worked out once per frame rather than once per card.
pub(super) struct View<'a> {
    pub st: &'a State,
    pub ctx: Ctx<'a>,
    /// The folder of the board drawn: what every message that writes names.
    pub cwd: &'a str,
    pub all: &'a [Task],
    /// `false` until the first read answers.
    pub loaded: bool,
    pub shown: Vec<&'a Task>,
    pub by_id: HashMap<&'a str, &'a Task>,
    /// The pane each assignee on the board leads to, once per assignee: the
    /// same few agents hold most of the tasks.
    pub panes: HashMap<&'a str, Option<AssigneePane>>,
    /// The time the frame is drawn at, for the clocks of cards in progress.
    pub now: i64,
    pub dock: bool,
    /// The task of this board whose sheet is open: its card is selected.
    pub selected: Option<&'a str>,
}

impl<'a> View<'a> {
    pub fn new(st: &'a State, ctx: Ctx<'a>, cwd: &'a str, dock: bool) -> View<'a> {
        let tasks = ctx.model.live.board.tasks(cwd);
        let all: &'a [Task] = tasks.unwrap_or(&[]);
        let shown = select::shown(all, &st.filters);
        // Built with every frame of the board: the names are resolved from
        // the memo (`panes_logic::with_finder`), not split again per assignee.
        let mut panes = HashMap::new();
        with_finder(&ctx.model.state().workspaces, |finder| {
            for assignee in all.iter().filter_map(|task| task.assignee.as_deref()) {
                panes
                    .entry(assignee)
                    .or_insert_with(|| pane_for(ctx, finder, assignee));
            }
        });
        View {
            st,
            ctx,
            cwd,
            loaded: tasks.is_some(),
            by_id: index_by_id(all),
            all,
            shown,
            panes,
            now: now_ms() as i64,
            dock,
            selected: st.open_task(cwd),
        }
    }

    pub fn t(&self) -> &'a Theme {
        self.ctx.theme
    }

    pub fn pane(&self, assignee: &str) -> Option<&AssigneePane> {
        self.panes.get(assignee).and_then(Option::as_ref)
    }
}

/// `paneFor`: the pane an assignee's name leads to, if one clearly does. The
/// workspace looked in first is the window's own.
fn pane_for(ctx: Ctx<'_>, finder: &mut Finding<'_>, assignee: &str) -> Option<AssigneePane> {
    let here = ctx.workspace().map(|workspace| workspace.id.as_str());
    let target = finder.find(assignee, here)?;
    let workspace = ctx.model.workspace(&target.workspace_id)?;
    let pane = workspace.panes.iter().find(|p| p.id == target.pane_id)?;
    Some(AssigneePane {
        workspace: workspace.id.clone(),
        id: pane.id.clone(),
        name: pane.name.clone(),
        agent: pane.agent.info().label,
        state: crate::panes_logic::state_glyph_state(ctx.model, &pane.id),
    })
}

/// The board of a workspace folder: full, or in the dock with the room its
/// grip may take (the width it has now and the widest it may be).
pub(super) fn board<'a>(
    st: &'a State,
    ctx: Ctx<'a>,
    cwd: &'a str,
    dock_room: Option<(f32, f32)>,
) -> Element<'a, Message> {
    let in_dock = dock_room.is_some();
    let v = View::new(st, ctx, cwd, in_dock);
    let t = v.t();
    let body: Element<'a, Message> = if let Some((width, _)) = dock_room {
        dock_body(v, width)
    } else {
        responsive(move |size| full_body(&v, size.width)).into()
    };
    let surface = container(body)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(look::surface(t, in_dock));
    match dock_room {
        Some((width, max)) => dock::panel(st, ctx, width, max, surface.into()),
        None => surface.into(),
    }
}

// ---- Full ------------------------------------------------------------------------------------

fn full_body<'a>(v: &View<'a>, width: f32) -> Element<'a, Message> {
    let t = v.t();
    let content: Element<'a, Message> = if !v.loaded {
        skeleton(v)
    } else if v.all.is_empty() && !v.st.composing.contains(v.cwd) {
        empty(v)
    } else {
        columns(v, width)
    };

    let page = column![
        bar_full(v, tab_toolbar::compact(width)),
        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(style::container::plain(t)),
    ];
    with_toast(v, page.into())
}

/// How wide each of the four columns is in a board `width` wide: a fourth of
/// the room, and no less than [`look::COLUMN_MIN`] once the columns scroll.
fn column_width(width: f32) -> f32 {
    let between = 3.0 * look::COLUMN_GAP + 2.0 * look::PAGE_PADDING.1;
    let share = ((width - between) / 4.0).max(0.0);
    if width < look::SCROLL_BELOW {
        share.max(look::COLUMN_MIN)
    } else {
        share
    }
}

/// The four columns side by side; under [`look::SCROLL_BELOW`] they keep a
/// readable width and scroll sideways.
fn columns<'a>(v: &View<'a>, width: f32) -> Element<'a, Message> {
    let each = column_width(width);
    let card_width = each - 2.0 * (CARDS_PADDING + BORDER_WIDTH);
    let scrolls = width < look::SCROLL_BELOW;
    let line = row(Column::ALL.into_iter().map(|which| {
        let column = column_view(v, which, false, false, card_width);
        container(column)
            .width(if scrolls {
                Length::Fixed(each)
            } else {
                Length::FillPortion(1)
            })
            .height(Length::Fill)
            .into()
    }))
    .spacing(look::COLUMN_GAP)
    .padding([look::PAGE_PADDING.0, look::PAGE_PADDING.1])
    .height(Length::Fill);
    if !scrolls {
        return line.into();
    }
    scrollable(line)
        .direction(scrollable::Direction::Horizontal(
            scrollable::Scrollbar::default(),
        ))
        .height(Length::Fill)
        .style(style::scrollable::default(v.t()))
        .into()
}

fn bar_full<'a>(v: &View<'a>, compact: bool) -> Element<'a, Message> {
    let t = v.t();
    let tr = v.ctx.i18n;
    let title = tab_toolbar::count(
        t,
        if v.loaded {
            count_text(v)
        } else {
            tr.t(Key::BoardTitle).into_owned()
        },
    );

    let mut leading = row![title, Space::new().width(space::MD)]
        .spacing(space::MD)
        .align_y(Alignment::Center);
    // Nothing to filter before the first read, or on an empty board.
    if v.st.filters.active() || !v.all.is_empty() {
        leading = leading.push(if compact {
            filter::folded(v)
        } else {
            filter::group(v)
        });
    }

    let note = tab_toolbar::note(
        t,
        tr.t(Key::BoardHint).into_owned(),
        tr.t(Key::BoardMcp).into_owned(),
        compact,
        Message::Board(Msg::OpenMcp),
    );
    let add = button::secondary(t, tr.t(Key::BoardAddTask))
        .icon(Icon::Plus)
        .size(button::Size::Bar)
        .on_press(Message::Board(Msg::StartAdding(
            v.cwd.to_owned(),
            Column::Todo,
        )));

    tab_toolbar::bar(
        t,
        leading,
        row![note, Element::from(add)]
            .spacing(space::MD)
            .align_y(Alignment::Center),
    )
}

/// "12 tasks", or "3 of 12 tasks" while a filter hides some.
fn count_text(v: &View<'_>) -> String {
    let tr = v.ctx.i18n;
    let n = v.all.len();
    if v.st.filters.active() {
        tr.tn(Key::BoardShown, n, &args!("shown" => v.shown.len()))
            .into_owned()
    } else {
        tr.tn(Key::BoardTasks, n, &[]).into_owned()
    }
}

// ---- Dock ------------------------------------------------------------------------------------

fn dock_body<'a>(v: View<'a>, width: f32) -> Element<'a, Message> {
    // The panel less its grip and the room around the cards.
    let card_width = width - look::GRIP_WIDTH - 2.0 * space::LG;
    let t = v.t();
    let tr = v.ctx.i18n;
    let filtering = v.st.filters.active();

    let sub = v.loaded.then(|| {
        let words = if filtering {
            count_text(&v)
        } else {
            tr.tn(Key::BoardDockActive, select::active_count(&v.shown), &[])
                .into_owned()
        };
        text(words)
            .size(font_size::LABEL)
            .font(fonts::mono())
            .style(style::text::subtle(t))
    });

    let filter_tool: Element<'a, Message> = {
        let tool = button::toggle_button(t, Icon::Filter, v.st.filter_bar)
            .small()
            .tip(tr.t(Key::BoardFilters))
            .on_press(Message::Board(Msg::ToggleFilterBar));
        // Filters left on with their row folded away: the button says so.
        if filtering && !v.st.filter_bar {
            let dot = container(
                container(Space::new())
                    .width(5.0)
                    .height(5.0)
                    .style(look::flag(t)),
            )
            .width(size::CONTROL_HEIGHT_SM)
            .height(size::CONTROL_HEIGHT_SM)
            .padding(3.0)
            .align_x(Horizontal::Right)
            .align_y(Vertical::Top);
            stack![tool, dot].into()
        } else {
            tool.into()
        }
    };

    let mut tools = row![
        text(tr.t(Key::BoardTitle))
            .size(font_size::UI)
            .font(fonts::ui_semibold())
    ]
    .spacing(space::SM)
    .align_y(Alignment::Center);
    if let Some(sub) = sub {
        tools = tools.push(sub);
    }
    let tools = tools
        .push(Space::new().width(Length::Fill))
        .push(filter_tool)
        .push(
            button::icon_button(t, Icon::Expand)
                .small()
                .tip(tr.t(Key::BoardDockExpand))
                .on_press(Message::Board(Msg::Expand)),
        )
        .push(
            button::icon_button(t, Icon::Close)
                .small()
                .tip(tr.t(Key::BoardDockHide))
                .on_press(Message::Board(Msg::HideDock)),
        );
    let bar = container(tools)
        .padding(Padding {
            top: 0.0,
            right: space::XS,
            bottom: 0.0,
            left: space::XL,
        })
        .center_y(look::DOCK_BAR_HEIGHT)
        .style(style::container::pane_head(t, false));

    let mut page = column![bar, rule::horizontal(1).style(style::rule::hairline(t))];
    if v.st.filter_bar {
        page = page
            .push(
                container(filter::group(&v))
                    .padding([space::SM, space::LG])
                    .width(Length::Fill),
            )
            .push(rule::horizontal(1).style(style::rule::hairline(t)));
    }

    let content: Element<'a, Message> = if !v.loaded {
        skeleton(&v)
    } else if v.all.is_empty() && !v.st.composing.contains(v.cwd) {
        empty(&v)
    } else {
        scrollable(
            column(
                Column::DOCK_ORDER
                    .into_iter()
                    .map(|c| column_view(&v, c, true, true, card_width)),
            )
            .padding(Padding {
                top: space::XS,
                bottom: space::LG,
                ..Padding::ZERO
            })
            .width(Length::Fill),
        )
        .height(Length::Fill)
        .style(style::scrollable::default(t))
        .into()
    };
    page = page.push(content);
    with_toast(&v, page.into())
}

// ---- Columns ---------------------------------------------------------------------------------

/// The room around a column's cards, across.
const CARDS_PADDING: f32 = 8.0;

/// A column. In the dock it is a section that folds; `flow` lets its cards
/// run the length of the page instead of scrolling inside the column.
fn column_view<'a>(
    v: &View<'a>,
    which: Column,
    in_dock: bool,
    flow: bool,
    card_width: f32,
) -> Element<'a, Message> {
    let t = v.t();
    let tr = v.ctx.i18n;
    let items = select::column_items(&v.shown, which);
    let folded = in_dock && v.st.folded.contains(&which);
    let name = tr.t(which.label_key()).into_owned();
    let over =
        v.st.press
            .as_ref()
            .is_some_and(|press| press.cwd == v.cwd && press.moved && press.over == Some(which));
    let add_label = tr
        .t_with(Key::BoardAddTo, &args!("column" => name.as_str()))
        .into_owned();

    let mark = hangar_ui::glyph::state(t, look::mark(which), look::MARK);
    let count = text(items.len().to_string())
        .size(type_scale::LABEL.size)
        .font(fonts::mono())
        .color(t.palette.text_muted);
    let plus = button::icon_button(t, Icon::Plus)
        .sized(button::Side::Xs)
        .tip(add_label.clone())
        .on_press(Message::Board(Msg::StartAdding(v.cwd.to_owned(), which)));

    let head: Element<'a, Message> = if in_dock {
        let caret = icon(Icon::ChevronRight, 10.0)
            .color(t.palette.text_muted)
            .rotate(if folded {
                Radians(0.0)
            } else {
                Radians(std::f32::consts::FRAC_PI_2)
            });
        let toggle = iced::widget::button(look::centred(
            row![
                caret,
                mark,
                text(name.clone())
                    .size(font_size::CAPTION)
                    .font(fonts::ui_semibold())
                    .color(t.palette.text_secondary),
                count
            ]
            .spacing(7.0)
            .align_y(Alignment::Center),
        ))
        .width(Length::Fill)
        .height(24.0)
        .padding([0.0, space::SM])
        .style(look::fold_button(t))
        .on_press(Message::Board(Msg::Fold(which)));
        container(
            row![toggle, plus]
                .spacing(space::XS)
                .align_y(Alignment::Center),
        )
        .padding(Padding {
            left: space::SM,
            right: space::MD,
            ..Padding::ZERO
        })
        .center_y(look::DOCK_COLUMN_HEAD)
        .into()
    } else {
        container(
            row![
                mark,
                text(name.clone())
                    .size(type_scale::SMALL.size)
                    .font(fonts::ui_semibold())
                    .wrapping(text::Wrapping::None),
                count,
                Space::new().width(Length::Fill),
                plus,
            ]
            .spacing(space::MD)
            .align_y(Alignment::Center),
        )
        .padding(Padding {
            left: space::XL,
            right: space::SM,
            ..Padding::ZERO
        })
        .center_y(look::COLUMN_HEAD)
        .into()
    };

    let mut parts = iced::widget::column![head];
    if !folded {
        let side = if in_dock { space::LG } else { CARDS_PADDING };
        let mut cards =
            iced::widget::column(items.iter().map(|task| card::card(v, task, card_width)))
                .spacing(CARDS_PADDING)
                .padding(Padding {
                    // Room for the focus ring of the first card, which the
                    // scrollable would cut.
                    top: space::XS,
                    right: side,
                    bottom: CARDS_PADDING,
                    left: side,
                });
        if let Some(field) = add_field(v, which, items.is_empty(), add_label) {
            cards = cards.push(field);
        }
        parts = parts.push(if flow {
            Element::from(cards)
        } else {
            scrollable(cards)
                .height(Length::Fill)
                .style(style::scrollable::default(t))
                .into()
        });
    }

    let surface = container(parts)
        .width(Length::Fill)
        .height(if flow { Length::Shrink } else { Length::Fill })
        // Off the column's hairline.
        .padding(if in_dock { 0.0 } else { BORDER_WIDTH })
        .style(look::column(t, over, in_dock));

    // While a card is pressed, each column says when the pointer is over it.
    let mut area = mouse_area(surface);
    if v.st.press.as_ref().is_some_and(|press| press.cwd == v.cwd) {
        area = area
            .on_enter(Message::Board(Msg::Enter(which)))
            .on_exit(Message::Board(Msg::Leave(which)));
    }
    if flow {
        // Sections one under another, the board's rule between them.
        iced::widget::column![area, rule::horizontal(1).style(style::rule::sep(t))]
            .width(Length::Fill)
            .into()
    } else {
        area.into()
    }
}

/// The row to type a card into, there once it has been asked for (the
/// column's "+", the bar's "Add…") or while it holds something typed. A
/// column with nothing in it has the dashed "Add…" in its place until then.
fn add_field<'a>(
    v: &View<'a>,
    column: Column,
    empty: bool,
    label: String,
) -> Option<Element<'a, Message>> {
    let t = v.t();
    let cwd = v.cwd.to_owned();
    let draft =
        v.st.drafts
            .get(&(cwd.clone(), column))
            .map_or("", String::as_str);
    let asked =
        v.st.adding
            .as_ref()
            .is_some_and(|(at, which)| at == v.cwd && *which == column);
    if !(empty || !draft.is_empty() || asked) {
        return None;
    }
    // A column with nothing in it offers a card in a dashed frame; pressing
    // it opens the field the card is typed in.
    if draft.is_empty() && !asked {
        return Some(dashed(
            t,
            Icon::Plus,
            v.ctx.t(Key::BoardAddTask).into_owned(),
            super::add_id(column),
            Message::Board(Msg::StartAdding(cwd, column)),
        ));
    }
    let placeholder = v.ctx.t(Key::BoardAdd).into_owned();
    let (typed_on, submitted_on) = (cwd.clone(), cwd.clone());
    let field = text_input(&placeholder, draft)
        .id(super::add_id(column))
        .size(font_size::UI)
        .padding([6.0, space::LG])
        .width(Length::Fill)
        .on_input(move |value| Message::Board(Msg::Draft(typed_on.clone(), column, value)))
        .on_submit(Message::Board(Msg::Submit(submitted_on, column)))
        .style(look::add_field(t));
    // Escape empties what was typed, and goes no further.
    let has_draft = !draft.is_empty();
    let cleared_on = cwd.clone();
    let field = hangar_ui::keys::on_field_key(field, move |key, _| match key {
        iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape) if has_draft => {
            Some(Message::Board(Msg::ClearDraft(cleared_on.clone(), column)))
        }
        _ => None,
    });
    // A shortcut the keymap takes is spent before the field sees it, so
    // Ctrl+Shift+Enter here splits a pane and does not also make the card.
    let field = crate::keymap::guard(v.ctx, field);
    let field = hangar_ui::tip::over(t, field, label);
    // Something typed can be taken to the full form: a description, a
    // priority, the files its worker may change. The field is the row's first
    // child either way: were it drawn bare until something is typed, the
    // first character would make it a new widget to iced, whose own state
    // (the keyboard with it) starts again.
    let mut line = row![field].spacing(space::XS).align_y(Alignment::Center);
    if has_draft {
        let details = button::icon_button(t, Icon::Pencil)
            .small()
            .tip(v.ctx.t(Key::BoardAddDetails))
            .on_press(Message::Board(Msg::AddDetails(cwd, column)));
        line = line.push(details);
    }
    Some(line.into())
}

// ---- Empty, skeleton, toast --------------------------------------------------------------------

/// No task yet: what the board is for, and the two ways to fill it.
fn empty<'a>(v: &View<'a>) -> Element<'a, Message> {
    let t = v.t();
    let tr = v.ctx.i18n;
    let dock = v.dock;
    let glyph = container(icon(Icon::Board, if dock { 18.0 } else { 22.0 }))
        .center(look::EMPTY_ICON_BOX)
        .style(look::empty_icon(t));
    let title = text(tr.t(Key::BoardEmptyTitle))
        .size(if dock {
            font_size::BODY
        } else {
            font_size::TITLE
        })
        .font(fonts::ui_semibold());
    let body = text(tr.t(Key::BoardEmptyBody))
        .size(if dock {
            font_size::UI
        } else {
            font_size::UI_LG
        })
        .style(style::text::muted(t))
        .align_x(Horizontal::Center)
        .width(Length::Fill);
    let actions = row![
        button::primary(t, tr.t(Key::BoardEmptyAdd))
            .icon(Icon::Plus)
            .small()
            .on_press(Message::Board(Msg::StartAdding(
                v.cwd.to_owned(),
                Column::Todo
            ))),
        button::secondary(t, tr.t(Key::BoardEmptyMcp))
            .icon(Icon::Plug)
            .small()
            .on_press(Message::Board(Msg::OpenMcp)),
    ]
    .spacing(space::MD);
    let content = column![
        glyph,
        title,
        container(body).max_width(look::EMPTY_TEXT_WIDTH),
        container(actions).padding(Padding {
            top: space::XL,
            ..Padding::ZERO
        })
    ]
    .spacing(space::MD)
    .align_x(Alignment::Center);
    let page = container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding([36.0, 24.0])
        .center_x(Length::Fill);
    if dock {
        page.align_y(Vertical::Top).into()
    } else {
        page.center_y(Length::Fill).into()
    }
}

/// The columns' outline while the first read is out.
fn skeleton<'a>(v: &View<'a>) -> Element<'a, Message> {
    let t = v.t();
    let shape: &[usize] = if v.dock { &[2, 1] } else { &[3, 2, 1, 2] };
    let columns = shape.iter().map(|cards| {
        let label = container(Space::new())
            .width(look::SKELETON_LABEL.0)
            .height(look::SKELETON_LABEL.1)
            .style(look::skeleton(t));
        let cards = (0..*cards).map(|n| {
            container(Space::new())
                .width(Length::Fill)
                .height(if n % 2 == 1 {
                    look::SKELETON_CARD_SHORT
                } else {
                    look::SKELETON_CARD
                })
                .style(look::skeleton(t))
                .into()
        });
        container(
            column![
                container(label).center_y(look::COLUMN_HEAD),
                column(cards).spacing(6.0)
            ]
            .padding([0.0, space::LG]),
        )
        .width(Length::FillPortion(1))
        .into()
    });
    container(row(columns))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// Why a move was refused, for a few seconds, at the foot of the board.
fn with_toast<'a>(v: &View<'a>, page: Element<'a, Message>) -> Element<'a, Message> {
    let t = v.t();
    let Some((_, words)) = &v.st.toast else {
        return page;
    };
    let toast = container(
        container(text(words.clone()).size(font_size::UI))
            .padding([space::MD, space::XL])
            .max_width(520.0)
            .style(look::toast(t)),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .padding(space::XXL)
    .center_x(Length::Fill)
    .align_y(Vertical::Bottom);
    stack![page, toast].into()
}
