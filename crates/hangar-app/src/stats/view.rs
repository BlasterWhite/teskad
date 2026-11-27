//! The statistics page: a bar of controls that scopes everything under it,
//! then cards on the work area's own background (`StatsView.tsx`).
//!
//! One row of controls scopes the whole page (the period, the project, and
//! whether the charts measure tokens or cost), so every number on screen is
//! about the same slice. What was picked lasts as long as the window does.

use std::sync::Arc;
use std::time::{Duration, Instant};

use hangar_i18n::{Key, Translator, args};
use hangar_theme::tokens::{font_size, scale, size, space};
use hangar_theme::{Theme, fonts, style};
use hangar_ui::button::{self as ui_button};
use hangar_ui::{Icon, focus, icon, menu, tab_toolbar, tip};
use iced::widget::canvas::{self, Canvas, Path, Stroke};
use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{Space, button, column, container, responsive, row, scrollable, text};
use iced::{Element, Length, Padding, Point, Radians, Rectangle, Renderer, alignment, mouse};

use super::charts::columns::Columns;
use super::charts::heatmap::Heatmap;
use super::charts::sparkline::sparkline;
use super::charts::{ChartId, KINDS};
use super::format::{
    self, format_change, format_compact, format_duration, format_integer, format_share, format_usd,
};
use super::model::{Granularity, Measure, PERIODS, Period, Ranked, Summary, change, model_agent};
use super::ranked::{self, List};
use super::ui::{self, Ui, measure_key, one_line};
use super::{Computed, Msg, State, sessions, tables};
use crate::ctx::Ctx;
use crate::message::Message;
use crate::state::types::AgentKind;

/// How many projects show before the rest fold under "N more".
const PROJECTS_SHOWN: usize = 7;
/// The page's container queries: the status line goes under 52 em, the title
/// under 40 em (of the 13 px interface font), the cards stack under 760 px
/// and the legend goes under 900.
const NO_STATUS_BELOW: f32 = 676.0;
const NO_TITLE_BELOW: f32 = 520.0;
/// The figures go two by two under this width (`ecrans.md` §6, « Place qui
/// manque »)…
const CARDS_2X2_BELOW: f32 = 1100.0;
/// …the three columns become one under this one…
const ONE_COLUMN_BELOW: f32 = 960.0;
/// …and the legend goes under the chart's title.
const LEGEND_BELOW_TITLE: f32 = 900.0;
/// The room around the cards and between them.
const PAGE_PADDING: f32 = scale::space::S4;
const GAP: f32 = scale::space::S3;
const PAGE_MAX_WIDTH: f32 = 1760.0;

fn msg(m: Msg) -> Message {
    Message::Stats(m)
}

/// Region `main` in the stats view.
pub fn view<'a>(st: &'a State, ctx: Ctx<'a>) -> Element<'a, Message> {
    let ui = Ui::new(&ctx);
    let computed = st.computed();
    responsive(move |bounds| page(st, ctx, ui, computed.clone(), bounds.width)).into()
}

fn page<'a>(
    st: &'a State,
    ctx: Ctx<'a>,
    ui: Ui,
    computed: Option<Arc<Computed>>,
    width: f32,
) -> Element<'a, Message> {
    let live = &ctx.model.live.stats;
    let bar = bar(st, ctx, ui, computed.as_deref(), width);
    let body: Element<'a, Message> = match &computed {
        Some(computed) => {
            let data = &computed.data.stats;
            if data.buckets.is_empty() && !live.refreshing {
                empty_state(ui, data)
            } else {
                content(st, ctx, ui, computed, data, width)
            }
        }
        None => match &live.error {
            Some(error) => hangar_ui::empty::failed(ui.t, ui.s(Key::StatsErrorTitle))
                .description(error.clone())
                .action(
                    ui_button::secondary(ui.t, ui.s(Key::StatsRetry))
                        .icon(Icon::Restart)
                        .on_press(msg(Msg::Refresh)),
                )
                .into(),
            None => hangar_ui::empty::loading(ui.t, ui.s(Key::StatsLoadingTitle))
                .description(ui.s(Key::StatsLoadingBody))
                .into(),
        },
    };
    // What the cache said, kept on screen and held back while the disk is
    // read.
    let stale = ui::is_stale(
        live.refreshing,
        live.data
            .as_ref()
            .is_some_and(|data| data.stats.scan.cached_only),
        live.refresh_started,
        ctx.now,
    );
    let body = if stale {
        iced::widget::stack![body, ui::stale_veil(ui.t)].into()
    } else {
        body
    };
    container(column![bar, body])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(ui::page(ui.t))
        .into()
}

// ---------------------------------------------------------------------------
// The bar
// ---------------------------------------------------------------------------

