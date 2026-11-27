//! « Orchestration »: how a workspace's orchestrator works (`OrchestrationPanel.tsx`):
//! what its agents may do without asking, whether a delivery lands by
//! itself, what checks it, where it lands, and on which models. How many
//! workers run at once and how many tries a task gets are not here: the
//! orchestrator decides both, and Hangar sets no limit on either.
//!
//! A form with a draft rather than switches saved as they flip, like the MCP
//! servers' panel: half of it is typed, and a check command saved one letter
//! at a time would be run as whatever it said when a delivery came in. What
//! the panel shows is what the workspace runs with, defaults filled in, each
//! default named beside its field.
//!
//! Which agent runs what is said at the top (`agents`), Codex with its
//! « beta », and the workers' agent is chosen here, per workspace. Each model
//! field offers the models of its agent (`models`): Claude Code's aliases on
//! a segmented control that fills the field, and under Codex's own fields,
//! in a section that folds, what Codex's server lists.

use std::fmt;

use iced::widget::{button as iced_button, column, container, row, space};
use iced::{Alignment, Background, Element, Length, Padding};

use hangar_i18n::{Key, args};
use hangar_theme::tokens::{self, font_size, radius};
use hangar_theme::{fonts, style};
use hangar_ui::{Icon, badge, button, dialog, focus, form, icon, input, text as typography};

use super::form::{
    DEFAULT_ORCHESTRATOR_MODEL, DEFAULT_REVIEWER_MODEL, DEFAULT_WORKER_MODEL, Draft, Field,
    MODEL_SUGGESTIONS, Problem, draft_of, draft_problems, normalize_draft, same_settings,
    stored_orchestration,
};
use super::{Msg, State, agents, models};
use crate::ctx::Ctx;
use crate::message::{Intent, Message};
use crate::shell::{Notice, brand_logo};
use crate::state::roles::{
    ORCHESTRATION_PERMISSION_MODES, RoleRefusal, orchestration_defaults, orchestration_of,
    orchestrator_of, worker_refusal, workers_of,
};
use crate::state::types::{AgentKind, WsId};
use crate::state::{Model, Patch, StoreAction, Workspace};

/// The window's width at most (`ecrans.md` §3), and the tallest its body
/// grows before it scrolls.
const WIDTH: f32 = 780.0;
const BODY_MAX_HEIGHT: f32 = 500.0;
/// The Fork symbol of its head.
const MARK: f32 = 36.0;
/// Between two parts of the body, and between a title and its block.
const SECTION_GAP: f32 = 20.0;
const BLOCK_GAP: f32 = 14.0;
/// A role's identity tile, the symbol in it and the column of its name.
const TILE: f32 = 28.0;
const TILE_SYMBOL: f32 = 16.0;
const ROLE_WIDTH: f32 = 150.0;
/// The column of the roles' names beside Codex's fields.
const CODEX_ROLE_WIDTH: f32 = 120.0;

/// One of the three model fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelField {
    Orchestrator,
    Worker,
    Reviewer,
}

/// The panel's messages.
#[derive(Debug, Clone)]
pub enum PanelMsg {
    /// A mode of `ORCHESTRATION_PERMISSION_MODES`, or empty for the person's own.
    Permission(String),
    AutoMerge(bool),
    /// The "Shared build cache" switch (saved with the rest, by Save).
    BuildCache(bool),
    /// "Empty": asks first, and is refused while a worker of the workspace runs.
    EmptyCache,
    /// The confirmation said no.
    EmptyCancel,
    /// The confirmation said yes.
    EmptyConfirm,
    Check(String),
    Branch(String),
    /// How many waiting deliveries share a check; none: the default.
    BatchSize(Option<u32>),
    /// The agent the workspace's workers run; none: the settings' default,
    /// else the orchestrator's.
    WorkerAgent(Option<AgentKind>),
    Model(ModelField, String),
    /// One of Codex's models: empty is Codex's own default.
    CodexModel(ModelField, String),
    /// Folds or unfolds the section of Codex's models.
    FoldCodex,
    /// Every field back to the value Hangar ships with.
    Reset,
    Save,
    /// "New orchestrator": closes the panel, then makes one.
    CreateOrchestrator,
}

/// The panel while it is open: the draft. Made again each time the panel
/// opens, so what was abandoned is gone, as the keyed React component's was.
#[derive(Debug)]
pub struct Panel {
    pub ws: WsId,
    draft: Draft,
    cache: Cache,
    /// The section of Codex's models is folded away. Open to begin with: a
    /// folded section hides what a workspace runs on.
    codex_folded: bool,
}

/// The workspace's build cache folder as the panel knows it: its size once
/// measured (off the UI thread), and whether "Empty" is waiting for a yes or
/// is deleting.
#[derive(Debug, Default)]
struct Cache {
    size: Option<u64>,
    asking: bool,
    emptying: bool,
}

impl Panel {
    pub fn open(m: &Model, ws: &str) -> Option<Panel> {
        let ws = m.workspace(ws)?;
        Some(Panel {
            ws: ws.id.clone(),
            draft: draft_of(ws),
            cache: Cache::default(),
            codex_folded: false,
        })
    }
}

/// Measures the workspace's cache folder on the blocking pool; the answer is
/// `Msg::CacheSize`.
pub fn measure(ws: &str) -> iced::Task<Message> {
    let ws = ws.to_owned();
    iced::Task::perform(
        async move {
            let dir = hangar_core::build_cache::dir_for(&ws);
            let size = tokio::task::spawn_blocking(move || hangar_core::build_cache::size_of(&dir))
                .await
                .unwrap_or(0);
            (ws, size)
        },
        |(ws, size)| Message::OrchestrationUi(Msg::CacheSize(ws, size)),
    )
}

/// The size came back.
pub fn cache_measured(st: &mut State, ws: &str, size: u64) {
    if let Some(panel) = st.panel.as_mut().filter(|panel| panel.ws == ws) {
        panel.cache.size = Some(size);
    }
}

/// The folder was emptied, or not: says so, and measures again.
pub fn cache_emptied(
    st: &mut State,
    m: &Model,
    ws: &str,
    result: Result<(), String>,
) -> iced::Task<Message> {
    if let Some(panel) = st.panel.as_mut().filter(|panel| panel.ws == ws) {
        panel.cache.emptying = false;
        panel.cache.asking = false;
    }
    let text = match result {
        Ok(()) => hangar_i18n::t(m.locale(), Key::OrchBuildCacheEmptied),
        Err(error) => hangar_i18n::t_with(
            m.locale(),
            Key::OrchBuildCacheEmptyFailed,
            &args!("error" => error.as_str()),
        ),
    };
    iced::Task::batch([
        iced::Task::done(Message::Intent(Intent::Notice(Notice::Agent(
            text.into_owned(),
        )))),
        measure(ws),
    ])
}

