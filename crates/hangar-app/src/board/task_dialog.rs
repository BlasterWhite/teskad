//! A task (`TaskDialog.tsx`): its sheet, down the right of the window, where
//! it is read and edited (`ecrans.md` §4, 04b), or the dialog a new one is
//! written in.
//!
//! Opening a task shows the board's version of it, and every edit goes to the
//! board when the field is left (the web app saved on blur; iced's fields have
//! no blur, so a field is saved when anything else in the dialog is used, when
//! Enter is pressed in a one-line field, and when the dialog closes). What an
//! agent changes meanwhile (the title it renames, the column it moves the
//! task to) replaces what is on screen, as it did.
//!
//! `Overlay::Task { task: None }` is the same dialog for a task that does not
//! exist yet; the board's own "add" fields make a card from a title alone, and
//! this is the way to give it a description, a priority and the files its
//! worker may change at once.

use hangar_core::board::Task as BoardTask;
use hangar_i18n::{Key, Translator, args};
use hangar_theme::tokens::scale::{self, type_scale};
use hangar_theme::tokens::{BORDER_WIDTH, font_size, radius, size, space};
use hangar_theme::{Theme, fonts, style};
use hangar_ui::button;
use hangar_ui::dialog;
use hangar_ui::glyph::GlyphState;
use hangar_ui::icon::{Icon, icon};
use hangar_ui::input::{NumberField, NumberMsg, number_input};
use hangar_ui::menu::{self, Entry};
use hangar_ui::tail_text::tail_text;
use hangar_ui::{focus, layer, tab_toolbar, tip};
use iced::keyboard::{Key as Key_, key::Named};
use iced::widget::text_editor::Binding;
use iced::widget::{
    Space, checkbox, column, container, mouse_area, opaque, pick_list, row, rule, scrollable, text,
    text_editor, text_input,
};
use iced::{Alignment, Color, Element, Length, Padding, Pixels, Task};

use crate::message::{Intent, Message};
use crate::shell::View as MainView;
use crate::state::Model;

use super::actions::{self, After, Draft, Edit, Errors, Mutation};
use super::card::{assignee_view, chip};
use super::format::{DateStyle, elapsed_label, short_task_id, time_today, when};
use super::look;
use super::model::{Column, MAX_BLOCK_REASON, Task as T, index_by_id, recheck_problem};
use super::view::View;
use super::{Msg as BoardMsg, State, card_id};

/// The thread scrolls to its newest message.
const THREAD: &str = "board-task-thread";
const TITLE_FIELD: &str = "board-task-title";
pub(super) const REASON_FIELD: &str = "board-task-reason";

/// A dialog's open task, and everything typed that is not on the board yet.
#[derive(Debug)]
pub struct Dialog {
    pub cwd: String,
    /// The task, or `None` for one being written.
    pub task: Option<String>,
    title: String,
    /// The title as the sheet's heading edits it: it wraps as a heading does,
    /// and holds the one line `title` holds.
    heading: text_editor::Content,
    description: text_editor::Content,
    /// The paths and globs the task's worker may change, one per line.
    owns: text_editor::Content,
    priority: NumberField,
    /// The column of a task being written.
    column: Column,
    /// The dependencies of a task being written.
    depends_on: Vec<String>,
    /// What the task said last time it was read: edits are saved against it,
    /// and a change that is not ours replaces what is typed.
    seen: Seen,
    comment: String,
    blocking: bool,
    reason: String,
    recheck: String,
    human_only: bool,
    note: String,
    error: Option<String>,
    confirm_delete: bool,
    /// The sheet's menus: the columns under the state's badge, and "…".
    column_menu: bool,
    more_menu: bool,
    /// How many comments the thread showed, to follow a new one.
    comments: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Seen {
    title: String,
    description: String,
    owns: String,
}

impl Seen {
    fn of(task: &BoardTask) -> Seen {
        Seen {
            title: task.title.clone(),
            description: task.description.clone(),
            owns: task.owns.join("\n"),
        }
    }
}

/// The paths of a "one per line" field, trimmed, without the empty lines.
fn patterns(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

/// What an editor holds, with the line endings of the board's file.
fn text_of(content: &text_editor::Content) -> String {
    content.text().replace("\r\n", "\n")
}

fn priority_field(value: u8) -> NumberField {
    NumberField::new(f64::from(value), 0.0, 9.0, 1.0)
}

impl Dialog {
    /// The dialog for `task` of the board in `cwd`, or for a new one.
    pub fn open(cwd: &str, task: Option<&str>, tasks: &[T]) -> Dialog {
        let found = task.and_then(|id| tasks.iter().find(|candidate| candidate.id == id));
        let seen = found.map(Seen::of).unwrap_or_default();
        Dialog {
            cwd: cwd.to_owned(),
            task: task.map(str::to_owned),
            title: seen.title.clone(),
            heading: text_editor::Content::with_text(&seen.title),
            description: text_editor::Content::with_text(&seen.description),
            owns: text_editor::Content::with_text(&seen.owns),
            priority: priority_field(found.map_or(1, |task| task.priority)),
            column: Column::Todo,
            depends_on: Vec::new(),
            comments: found.map_or(0, |task| task.comments.len()),
            seen,
            comment: String::new(),
            blocking: false,
            reason: String::new(),
            recheck: String::new(),
            human_only: false,
            note: String::new(),
            error: None,
            confirm_delete: false,
            column_menu: false,
            more_menu: false,
        }
    }

    /// The form that puts the task on hold, open.
    pub(super) fn start_block(&mut self) {
        self.blocking = true;
    }

    pub fn is_new(&self) -> bool {
        self.task.is_none()
    }

    /// A task being written starts from what was typed in a column's add
    /// field: its title, and the column.
    pub fn seed(&mut self, column: Column, title: String) {
        self.column = column;
        self.title = title.trim().to_owned();
    }

    /// Follows the task as the board has it now: what is not ours replaces
    /// what is typed. Answers whether the thread grew.
    pub fn follow(&mut self, task: &BoardTask) -> bool {
        let now = Seen::of(task);
        if now.title != self.seen.title {
            self.title = now.title.clone();
            self.heading = text_editor::Content::with_text(&now.title);
        }
        if now.description != self.seen.description {
            self.description = text_editor::Content::with_text(&now.description);
        }
        if now.owns != self.seen.owns {
            self.owns = text_editor::Content::with_text(&now.owns);
        }
        self.seen = now;
        if (self.priority.value() - f64::from(task.priority)).abs() > f64::EPSILON
            && self.priority.is_valid()
        {
            self.priority.set(f64::from(task.priority));
        }
        let grew = task.comments.len() != self.comments;
        self.comments = task.comments.len();
        grew
    }

    /// What was typed and not saved yet, as the edits that save it.
    fn pending(&mut self) -> Vec<Edit> {
        if self.is_new() {
            return Vec::new();
        }
        let mut edits = Vec::new();
        if self.title != self.seen.title {
            edits.push(Edit::Title(self.title.clone()));
            self.seen.title = self.title.clone();
        }
        let description = text_of(&self.description);
        if description != self.seen.description {
            edits.push(Edit::Description(description.clone()));
            self.seen.description = description;
        }
        let owns = patterns(&text_of(&self.owns));
        if owns.join("\n") != self.seen.owns {
            self.seen.owns = owns.join("\n");
            edits.push(Edit::Owns(owns));
        }
        edits
    }

    /// The heading was typed in: the title is its text on one line (a paste
    /// may bring more).
    fn edit_heading(&mut self, action: text_editor::Action) {
        let edits = action.is_edit();
        self.heading.perform(action);
        if !edits {
            return;
        }
        let typed = text_of(&self.heading);
        self.title = typed.replace('\n', " ");
        if typed.contains('\n') {
            self.heading = text_editor::Content::with_text(&self.title);
            self.heading
                .perform(text_editor::Action::Move(text_editor::Motion::DocumentEnd));
        }
    }

    /// The block form and the unblock note, back to rest.
    fn finish_block(&mut self) {
        self.blocking = false;
        self.reason.clear();
        self.recheck.clear();
        self.human_only = false;
        self.note.clear();
        self.error = None;
    }

    fn draft(&self) -> Draft {
        Draft {
            title: self.title.trim().to_owned(),
            description: text_of(&self.description),
            column: self.column,
            priority: self.priority.value().round().clamp(0.0, 9.0) as u8,
            owns: patterns(&text_of(&self.owns)),
            depends_on: self.depends_on.clone(),
        }
    }
}

/// Messages of the dialog, routed as `Msg::Dialog`.
#[derive(Debug, Clone)]
pub enum Msg {
    Title(String),
    SaveTitle,
    Heading(text_editor::Action),
    ColumnMenu(bool),
    MoreMenu(bool),
    Description(text_editor::Action),
    Owns(text_editor::Action),
    Column(Column),
    Priority(NumberMsg),
    Release,
    StartBlock,
    CancelBlock,
    Reason(String),
    Recheck(String),
    HumanOnly(bool),
    ConfirmBlock,
    Note(String),
    Unblock,
    Comment(String),
    Send,
    AskDelete,
    CancelDelete,
    ConfirmDelete,
    Create,
    AddDependency(Dependency),
    RemoveDependency(String),
    Close,
}

impl Msg {
    /// Typing in a field is not leaving it.
    fn is_typing(&self) -> bool {
        matches!(
            self,
            Msg::Title(_)
                | Msg::Heading(_)
                | Msg::Description(_)
                | Msg::Owns(_)
                | Msg::Reason(_)
                | Msg::Recheck(_)
                | Msg::Note(_)
                | Msg::Comment(_)
        )
    }
}

/// A task a dependency can be added from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    pub id: String,
    label: String,
}

impl std::fmt::Display for Dependency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

fn task_named<'a>(m: &'a Model, cwd: &str, id: &str) -> Option<&'a T> {
    m.live
        .board
        .tasks(cwd)?
        .iter()
        .find(|candidate| candidate.id == id)
}