/// A radio group drawn as a segmented control.
fn choice<'a, V: Copy + PartialEq + 'a>(
    ui: Ui,
    options: Vec<(V, String, String)>,
    value: V,
    on_select: fn(V) -> Message,
) -> Element<'a, Message> {
    let t = ui.t;
    let segments = options.into_iter().map(|(option, label, hint)| {
        let segment = button(
            container(
                text(label)
                    .size(font_size::UI)
                    .font(fonts::ui_medium())
                    .wrapping(text::Wrapping::None),
            )
            .center_y(Length::Fill),
        )
        .padding(Padding::from([0.0, 9.0]))
        .height(Length::Fill)
        .style(style::button::tab(t, option == value))
        .on_press(on_select(option));
        let segment = focus::ring(t, segment)
            .radius(scale::radius::SM)
            .on_activate(on_select(option));
        tip::bottom(t, segment, hint)
    });
    container(row(segments).spacing(space::XXS))
        .padding(space::XXS)
        .height(scale::size::CONTROL_SM)
        .style(style::container::segmented(t))
        .into()
}

/// Reading the transcripts: a ring turning, never a bar that pretends to know
/// how far along it is (`.stats__spinner`).
struct Spinner {
    theme: &'static Theme,
    angle: f32,
}

impl canvas::Program<Message> for Spinner {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &iced::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let centre = Point::new(bounds.width / 2.0, bounds.height / 2.0);
        let radius = bounds.width.min(bounds.height) / 2.0 - 1.0;
        frame.stroke(
            &Path::circle(centre, radius),
            Stroke::default()
                .with_color(self.theme.palette.border_strong)
                .with_width(1.5),
        );
        let arc = Path::new(|path| {
            path.arc(canvas::path::Arc {
                center: centre,
                radius,
                start_angle: Radians(self.angle),
                end_angle: Radians(self.angle + std::f32::consts::FRAC_PI_2),
            });
        });
        frame.stroke(
            &arc,
            Stroke::default()
                .with_color(self.theme.palette.accent)
                .with_width(1.5),
        );
        vec![frame.into_geometry()]
    }
}

/// How far a spinner has turned `since` it started: one turn in 0.9 s.
pub fn spin_angle(since: Option<Instant>, now: Instant) -> f32 {
    let elapsed = since.map_or(Duration::ZERO, |since| now.saturating_duration_since(since));
    (elapsed.as_secs_f32() / 0.9).fract() * std::f32::consts::TAU
}

fn status<'a>(ctx: Ctx<'a>, ui: Ui) -> Option<Element<'a, Message>> {
    let t = ui.t;
    let live = &ctx.model.live.stats;
    let small = |content: String| {
        text(content)
            .size(font_size::CAPTION_LG)
            .wrapping(text::Wrapping::None)
    };
    if live.refreshing {
        let spinner = Canvas::new(Spinner {
            theme: t,
            angle: spin_angle(live.refresh_started, ctx.now),
        })
        .width(10)
        .height(10);
        return Some(
            row![
                spinner,
                small(ui.s(Key::StatsRefreshing)).color(t.palette.text_muted)
            ]
            .spacing(7)
            .align_y(alignment::Vertical::Center)
            .into(),
        );
    }
    if let Some(checked) = live.checked_at {
        let time = format::prose_time(checked, ui.locale());
        return Some(
            row![
                icon(Icon::Check, 12.0).color(t.palette.status.completed),
                small(
                    ui.tr
                        .t_with(Key::StatsUpToDate, &args!("time" => time))
                        .into_owned(),
                )
                .color(t.palette.text_muted)
            ]
            .spacing(space::SM)
            .align_y(alignment::Vertical::Center)
            .into(),
        );
    }
    live.data
        .as_ref()
        .filter(|data| data.stats.scan.cached_only)
        .map(|_| {
            small(ui.s(Key::StatsFromCache))
                .color(t.palette.text_muted)
                .into()
        })
}