/// A worker of the workspace is running: its builds are in the cache.
fn refuse_while_busy(panel: &mut Panel, m: &Model) -> Option<iced::Task<Message>> {
    let ws = m.workspace(&panel.ws)?;
    if workers_of(ws).is_empty() {
        return None;
    }
    panel.cache.asking = false;
    let text = hangar_i18n::t_with(
        m.locale(),
        Key::OrchBuildCacheBusy,
        &args!("workspace" => ws.name.as_str()),
    );
    Some(iced::Task::done(Message::Intent(Intent::Notice(
        Notice::Problem(text.into_owned()),
    ))))
}

/// Deletes the folder on the blocking pool; the answer is `Msg::CacheEmptied`.
fn empty_cache(ws: &str) -> iced::Task<Message> {
    let ws = ws.to_owned();
    iced::Task::perform(
        async move {
            let dir = hangar_core::build_cache::dir_for(&ws);
            let result = tokio::task::spawn_blocking(move || hangar_core::build_cache::empty(&dir))
                .await
                .map_err(|error| error.to_string())
                .and_then(|done| done.map_err(|error| error.to_string()));
            (ws, result)
        },
        |(ws, result)| Message::OrchestrationUi(Msg::CacheEmptied(ws, result)),
    )
}

/// "412 MB", "1,5 Go".
fn size_text(locale: hangar_i18n::Locale, bytes: u64) -> String {
    let megabytes = bytes as f64 / (1024.0 * 1024.0);
    let french = locale == hangar_i18n::Locale::Fr;
    let (value, unit) = match (megabytes >= 1024.0, french) {
        (true, true) => (megabytes / 1024.0, "Go"),
        (true, false) => (megabytes / 1024.0, "GB"),
        (false, true) => (megabytes.round(), "Mo"),
        (false, false) => (megabytes.round(), "MB"),
    };
    let rounded = (value * 10.0).round() / 10.0;
    let number = if rounded.fract() == 0.0 {
        format!("{rounded:.0}")
    } else {
        format!("{rounded:.1}")
    };
    if french {
        format!("{} {unit}", number.replace('.', ","))
    } else {
        format!("{number} {unit}")
    }
}

fn model_field(draft: &mut Draft, which: ModelField) -> &mut String {
    match which {
        ModelField::Orchestrator => &mut draft.orchestrator_model,
        ModelField::Worker => &mut draft.worker_model,
        ModelField::Reviewer => &mut draft.reviewer_model,
    }
}

fn codex_model_field(draft: &mut Draft, which: ModelField) -> &mut Option<String> {
    match which {
        ModelField::Orchestrator => &mut draft.codex_models.orchestrator,
        ModelField::Worker => &mut draft.codex_models.worker,
        ModelField::Reviewer => &mut draft.codex_models.reviewer,
    }
}

/// What the panel's messages do. `Save` writes the workspace and says so.
pub fn update(st: &mut State, m: &mut Model, msg: PanelMsg) -> iced::Task<Message> {
    let Some(panel) = st.panel.as_mut() else {
        return iced::Task::none();
    };
    match msg {
        PanelMsg::Permission(mode) => panel.draft.permission_mode = mode,
        PanelMsg::AutoMerge(on) => panel.draft.auto_merge = on,
        PanelMsg::BuildCache(on) => panel.draft.shared_build_cache = on,
        PanelMsg::EmptyCache => {
            if let Some(refused) = refuse_while_busy(panel, m) {
                return refused;
            }
            panel.cache.asking = panel.cache.size != Some(0);
        }
        PanelMsg::EmptyCancel => panel.cache.asking = false,
        PanelMsg::EmptyConfirm => {
            if let Some(refused) = refuse_while_busy(panel, m) {
                return refused;
            }
            if !panel.cache.asking || panel.cache.emptying {
                return iced::Task::none();
            }
            panel.cache.asking = false;
            panel.cache.emptying = true;
            return empty_cache(&panel.ws);
        }
        PanelMsg::Check(text) => panel.draft.check_command = text,
        PanelMsg::Branch(text) => panel.draft.branch = text,
        PanelMsg::BatchSize(size) => panel.draft.batch_size = size,
        PanelMsg::WorkerAgent(agent) => panel.draft.worker_agent = agent,
        PanelMsg::Model(which, text) => *model_field(&mut panel.draft, which) = text,
        PanelMsg::CodexModel(which, text) => {
            // Blank is none, so that an emptied field is not saved as "" and
            // the workspace goes back to Codex's own default.
            *codex_model_field(&mut panel.draft, which) = (!text.trim().is_empty()).then_some(text);
        }
        PanelMsg::FoldCodex => {
            // Held open by a refused name: the press folds nothing, now or
            // once the name is fixed.
            if !codex_refused(&draft_problems(&panel.draft)) {
                panel.codex_folded = !panel.codex_folded;
            }
        }
        PanelMsg::Reset => panel.draft = draft_of(&Workspace::default()),
        PanelMsg::Save => return save(panel, m),
        PanelMsg::CreateOrchestrator => {
            let ws = panel.ws.clone();
            return iced::Task::batch([
                iced::Task::done(Message::Intent(Intent::CloseOverlay)),
                iced::Task::done(Message::Intent(Intent::CreateOrchestrator(ws))),
            ]);
        }
    }
    iced::Task::none()
}

/// Saves the draft as the workspace's settings, unless a field is refused or
/// nothing changed (the button is disabled then, and Enter in a field comes
/// here too).
fn save(panel: &Panel, m: &mut Model) -> iced::Task<Message> {
    let Some(ws) = m.workspace(&panel.ws) else {
        return iced::Task::none();
    };
    let saved = orchestration_of(ws);
    let next = normalize_draft(&panel.draft);
    if !draft_problems(&panel.draft).is_empty() || same_settings(&next, &saved) {
        return iced::Task::none();
    }
    let name = ws.name.clone();
    let stored = stored_orchestration(&next);
    m.apply(StoreAction::UpdateWorkspace(
        panel.ws.clone(),
        Patch::new(move |ws: &mut Workspace| ws.orchestration = stored.clone()),
    ));
    let text = hangar_i18n::t_with(
        m.locale(),
        Key::OrchSaved,
        &args!("workspace" => name.as_str()),
    );
    iced::Task::batch([
        iced::Task::done(Message::Intent(Intent::Notice(Notice::Agent(
            text.into_owned(),
        )))),
        iced::Task::done(Message::Intent(Intent::CloseOverlay)),
    ])
}

fn msg(message: PanelMsg) -> Message {
    Message::OrchestrationUi(Msg::Panel(message))
}

/// An option of the permission list.
#[derive(Debug, Clone, PartialEq)]
struct Mode {
    id: &'static str,
    label: String,
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label)
    }
}