/// Saves what is pending, as mutations.
fn save(cwd: &str, id: &str, edits: Vec<Edit>) -> Task<Message> {
    Task::batch(edits.into_iter().map(|edit| {
        actions::run(
            cwd,
            Mutation::Update {
                id: id.to_owned(),
                edit,
            },
            Errors::Swallow,
            After::Nothing,
        )
    }))
}

/// A task for the dialog to open on, when it is opened from `Intent::OpenTask`.
pub fn opened(st: &mut State, m: &Model, cwd: &str, task: Option<&str>) -> Task<Message> {
    if st
        .dialog
        .as_ref()
        .is_some_and(|open| open.cwd == cwd && open.task.as_deref() == task)
    {
        return Task::none();
    }
    let tasks = m.live.board.tasks(cwd).unwrap_or(&[]);
    let dialog = Dialog::open(cwd, task, tasks);
    let thread = dialog.comments > 0;
    st.dialog = Some(dialog);
    if thread {
        iced::widget::operation::snap_to_end(THREAD)
    } else {
        Task::none()
    }
}

/// The dialog is gone: what was typed is saved, and the keyboard goes back to
/// the card that opened it. That card is re-made when an agent moves the task,
/// and a deleted task has none: the column's add field takes it then.
pub fn closed(st: &mut State, m: &Model) -> Task<Message> {
    let Some(mut dialog) = st.dialog.take() else {
        return Task::none();
    };
    let Some(id) = dialog.task.clone() else {
        return Task::none();
    };
    let edits = dialog.pending();
    let save_edits = save(&dialog.cwd, &id, edits);
    let focus = match task_named(m, &dialog.cwd, &id) {
        Some(task) => iced::widget::operation::focus(card_id(&task.id)),
        None => Task::none(),
    };
    Task::batch([save_edits, focus])
}

pub fn update(st: &mut State, m: &mut Model, msg: Msg) -> Task<Message> {
    let tr = Translator::new(m.locale());
    let Some(dialog) = st.dialog.as_mut() else {
        return Task::none();
    };
    let cwd = dialog.cwd.clone();
    let id = dialog.task.clone();

    // Using anything else is leaving the field that was typed in.
    let flush = if msg.is_typing() {
        Task::none()
    } else {
        match &id {
            Some(id) => save(&cwd, id, dialog.pending()),
            None => Task::none(),
        }
    };

    let act = match msg {
        Msg::Title(text) => {
            dialog.title = text;
            Task::none()
        }
        // Saved with everything else that was typed, above.
        Msg::SaveTitle => Task::none(),
        Msg::Heading(action) => {
            dialog.edit_heading(action);
            Task::none()
        }
        Msg::ColumnMenu(open) => {
            dialog.column_menu = open;
            Task::none()
        }
        Msg::MoreMenu(open) => {
            dialog.more_menu = open;
            Task::none()
        }
        Msg::Description(action) => {
            dialog.description.perform(action);
            Task::none()
        }
        Msg::Owns(action) => {
            dialog.owns.perform(action);
            Task::none()
        }
        Msg::Column(column) => match &id {
            Some(id) => {
                dialog.column_menu = false;
                mutate(&cwd, id, Edit::Column(column), Errors::Swallow)
            }
            None => {
                dialog.column = column;
                Task::none()
            }
        },
        Msg::Priority(change) => match dialog.priority.update(change) {
            Some(value) => match &id {
                Some(id) => mutate(&cwd, id, Edit::Priority(value as u8), Errors::Swallow),
                None => Task::none(),
            },
            None => Task::none(),
        },
        Msg::Release => match &id {
            Some(id) => mutate(&cwd, id, Edit::Release, Errors::Swallow),
            None => Task::none(),
        },
        Msg::StartBlock => {
            dialog.start_block();
            focus_soon(REASON_FIELD)
        }
        Msg::CancelBlock => {
            dialog.blocking = false;
            dialog.error = None;
            Task::none()
        }
        Msg::Reason(text) => {
            dialog.reason = text.chars().take(MAX_BLOCK_REASON).collect();
            Task::none()
        }
        Msg::Recheck(text) => {
            dialog.recheck = text;
            Task::none()
        }
        Msg::HumanOnly(on) => {
            dialog.human_only = on;
            Task::none()
        }
        Msg::ConfirmBlock => {
            if dialog.reason.trim().is_empty() {
                dialog.error = Some(tr.t(Key::BoardBlockReasonRequired).into_owned());
                Task::none()
            } else if let Some(problem) =
                recheck_problem(&dialog.recheck, &hangar_core::board::today())
            {
                dialog.error = Some(tr.t(problem.message_key()).into_owned());
                Task::none()
            } else if let Some(id) = &id {
                dialog.error = None;
                mutate(
                    &cwd,
                    id,
                    Edit::Block {
                        reason: dialog.reason.clone(),
                        recheck: dialog.recheck.clone(),
                        human_only: dialog.human_only,
                    },
                    Errors::ToDialog,
                )
            } else {
                Task::none()
            }
        }
        Msg::Note(text) => {
            dialog.note = text;
            Task::none()
        }
        Msg::Unblock => match &id {
            Some(id) => mutate(
                &cwd,
                id,
                Edit::Unblock {
                    note: dialog.note.clone(),
                },
                Errors::ToDialog,
            ),
            None => Task::none(),
        },
        Msg::Comment(text) => {
            dialog.comment = text;
            Task::none()
        }
        Msg::Send => {
            let text = dialog.comment.trim().to_owned();
            match (&id, text.is_empty()) {
                (Some(id), false) => {
                    dialog.comment.clear();
                    actions::run(
                        &cwd,
                        Mutation::Comment {
                            id: id.clone(),
                            text,
                        },
                        Errors::Swallow,
                        After::Nothing,
                    )
                }
                _ => Task::none(),
            }
        }
        Msg::AskDelete => {
            dialog.more_menu = false;
            dialog.confirm_delete = true;
            Task::none()
        }
        Msg::CancelDelete => {
            dialog.confirm_delete = false;
            Task::none()
        }
        Msg::ConfirmDelete => match &id {
            Some(id) => actions::run(
                &cwd,
                Mutation::Delete { id: id.clone() },
                Errors::Swallow,
                After::CloseAlways,
            ),
            None => Task::none(),
        },
        Msg::Create => {
            let draft = dialog.draft();
            if draft.title.is_empty() {
                Task::none()
            } else {
                dialog.error = None;
                actions::run(
                    &cwd,
                    Mutation::Create(draft),
                    Errors::ToDialog,
                    After::CloseOnOk,
                )
            }
        }
        Msg::AddDependency(choice) => match &id {
            Some(id) => {
                let mut ids = task_named(m, &cwd, id)
                    .map(|task| task.depends_on.clone())
                    .unwrap_or_default();
                if !ids.contains(&choice.id) {
                    ids.push(choice.id);
                }
                mutate(&cwd, id, Edit::DependsOn(ids), Errors::Swallow)
            }
            None => {
                if !dialog.depends_on.contains(&choice.id) {
                    dialog.depends_on.push(choice.id);
                }
                Task::none()
            }
        },
        Msg::RemoveDependency(dep) => match &id {
            Some(id) => {
                let mut ids = task_named(m, &cwd, id)
                    .map(|task| task.depends_on.clone())
                    .unwrap_or_default();
                ids.retain(|candidate| candidate != &dep);
                mutate(&cwd, id, Edit::DependsOn(ids), Errors::Swallow)
            }
            None => {
                dialog.depends_on.retain(|candidate| candidate != &dep);
                Task::none()
            }
        },
        Msg::Close => Task::done(Message::Intent(Intent::CloseOverlay)),
    };
    Task::batch([flush, act])
}