fn bar<'a>(
    st: &'a State,
    ctx: Ctx<'a>,
    ui: Ui,
    computed: Option<&Computed>,
    width: f32,
) -> Element<'a, Message> {
    let t = ui.t;
    let live = &ctx.model.live.stats;

    let periods = PERIODS
        .iter()
        .map(|period| {
            let (label, hint) = period_keys(*period);
            (*period, ui.s(label), ui.s(hint))
        })
        .collect();
    let period = choice(ui, periods, st.period, |period| msg(Msg::SetPeriod(period)));

    let measures = [Measure::Tokens, Measure::Cost]
        .into_iter()
        .map(|measure| {
            let hint = match measure {
                Measure::Tokens => Key::StatsMeasureHintTokens,
                Measure::Cost => Key::StatsMeasureHintCost,
            };
            (measure, ui.s(measure_key(measure)), ui.s(hint))
        })
        .collect();
    let measure = choice(ui, measures, st.measure, |measure| {
        msg(Msg::SetMeasure(measure))
    });

    // A filter on a project the period no longer has keeps its name.
    let project_name = st.project.as_ref().and_then(|key| {
        computed.and_then(|computed| {
            computed
                .summary
                .projects
                .iter()
                .find(|row| &row.key == key)
                .map(|row| row.label.clone())
                .or_else(|| {
                    computed
                        .projects
                        .iter()
                        .find(|project| &project.key == key)
                        .map(|project| project.name.clone())
                })
        })
    });
    let filtered = st.project.is_some();
    let label = project_name.unwrap_or_else(|| ui.s(Key::StatsAllProjects));
    let tint = if filtered {
        t.palette.accent_text
    } else {
        t.palette.text_muted
    };
    let picker_face = row![
        icon(Icon::Folder, 14.0).color(tint),
        container(one_line(label).size(font_size::UI).font(fonts::ui_medium()))
            .max_width(210)
            .clip(true),
        icon(Icon::ChevronDown, 12.0).color(t.palette.text_muted),
    ]
    .spacing(space::SM)
    .align_y(alignment::Vertical::Center);
    let picker_button = button(picker_face)
        .padding(Padding {
            top: 0.0,
            right: space::MD,
            bottom: 0.0,
            left: 9.0,
        })
        .height(scale::size::CONTROL_SM)
        .style(tab_toolbar::filter_style(t, filtered))
        .on_press_maybe(computed.map(|_| msg(Msg::ProjectMenu(!st.menu_open))));
    let picker_button = focus::ring(t, picker_button)
        .radius(scale::radius::SM)
        .on_activate_maybe(computed.map(|_| msg(Msg::ProjectMenu(!st.menu_open))));
    let picker = tip::bottom(t, picker_button, ui.s(Key::StatsProjectFilter));
    let entries = (st.menu_open)
        .then_some(computed)
        .flatten()
        .map(|computed| project_entries(st, ui, &computed.summary));
    let picker = menu::dropdown(t, picker, entries).on_dismiss(msg(Msg::ProjectMenu(false)));
    let mut project = row![picker].spacing(2).align_y(alignment::Vertical::Center);
    if filtered {
        project = project.push(
            ui_button::icon_button(t, Icon::Close)
                .small()
                .tip(ui.s(Key::StatsClearProject))
                .on_press(msg(Msg::SetProject(None))),
        );
    }

    let refresh = ui_button::icon_button(t, Icon::Restart)
        .tip(ui.s(Key::StatsRefresh))
        .on_press_maybe((!live.refreshing).then(|| msg(Msg::Refresh)));

    let mut left = row![]
        .spacing(space::XL)
        .align_y(alignment::Vertical::Center);
    if width >= NO_TITLE_BELOW {
        left = left.push(tab_toolbar::count(t, ui.s(Key::StatsTitle)));
    }
    left = left.push(period).push(project);

    let mut right = row![]
        .spacing(space::XL)
        .align_y(alignment::Vertical::Center);
    if width >= NO_STATUS_BELOW
        && let Some(status) = status(ctx, ui)
    {
        right = right.push(status);
    }
    right = right.push(measure).push(refresh);

    tab_toolbar::bar(t, left, right)
}

fn period_keys(period: Period) -> (Key, Key) {
    match period {
        Period::D7 => (Key::StatsPeriod7d, Key::StatsPeriodHint7d),
        Period::D30 => (Key::StatsPeriod30d, Key::StatsPeriodHint30d),
        Period::D90 => (Key::StatsPeriod90d, Key::StatsPeriodHint90d),
        Period::All => (Key::StatsPeriodAll, Key::StatsPeriodHintAll),
    }
}

/// The project menu: "All projects", a separator, then each project with a
/// check mark and its share.
fn project_entries(st: &State, ui: Ui, summary: &Summary) -> Vec<menu::Entry<Message>> {
    let locale = ui.locale();
    let mut entries: Vec<menu::Entry<Message>> = vec![
        menu::item(ui.s(Key::StatsAllProjects))
            .checked(st.project.is_none())
            .on_select(msg(Msg::SetProject(None)))
            .into(),
        menu::separator(),
    ];
    for row in &summary.projects {
        entries.push(
            menu::item(row.label.clone())
                .checked(st.project.as_deref() == Some(row.key.as_str()))
                .shortcut(format_share(row.share, locale))
                .on_select(msg(Msg::SetProject(Some(row.key.clone()))))
                .into(),
        );
    }
    entries
}

// ---------------------------------------------------------------------------
// States of the whole page
// ---------------------------------------------------------------------------