/// `Overlay::Orchestration`.
pub fn view<'a>(st: &'a State, ctx: Ctx<'a>, ws_id: &'a WsId) -> Element<'a, Message> {
    let t = ctx.theme;
    let (Some(panel), Some(ws)) = (
        st.panel.as_ref().filter(|panel| &panel.ws == ws_id),
        ctx.model.workspace(ws_id),
    ) else {
        return space().into();
    };
    let draft = &panel.draft;
    let problems = draft_problems(draft);
    let next = normalize_draft(draft);
    let blocked = !problems.is_empty();
    let changed = !same_settings(&next, &orchestration_of(ws));
    let at_defaults = same_settings(&next, &orchestration_defaults());

    let body = column![
        typography::label(t, ctx.t(Key::OrchIntro)),
        status(ctx, ws),
        section(
            ctx,
            Key::OrchSectionWorkers,
            None,
            form::group(
                t,
                vec![
                    worker_agent_row(ctx, ws, draft),
                    permission_row(ctx, draft),
                    form::row(
                        t,
                        ctx.t(Key::OrchBuildCache),
                        input::switch(t, draft.shared_build_cache)
                            .on_toggle(|on| msg(PanelMsg::BuildCache(on)))
                    )
                    .hint(ctx.t(Key::OrchBuildCacheHint))
                    .into(),
                    cache_row(ctx, &panel.cache),
                ]
            )
        ),
        section(
            ctx,
            Key::OrchSectionDeliveries,
            None,
            form::group(
                t,
                vec![
                    form::row(
                        t,
                        ctx.t(Key::OrchAutoMerge),
                        input::switch(t, draft.auto_merge)
                            .on_toggle(|on| msg(PanelMsg::AutoMerge(on)))
                    )
                    .hint(ctx.t(Key::OrchAutoMergeHint))
                    .into(),
                    text_row(
                        ctx,
                        Key::OrchCheckCommand,
                        ctx.t(Key::OrchCheckHint),
                        &draft.check_command,
                        label(ctx, Key::OrchCheckPlaceholder),
                        None,
                        PanelMsg::Check,
                    ),
                    text_row(
                        ctx,
                        Key::OrchBranch,
                        ctx.t(Key::OrchBranchHint),
                        &draft.branch,
                        label(ctx, Key::OrchBranchPlaceholder),
                        (problems.get(Field::Branch) == Some(Problem::Branch))
                            .then(|| ctx.t(Key::OrchBranchInvalid).into_owned()),
                        PanelMsg::Branch,
                    ),
                    batch_row(ctx, draft),
                ]
            )
        ),
        column![
            section(
                ctx,
                Key::OrchSectionModels,
                Some(Key::OrchModelsHint),
                form::group(
                    t,
                    vec![
                        model_row(ctx, &problems, draft, ModelField::Orchestrator),
                        model_row(ctx, &problems, draft, ModelField::Worker),
                        model_row(ctx, &problems, draft, ModelField::Reviewer),
                    ]
                )
            ),
            codex_models(ctx, &problems, draft, panel.codex_folded),
        ]
        .spacing(BLOCK_GAP),
    ]
    .spacing(SECTION_GAP);

    let sub = format!("{} · {}", ws.name, ws.cwd);
    let reset = button::ghost(t, ctx.t(Key::OrchReset))
        .size(button::Size::Large)
        .tip(ctx.t(Key::OrchResetHint))
        .on_press_maybe((!at_defaults).then(|| msg(PanelMsg::Reset)));
    let cancel = button::secondary(t, ctx.t(Key::CreateCancel))
        .size(button::Size::Large)
        .on_press(Message::Intent(Intent::CloseOverlay));
    let save = button::primary(t, ctx.t(Key::OrchSave))
        .size(button::Size::Large)
        .on_press_maybe((!blocked && changed).then(|| msg(PanelMsg::Save)));

    let shown = dialog::dialog(t, ctx.t(Key::OrchTitle), body)
        .mark(brand_logo::logo(t, MARK))
        .subtitle(sub)
        .note(ctx.t(Key::OrchApplies))
        .width(WIDTH)
        .body_max_height(BODY_MAX_HEIGHT)
        .body_padding(Padding::from([SECTION_GAP, tokens::space::XXXL]))
        .close_tip(ctx.t(Key::McpClose))
        .on_close(Message::Intent(Intent::CloseOverlay))
        .action(reset)
        .action(cancel)
        .action(save);
    dialog::over(
        t,
        space().width(Length::Fill).height(Length::Fill),
        Some(shown),
    )
    .into()
}

/// A part of the window: its title in 14 px, what it is about under it if it
/// says, and its block.
fn section<'a>(
    ctx: Ctx<'a>,
    title: Key,
    hint: Option<Key>,
    block: Element<'a, Message>,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let mut head = column![typography::body(t, ctx.t(title)).font(fonts::ui_semibold())]
        .spacing(tokens::space::XS);
    if let Some(hint) = hint {
        head = head.push(form::hint(t, ctx.t(hint)));
    }
    column![head, block].spacing(BLOCK_GAP).into()
}

/// A label without arguments, for the widgets that keep a `&str` for the
/// whole frame. The dictionary lends such a label as a `'static` borrow
/// (hangar-i18n, `t`), so nothing is allocated.
fn label(ctx: Ctx<'_>, key: Key) -> &'static str {
    match ctx.t(key) {
        std::borrow::Cow::Borrowed(text) => text,
        std::borrow::Cow::Owned(text) => Box::leak(text.into_boxed_str()),
    }
}

/// "Pinned: claude-1", "2 workers running", or "No orchestrator" with the way
/// to make one; then which agent the orchestrator and its workers run.
fn status<'a>(ctx: Ctx<'a>, ws: &'a Workspace) -> Element<'a, Message> {
    let t = ctx.theme;
    let orchestrator = orchestrator_of(ws);
    let workers = workers_of(ws).len();
    let agents = typography::hint(t, agents::of(ctx.model, ws).sentence(ctx.i18n));
    let tint = if orchestrator.is_some() {
        t.palette.accent_text
    } else {
        t.palette.text_muted
    };

    let words: Element<'a, Message> = match orchestrator {
        Some(pane) => column![
            typography::body(
                t,
                ctx.i18n
                    .t_with(Key::OrchStatusPinned, &args!("name" => pane.name.as_str()))
            )
            .font(fonts::ui_semibold()),
            typography::hint(
                t,
                if workers > 0 {
                    ctx.i18n.tn(Key::OrchStatusWorkers, workers, &[])
                } else {
                    ctx.t(Key::OrchStatusNoWorker)
                }
            ),
            agents,
        ]
        .spacing(2)
        .into(),
        None => column![
            typography::body(t, ctx.t(Key::OrchStatusNone)).font(fonts::ui_semibold()),
            typography::hint(t, ctx.t(Key::OrchStatusNoneHint)),
            agents,
        ]
        .spacing(2)
        .into(),
    };

    let mut line = row![
        icon(Icon::Orchestrator, 16.0).color(tint),
        container(words).width(Length::Fill),
    ]
    .spacing(tokens::space::XL)
    .align_y(Alignment::Center);
    if orchestrator.is_none() {
        line = line.push(
            button::secondary(t, ctx.t(Key::CmdWorkspaceNewOrchestrator))
                .icon(Icon::Plus)
                .small()
                .on_press(msg(PanelMsg::CreateOrchestrator)),
        );
    }
    container(line)
        .padding(Padding::from([tokens::space::XL, tokens::space::XXL]))
        .width(Length::Fill)
        .style(style::container::card(t))
        .into()
}