fn mutate(cwd: &str, id: &str, edit: Edit, errors: Errors) -> Task<Message> {
    actions::run(
        cwd,
        Mutation::Update {
            id: id.to_owned(),
            edit,
        },
        errors,
        After::Nothing,
    )
}

/// The focus to a field that is on screen after this message's view, which
/// the operation would otherwise run before.
pub fn focus_soon(id: &'static str) -> Task<Message> {
    Task::perform(
        async { tokio::time::sleep(std::time::Duration::from_millis(30)).await },
        move |()| Message::Board(BoardMsg::FocusField(id)),
    )
}

/// A change of the board answered: clear the forms it settled, say why it was
/// refused, and close the dialog when it was asked to.
pub fn mutated(
    st: &mut State,
    errors: Errors,
    after: After,
    result: &Result<(), String>,
) -> Task<Message> {
    let mut close = false;
    if let Some(dialog) = st.dialog.as_mut() {
        match result {
            Ok(()) => {
                if errors == Errors::ToDialog {
                    dialog.finish_block();
                }
                close = after != After::Nothing;
            }
            Err(error) => {
                if errors == Errors::ToDialog {
                    dialog.error = Some(error.clone());
                }
                close = after == After::CloseAlways;
            }
        }
    }
    if close {
        Task::done(Message::Intent(Intent::CloseOverlay))
    } else {
        Task::none()
    }
}

// ---- The view --------------------------------------------------------------------------------

fn label<'a>(t: &'a Theme, words: impl Into<String>, hint: Option<String>) -> Element<'a, Message> {
    let mut line = row![
        text(words.into())
            .size(font_size::CAPTION)
            .font(fonts::ui_semibold())
            .style(style::text::muted(t))
    ]
    .spacing(space::MD)
    .align_y(Alignment::Center);
    if let Some(hint) = hint {
        line = line.push(
            text(hint)
                .size(font_size::CAPTION)
                .style(style::text::subtle(t)),
        );
    }
    line.into()
}

fn field<'a>(t: &'a Theme, placeholder: &str, value: &str) -> text_input::TextInput<'a, Message> {
    text_input(placeholder, value)
        .size(font_size::BODY)
        .padding([6.0, space::LG])
        .style(style::text_input::field(t))
}

fn editor<'a>(
    t: &'a Theme,
    content: &'a text_editor::Content,
    min_height: f32,
    on_action: impl Fn(text_editor::Action) -> Message + 'a,
) -> Element<'a, Message> {
    text_editor(content)
        .on_action(on_action)
        .size(font_size::BODY)
        .padding([6.0, space::LG])
        .min_height(min_height)
        .style(style::text_editor::field(t))
        .into()
}

/// A field with the keymap asked first (`keymap::guard`): a shortcut it takes
/// is spent before the field sees it, so Ctrl+Shift+Enter in the title splits
/// a pane behind the dialog and does not also save the task.
fn guarded<'a>(v: &View<'a>, field: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    crate::keymap::guard(v.ctx, field)
}

fn stack_of<'a>(parts: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    column(parts).spacing(space::SM).into()
}

fn msg(inner: Msg) -> Message {
    Message::Board(BoardMsg::Dialog(inner))
}

/// `Overlay::Task`: a task's sheet, or the dialog of a new one, over the
/// window.
pub fn view<'a>(
    st: &'a State,
    ctx: crate::ctx::Ctx<'a>,
    cwd: &'a str,
    task: Option<&'a str>,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let page = Space::new().width(Length::Fill).height(Length::Fill);
    let Some(dialog) = st
        .dialog
        .as_ref()
        .filter(|open| open.cwd == cwd && open.task.as_deref() == task)
    else {
        return page.into();
    };
    let v = View::new(st, ctx, cwd);
    let found = task.and_then(|id| v.all.iter().find(|candidate| candidate.id == id));
    match (task, found) {
        (Some(_), Some(found)) => {
            // No veil: the board stays as it is behind the sheet, and a press
            // anywhere else closes it.
            layer::modal(page, Some(placed(ctx, sheet(&v, dialog, found))))
                .margin(0.0)
                .on_dismiss(msg(Msg::Close))
                .into()
        }
        // The task is gone (an agent deleted it): the board closes the dialog
        // with the next read; until then there is nothing to show.
        (Some(_), None) => page.into(),
        (None, _) => dialog::veiled(t, page, Some(create_form(&v, dialog)))
            .on_dismiss(msg(Msg::Close))
            .into(),
    }
}

/// Where the sheet goes in the window: down its right side, under the top bar
/// (and the board's own bar when the board is what it covers) and over the
/// status bar. In a window whose content is under [`look::SHEET_FULL_BELOW`]
/// it takes the whole content.
fn placed<'a>(ctx: crate::ctx::Ctx<'a>, sheet: Element<'a, Message>) -> Element<'a, Message> {
    // A press on what the sheet does not cover closes it.
    fn outside<'a>(width: Length, height: Length) -> Element<'a, Message> {
        mouse_area(Space::new().width(width).height(height))
            .on_press(msg(Msg::Close))
            .into()
    }
    let board = ctx.view() == MainView::Board;
    let top = size::TOOLBAR_HEIGHT + BORDER_WIDTH + if board { tab_toolbar::HEIGHT } else { 0.0 };
    let bottom = crate::status_bar::height(ctx) + BORDER_WIDTH;
    let content = ctx
        .model
        .ui
        .body_width
        .get(&ctx.window)
        .copied()
        .unwrap_or(ctx.metrics.body_width);
    let full = content < look::SHEET_FULL_BELOW;
    let beside: Element<'a, Message> = if !full {
        outside(Length::Fill, Length::Fill)
    } else if ctx.panels().navigator {
        outside(
            Length::Fixed(scale::size::SIDEBAR + BORDER_WIDTH),
            Length::Fill,
        )
    } else {
        Space::new().into()
    };
    let panel = container(opaque(sheet))
        .width(if full {
            Length::Fill
        } else {
            Length::Fixed(look::SHEET_WIDTH)
        })
        .height(Length::Fill);
    column![
        outside(Length::Fill, Length::Fixed(top)),
        row![beside, panel].height(Length::Fill),
        outside(Length::Fill, Length::Fixed(bottom)),
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn shell<'a>(
    t: &'a Theme,
    head: Element<'a, Message>,
    body: Element<'a, Message>,
    foot: Element<'a, Message>,
) -> Element<'a, Message> {
    let line = || rule::horizontal(1).style(style::rule::hairline(t));
    container(column![
        head,
        line(),
        container(
            scrollable(container(body).padding(Padding {
                top: space::XXL,
                right: space::XXL,
                bottom: 18.0,
                left: space::XXL,
            }))
            .style(style::scrollable::default(t)),
        )
        .max_height(look::DIALOG_BODY_MAX),
        line(),
        foot,
    ])
    .width(Length::Fill)
    .max_width(look::DIALOG_WIDTH)
    .style(style::container::modal(t))
    .into()
}