/// Nothing to show: where it looked, the first thing to check when a machine
/// with transcripts shows none.
fn empty_state<'a>(ui: Ui, data: &hangar_core::stats::UsageStats) -> Element<'a, Message> {
    let t = ui.t;
    let mut state = hangar_ui::empty::empty(t, Icon::Chart, ui.s(Key::StatsEmptyTitle))
        .description(ui.s(Key::StatsEmptyBody));
    if !data.sources.is_empty() {
        let mut sources = column![].spacing(space::XS);
        for source in &data.sources {
            let count = if source.found {
                ui.tr
                    .tn(Key::StatsSourceFiles, source.files, &[])
                    .into_owned()
            } else {
                ui.s(Key::StatsSourceMissing)
            };
            sources = sources.push(
                row![
                    container(
                        one_line(source.root.clone())
                            .size(font_size::CAPTION)
                            .font(fonts::mono())
                            .color(t.palette.text_secondary)
                    )
                    .width(Length::Fill)
                    .clip(true),
                    text(count)
                        .size(font_size::CAPTION)
                        .color(t.palette.text_muted),
                ]
                .spacing(space::XXL)
                .align_y(alignment::Vertical::Center),
            );
        }
        state = state.action(
            container(sources)
                .padding(Padding::from([space::MD, space::XL]))
                .style(style::container::chip(t)),
        );
    }
    state.into()
}

// ---------------------------------------------------------------------------
// Cards
// ---------------------------------------------------------------------------

/// A card of the page: a title, a quiet hint, and what it holds.
fn card<'a>(
    ui: Ui,
    title: String,
    hint: Option<String>,
    actions: Option<Element<'a, Message>>,
    body: Element<'a, Message>,
) -> Element<'a, Message> {
    let t = ui.t;
    let mut titles = row![
        one_line(title)
            .size(font_size::BODY)
            .font(fonts::ui_semibold())
            .color(t.palette.text)
    ]
    .spacing(space::MD)
    .align_y(alignment::Vertical::Bottom);
    if let Some(hint) = hint {
        titles = titles.push(
            one_line(hint)
                .size(font_size::CAPTION_LG)
                .color(t.palette.text_muted),
        );
    }
    let mut head = row![container(titles).width(Length::Fill).clip(true)]
        .spacing(space::XL)
        .align_y(alignment::Vertical::Center);
    if let Some(actions) = actions {
        head = head.push(actions);
    }
    container(column![head, body].spacing(space::XL))
        .padding(PAGE_PADDING)
        .width(Length::Fill)
        .style(ui::card(t))
        .into()
}

/// Chart or table: the same values, two ways to read them.
fn table_toggle<'a>(ui: Ui, table: bool, on_press: Message) -> Element<'a, Message> {
    let words = ui.s(if table {
        Key::StatsShowChart
    } else {
        Key::StatsShowTable
    });
    ui_button::toggle_button(ui.t, if table { Icon::Chart } else { Icon::Table }, table)
        .tip(words)
        .on_press(on_press)
        .into()
}

fn legend<'a>(ui: Ui) -> Element<'a, Message> {
    let t = ui.t;
    let mut items = row![]
        .spacing(space::XXL)
        .align_y(alignment::Vertical::Center);
    for kind in KINDS.iter().rev() {
        items = items.push(
            row![
                container(Space::new())
                    .width(8)
                    .height(8)
                    .style(ui::swatch(kind.color(t), 2.0)),
                text(ui.s(kind.key()))
                    .size(font_size::CAPTION_LG)
                    .color(t.palette.text_secondary),
            ]
            .spacing(space::SM)
            .align_y(alignment::Vertical::Center),
        );
    }
    items.into()
}

fn muted_line<'a>(ui: Ui, words: String) -> Element<'a, Message> {
    text(words)
        .size(font_size::UI)
        .color(ui.t.palette.text_muted)
        .into()
}

// ---------------------------------------------------------------------------
// Key figures
// ---------------------------------------------------------------------------

/// The trend under a key figure: "▲ +24 % vs the 30 days before".
fn trend<'a>(ui: Ui, now: f64, before: f64, days: i64) -> Element<'a, Message> {
    let t = ui.t;
    let locale = ui.locale();
    let line = |content: String| {
        text(content)
            .size(font_size::LABEL)
            .color(t.palette.text_muted)
    };
    let Some(ratio) = change(now, before) else {
        let words = if now > 0.0 {
            ui.tr
                .t_with(Key::StatsTrendNew, &args!("n" => days))
                .into_owned()
        } else {
            String::new()
        };
        return line(words).into();
    };
    // Rounded to nothing, it is no trend.
    if ratio.abs() < 0.005 {
        return line(
            ui.tr
                .t_with(Key::StatsTrendFlat, &args!("n" => days))
                .into_owned(),
        )
        .into();
    }
    // Up or down is not good or bad here (more work done, more tokens spent),
    // so the arrow wears no colour.
    row![
        text(if ratio > 0.0 { "▲" } else { "▼" })
            .size(font_size::LABEL * 0.72)
            .color(t.palette.text_muted),
        text(format_change(ratio, locale))
            .size(font_size::LABEL)
            .font(fonts::ui_semibold())
            .color(t.palette.text_muted),
        line(
            ui.tr
                .t_with(Key::StatsTrendVs, &args!("n" => days))
                .into_owned()
        ),
    ]
    .spacing(4)
    .align_y(alignment::Vertical::Center)
    .into()
}