/// The cache's size and what empties it: a button, which asks first.
fn cache_row<'a>(ctx: Ctx<'a>, cache: &Cache) -> Element<'a, Message> {
    let t = ctx.theme;
    let size = match cache.size {
        Some(bytes) => size_text(ctx.model.locale(), bytes),
        None => ctx.t(Key::OrchBuildCacheMeasuring).into_owned(),
    };
    let control: Element<'a, Message> = if cache.asking {
        row![
            typography::hint(t, ctx.t(Key::OrchBuildCacheEmptyAsk)),
            button::danger(t, ctx.t(Key::OrchBuildCacheEmptyConfirm))
                .small()
                .on_press(msg(PanelMsg::EmptyConfirm)),
            button::ghost(t, ctx.t(Key::CreateCancel))
                .small()
                .on_press(msg(PanelMsg::EmptyCancel)),
        ]
        .spacing(tokens::space::LG)
        .align_y(Alignment::Center)
        .into()
    } else {
        row![
            typography::body(t, size),
            button::secondary(t, ctx.t(Key::OrchBuildCacheEmpty))
                .small()
                .on_press_maybe(
                    (cache.size.is_some_and(|bytes| bytes > 0) && !cache.emptying)
                        .then(|| msg(PanelMsg::EmptyCache))
                ),
        ]
        .spacing(tokens::space::LG)
        .align_y(Alignment::Center)
        .into()
    };
    form::row(t, ctx.t(Key::OrchBuildCacheSize), control).into()
}

fn permission_row<'a>(ctx: Ctx<'a>, draft: &'a Draft) -> Element<'a, Message> {
    let t = ctx.theme;
    let mut options = vec![Mode {
        id: "",
        label: ctx.t(Key::OrchPermissionTheirs).into_owned(),
    }];
    for mode in ORCHESTRATION_PERMISSION_MODES {
        options.push(Mode {
            id: mode,
            label: ctx
                .t(match mode {
                    "auto" => Key::OrchPermissionAuto,
                    "acceptEdits" => Key::OrchPermissionAcceptEdits,
                    _ => Key::OrchPermissionBypassPermissions,
                })
                .into_owned(),
        });
    }
    let chosen = options
        .iter()
        .find(|option| option.id == draft.permission_mode)
        .cloned();
    let select = input::keyboard_select(t, options, chosen, |mode: Mode| {
        msg(PanelMsg::Permission(mode.id.to_owned()))
    });
    form::row(
        t,
        ctx.t(Key::OrchPermissionMode),
        container(select).width(300.0),
    )
    .hint(ctx.t(Key::OrchPermissionHint))
    .into()
}

/// An option of the workers' agent list: none is the settings' default, else
/// the orchestrator's agent.
#[derive(Debug, Clone, PartialEq)]
struct AgentChoice {
    agent: Option<AgentKind>,
    label: String,
}

impl fmt::Display for AgentChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label)
    }
}

/// "Workers' agent": the agent this workspace's workers run, the choice that
/// names none saying which it comes to. Why that agent would not start, when
/// it would not, stands where the hint does.
fn worker_agent_row<'a>(ctx: Ctx<'a>, ws: &'a Workspace, draft: &'a Draft) -> Element<'a, Message> {
    let t = ctx.theme;
    let (options, chosen) = worker_choice(ctx, ws, draft);
    let select = input::keyboard_select(t, options, chosen, |choice: AgentChoice| {
        msg(PanelMsg::WorkerAgent(choice.agent))
    });
    let hint = worker_problem(ctx, ws, draft)
        .unwrap_or_else(|| ctx.t(Key::OrchWorkerAgentHint).into_owned());
    form::row(
        t,
        ctx.t(Key::OrchWorkerAgent),
        container(select).width(300.0),
    )
    .hint(hint)
    .into()
}

/// The workers' agents offered, Codex with its « beta », and the one the
/// draft holds.
fn worker_choice(
    ctx: Ctx<'_>,
    ws: &Workspace,
    draft: &Draft,
) -> (Vec<AgentChoice>, Option<AgentChoice>) {
    let unchosen = Draft {
        worker_agent: None,
        ..draft.clone()
    };
    let fallback = agents::with(ctx.model, ws, &normalize_draft(&unchosen)).workers;
    let from = if ctx.model.settings().default_worker_agent.is_some() {
        Key::OrchWorkerAgentSettings
    } else {
        Key::OrchWorkerAgentSame
    };
    let mut options = vec![AgentChoice {
        agent: None,
        label: ctx
            .i18n
            .t_with(
                from,
                &args!("agent" => crate::accounts::agent_label(ctx.i18n, fallback)),
            )
            .into_owned(),
    }];
    options.extend(AgentKind::ALL.into_iter().map(|kind| AgentChoice {
        agent: Some(kind),
        label: crate::accounts::agent_label(ctx.i18n, kind),
    }));
    let chosen = options
        .iter()
        .find(|option| option.agent == draft.worker_agent)
        .cloned();
    (options, chosen)
}

/// Why the agent the draft's workers would run could not start them, in the
/// person's words: it is not on this computer, or what Hangar hands a worker
/// its brief through is turned off (`roles::worker_refusal`).
fn worker_problem(ctx: Ctx<'_>, ws: &Workspace, draft: &Draft) -> Option<String> {
    let kind = agents::with(ctx.model, ws, &normalize_draft(draft)).workers;
    let label = kind.info().label;
    // A workspace on a node runs the node's agents, which this computer
    // cannot tell here.
    if ws.node.is_none() && !crate::shell::setup::installed(ctx.model, kind) {
        let text = ctx
            .i18n
            .t_with(Key::OrchAgentNotInstalled, &args!("agent" => label));
        return Some(text.into_owned());
    }
    let text = match worker_refusal(kind, ctx.model.settings())? {
        RoleRefusal::SwitchedOff { label, .. } => {
            // The setting as « Settings › Sessions » names it: the switch of
            // the side channel this agent's panes use (`roles::side_channel_on`).
            let switch = if kind.info().caps.headless.app_server {
                Key::SettingsCodexAttach
            } else {
                Key::SettingsClaudeBridge
            };
            ctx.i18n.t_with(
                Key::OrchWorkerAgentOff,
                &args!("agent" => label, "switch" => ctx.t(switch).into_owned()),
            )
        }
        RoleRefusal::NoAgent | RoleRefusal::Unable { .. } => ctx
            .i18n
            .t_with(Key::OrchWorkerAgentUnable, &args!("agent" => label)),
    };
    Some(text.into_owned())
}