fn head<'a>(
    v: &View<'a>,
    column: Option<Column>,
    title: String,
    id: Option<&str>,
) -> Element<'a, Message> {
    let t = v.t();
    let tr = v.ctx.i18n;
    let mut line = row![].spacing(9.0).align_y(Alignment::Center);
    if let Some(column) = column {
        line = line.push(hangar_ui::glyph::state(
            t,
            look::mark(column),
            look::MARK_LARGE,
        ));
    }
    let mut heading = row![text(title).size(font_size::BODY).font(fonts::ui_semibold())]
        .spacing(space::MD)
        .align_y(Alignment::Center);
    if let Some(id) = id {
        heading = heading.push(tip::over(
            t,
            text(format!("#{}", short_task_id(id)))
                .size(font_size::CAPTION)
                .font(fonts::mono())
                .style(style::text::subtle(t)),
            id.to_owned(),
        ));
    }
    line = line.push(heading);
    if let Some(column) = column {
        line = line.push(
            text(format!("· {}", tr.t(column.label_key())))
                .size(font_size::UI)
                .style(style::text::subtle(t)),
        );
    }
    line = line.push(Space::new().width(Length::Fill)).push(
        button::icon_button(t, Icon::Close)
            .tip(tr.t(Key::TaskClose))
            .on_press(msg(Msg::Close)),
    );
    container(line)
        .padding(Padding {
            top: 0.0,
            right: space::MD,
            bottom: 0.0,
            left: space::XXL,
        })
        .center_y(48.0)
        .into()
}

/// A new task's title and description, then its column and priority.
fn fields<'a>(v: &View<'a>, dialog: &'a Dialog) -> Vec<Element<'a, Message>> {
    let t = v.t();
    let tr = v.ctx.i18n;
    let mut parts: Vec<Element<'a, Message>> = Vec::new();

    parts.push(stack_of(vec![
        label(t, tr.t(Key::TaskName), None),
        guarded(
            v,
            field(t, "", &dialog.title)
                .id(TITLE_FIELD)
                .font(fonts::ui_semibold())
                .on_input(|text| msg(Msg::Title(text)))
                .on_submit(msg(Msg::Create)),
        ),
    ]));
    parts.push(stack_of(vec![
        label(
            t,
            tr.t(Key::TaskDescription),
            Some(tr.t(Key::TaskDescriptionHint).into_owned()),
        ),
        description_editor(v, dialog),
    ]));

    let choices = column_choices(tr, false);
    let selected = choices
        .iter()
        .find(|choice| choice.column == dialog.column)
        .cloned();
    let column_field = stack_of(vec![
        label(t, tr.t(Key::TaskColumn), None),
        hangar_ui::input::select(t, choices, selected, |choice| {
            msg(Msg::Column(choice.column))
        }),
    ]);
    let priority_field = stack_of(vec![
        label(t, tr.t(Key::TaskPriority), None),
        priority_stepper(v, dialog),
    ]);
    parts.push(
        row![
            container(column_field).width(Length::FillPortion(10)),
            container(priority_field).width(Length::FillPortion(12)),
        ]
        .spacing(space::XL)
        .into(),
    );
    parts
}

/// The columns a task can be put in. While it is blocked it stays in To do:
/// the others are not offered.
fn column_choices(tr: Translator, blocked: bool) -> Vec<ColumnChoice> {
    Column::ALL
        .into_iter()
        .filter(|candidate| !blocked || *candidate == Column::Todo)
        .map(|candidate| ColumnChoice {
            column: candidate,
            label: tr.t(candidate.label_key()).into_owned(),
        })
        .collect()
}

fn description_editor<'a>(v: &View<'a>, dialog: &'a Dialog) -> Element<'a, Message> {
    guarded(
        v,
        editor(v.t(), &dialog.description, 92.0, |action| {
            msg(Msg::Description(action))
        }),
    )
}

/// The number agents set, and the way it reads on a card.
fn priority_stepper<'a>(v: &View<'a>, dialog: &'a Dialog) -> Element<'a, Message> {
    let value = dialog.priority.value().round() as u8;
    row![
        container(guarded(
            v,
            number_input(v.t(), &dialog.priority, |change| {
                msg(Msg::Priority(change))
            }),
        ))
        .width(120.0),
        super::card::priority(v, value),
    ]
    .spacing(space::LG)
    .align_y(Alignment::Center)
    .into()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ColumnChoice {
    column: Column,
    label: String,
}

impl std::fmt::Display for ColumnChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

/// Files the task's worker may change, one per line.
fn owns_field<'a>(v: &View<'a>, dialog: &'a Dialog) -> Element<'a, Message> {
    let t = v.t();
    let tr = v.ctx.i18n;
    stack_of(vec![
        label(
            t,
            tr.t(Key::TaskOwns),
            Some(tr.t(Key::TaskOwnsHint).into_owned()),
        ),
        owns_editor(v, dialog),
    ])
}

fn owns_editor<'a>(v: &View<'a>, dialog: &'a Dialog) -> Element<'a, Message> {
    guarded(
        v,
        text_editor(&dialog.owns)
            .on_action(|action| msg(Msg::Owns(action)))
            .size(type_scale::CAPTION.size)
            .font(fonts::mono())
            .padding([6.0, space::LG])
            .min_height(56.0)
            .style(style::text_editor::field(v.t())),
    )
}

/// The task's sheet: its head stays, and the rest scrolls under it.
fn sheet<'a>(v: &View<'a>, dialog: &'a Dialog, task: &'a T) -> Element<'a, Message> {
    let t = v.t();
    let tr = v.ctx.i18n;
    let column_of = Column::of(task).unwrap_or(Column::Todo);

    let mut body: Vec<Element<'a, Message>> = vec![heading(v, dialog)];
    if let Some(block) = block_section(v, dialog, task) {
        body.push(block);
    }
    body.push(properties(v, dialog, task, column_of));
    body.push(section(
        t,
        tr.t(Key::TaskDescription),
        None,
        Some(tr.t(Key::TaskDescriptionHint).into_owned()),
        description_editor(v, dialog),
    ));
    if let Some(dependencies) = dependency_section(v, dialog, task) {
        body.push(dependencies);
    }
    body.push(section(
        t,
        tr.t(Key::TaskOwns),
        Some(patterns(&text_of(&dialog.owns)).len()),
        Some(tr.t(Key::TaskOwnsHint).into_owned()),
        owns_editor(v, dialog),
    ));
    body.push(section(
        t,
        tr.t(Key::TaskThread),
        Some(task.comments.len()),
        Some(tr.t(Key::TaskThreadHint).into_owned()),
        thread(v, dialog, task),
    ));

    let rule_in = move |color: Color| {
        move |theme: &iced::Theme| rule::Style {
            color,
            ..style::rule::hairline(t)(theme)
        }
    };
    let mut page = column![
        sheet_head(v, dialog, task, column_of),
        rule::horizontal(BORDER_WIDTH).style(rule_in(t.palette.border)),
    ];
    if dialog.confirm_delete {
        page = page
            .push(confirm_strip(v))
            .push(rule::horizontal(BORDER_WIDTH).style(rule_in(t.palette.border)));
    }
    let page = page.push(
        scrollable(
            container(column(body).spacing(18.0))
                .padding([18.0, 20.0])
                .width(Length::Fill),
        )
        .height(Length::Fill)
        .style(style::scrollable::default(t)),
    );
    container(row![
        rule::vertical(BORDER_WIDTH).style(rule_in(t.palette.border_strong)),
        page.width(Length::Fill),
    ])
    .width(Length::Fill)
    .height(Length::Fill)
    .style(look::sheet(t))
    .into()
}