struct Kpi {
    label: String,
    value: String,
    trend: Option<(f64, f64)>,
    detail: Option<String>,
    spark: Vec<f64>,
}

fn kpi<'a>(ui: Ui, kpi: Kpi, days: i64, since: Option<String>) -> Element<'a, Message> {
    let t = ui.t;
    let mut body = column![
        row![
            container(
                one_line(kpi.label)
                    .size(font_size::LABEL)
                    .font(fonts::ui_medium())
                    .color(t.palette.text_secondary)
            )
            .width(Length::Fill)
            .clip(true),
            sparkline(t, kpi.spark),
        ]
        .spacing(space::MD)
        .align_y(alignment::Vertical::Top),
        one_line(kpi.value)
            .size(28.0)
            .font(fonts::display())
            .color(t.palette.text),
    ]
    .spacing(3);
    body = body.push(Space::new().height(1));
    match (kpi.trend, &since) {
        (Some((now, before)), _) => body = body.push(trend(ui, now, before, days)),
        (None, Some(since)) => {
            body = body.push(
                text(since.clone())
                    .size(font_size::LABEL)
                    .color(t.palette.text_muted),
            );
        }
        _ => {}
    }
    if let Some(detail) = kpi.detail {
        body = body.push(
            text(detail)
                .size(font_size::LABEL)
                .color(t.palette.text_muted),
        );
    }
    container(body)
        .padding(PAGE_PADDING)
        .width(Length::Fill)
        .style(ui::card(t))
        .into()
}

// ---------------------------------------------------------------------------
// The page
// ---------------------------------------------------------------------------