/// An option of the batch size list: none is the default.
#[derive(Debug, Clone, PartialEq)]
struct Size {
    size: Option<u32>,
    label: String,
}

impl fmt::Display for Size {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label)
    }
}

/// The sizes offered: one at a time, a few, the default, more.
const BATCH_SIZES: [u32; 6] = [1, 2, 3, 4, 6, 8];

/// "Deliveries checked together": how many waiting deliveries share one run
/// of the check (the merge queue, docs/native/DELIVERIES.md).
fn batch_row<'a>(ctx: Ctx<'a>, draft: &'a Draft) -> Element<'a, Message> {
    let t = ctx.theme;
    let (options, chosen) = batch_choice(ctx, draft);
    let select = input::keyboard_select(t, options, chosen, |size: Size| {
        msg(PanelMsg::BatchSize(size.size))
    });
    form::row(t, ctx.t(Key::OrchBatchSize), container(select).width(300.0))
        .hint(ctx.t(Key::OrchBatchSizeHint))
        .into()
}

/// The sizes offered, and the one the draft holds.
fn batch_choice(ctx: Ctx<'_>, draft: &Draft) -> (Vec<Size>, Option<Size>) {
    let default = hangar_core::delivery::BATCH_SIZE as u32;
    let options: Vec<Size> = BATCH_SIZES
        .iter()
        .map(|&size| Size {
            size: (size != default).then_some(size),
            label: if size == default {
                ctx.i18n
                    .t_with(Key::OrchBatchSizeDefault, &args!("n" => size))
                    .into_owned()
            } else if size == 1 {
                ctx.t(Key::OrchBatchSizeAlone).into_owned()
            } else {
                size.to_string()
            },
        })
        .collect();
    let wanted = draft.batch_size.filter(|size| *size != default);
    let chosen = options
        .iter()
        .find(|option| option.size == wanted)
        .cloned()
        .or_else(|| {
            // A size set by hand in the file, not one of the list.
            wanted.map(|size| Size {
                size: Some(size),
                label: size.to_string(),
            })
        });
    (options, chosen)
}

/// A text row: the check command, the branch.
fn text_row<'a>(
    ctx: Ctx<'a>,
    label_key: Key,
    hint: impl Into<String>,
    value: &'a str,
    placeholder: &'static str,
    error: Option<String>,
    on_input: fn(String) -> PanelMsg,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let field = input::text_field(t, value)
        .placeholder(placeholder)
        .mono()
        .error(error)
        .on_input(move |text| msg(on_input(text)))
        .on_submit(msg(PanelMsg::Save));
    form::row(t, ctx.t(label_key), container(field).width(300.0))
        .hint(hint)
        .into()
}

/// The models Codex's server lists, each one press away (the `datalist` of
/// the web panel); while it is being asked, says so.
fn suggestions<'a>(
    ctx: Ctx<'a>,
    kind: AgentKind,
    pick: impl Fn(String) -> PanelMsg,
) -> Option<Element<'a, Message>> {
    let t = ctx.theme;
    let Some(names) = models::offered(ctx.model, kind) else {
        if models::asking(ctx.model, kind) {
            let asking = ctx
                .i18n
                .t_with(Key::OrchModelsAsking, &args!("agent" => kind.info().label));
            return Some(typography::hint(t, asking.into_owned()).into());
        }
        return None;
    };
    let links: Vec<Element<'a, Message>> = names
        .into_iter()
        .map(|name| button::link(t, name.clone(), msg(pick(name))))
        .collect();
    Some(row(links).spacing(tokens::space::LG).wrap().into())
}

/// A role's identity tile: the orchestrator's is the brand's node, the
/// workers' its branches, the reviewer's an eye in the colour of what passed.
fn tile<'a>(ctx: Ctx<'a>, which: ModelField) -> Element<'a, Message> {
    let t = ctx.theme;
    let symbol: Element<'a, Message> = match which {
        ModelField::Orchestrator => return brand_logo::orchestrator_tile(TILE),
        ModelField::Worker => brand_logo::branches(t, TILE_SYMBOL),
        ModelField::Reviewer => icon(Icon::Eye, TILE_SYMBOL)
            .color(t.palette.status.completed)
            .into(),
    };
    let fill = form::between(t.palette.surface_raised, t.palette.selection_bg, 0.4);
    container(symbol)
        .center_x(TILE)
        .center_y(TILE)
        .style(move |_: &iced::Theme| container::Style {
            background: Some(Background::Color(fill)),
            border: iced::border::rounded(radius::LG),
            ..container::Style::default()
        })
        .into()
}

/// A role's name, as its rows call it.
fn role_name(which: ModelField) -> Key {
    match which {
        ModelField::Orchestrator => Key::OrchOrchestratorModel,
        ModelField::Worker => Key::OrchWorkerModel,
        ModelField::Reviewer => Key::OrchReviewerModel,
    }
}

/// The aliases Claude Code answers to, the one the field holds chosen: a
/// press fills the field, and a full id typed there chooses none.
fn aliases<'a>(ctx: Ctx<'a>, which: ModelField, value: &str) -> Element<'a, Message> {
    let options = MODEL_SUGGESTIONS
        .map(|name| (name.to_owned(), name.to_owned()))
        .to_vec();
    input::segmented_accent(ctx.theme, options, &value.trim().to_owned(), move |name| {
        msg(PanelMsg::Model(which, name))
    })
}

/// A role's row: its tile, its name over its default, the aliases and the
/// field that takes any model id.
fn model_row<'a>(
    ctx: Ctx<'a>,
    problems: &super::form::Problems,
    draft: &'a Draft,
    which: ModelField,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let (value, default, field) = match which {
        ModelField::Orchestrator => (
            &draft.orchestrator_model,
            DEFAULT_ORCHESTRATOR_MODEL,
            Field::OrchestratorModel,
        ),
        ModelField::Worker => (
            &draft.worker_model,
            DEFAULT_WORKER_MODEL,
            Field::WorkerModel,
        ),
        ModelField::Reviewer => (
            &draft.reviewer_model,
            DEFAULT_REVIEWER_MODEL,
            Field::ReviewerModel,
        ),
    };
    let error = (problems.get(field) == Some(Problem::Model))
        .then(|| ctx.t(Key::OrchModelInvalid).into_owned());
    let input = input::text_field(t, value)
        .placeholder(default)
        .mono()
        .error(error)
        .on_input(move |text| msg(PanelMsg::Model(which, text)))
        .on_submit(msg(PanelMsg::Save));
    let name = column![
        form::label(t, ctx.t(role_name(which))),
        typography::mono(
            t,
            ctx.i18n
                .t_with(Key::OrchDefault, &args!("value" => default))
        )
        .size(tokens::scale::type_scale::LABEL.size),
    ]
    .width(ROLE_WIDTH);
    container(
        row![
            tile(ctx, which),
            name,
            aliases(ctx, which, value),
            container(input).width(Length::Fill),
        ]
        .spacing(tokens::space::XXL)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([14.0, tokens::space::XXL]))
    .into()
}