/// The head: the task's id, the badge of its column (which moves it), the way
/// to block it or to lift its block, "…" and close.
fn sheet_head<'a>(
    v: &View<'a>,
    dialog: &'a Dialog,
    task: &'a T,
    column_of: Column,
) -> Element<'a, Message> {
    let t = v.t();
    let tr = v.ctx.i18n;
    let id = text(format!("#{}", short_task_id(&task.id)))
        .size(type_scale::CAPTION.size)
        .font(fonts::mono())
        .color(t.palette.text_muted);

    let open = msg(Msg::ColumnMenu(!dialog.column_menu));
    let badge = iced::widget::button(look::centred(
        row![
            hangar_ui::glyph::state(t, look::mark(column_of), 10.0),
            text(tr.t(column_of.label_key()))
                .size(type_scale::CAPTION.size)
                .wrapping(text::Wrapping::None),
            icon(Icon::ChevronDown, 9.0),
        ]
        .spacing(space::SM)
        .align_y(Alignment::Center),
    ))
    .height(22.0)
    .padding([0.0, space::MD])
    .style(look::state_pill(t))
    .on_press(open.clone());
    let badge = focus::ring(t, badge).radius(radius::FULL).on_activate(open);
    let columns = dialog.column_menu.then(|| {
        column_choices(tr, task.blocked.is_some())
            .into_iter()
            .map(|choice| {
                Entry::from(
                    menu::item(choice.label)
                        .checked(choice.column == column_of)
                        .on_select(msg(Msg::Column(choice.column))),
                )
            })
            .collect::<Vec<_>>()
    });
    let badge = menu::dropdown(t, tip::bottom(t, badge, tr.t(Key::TaskColumn)), columns)
        .on_dismiss(msg(Msg::ColumnMenu(false)));

    let mut line = row![
        tip::bottom(t, id, task.id.clone()),
        Element::from(badge),
        Space::new().width(Length::Fill),
    ]
    .spacing(space::LG)
    .align_y(Alignment::Center);

    if task.blocked.is_some() {
        line = line.push(
            button::secondary(t, tr.t(Key::BoardUnblock))
                .icon(Icon::Unlock)
                .on_press(msg(Msg::Unblock)),
        );
    } else if !dialog.blocking && matches!(column_of, Column::Todo | Column::Doing) {
        line = line.push(
            button::secondary(t, tr.t(Key::BoardBlock))
                .icon(Icon::Lock)
                .on_press(msg(Msg::StartBlock)),
        );
    }

    let more = button::icon_button(t, Icon::More)
        .sized(button::Side::Sm)
        .tip(tr.t(Key::SidebarMore))
        .on_press(msg(Msg::MoreMenu(!dialog.more_menu)));
    let entries = dialog.more_menu.then(|| {
        vec![Entry::from(
            menu::item(tr.t(Key::TaskDelete).into_owned())
                .icon(Icon::Trash)
                .danger()
                .on_select(msg(Msg::AskDelete)),
        )]
    });
    let more = menu::dropdown(t, more, entries)
        .align_end()
        .on_dismiss(msg(Msg::MoreMenu(false)));
    let line = line.push(Element::from(more)).push(
        button::icon_button(t, Icon::Close)
            .sized(button::Side::Sm)
            .tip(tr.t(Key::TaskClose))
            .on_press(msg(Msg::Close)),
    );
    container(line)
        .padding(Padding {
            top: 0.0,
            right: space::MD,
            bottom: 0.0,
            left: 20.0,
        })
        .center_y(look::SHEET_HEAD)
        .into()
}

/// "Delete" was picked in "…": it asks once more, under the head.
fn confirm_strip<'a>(v: &View<'a>) -> Element<'a, Message> {
    let t = v.t();
    let tr = v.ctx.i18n;
    container(
        row![
            text(tr.t(Key::TaskDelete))
                .size(type_scale::SMALL.size)
                .color(t.palette.status.failed),
            Space::new().width(Length::Fill),
            button::ghost(t, tr.t(Key::CreateCancel)).on_press(msg(Msg::CancelDelete)),
            button::danger(t, tr.t(Key::TaskDeleteConfirm)).on_press(msg(Msg::ConfirmDelete)),
        ]
        .spacing(space::MD)
        .align_y(Alignment::Center),
    )
    .padding([space::MD, 20.0])
    .width(Length::Fill)
    .into()
}

/// The title, in the display face. Enter saves it.
fn heading<'a>(v: &View<'a>, dialog: &'a Dialog) -> Element<'a, Message> {
    let editor = text_editor(&dialog.heading)
        .id(TITLE_FIELD)
        .on_action(|action| msg(Msg::Heading(action)))
        .key_binding(|press| match press.key {
            Key_::Named(Named::Enter) => Some(Binding::Custom(msg(Msg::SaveTitle))),
            _ => Binding::from_key_press(press),
        })
        .font(fonts::display())
        .size(19.0)
        .line_height(text::LineHeight::Absolute(Pixels(25.0)))
        .wrapping(text::Wrapping::WordOrGlyph)
        .padding([2.0, space::XS])
        .style(look::heading_field(v.t()));
    guarded(v, editor)
}

/// A section of the sheet: its name, how many it holds, a word about it, and
/// what it holds.
fn section<'a>(
    t: &'a Theme,
    title: impl Into<String>,
    count: Option<usize>,
    hint: Option<String>,
    body: Element<'a, Message>,
) -> Element<'a, Message> {
    let mut head = row![
        text(title.into())
            .size(type_scale::CAPTION.size)
            .font(fonts::ui_semibold())
            .color(t.palette.text_muted)
            .wrapping(text::Wrapping::None),
    ]
    .spacing(space::MD)
    .align_y(Alignment::Center);
    if let Some(count) = count {
        head = head.push(
            text(count.to_string())
                .size(type_scale::CAPTION.size)
                .font(fonts::mono())
                .color(t.palette.text_muted),
        );
    }
    if let Some(hint) = hint {
        head = head.push(
            text(hint)
                .size(type_scale::LABEL.size)
                .color(t.palette.text_muted)
                .width(Length::Fill),
        );
    }
    column![head, body].spacing(space::MD).into()
}

/// The grid of what the task is: who holds it, since when, how urgent it is,
/// its labels, and when it was written.
fn properties<'a>(
    v: &View<'a>,
    dialog: &'a Dialog,
    task: &'a T,
    column_of: Column,
) -> Element<'a, Message> {
    let t = v.t();
    let tr = v.ctx.i18n;
    let line = |name: Key, value: Element<'a, Message>| -> Element<'a, Message> {
        row![
            text(tr.t(name))
                .size(type_scale::SMALL.size)
                .color(t.palette.text_muted)
                .width(look::PROPERTY_LABEL),
            container(value).width(Length::Fill),
        ]
        .align_y(Alignment::Center)
        .into()
    };

    let who: Element<'a, Message> = match &task.assignee {
        Some(assignee) => {
            let mut who = row![assignee_view(
                v,
                assignee,
                column_of == Column::Done,
                true,
                true
            )]
            .spacing(space::MD)
            .align_y(Alignment::Center);
            if let Some(pane) = v.pane(assignee) {
                who = who.push(button::link(
                    t,
                    tr.t(Key::TaskOpenPane),
                    Message::Board(BoardMsg::FocusAgent {
                        ws: pane.workspace.clone(),
                        pane: pane.id.clone(),
                        from_dialog: true,
                    }),
                ));
            }
            who.push(
                button::ghost(t, tr.t(Key::TaskRelease))
                    .small()
                    .on_press(msg(Msg::Release)),
            )
            .wrap()
            .into()
        }
        None => text(tr.t(Key::TaskFree))
            .size(type_scale::SMALL.size)
            .color(t.palette.text_muted)
            .into(),
    };

    let mut lines = vec![line(Key::TaskAssignee, who)];
    if let (Column::Doing, Some(since)) = (column_of, task.doing_since) {
        let started = match time_today(since, v.now as u64, tr.locale()) {
            Some(time) => tr
                .t_with(Key::TaskToday, &args!("time" => time.as_str()))
                .into_owned(),
            None => when(since, tr.locale(), DateStyle::Short),
        };
        lines.push(line(
            Key::TaskDoingSince,
            row![
                text(started).size(type_scale::SMALL.size),
                text(format!("· {}", elapsed_label(v.now - since as i64, tr)))
                    .size(type_scale::SMALL.size)
                    .color(t.palette.text_muted),
            ]
            .spacing(space::MD)
            .into(),
        ));
    }
    lines.push(line(Key::TaskPriority, priority_stepper(v, dialog)));
    if !task.labels.is_empty() {
        lines.push(line(
            Key::TaskLabels,
            row(task.labels.iter().map(|label| chip(t, label)))
                .spacing(space::SM)
                .wrap()
                .into(),
        ));
    }
    lines.push(
        text(
            tr.t_with(
                Key::TaskCreatedAt,
                &args!("date" => when(task.created_at, tr.locale(), DateStyle::Short).as_str()),
            )
            .into_owned(),
        )
        .size(type_scale::LABEL.size)
        .color(t.palette.text_muted)
        .into(),
    );
    column(lines.into_iter().map(|line| {
        container(line)
            .center_y(Length::Shrink)
            .padding([2.0, 0.0])
            .into()
    }))
    .spacing(space::XS)
    .into()
}