fn content<'a>(
    st: &'a State,
    ctx: Ctx<'a>,
    ui: Ui,
    computed: &Arc<Computed>,
    data: &hangar_core::stats::UsageStats,
    width: f32,
) -> Element<'a, Message> {
    let t = ui.t;
    let locale = ui.locale();
    let live = &ctx.model.live.stats;
    let summary = &computed.summary;
    let page_width = width - 2.0 * PAGE_PADDING;
    let two_by_two = width < CARDS_2X2_BELOW;
    let one_column = width < ONE_COLUMN_BELOW;
    let days = summary.range.days;
    let measure = st.measure;
    let measure_name = ui.s(measure_key(measure));

    let mut page = column![].spacing(GAP);
    if let Some(error) = &live.error {
        page = page.push(
            container(
                row![
                    hangar_ui::glyph::state(t, hangar_ui::glyph::GlyphState::Failed, 14.0).still(),
                    text(format!("{} — {error}", ui.s(Key::StatsErrorTitle)))
                        .size(font_size::UI)
                        .color(t.palette.status.failed)
                        .width(Length::Fill),
                    ui_button::secondary(t, ui.s(Key::StatsRetry))
                        .icon(Icon::Restart)
                        .small()
                        .on_press(msg(Msg::Refresh)),
                ]
                .spacing(space::MD)
                .align_y(alignment::Vertical::Center),
            )
            .padding(Padding::from([space::MD, space::XL]))
            .width(Length::Fill)
            .style(style::container::badge_danger(t)),
        );
    }

    // Key figures.
    let totals = &summary.totals;
    let previous = summary.previous.as_ref();
    let since = (st.period == Period::All)
        .then_some(summary.first_at)
        .flatten()
        .map(|first| {
            ui.tr
                .t_with(
                    Key::StatsSince,
                    &args!("date" => format::long_date(first, locale)),
                )
                .into_owned()
        });
    let spark = |figure: &dyn Fn(&super::model::Totals) -> f64| -> Vec<f64> {
        summary
            .series
            .iter()
            .map(|point| figure(&point.totals))
            .collect()
    };
    let cache_share = if totals.tokens > 0 {
        totals.cache_read as f64 / totals.tokens as f64
    } else {
        0.0
    };
    let kpis = [
        Kpi {
            label: ui.s(Key::StatsKpiTokens),
            value: format_compact(totals.tokens as f64, locale),
            trend: previous.map(|before| (totals.tokens as f64, before.tokens as f64)),
            detail: Some(
                ui.tr
                    .t_with(
                        Key::StatsKpiTokensDetail,
                        &args!("share" => format_share(cache_share, locale)),
                    )
                    .into_owned(),
            ),
            spark: spark(&|totals| totals.tokens as f64),
        },
        Kpi {
            label: ui.s(Key::StatsKpiCost),
            value: format_usd(totals.cost, locale, false),
            trend: previous.map(|before| (totals.cost, before.cost)),
            detail: Some(ui.s(Key::StatsKpiCostNote)),
            spark: spark(&|totals| totals.cost),
        },
        Kpi {
            label: ui.s(Key::StatsKpiActive),
            value: format_duration(totals.active_ms as f64, ui.tr),
            trend: previous.map(|before| (totals.active_ms as f64, before.active_ms as f64)),
            detail: (summary.active_days > 0).then(|| {
                ui.tr
                    .t_with(
                        Key::StatsKpiActiveDetail,
                        &args!("value" => format_duration(
                            totals.active_ms as f64 / summary.active_days as f64,
                            ui.tr,
                        )),
                    )
                    .into_owned()
            }),
            spark: spark(&|totals| totals.active_ms as f64),
        },
        Kpi {
            label: ui.s(Key::StatsKpiSessions),
            value: format_integer(totals.sessions as f64, locale),
            trend: previous.map(|before| (totals.sessions as f64, before.sessions as f64)),
            detail: Some(ui.tr.tn(Key::StatsPrompts, totals.turns, &[]).into_owned()),
            spark: spark(&|totals| totals.sessions as f64),
        },
    ];
    let mut cells: Vec<Element<'a, Message>> = kpis
        .into_iter()
        .map(|figure| kpi(ui, figure, days, since.clone()))
        .collect();
    if two_by_two {
        let second = cells.split_off(2);
        let mut rows = column![].spacing(GAP);
        rows = rows.push(row(cells).spacing(GAP));
        rows = rows.push(row(second).spacing(GAP));
        page = page.push(rows);
    } else {
        page = page.push(row(cells).spacing(GAP));
    }

    // The period, column by column.
    let week = summary.granularity == Granularity::Week;
    let daily_title = ui.s(if week {
        Key::StatsWeeklyTitle
    } else {
        Key::StatsDailyTitle
    });
    let daily_hint = ui
        .tr
        .t_with(
            Key::StatsDailyHint,
            &args!("measure" => measure_name.as_str()),
        )
        .into_owned();
    let legend_inline = page_width >= LEGEND_BELOW_TITLE;
    let mut actions = row![]
        .spacing(space::XL)
        .align_y(alignment::Vertical::Center);
    if legend_inline {
        actions = actions.push(legend(ui));
    }
    actions = actions.push(table_toggle(ui, st.daily_table, msg(Msg::ToggleDailyTable)));
    let daily_body: Element<'a, Message> = if st.daily_table {
        tables::columns(ui, summary, measure)
    } else {
        Columns {
            theme: t,
            locale,
            measure,
            summary: summary.clone(),
            active: st.columns,
            keyboard: st.keyboard,
            focused: st.focus == Some(ChartId::Columns),
            empty: (ui.s(Key::StatsNoData), ui.s(Key::StatsNoDataHint)),
            on_event: |event| msg(Msg::Chart(ChartId::Columns, event)),
        }
        .view(ui.tr)
    };
    let daily_body: Element<'a, Message> = if legend_inline {
        daily_body
    } else {
        column![legend(ui), daily_body].spacing(space::XL).into()
    };
    page = page.push(card(
        ui,
        daily_title,
        Some(daily_hint),
        Some(actions.into()),
        daily_body,
    ));

    // Three columns (By project, Week rhythm, By model over Heaviest
    // sessions); one column, in that order, under 960 px.
    let mut first = column![projects_card(st, ui, summary)]
        .spacing(GAP)
        .width(Length::Fill);
    if both_agents(summary) {
        first = first.push(agents_card(st, ui, summary));
    }
    let second = column![heat_card(st, ui, summary)]
        .spacing(GAP)
        .width(Length::Fill);
    let third = column![
        models_card(st, ui, summary),
        sessions_card(ui, summary, measure, &measure_name)
    ]
    .spacing(GAP)
    .width(Length::Fill);
    if one_column {
        page = page.push(first).push(second).push(third);
    } else {
        page = page.push(
            row![first, second, third]
                .spacing(GAP)
                .align_y(alignment::Vertical::Top),
        );
    }

    page = page.push(method(ui, data));

    scrollable(
        container(container(page).max_width(PAGE_MAX_WIDTH))
            .width(Length::Fill)
            .center_x(Length::Fill)
            .padding(PAGE_PADDING),
    )
    .direction(Direction::Vertical(Scrollbar::new().width(size::SCROLLBAR)))
    .style(style::scrollable::default(t))
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn sessions_and_time(ui: Ui) -> impl Fn(&super::model::Ranked) -> String {
    move |row| {
        format!(
            "{} · {}",
            ui.tr
                .tn(Key::StatsSessions, row.totals.sessions, &[])
                .into_owned(),
            format_duration(row.totals.active_ms as f64, ui.tr)
        )
    }
}