/// One of Codex's names is refused: its section stays open.
fn codex_refused(problems: &super::form::Problems) -> bool {
    [
        Field::CodexOrchestratorModel,
        Field::CodexWorkerModel,
        Field::CodexReviewerModel,
    ]
    .into_iter()
    .any(|field| problems.get(field).is_some())
}

/// Codex's models, in a block that folds: a field per role, empty being
/// Codex's own default. A field that is refused keeps the block open, so
/// that what holds « Save » back is in sight.
fn codex_models<'a>(
    ctx: Ctx<'a>,
    problems: &super::form::Problems,
    draft: &'a Draft,
    folded: bool,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let open = codex_refused(problems) || !folded;
    let chevron = if open {
        Icon::ChevronDown
    } else {
        Icon::ChevronRight
    };
    let title = row![
        icon(chevron, 12.0),
        form::label(t, ctx.t(Key::OrchCodexSection)),
        badge::beta(t, ctx.t(Key::SettingsAgentBeta).to_uppercase()),
        space::horizontal(),
        form::hint(t, ctx.t(Key::OrchCodexSectionNote)),
    ]
    .spacing(tokens::space::MD)
    .align_y(Alignment::Center);
    let toggle = iced_button(title)
        .padding(Padding::from([tokens::space::XL, tokens::space::XXL]))
        .width(Length::Fill)
        .style(style::button::plain(t))
        .on_press(msg(PanelMsg::FoldCodex));
    let toggle = focus::ring(t, toggle)
        .radius(radius::XL)
        .on_activate(msg(PanelMsg::FoldCodex));
    let mut block = column![toggle];
    if open {
        let mut fields =
            column![form::hint(t, ctx.t(Key::OrchCodexModelHint))].spacing(tokens::space::LG);
        for which in [
            ModelField::Orchestrator,
            ModelField::Worker,
            ModelField::Reviewer,
        ] {
            fields = fields.push(codex_model_row(ctx, problems, draft, which));
        }
        block = block.push(container(fields).padding(Padding {
            top: tokens::space::XS,
            right: tokens::space::XXL,
            bottom: tokens::space::XXL,
            left: tokens::space::XXL,
        }));
    }
    form::group(t, vec![block.into()])
}

/// One of Codex's model rows: the role, its field, and the models Codex
/// lists, one press away.
fn codex_model_row<'a>(
    ctx: Ctx<'a>,
    problems: &super::form::Problems,
    draft: &'a Draft,
    which: ModelField,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let (value, field) = match which {
        ModelField::Orchestrator => (
            &draft.codex_models.orchestrator,
            Field::CodexOrchestratorModel,
        ),
        ModelField::Worker => (&draft.codex_models.worker, Field::CodexWorkerModel),
        ModelField::Reviewer => (&draft.codex_models.reviewer, Field::CodexReviewerModel),
    };
    let error = (problems.get(field) == Some(Problem::Model))
        .then(|| ctx.t(Key::OrchModelInvalid).into_owned());
    let input = input::text_field(t, value.as_deref().unwrap_or_default())
        .placeholder(label(ctx, Key::OrchCodexModelDefault))
        .mono()
        .error(error)
        .on_input(move |text| msg(PanelMsg::CodexModel(which, text)))
        .on_submit(msg(PanelMsg::Save));
    let mut control = column![input].spacing(tokens::space::SM);
    if let Some(names) = suggestions(ctx, AgentKind::Codex, move |name| {
        PanelMsg::CodexModel(which, name)
    }) {
        control = control.push(names);
    }
    // The name stays level with the field, whatever comes under it.
    let name = container(typography::label(t, ctx.t(role_name(which))).size(font_size::UI))
        .width(CODEX_ROLE_WIDTH)
        .center_y(tokens::scale::size::CONTROL_MD);
    row![name, control.width(Length::Fill)]
        .spacing(tokens::space::XXL)
        .into()
}

#[cfg(test)]
mod tests {
    //! The panel drawn as a window would lay it out.

    use super::*;
    use crate::app::Packages;
    use crate::navigator::testing::{ctx, model_of, simulate};
    use crate::state::types::AppState;

    /// Hangar sets no limit on the workers (owner's decision, 2026-11-04):
    /// the panel has no field for how many run at once or how many times a
    /// task is tried, and says the orchestrator decides.
    #[test]
    fn offers_no_limit_on_the_workers_and_says_the_orchestrator_decides() {
        let model = model_of(AppState {
            workspaces: vec![Workspace {
                id: "w1".into(),
                name: "Shop".into(),
                cwd: "/code/shop".into(),
                ..Workspace::default()
            }],
            ..AppState::default()
        });
        let st = State {
            panel: Panel::open(&model, "w1"),
            ..State::default()
        };
        let pkg = Packages::default();
        let ws: WsId = "w1".into();
        let mut ui = simulate(view(&st, ctx(&model, &pkg), &ws));
        for shown in [
            "Permissions",
            "Merge by itself",
            "Check command",
            "Target branch",
        ] {
            assert!(ui.find(shown).is_ok(), "{shown}");
        }
        for gone in ["Workers at once", "Attempts per task"] {
            assert!(ui.find(gone).is_err(), "{gone}");
        }
        let intro = "It decides how many workers run at once and how many times a task is tried.";
        assert!(
            ctx(&model, &pkg).t(Key::OrchIntro).contains(intro),
            "{intro}"
        );
    }

    fn shop(workers: bool) -> Model {
        let mut panes = vec![];
        if workers {
            panes.push(crate::state::types::Pane {
                id: "p1".into(),
                name: "Sam".into(),
                role: Some(crate::state::types::PaneRole::Worker),
                ..crate::state::types::Pane::default()
            });
        }
        model_of(AppState {
            workspaces: vec![Workspace {
                id: "w1".into(),
                name: "Shop".into(),
                cwd: "/code/shop".into(),
                panes,
                ..Workspace::default()
            }],
            ..AppState::default()
        })
    }

    fn opened(model: &Model) -> State {
        State {
            panel: Panel::open(model, "w1"),
            ..State::default()
        }
    }

    #[test]
    fn shows_the_build_cache_switch_its_size_and_empty() {
        let model = shop(false);
        let mut st = opened(&model);
        let pkg = Packages::default();
        let ws: WsId = "w1".into();
        {
            let mut ui = simulate(view(&st, ctx(&model, &pkg), &ws));
            assert!(ui.find("Shared build cache").is_ok());
            assert!(ui.find("Measuring…").is_ok());
        }
        cache_measured(&mut st, "w1", 3 * 1024 * 1024 * 1024);
        let mut ui = simulate(view(&st, ctx(&model, &pkg), &ws));
        assert!(ui.find("3 GB").is_ok());
        assert!(ui.find("Empty").is_ok());
    }