/// What the task waits on, which can be changed, and what waits on it.
fn dependency_section<'a>(
    v: &View<'a>,
    dialog: &'a Dialog,
    task: &'a T,
) -> Option<Element<'a, Message>> {
    let t = v.t();
    let tr = v.ctx.i18n;
    let blocked: Vec<&'a T> = v
        .all
        .iter()
        .filter(|other| other.depends_on.contains(&task.id))
        .collect();
    let waits = task
        .depends_on
        .iter()
        .filter(|id| v.by_id.contains_key(id.as_str()))
        .count();
    if blocked.is_empty() && waits == 0 && v.all.len() < 2 {
        return None;
    }
    let mut parts = column![].spacing(space::MD);
    if waits > 0 || v.all.len() > 1 {
        parts = parts.push(dependencies(v, dialog, Some(task)));
    }
    let count = waits + blocked.len();
    if !blocked.is_empty() {
        let rows = blocked.into_iter().map(|other| {
            let column_of = Column::of(other).unwrap_or(Column::Todo);
            row![
                hangar_ui::glyph::state(t, look::mark(column_of), 10.0).still(),
                text(format!("#{}", short_task_id(&other.id)))
                    .size(type_scale::CAPTION.size)
                    .font(fonts::mono())
                    .color(t.palette.text_muted),
                Element::from(
                    tail_text(other.title.as_str())
                        .cut_end()
                        .readable()
                        .size(type_scale::SMALL.size)
                        .color(t.palette.text_secondary)
                ),
            ]
            .spacing(space::MD)
            .align_y(Alignment::Center)
            .into()
        });
        parts = parts.push(
            column![
                label(t, tr.t(Key::TaskBlocks), None),
                column(rows).spacing(space::SM)
            ]
            .spacing(space::SM),
        );
    }
    Some(section(
        t,
        tr.t(Key::TaskDependenciesSection),
        Some(count),
        None,
        parts.into(),
    ))
}

fn stack_column<'a>(parts: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    column(parts).spacing(14.0).into()
}

fn create_form<'a>(v: &View<'a>, dialog: &'a Dialog) -> Element<'a, Message> {
    let t = v.t();
    let tr = v.ctx.i18n;
    let mut body = fields(v, dialog);
    body.push(owns_field(v, dialog));
    body.push(dependencies(v, dialog, None));
    if let Some(error) = &dialog.error {
        body.push(
            text(error.as_str())
                .size(font_size::UI)
                .style(style::text::danger(t))
                .into(),
        );
    }
    let can_create = !dialog.title.trim().is_empty();
    let foot = row![
        Space::new().width(Length::Fill),
        button::ghost(t, tr.t(Key::CreateCancel)).on_press(msg(Msg::Close)),
        button::primary(t, tr.t(Key::TaskCreate))
            .on_press_maybe(can_create.then(|| msg(Msg::Create))),
    ]
    .spacing(space::MD)
    .align_y(Alignment::Center);
    shell(
        t,
        head(
            v,
            Some(dialog.column),
            tr.t(Key::TaskNew).into_owned(),
            None,
        ),
        stack_column(body),
        dialog::foot(t, foot),
    )
}

/// Why a task is on hold, with a note to lift it with; or the form that puts
/// it on hold. Errors the board gives are shown here and nowhere else.
/// `None` when there is nothing to say: the sheet's head has the buttons.
fn block_section<'a>(
    v: &View<'a>,
    dialog: &'a Dialog,
    task: &'a T,
) -> Option<Element<'a, Message>> {
    let t = v.t();
    let tr = v.ctx.i18n;
    let mut parts: Vec<Element<'a, Message>> = Vec::new();

    if let Some(blocked) = &task.blocked {
        parts.push(
            row![
                hangar_ui::glyph::state(t, GlyphState::Failed, 12.0),
                text(
                    tr.t_with(
                        Key::BoardBlockedFor,
                        &args!("reason" => blocked.reason.as_str())
                    )
                    .into_owned()
                )
                .size(type_scale::SMALL.size)
                .color(t.palette.status.failed)
                .width(Length::Fill),
            ]
            .spacing(space::SM)
            .into(),
        );
        let mut since = tr
            .t_with(
                Key::BoardBlockedSince,
                &args!("by" => blocked.by.as_str(),
                       "date" => when(blocked.since, tr.locale(), DateStyle::Short).as_str()),
            )
            .into_owned();
        if let Some(day) = &blocked.recheck {
            since.push_str(&format!(
                " · {}",
                tr.t_with(Key::BoardBlockedRecheck, &args!("date" => day.as_str()))
            ));
        }
        if blocked.human_only {
            since.push_str(&format!(" · {}", tr.t(Key::BoardBlockedPerson)));
        }
        parts.push(
            text(since)
                .size(type_scale::CAPTION.size)
                .color(t.palette.text_muted)
                .into(),
        );
        parts.push(guarded(
            v,
            field(t, &tr.t(Key::BoardUnblockNote), &dialog.note)
                .on_input(|text| msg(Msg::Note(text)))
                .on_submit(msg(Msg::Unblock)),
        ));
    } else if dialog.blocking {
        parts.push(label(
            t,
            tr.t(Key::BoardBlockReason),
            Some(tr.t(Key::BoardBlockHint).into_owned()),
        ));
        parts.push(guarded(
            v,
            field(t, "", &dialog.reason)
                .id(REASON_FIELD)
                .on_input(|text| msg(Msg::Reason(text)))
                .on_submit(msg(Msg::ConfirmBlock)),
        ));
        parts.push(label(t, tr.t(Key::BoardBlockRecheck), None));
        parts.push(guarded(
            v,
            field(t, "YYYY-MM-DD", &dialog.recheck)
                .on_input(|text| msg(Msg::Recheck(text)))
                .on_submit(msg(Msg::ConfirmBlock)),
        ));
        parts.push(
            checkbox(dialog.human_only)
                .label(tr.t(Key::BoardBlockHumanOnly))
                .on_toggle(|on| msg(Msg::HumanOnly(on)))
                .size(16)
                .text_size(font_size::BODY)
                .style(style::checkbox::default(t))
                .into(),
        );
        parts.push(
            row![
                button::ghost(t, tr.t(Key::CreateCancel)).on_press(msg(Msg::CancelBlock)),
                button::secondary(t, tr.t(Key::BoardBlockConfirm)).on_press(msg(Msg::ConfirmBlock)),
            ]
            .spacing(space::MD)
            .into(),
        );
    }
    if let Some(error) = &dialog.error {
        parts.push(
            text(error.as_str())
                .size(type_scale::SMALL.size)
                .color(t.palette.status.failed)
                .into(),
        );
    }
    if parts.is_empty() {
        return None;
    }
    Some(
        container(column(parts).spacing(space::MD))
            .padding([space::LG, space::XL])
            .width(Length::Fill)
            .style(look::blocked_inset(t))
            .into(),
    )
}