fn projects_card<'a>(st: &State, ui: Ui, summary: &Summary) -> Element<'a, Message> {
    let rows = &summary.projects;
    let body: Element<'a, Message> = if rows.is_empty() {
        muted_line(ui, ui.s(Key::StatsNoData))
    } else {
        // Seven show; a project that is filtered on stays in sight.
        let foldable = rows.len() > PROJECTS_SHOWN + 1;
        let shown: Vec<_> = if st.all_projects || !foldable {
            rows.clone()
        } else {
            rows.iter()
                .enumerate()
                .filter(|(at, row)| *at < PROJECTS_SHOWN || st.project.as_deref() == Some(&row.key))
                .map(|(_, row)| row.clone())
                .collect()
        };
        let hidden = rows.len() - shown.len();
        let sub = sessions_and_time(ui);
        let selected = st.project.clone();
        let tip_ui = ui;
        let pick_tip = move |row: &super::model::Ranked| {
            if selected.as_deref() == Some(row.key.as_str()) {
                tip_ui.s(Key::StatsClearProject)
            } else {
                tip_ui
                    .tr
                    .t_with(Key::StatsShowOnly, &args!("name" => row.label.as_str()))
                    .into_owned()
            }
        };
        let mut body = column![ranked::list(
            ui,
            List {
                rows: &shown,
                measure: st.measure,
                selected: st.project.as_deref(),
                pick: Some(|key| msg(Msg::TogglePick(key))),
                pick_tip: Some(&pick_tip),
                sub: Some(&sub),
            },
        )]
        .spacing(space::MD);
        if foldable && (hidden > 0 || st.all_projects) {
            let words = if st.all_projects {
                ui.s(Key::StatsFewerProjects)
            } else {
                ui.tr.tn(Key::StatsMoreProjects, hidden, &[]).into_owned()
            };
            body = body.push(
                ui_button::ghost(ui.t, words)
                    .small()
                    .on_press(msg(Msg::ToggleAllProjects)),
            );
        }
        body.into()
    };
    card(
        ui,
        ui.s(Key::StatsProjectsTitle),
        Some(ui.s(Key::StatsProjectsHint)),
        None,
        body,
    )
}

/// Whether the period holds the work of more than one agent. Only then do
/// the "by agent" card and the agent headings of the model breakdown show: a
/// ranking of one says nothing (owner's decision, 2026-11-03), so a user of a
/// single agent has the page as it always was.
pub(super) fn both_agents(summary: &Summary) -> bool {
    summary.agents.len() > 1
}

/// The line under an agent's row: what the ranking's own figure leaves out
/// (the cost when it ranks tokens, the tokens when it ranks cost), then the
/// active time. A kind none of whose models has a price says so rather than
/// show a cost of nothing.
pub(super) fn agent_details(tr: Translator, row: &Ranked, measure: Measure) -> String {
    let locale = tr.locale();
    let other = match measure {
        Measure::Tokens if row.unpriced && row.totals.cost <= 0.0 => {
            tr.t(Key::StatsUnpriced).into_owned()
        }
        Measure::Tokens => format_usd(row.totals.cost, locale, false),
        Measure::Cost => tr
            .t_with(
                Key::StatsTokensValue,
                &args!("value" => format_compact(row.totals.tokens as f64, locale)),
            )
            .into_owned(),
    };
    let active = tr
        .t_with(
            Key::StatsSessionActive,
            &args!("value" => format_duration(row.totals.active_ms as f64, tr)),
        )
        .into_owned();
    format!("{other} · {active}")
}

/// Claude Code and Codex side by side: tokens, cost and active time of each.
fn agents_card<'a>(st: &State, ui: Ui, summary: &Summary) -> Element<'a, Message> {
    let measure = st.measure;
    let sub = move |row: &Ranked| agent_details(ui.tr, row, measure);
    let body = ranked::list(
        ui,
        List {
            rows: &summary.agents,
            measure,
            selected: None,
            pick: None,
            pick_tip: None,
            sub: Some(&sub),
        },
    );
    card(ui, ui.s(Key::StatsAgentsTitle), None, None, body)
}

/// The models of the period under the agent each runs under (see
/// [`model_agent`]), the agents in the order they rank, so the model names
/// read as "GPT-5.3 Codex" under Codex and "Opus 5.5" under Claude Code. An
/// agent none of whose models appears has no group.
pub(super) fn models_by_agent(summary: &Summary) -> Vec<(AgentKind, Vec<Ranked>)> {
    summary
        .agents
        .iter()
        .filter_map(|agent| AgentKind::from_id(&agent.key))
        .filter_map(|kind| {
            let rows: Vec<Ranked> = summary
                .models
                .iter()
                .filter(|row| model_agent(&row.key) == kind)
                .cloned()
                .collect();
            (!rows.is_empty()).then_some((kind, rows))
        })
        .collect()
}