    #[test]
    fn the_switch_is_part_of_the_draft_and_on_is_what_gets_saved() {
        let mut model = shop(false);
        let mut st = opened(&model);
        assert!(!st.panel.as_ref().unwrap().draft.shared_build_cache);
        let _ = update(&mut st, &mut model, PanelMsg::BuildCache(true));
        let draft = &st.panel.as_ref().unwrap().draft;
        assert!(draft.shared_build_cache);
        assert!(normalize_draft(draft).shared_build_cache);
        // It is a change to save, and not the defaults any more.
        assert!(!same_settings(
            &normalize_draft(draft),
            &orchestration_defaults()
        ));
    }

    #[test]
    fn shows_codex_s_models_in_their_beta_section_which_folds() {
        let mut model = shop(false);
        let mut st = opened(&model);
        let pkg = Packages::default();
        let ws: WsId = "w1".into();
        {
            let mut ui = simulate(view(&st, ctx(&model, &pkg), &ws));
            for shown in [
                "Orchestrator",
                "Workers",
                "Reviewer",
                "Codex models",
                "BETA",
                "Used when a role runs on Codex",
                "A Codex model id. Empty: the model Codex picks itself.",
            ] {
                assert!(ui.find(shown).is_ok(), "{shown}");
            }
        }
        {
            // Below the dialog's fold, so pressed on its own.
            let draft = &st.panel.as_ref().unwrap().draft;
            let problems = draft_problems(draft);
            let mut ui = simulate(codex_models(ctx(&model, &pkg), &problems, draft, false));
            ui.click("Codex models").expect("the section's title");
            let said: Vec<Message> = ui.into_messages().collect();
            assert!(
                matches!(
                    said.as_slice(),
                    [Message::OrchestrationUi(Msg::Panel(PanelMsg::FoldCodex))]
                ),
                "{said:?}"
            );
        }
        let note = "A Codex model id. Empty: the model Codex picks itself.";
        let _ = update(&mut st, &mut model, PanelMsg::FoldCodex);
        {
            let mut ui = simulate(view(&st, ctx(&model, &pkg), &ws));
            assert!(ui.find("Codex models").is_ok());
            assert!(ui.find(note).is_err());
        }
        // A refused name keeps the section open: it is what holds Save back.
        let _ = update(
            &mut st,
            &mut model,
            PanelMsg::CodexModel(ModelField::Worker, "two words".into()),
        );
        {
            let mut ui = simulate(view(&st, ctx(&model, &pkg), &ws));
            assert!(ui.find(note).is_ok());
        }
        // A press while it is held open changes nothing, then or later.
        let folded = st.panel.as_ref().unwrap().codex_folded;
        let _ = update(&mut st, &mut model, PanelMsg::FoldCodex);
        assert_eq!(st.panel.as_ref().unwrap().codex_folded, folded);
    }

    /// In the smallest main window (`MAIN_MIN_SIZE`), as in a taller one
    /// that is still short of the whole dialog, the body gives up its room
    /// and the foot keeps its buttons.
    #[test]
    fn the_foot_stays_in_sight_in_a_short_window() {
        let mut model = shop(false);
        let mut st = opened(&model);
        // Something to save: « Save » is greyed otherwise.
        let merges = orchestration_defaults().auto_merge;
        let _ = update(&mut st, &mut model, PanelMsg::AutoMerge(!merges));
        let pkg = Packages::default();
        let ws: WsId = "w1".into();
        for height in [560.0, 600.0, 640.0] {
            let settings = iced::Settings {
                fonts: hangar_theme::fonts::ALL
                    .map(std::borrow::Cow::Borrowed)
                    .to_vec(),
                default_font: fonts::ui(),
                ..iced::Settings::default()
            };
            let window = iced::Size::new(900.0, height);
            let drawn = view(&st, ctx(&model, &pkg), &ws);
            let mut ui = iced_test::Simulator::with_size(settings, window, drawn);
            for button in ["Cancel", "Save"] {
                let words = ui.find(button).expect(button).bounds();
                assert!(
                    words.y + words.height <= height - 8.0,
                    "{button} at {height}: {words:?}"
                );
            }
            ui.click("Save").expect("Save");
            let said: Vec<Message> = ui.into_messages().collect();
            assert!(
                matches!(
                    said.as_slice(),
                    [Message::OrchestrationUi(Msg::Panel(PanelMsg::Save))]
                ),
                "{height}: {said:?}"
            );
        }
    }

    #[test]
    fn an_alias_fills_the_field_of_its_role() {
        let model = shop(false);
        let pkg = Packages::default();
        let mut ui = simulate(aliases(ctx(&model, &pkg), ModelField::Worker, "sonnet"));
        ui.click("haiku").expect("Claude Code's alias");
        let said: Vec<Message> = ui.into_messages().collect();
        assert!(
            matches!(
                said.as_slice(),
                [Message::OrchestrationUi(Msg::Panel(PanelMsg::Model(
                    ModelField::Worker,
                    name
                )))] if name == "haiku"
            ),
            "{said:?}"
        );
    }

    #[test]
    fn a_codex_model_is_part_of_the_draft_and_emptying_it_goes_back_to_codex_s_default() {
        let mut model = shop(false);
        let mut st = opened(&model);
        assert_eq!(st.panel.as_ref().unwrap().draft.codex_models.worker, None);
        let _ = update(
            &mut st,
            &mut model,
            PanelMsg::CodexModel(ModelField::Worker, " gpt-5.5 ".into()),
        );
        let saved = normalize_draft(&st.panel.as_ref().unwrap().draft);
        // Trimmed on the way out; Claude Code's worker model is untouched.
        assert_eq!(saved.codex_models.worker.as_deref(), Some("gpt-5.5"));
        assert_eq!(saved.worker_model, orchestration_defaults().worker_model);
        assert!(!same_settings(&saved, &orchestration_defaults()));
        let _ = update(
            &mut st,
            &mut model,
            PanelMsg::CodexModel(ModelField::Worker, "  ".into()),
        );
        let draft = &st.panel.as_ref().unwrap().draft;
        assert_eq!(draft.codex_models.worker, None);
        assert!(same_settings(
            &normalize_draft(draft),
            &orchestration_defaults()
        ));
    }