/// The tasks this one waits on, with the way to add and to drop one. A
/// dependency that no longer exists is not listed (and so is not droppable
/// here): the card still says the task is blocked by "a deleted task".
fn dependencies<'a>(v: &View<'a>, dialog: &'a Dialog, task: Option<&'a T>) -> Element<'a, Message> {
    let t = v.t();
    let tr = v.ctx.i18n;
    let ids: &[String] = match task {
        Some(task) => &task.depends_on,
        None => &dialog.depends_on,
    };
    let own = task.map(|task| task.id.as_str());
    let by_id = index_by_id(v.all);

    let rows = ids
        .iter()
        .filter_map(|dep| by_id.get(dep.as_str()).copied())
        .map(|dep| {
            let done = dep.column == "done";
            let mark = if done {
                icon(Icon::Check, 11.0).color(t.inks.success())
            } else {
                icon(Icon::Lock, 11.0).color(t.inks.danger())
            };
            let title = text(dep.title.as_str())
                .size(font_size::UI)
                .wrapping(text::Wrapping::None)
                .style(look::ink(if done {
                    t.inks.text.muted
                } else {
                    t.inks.text.primary
                }))
                .width(Length::Fill);
            let column_name = Column::of(dep)
                .map(|column| tr.t(column.label_key()).into_owned())
                .unwrap_or_default();
            let id = dep.id.clone();
            container(
                row![
                    mark,
                    text(format!("#{}", short_task_id(&dep.id)))
                        .size(font_size::LABEL)
                        .font(fonts::mono())
                        .style(style::text::subtle(t)),
                    title,
                    text(column_name)
                        .size(font_size::CAPTION)
                        .style(style::text::subtle(t)),
                    button::icon_button(t, Icon::Close)
                        .small()
                        .icon_size(10.0)
                        .tip(tr.t(Key::TaskRemoveDependency))
                        .on_press(msg(Msg::RemoveDependency(id))),
                ]
                .spacing(space::MD)
                .align_y(Alignment::Center),
            )
            .padding([0.0, space::XL])
            .center_y(look::FIELD_HEIGHT)
            .into()
        });

    // Tasks that could be added: not this one, not already listed.
    let candidates: Vec<Dependency> = v
        .all
        .iter()
        .filter(|other| Some(other.id.as_str()) != own && !ids.contains(&other.id))
        .map(|other| Dependency {
            id: other.id.clone(),
            label: format!("#{} · {}", short_task_id(&other.id), other.title),
        })
        .collect();

    let mut parts: Vec<Element<'a, Message>> = Vec::new();
    let listed: Vec<Element<'a, Message>> = rows.collect();
    if !listed.is_empty() || !candidates.is_empty() {
        parts.push(label(t, tr.t(Key::TaskDependencies), None));
    }
    if !listed.is_empty() {
        let mut list = column![];
        for (n, listed_row) in listed.into_iter().enumerate() {
            if n > 0 {
                list = list.push(rule::horizontal(1).style(style::rule::hairline(t)));
            }
            list = list.push(listed_row);
        }
        parts.push(container(list).style(look::deps(t)).into());
    }
    if !candidates.is_empty() {
        parts.push(
            pick_list(candidates, None::<Dependency>, |choice| {
                msg(Msg::AddDependency(choice))
            })
            .placeholder(tr.t(Key::TaskAddDependency).into_owned())
            .text_size(font_size::UI)
            .padding([5.0, space::LG])
            .width(Length::Fill)
            .style(style::pick_list::field(t))
            .menu_style(style::pick_list::menu(t))
            .into(),
        );
    }
    column(parts).spacing(space::SM).into()
}