/// The models the period's tokens went to. It shared a card with the agents
/// until Claude Code became the only one (owner's decision, 2026-11-03): a
/// ranking of one says nothing. With Codex back, the models fall under the
/// agent they ran in once both have done something in the period.
fn models_card<'a>(st: &State, ui: Ui, summary: &Summary) -> Element<'a, Message> {
    let t = ui.t;
    let mut body = column![].spacing(space::XL);
    let list = |rows: &[Ranked]| {
        ranked::list(
            ui,
            List {
                rows,
                measure: st.measure,
                selected: None,
                pick: None,
                pick_tip: None,
                sub: None,
            },
        )
    };
    if summary.models.is_empty() {
        body = body.push(muted_line(ui, ui.s(Key::StatsNoData)));
    } else if both_agents(summary) {
        for (kind, rows) in models_by_agent(summary) {
            body = body.push(
                column![
                    one_line(kind.info().label)
                        .size(font_size::CAPTION_LG)
                        .font(fonts::ui_semibold())
                        .color(t.palette.text_muted),
                    list(&rows),
                ]
                .spacing(space::SM),
            );
        }
    } else {
        body = body.push(list(&summary.models));
    }
    if summary.unpriced_tokens > 0 {
        body = body.push(
            text(
                ui.tr
                    .t_with(
                        Key::StatsUnpricedNote,
                        &args!("tokens" => format_compact(summary.unpriced_tokens as f64, ui.locale())),
                    )
                    .into_owned(),
            )
            .size(font_size::CAPTION_LG)
            .color(t.palette.text_muted),
        );
    }
    card(ui, ui.s(Key::StatsModelsTitle), None, None, body.into())
}

fn heat_card<'a>(st: &State, ui: Ui, summary: &Arc<Summary>) -> Element<'a, Message> {
    let title = ui.s(Key::StatsHeatTitle);
    let body: Element<'a, Message> = if st.heat_table {
        tables::heat(ui, summary)
    } else {
        Heatmap {
            theme: ui.t,
            locale: ui.locale(),
            summary: summary.clone(),
            active: st.heat,
            keyboard: st.keyboard,
            focused: st.focus == Some(ChartId::Heat),
            hours: [0usize, 6, 12, 18].map(|hour| {
                ui.tr
                    .t_with(Key::StatsHeatHour, &args!("h" => hour))
                    .into_owned()
            }),
            on_event: |event| msg(Msg::Chart(ChartId::Heat, event)),
        }
        .view(ui.tr)
    };
    card(
        ui,
        title,
        Some(ui.s(Key::StatsHeatHint)),
        Some(table_toggle(ui, st.heat_table, msg(Msg::ToggleHeatTable))),
        body,
    )
}

fn sessions_card<'a>(
    ui: Ui,
    summary: &Summary,
    measure: Measure,
    measure_name: &str,
) -> Element<'a, Message> {
    let hint = ui
        .tr
        .t_with(
            Key::StatsSessionsHint,
            &args!("measure" => measure_name.to_lowercase()),
        )
        .into_owned();
    card(
        ui,
        ui.s(Key::StatsSessionsTitle),
        Some(hint),
        None,
        sessions::list(ui, &summary.sessions, measure, |key| {
            msg(Msg::SetProject(Some(key)))
        }),
    )
}

/// The footer: where the numbers come from.
fn method<'a>(ui: Ui, data: &hangar_core::stats::UsageStats) -> Element<'a, Message> {
    let t = ui.t;
    let line = |words: String| {
        text(words)
            .size(font_size::CAPTION_LG)
            .color(t.palette.text_muted)
    };
    let mut lines = column![
        row![
            icon(Icon::Info, 13.0).color(t.palette.text_muted),
            text(ui.s(Key::StatsMethodTitle))
                .size(font_size::UI)
                .font(fonts::ui_semibold())
                .color(t.palette.text_muted),
        ]
        .spacing(6)
        .align_y(alignment::Vertical::Center),
        line(ui.s(Key::StatsMethodSources)),
        line(
            ui.tr
                .t_with(
                    Key::StatsMethodActive,
                    &args!("cap" => format_duration(data.active_gap_cap_ms as f64, ui.tr)),
                )
                .into_owned()
        ),
        line(ui.s(Key::StatsMethodCost)),
    ]
    .spacing(5);
    if !data.scan.cached_only {
        lines = lines.push(
            text(
                ui.tr
                    .t_with(
                        Key::StatsMethodScan,
                        &args!("files" => data.scan.files, "read" => data.scan.parsed, "ms" => data.scan.ms),
                    )
                    .into_owned(),
            )
            .size(font_size::LABEL)
            .font(fonts::mono())
            .color(t.palette.text_muted),
        );
    }
    container(lines)
        .max_width(780)
        .padding(Padding {
            top: 4.0,
            right: 2.0,
            bottom: 0.0,
            left: 2.0,
        })
        .into()
}