    #[test]
    fn the_batch_size_shows_its_default_and_a_choice_is_what_gets_saved() {
        let mut model = shop(false);
        let mut st = opened(&model);
        let pkg = Packages::default();
        let ws: WsId = "w1".into();
        {
            let mut ui = simulate(view(&st, ctx(&model, &pkg), &ws));
            assert!(ui.find("Deliveries checked together").is_ok());
        }
        let label = |model: &Model, st: &State| {
            let draft = &st.panel.as_ref().unwrap().draft;
            batch_choice(ctx(model, &pkg), draft).1.unwrap().label
        };
        assert_eq!(label(&model, &st), "4 (default)");
        let _ = update(&mut st, &mut model, PanelMsg::BatchSize(Some(1)));
        assert_eq!(label(&model, &st), "1, one at a time");
        let draft = &st.panel.as_ref().unwrap().draft;
        assert_eq!(normalize_draft(draft).batch_size, Some(1));
        assert!(!same_settings(
            &normalize_draft(draft),
            &orchestration_defaults()
        ));
        // Back to the default: nothing to store.
        let _ = update(&mut st, &mut model, PanelMsg::BatchSize(None));
        let draft = &st.panel.as_ref().unwrap().draft;
        assert!(same_settings(
            &normalize_draft(draft),
            &orchestration_defaults()
        ));
    }

    #[test]
    fn empty_asks_first_and_cancel_leaves_the_cache_alone() {
        let mut model = shop(false);
        let mut st = opened(&model);
        cache_measured(&mut st, "w1", 5_000_000);
        let _ = update(&mut st, &mut model, PanelMsg::EmptyCache);
        assert!(st.panel.as_ref().unwrap().cache.asking);
        let pkg = Packages::default();
        let ws: WsId = "w1".into();
        {
            let mut ui = simulate(view(&st, ctx(&model, &pkg), &ws));
            assert!(ui.find("Yes, empty").is_ok());
        }
        let _ = update(&mut st, &mut model, PanelMsg::EmptyCancel);
        let cache = &st.panel.as_ref().unwrap().cache;
        assert!(!cache.asking);
        assert!(!cache.emptying);
    }

    #[test]
    fn confirming_without_having_been_asked_does_nothing() {
        let mut model = shop(false);
        let mut st = opened(&model);
        cache_measured(&mut st, "w1", 5_000_000);
        let _ = update(&mut st, &mut model, PanelMsg::EmptyConfirm);
        assert!(!st.panel.as_ref().unwrap().cache.emptying);
    }

    #[test]
    fn empty_is_refused_while_a_worker_of_the_workspace_runs() {
        let mut model = shop(true);
        let mut st = opened(&model);
        cache_measured(&mut st, "w1", 5_000_000);
        let _ = update(&mut st, &mut model, PanelMsg::EmptyCache);
        assert!(!st.panel.as_ref().unwrap().cache.asking);
        // Nor does a yes go through, if a worker started after the question.
        st.panel.as_mut().unwrap().cache.asking = true;
        let _ = update(&mut st, &mut model, PanelMsg::EmptyConfirm);
        let cache = &st.panel.as_ref().unwrap().cache;
        assert!(!cache.asking);
        assert!(!cache.emptying);
    }

    #[test]
    fn says_which_agent_the_orchestrator_and_its_workers_run() {
        let mut model = shop(false);
        model.ui.claude_installed = true;
        let st = opened(&model);
        let pkg = Packages::default();
        let ws: WsId = "w1".into();
        let mut ui = simulate(view(&st, ctx(&model, &pkg), &ws));
        assert!(
            ui.find("Orchestrator: Claude Code · workers: Claude Code")
                .is_ok()
        );
        assert!(ui.find("Workers' agent").is_ok());
    }

    #[test]
    fn the_workers_agent_is_chosen_here_codex_with_its_beta() {
        let mut model = shop(false);
        model.ui.claude_installed = true;
        let mut st = opened(&model);
        let pkg = Packages::default();
        let ws = model.workspace("w1").unwrap().clone();
        let labels = |model: &Model, st: &State| {
            let draft = &st.panel.as_ref().unwrap().draft;
            let (options, chosen) = worker_choice(ctx(model, &pkg), &ws, draft);
            let labels: Vec<String> = options.into_iter().map(|o| o.label).collect();
            (labels, chosen.map(|chosen| chosen.label))
        };
        let (options, chosen) = labels(&model, &st);
        assert_eq!(
            options,
            [
                "Same as the orchestrator: Claude Code",
                "Claude Code",
                "Codex (beta)"
            ]
        );
        assert_eq!(
            chosen.as_deref(),
            Some("Same as the orchestrator: Claude Code")
        );

        let _ = update(
            &mut st,
            &mut model,
            PanelMsg::WorkerAgent(Some(AgentKind::Codex)),
        );
        assert_eq!(labels(&model, &st).1.as_deref(), Some("Codex (beta)"));
        let draft = &st.panel.as_ref().unwrap().draft;
        assert_eq!(normalize_draft(draft).worker_agent, Some(AgentKind::Codex));
        // Codex is not on this computer: the row says so.
        let problem = worker_problem(ctx(&model, &pkg), &ws, draft);
        assert_eq!(
            problem.as_deref(),
            Some("Codex is not installed on this computer: its workers would not start.")
        );
        model.ui.codex_installed = true;
        assert_eq!(worker_problem(ctx(&model, &pkg), &ws, draft), None);
    }

    #[test]
    fn codex_s_model_fields_offer_what_codex_lists_one_press_away() {
        let mut model = shop(false);
        let pkg = Packages::default();
        {
            // Not asked yet: nothing to offer, and nothing said.
            let offered = suggestions(ctx(&model, &pkg), AgentKind::Codex, |name| {
                PanelMsg::CodexModel(ModelField::Worker, name)
            });
            assert!(offered.is_none());
        }
        models::answered(
            &mut model,
            AgentKind::Codex,
            Ok(vec!["gpt-b".into(), "gpt-a".into()]),
        );
        let offered = suggestions(ctx(&model, &pkg), AgentKind::Codex, |name| {
            PanelMsg::CodexModel(ModelField::Worker, name)
        });
        let mut ui = simulate(offered.expect("what Codex lists"));
        ui.click("gpt-a").expect("Codex's model is offered");
        let said: Vec<Message> = ui.into_messages().collect();
        assert!(
            said.iter().any(|message| matches!(
                message,
                Message::OrchestrationUi(Msg::Panel(PanelMsg::CodexModel(
                    ModelField::Worker,
                    name
                ))) if name == "gpt-a"
            )),
            "{said:?}"
        );
    }

    #[test]
    fn sizes_read_in_megabytes_then_gigabytes() {
        use hangar_i18n::Locale;
        assert_eq!(size_text(Locale::En, 0), "0 MB");
        assert_eq!(size_text(Locale::En, 412 * 1024 * 1024), "412 MB");
        assert_eq!(size_text(Locale::En, 1536 * 1024 * 1024), "1.5 GB");
        assert_eq!(size_text(Locale::Fr, 1536 * 1024 * 1024), "1,5 Go");
        assert_eq!(size_text(Locale::Fr, 12 * 1024 * 1024), "12 Mo");
    }
}