/// The messages between the people and agents working on the task, the
/// newest at the bottom, and the way to add one.
fn thread<'a>(v: &View<'a>, dialog: &'a Dialog, task: &'a T) -> Element<'a, Message> {
    let t = v.t();
    let tr = v.ctx.i18n;

    let messages: Element<'a, Message> = if task.comments.is_empty() {
        text(tr.t(Key::TaskNoComments))
            .size(type_scale::SMALL.size)
            .color(t.palette.text_muted)
            .into()
    } else {
        let list = column(task.comments.iter().map(|entry| {
            let at = time_today(entry.created_at, v.now as u64, tr.locale())
                .unwrap_or_else(|| when(entry.created_at, tr.locale(), DateStyle::Short));
            container(
                column![
                    row![
                        text(entry.author.as_str())
                            .size(type_scale::CAPTION.size)
                            .font(fonts::ui_semibold()),
                        text(at)
                            .size(type_scale::LABEL.size)
                            .font(fonts::mono())
                            .color(t.palette.text_muted),
                    ]
                    .spacing(space::MD)
                    .align_y(Alignment::Center),
                    text(entry.text.as_str())
                        .size(type_scale::SMALL.size)
                        .color(t.palette.text_secondary)
                        .wrapping(text::Wrapping::WordOrGlyph),
                ]
                .spacing(space::XS),
            )
            .padding([space::LG, space::XL])
            .width(Length::Fill)
            .style(look::comment(t))
            .into()
        }))
        .spacing(space::MD);
        container(
            scrollable(list.width(Length::Fill))
                .id(THREAD)
                .style(style::scrollable::default(t)),
        )
        .max_height(look::THREAD_MAX_HEIGHT)
        .into()
    };

    let compose = row![
        guarded(
            v,
            field(t, &tr.t(Key::TaskWriteComment), &dialog.comment)
                .on_input(|text| msg(Msg::Comment(text)))
                .on_submit(msg(Msg::Send)),
        ),
        button::secondary(t, tr.t(Key::TaskSend))
            .icon(Icon::Send)
            .size(button::Size::Large)
            .on_press_maybe((!dialog.comment.trim().is_empty()).then(|| msg(Msg::Send))),
    ]
    .spacing(space::MD)
    .align_y(Alignment::Center);

    column![messages, compose].spacing(space::MD).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::model::test_task;

    #[test]
    fn splits_the_files_a_task_owns_one_per_line() {
        assert_eq!(
            patterns("  src/**\n\n docs/ \r\nREADME.md"),
            vec!["src/**", "docs/", "README.md"]
        );
        assert!(patterns(" \n ").is_empty());
    }

    #[test]
    fn opens_on_the_task_as_the_board_has_it() {
        let mut task = test_task("a");
        task.title = "Fix it".into();
        task.description = "Why".into();
        task.owns = vec!["src/".into(), "docs/".into()];
        task.priority = 3;
        let dialog = Dialog::open("/repo", Some("a"), &[task]);
        assert_eq!(dialog.title, "Fix it");
        assert_eq!(dialog.priority.value(), 3.0);
        assert_eq!(dialog.seen.owns, "src/\ndocs/");
        assert!(!dialog.is_new());
        assert!(Dialog::open("/repo", None, &[]).is_new());
    }

    #[test]
    fn saves_only_what_changed_and_once() {
        let mut task = test_task("a");
        task.title = "Old".into();
        let mut dialog = Dialog::open("/repo", Some("a"), &[task]);
        assert!(dialog.pending().is_empty());

        dialog.title = "New".into();
        dialog
            .owns
            .perform(text_editor::Action::Edit(text_editor::Edit::Paste(
                std::sync::Arc::new("src/**".into()),
            )));
        let edits = dialog.pending();
        assert_eq!(
            edits,
            vec![Edit::Title("New".into()), Edit::Owns(vec!["src/**".into()])]
        );
        // Saved: nothing left to save.
        assert!(dialog.pending().is_empty());
    }

    #[test]
    fn a_change_that_is_not_ours_replaces_what_is_typed() {
        let task = test_task("a");
        let mut dialog = Dialog::open("/repo", Some("a"), std::slice::from_ref(&task));
        dialog.title = "half typed".into();

        let mut renamed = task.clone();
        renamed.title = "Renamed by an agent".into();
        dialog.follow(&renamed);
        assert_eq!(dialog.title, "Renamed by an agent");

        // Nothing changed on the board: what is typed stays.
        dialog.title = "typing again".into();
        dialog.follow(&renamed);
        assert_eq!(dialog.title, "typing again");
    }

    #[test]
    fn follows_the_thread_as_it_grows() {
        let task = test_task("a");
        let mut dialog = Dialog::open("/repo", Some("a"), std::slice::from_ref(&task));
        assert!(!dialog.follow(&task));
        let mut talked = task;
        talked.comments.push(hangar_core::board::Comment {
            id: "c".into(),
            author: "Ava".into(),
            text: "hi".into(),
            created_at: 1,
        });
        assert!(dialog.follow(&talked));
        assert!(!dialog.follow(&talked));
    }

    #[test]
    fn a_new_task_is_drafted_from_the_form() {
        let mut dialog = Dialog::open("/repo", None, &[]);
        dialog.title = "  Ship  ".into();
        dialog.column = Column::Doing;
        dialog.depends_on = vec!["x".into()];
        let draft = dialog.draft();
        assert_eq!(draft.title, "Ship");
        assert_eq!(draft.column, Column::Doing);
        assert_eq!(draft.priority, 1);
        assert_eq!(draft.depends_on, vec!["x".to_owned()]);
        // Nothing is saved against a task that does not exist yet.
        assert!(dialog.pending().is_empty());
    }

    #[test]
    fn a_block_settles_the_forms_and_a_refusal_stays_on_screen() {
        let mut st = State::default();
        let mut dialog = Dialog::open("/repo", Some("a"), &[test_task("a")]);
        dialog.blocking = true;
        dialog.reason = "release".into();
        st.dialog = Some(dialog);

        let refused: Result<(), String> = Err("it is a refusal".into());
        let _ = mutated(&mut st, Errors::ToDialog, After::Nothing, &refused);
        let open = st.dialog.as_ref().expect("still open");
        assert_eq!(open.error.as_deref(), Some("it is a refusal"));
        assert!(open.blocking);

        let _ = mutated(&mut st, Errors::ToDialog, After::Nothing, &Ok(()));
        let open = st.dialog.as_ref().expect("still open");
        assert!(open.error.is_none() && !open.blocking && open.reason.is_empty());
    }

    // ---- What the buttons do -----------------------------------------------

    use crate::board::snapshot::{CWD, model, tasks};
    use hangar_i18n::Locale;

    const DOING: &str = "3d4e0000-aaaa-bbbb-cccc-dddddddddddd";

    /// A dialog open on a task of the sample board, and the model it is on.
    fn open_on(id: Option<&str>) -> (State, Model) {
        let m = model(Locale::En, "graphite", Some(tasks()));
        let st = State {
            dialog: Some(Dialog::open(
                CWD,
                id,
                m.live.board.tasks(CWD).unwrap_or(&[]),
            )),
            ..State::default()
        };
        (st, m)
    }

    fn send(st: &mut State, m: &mut Model, msg: Msg) -> usize {
        update(st, m, msg).units()
    }

    fn dialog(st: &State) -> &Dialog {
        st.dialog.as_ref().expect("an open dialog")
    }

    #[test]
    fn a_block_needs_a_reason_and_a_day_that_is_not_past() {
        let (mut st, mut m) = open_on(Some(DOING));
        send(&mut st, &mut m, Msg::StartBlock);
        assert!(dialog(&st).blocking);

        // Nothing said: the form says what is missing, and writes nothing.
        send(&mut st, &mut m, Msg::Reason("   ".into()));
        assert_eq!(send(&mut st, &mut m, Msg::ConfirmBlock), 0);
        assert_eq!(dialog(&st).error.as_deref(), Some("Say what it waits for."));

        send(&mut st, &mut m, Msg::Reason("the npm release".into()));
        send(&mut st, &mut m, Msg::Recheck("2000-01-01".into()));
        assert_eq!(send(&mut st, &mut m, Msg::ConfirmBlock), 0);
        assert_eq!(dialog(&st).error.as_deref(), Some("That day has passed."));

        send(&mut st, &mut m, Msg::Recheck("next week".into()));
        assert_eq!(send(&mut st, &mut m, Msg::ConfirmBlock), 0);
        assert_eq!(
            dialog(&st).error.as_deref(),
            Some("Write the day as YYYY-MM-DD.")
        );

        // A good one goes to the board, and the error is gone.
        send(&mut st, &mut m, Msg::Recheck("2999-01-01".into()));
        assert_eq!(send(&mut st, &mut m, Msg::ConfirmBlock), 1);
        assert!(dialog(&st).error.is_none());
    }

    #[test]
    fn the_reason_stops_at_what_the_board_keeps() {
        let (mut st, mut m) = open_on(Some(DOING));
        send(
            &mut st,
            &mut m,
            Msg::Reason("x".repeat(MAX_BLOCK_REASON + 40)),
        );
        assert_eq!(dialog(&st).reason.chars().count(), MAX_BLOCK_REASON);
    }

    #[test]
    fn cancelling_a_block_closes_the_form_and_its_error() {
        let (mut st, mut m) = open_on(Some(DOING));
        send(&mut st, &mut m, Msg::StartBlock);
        send(&mut st, &mut m, Msg::ConfirmBlock);
        assert!(dialog(&st).error.is_some());
        send(&mut st, &mut m, Msg::CancelBlock);
        assert!(!dialog(&st).blocking && dialog(&st).error.is_none());
    }

    #[test]
    fn a_comment_is_sent_trimmed_and_a_blank_one_is_not() {
        let (mut st, mut m) = open_on(Some(DOING));
        send(&mut st, &mut m, Msg::Comment("   ".into()));
        assert_eq!(send(&mut st, &mut m, Msg::Send), 0);

        send(&mut st, &mut m, Msg::Comment("  a word ".into()));
        assert_eq!(send(&mut st, &mut m, Msg::Send), 1);
        assert!(dialog(&st).comment.is_empty());
    }

    #[test]
    fn delete_asks_twice() {
        let (mut st, mut m) = open_on(Some(DOING));
        assert_eq!(send(&mut st, &mut m, Msg::AskDelete), 0);
        assert!(dialog(&st).confirm_delete);
        send(&mut st, &mut m, Msg::CancelDelete);
        assert!(!dialog(&st).confirm_delete);
        send(&mut st, &mut m, Msg::AskDelete);
        assert_eq!(send(&mut st, &mut m, Msg::ConfirmDelete), 1);
    }

    #[test]
    fn leaving_a_field_saves_it_and_typing_in_it_does_not() {
        let (mut st, mut m) = open_on(Some(DOING));
        assert_eq!(send(&mut st, &mut m, Msg::Title("Renamed".into())), 0);
        // Using the release button leaves the title field: it is saved,
        // then the release goes.
        assert_eq!(send(&mut st, &mut m, Msg::Release), 2);
        // Nothing is left to save.
        assert_eq!(send(&mut st, &mut m, Msg::Release), 1);
    }

    #[test]
    fn the_priority_stepper_saves_each_step() {
        let (mut st, mut m) = open_on(Some(DOING));
        assert_eq!(send(&mut st, &mut m, Msg::Priority(NumberMsg::Step(1))), 1);
        assert_eq!(dialog(&st).priority.value(), 2.0);
        // Typing half a number saves nothing until it reads as one.
        assert_eq!(
            send(
                &mut st,
                &mut m,
                Msg::Priority(NumberMsg::Input(String::new()))
            ),
            0
        );
    }

    #[test]
    fn writing_a_task_changes_the_form_not_the_board() {
        let (mut st, mut m) = open_on(None);
        assert_eq!(send(&mut st, &mut m, Msg::Column(Column::Review)), 0);
        assert_eq!(dialog(&st).column, Column::Review);

        let other = Dependency {
            id: "x".into(),
            label: "#x".into(),
        };
        send(&mut st, &mut m, Msg::AddDependency(other.clone()));
        send(&mut st, &mut m, Msg::AddDependency(other));
        assert_eq!(dialog(&st).depends_on, vec!["x".to_owned()]);
        send(&mut st, &mut m, Msg::RemoveDependency("x".into()));
        assert!(dialog(&st).depends_on.is_empty());

        // No title: no task. With one: it goes to the board.
        assert_eq!(send(&mut st, &mut m, Msg::Create), 0);
        send(&mut st, &mut m, Msg::Title("A new one".into()));
        assert_eq!(send(&mut st, &mut m, Msg::Create), 1);
    }

    #[test]
    fn a_dependency_of_an_open_task_goes_to_the_board() {
        let (mut st, mut m) = open_on(Some(DOING));
        let dep = Dependency {
            id: "5b3c0000-aaaa-bbbb-cccc-dddddddddddd".into(),
            label: "#5b3c".into(),
        };
        assert_eq!(send(&mut st, &mut m, Msg::AddDependency(dep)), 1);
        assert_eq!(
            send(&mut st, &mut m, Msg::RemoveDependency("gone".into())),
            1
        );
    }

    #[test]
    fn closing_the_dialog_asks_the_shell_to_close_the_overlay() {
        let (mut st, mut m) = open_on(Some(DOING));
        assert_eq!(send(&mut st, &mut m, Msg::Close), 1);
    }

    #[test]
    fn a_finished_write_closes_the_dialog_when_it_was_asked_to() {
        let (mut st, _) = open_on(None);
        // A refusal of a new task stays on screen, with its reason.
        let refused: Result<(), String> = Err("no".into());
        assert_eq!(
            mutated(&mut st, Errors::ToDialog, After::CloseOnOk, &refused).units(),
            0
        );
        assert_eq!(dialog(&st).error.as_deref(), Some("no"));
        assert_eq!(
            mutated(&mut st, Errors::ToDialog, After::CloseOnOk, &Ok(())).units(),
            1
        );
        // A delete closes it whatever the board answered.
        assert_eq!(
            mutated(&mut st, Errors::Swallow, After::CloseAlways, &refused).units(),
            1
        );
    }
}
