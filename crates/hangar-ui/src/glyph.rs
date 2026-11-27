//! An agent's state, drawn the same everywhere (`StateGlyph.tsx`), and the
//! context gauge of a pane (`PaneGauge.tsx`).
//!
//! Colour and shape both carry the state, so it reads without telling green
//! from amber, as the Teskad brand pack's status icons ask: an arc turning
//! once every 1.4 s while working (still when the system asks for less
//! motion), a solid dot when it is your turn (the pack's "active"), an empty
//! ring when idle (its "inactive"), a ticked disc when completed and a crossed
//! disc when failed. A diamond keeps "waiting for an answer", which the pack
//! has no icon for. `Starting` and `Exited` are the shell's own states, before
//! or after there is an agent to report on.
//!
//! ```ignore
//! use hangar_ui::glyph::{self, GlyphState};
//!
//! row![glyph::state(t, GlyphState::Working, 8.0), text("Ben")]
//! glyph::gauge(t, 72).tip("72 % of 200k tokens")
//! ```

use std::f32::consts::TAU;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use hangar_theme::tokens::{font_size, space};
use hangar_theme::{AgentState, Theme};
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad};
use iced::advanced::svg;
use iced::advanced::widget::Tree;
use iced::advanced::{Shell, Widget};
use iced::mouse;
use iced::widget::{canvas, row, text};
use iced::{
    Alignment, Background, Border, Color, Element, Event, Length, Point, Radians, Rectangle, Size,
    window,
};

use crate::tip;

/// The states a glyph can show: an agent's four, the shell's two, and the
/// outcome of a task or a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlyphState {
    Working,
    Waiting,
    Yours,
    Idle,
    /// The shell is coming up.
    Starting,
    /// The process ended.
    Exited,
    /// A task or a step that went through.
    Completed,
    /// A task or a step that did not.
    Failed,
}

impl From<AgentState> for GlyphState {
    fn from(state: AgentState) -> Self {
        match state {
            AgentState::Working => GlyphState::Working,
            AgentState::Waiting => GlyphState::Waiting,
            AgentState::Yours => GlyphState::Yours,
            AgentState::Idle => GlyphState::Idle,
        }
    }
}

impl GlyphState {
    /// Whether the glyph moves, and so needs the window redrawn to keep going.
    pub fn animates(self) -> bool {
        matches!(self, GlyphState::Working | GlyphState::Starting)
    }
}

/// The colours a glyph is drawn in, from the theme or from the row it sits on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tint {
    /// The fill of the dot, the diamond, the ring or the disc.
    pub fill: Color,
    /// The tick or the cross drawn inside a disc: the theme's background.
    pub ink: Color,
}

impl Tint {
    pub fn of(t: &Theme, state: GlyphState) -> Tint {
        let fill = match state {
            GlyphState::Working => t.inks.running.fill,
            GlyphState::Waiting => t.inks.waiting.fill,
            GlyphState::Yours => t.inks.yours.fill,
            GlyphState::Idle | GlyphState::Exited | GlyphState::Starting => t.inks.idle.fill,
            GlyphState::Completed => t.inks.success(),
            GlyphState::Failed => t.inks.danger(),
        };
        Tint {
            fill,
            ink: t.palette.bg,
        }
    }
}

/// Whether the person asked the system for less motion, as the app read it.
static REDUCED_MOTION: AtomicBool = AtomicBool::new(false);

/// Tells every glyph whether to hold still: the arc of a working glyph stops
/// and a starting ring stays lit.
pub fn set_reduced_motion(reduced: bool) {
    REDUCED_MOTION.store(reduced, Ordering::Relaxed);
}

/// Whether glyphs hold still.
pub fn reduced_motion() -> bool {
    REDUCED_MOTION.load(Ordering::Relaxed)
}

/// Where a turn of `period` is, from 0 to 1. All glyphs on screen move
/// together, which reads as one rhythm and not as a swarm.
fn turn(period: Duration) -> f32 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let start = START.get_or_init(Instant::now);
    (start.elapsed().as_secs_f32() % period.as_secs_f32()) / period.as_secs_f32()
}

/// One turn of a working glyph's arc: 1.4 s, linear.
const SPIN: Duration = Duration::from_millis(1400);

/// The angle of the working arc at `turn` (0 to 1) of its spin, in radians.
pub fn spin_angle(turn: f32) -> f32 {
    turn * TAU
}

/// Whether a starting glyph is lit at `turn` of its 1.2 s blink.
pub fn blink_lit(turn: f32) -> bool {
    turn < 0.5
}

/// A rounded square turned a quarter, for the waiting state: drawn from an SVG
/// since a quad cannot be rotated.
static DIAMOND: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(
        concat!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10">"##,
            r##"<rect x="1.47" y="1.47" width="7.06" height="7.06" rx="1.3" "##,
            r##"transform="rotate(45 5 5)" fill="#000"/></svg>"##
        )
        .as_bytes()
        .to_vec(),
    )
});

/// A quarter of a ring, from twelve to three o'clock, on the pack's 16-unit
/// box: the working glyph's arc, turned by the renderer.
static ARC: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(
        concat!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none">"##,
            r##"<path d="M8 1.2a6.8 6.8 0 0 1 6.8 6.8" stroke="#000" stroke-width="2.4" "##,
            r##"stroke-linecap="round"/></svg>"##
        )
        .as_bytes()
        .to_vec(),
    )
});

/// The tick of a completed glyph, on the same box.
static TICK: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(
        concat!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none">"##,
            r##"<path d="M5 8.2 7.2 10.3 11 6" stroke="#000" stroke-width="1.8" "##,
            r##"stroke-linecap="round" stroke-linejoin="round"/></svg>"##
        )
        .as_bytes()
        .to_vec(),
    )
});

/// The cross of a failed glyph.
static CROSS: LazyLock<svg::Handle> = LazyLock::new(|| {
    svg::Handle::from_memory(
        concat!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none">"##,
            r##"<path d="M5.5 5.5l5 5m0-5l-5 5" stroke="#000" stroke-width="1.8" "##,
            r##"stroke-linecap="round"/></svg>"##
        )
        .as_bytes()
        .to_vec(),
    )
});

/// Draws `handle` in `color` over `rect`, turned by `angle` about its centre.
fn symbol<R: svg::Renderer>(
    renderer: &mut R,
    handle: &svg::Handle,
    rect: Rectangle,
    color: Color,
    angle: f32,
) {
    renderer.draw_svg(
        svg::Svg {
            handle: handle.clone(),
            color: Some(Color { a: 1.0, ..color }),
            rotation: Radians(angle),
            opacity: color.a,
        },
        rect,
        rect,
    );
}

/// Draws the glyph of `state` in the square `bounds`.
///
/// `animate` is off for a glyph that is part of a static drawing (a menu
/// line), where a pulse would only be noise.
pub fn draw<R: iced::advanced::Renderer + svg::Renderer>(
    renderer: &mut R,
    state: GlyphState,
    bounds: Rectangle,
    tint: Tint,
    animate: bool,
) {
    let side = bounds.width.min(bounds.height);
    let centre = bounds.center();
    let dot = Rectangle::new(
        Point::new(centre.x - side / 2.0, centre.y - side / 2.0),
        Size::new(side, side),
    );
    let disc = |renderer: &mut R, rect: Rectangle, fill: Color| {
        renderer.fill_quad(
            Quad {
                bounds: rect,
                border: Border {
                    radius: (rect.width / 2.0).into(),
                    ..Border::default()
                },
                ..Quad::default()
            },
            Background::Color(fill),
        );
    };
    let ring = |renderer: &mut R, rect: Rectangle, width: f32, color: Color| {
        renderer.fill_quad(
            Quad {
                bounds: rect,
                border: Border {
                    radius: (rect.width / 2.0).into(),
                    width,
                    color,
                },
                ..Quad::default()
            },
            Background::Color(Color::TRANSPARENT),
        );
    };

    match state {
        GlyphState::Working => {
            // A faint track and the arc on it, turning while it can.
            ring(renderer, dot, side * 0.15, tint.fill.scale_alpha(0.25));
            let angle = if animate && !reduced_motion() {
                spin_angle(turn(SPIN))
            } else {
                0.0
            };
            symbol(renderer, &ARC, dot, tint.fill, angle);
        }
        GlyphState::Completed => {
            disc(renderer, dot, tint.fill);
            symbol(renderer, &TICK, dot, tint.ink, 0.0);
        }
        GlyphState::Failed => {
            disc(renderer, dot, tint.fill);
            symbol(renderer, &CROSS, dot, tint.ink, 0.0);
        }
        GlyphState::Yours => disc(renderer, dot, tint.fill),
        GlyphState::Waiting => {
            // 0.86 of the dot's box turned a quarter is 1.22 of it across.
            let across = side * 1.216;
            let rect = Rectangle::new(
                Point::new(centre.x - across / 2.0, centre.y - across / 2.0),
                Size::new(across, across),
            );
            renderer.draw_svg(
                svg::Svg {
                    handle: DIAMOND.clone(),
                    color: Some(Color {
                        a: 1.0,
                        ..tint.fill
                    }),
                    rotation: Radians(0.0),
                    opacity: tint.fill.a,
                },
                rect,
                rect,
            );
        }
        GlyphState::Idle | GlyphState::Exited => ring(renderer, dot, 1.5, tint.fill),
        GlyphState::Starting => {
            let lit = !animate || reduced_motion() || blink_lit(turn(Duration::from_millis(1200)));
            ring(
                renderer,
                dot,
                1.5,
                if lit {
                    tint.fill
                } else {
                    tint.fill.scale_alpha(0.25)
                },
            );
        }
    }
}

/// The widget [`state`] makes.
pub struct StateGlyph {
    state: GlyphState,
    side: f32,
    tint: Tint,
    animate: bool,
}

impl<Message, Theme> Widget<Message, Theme, iced::Renderer> for StateGlyph {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fixed(self.side), Length::Fixed(self.side))
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::atomic(limits, self.side, self.side)
    }

    fn update(
        &mut self,
        _tree: &mut Tree,
        event: &Event,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn iced::advanced::Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        // The motion is drawn from the clock, so the window only has to be
        // asked to redraw: about 25 times a second, which is smooth enough
        // for an arc that turns in 1.4 s and cheap enough for a window of
        // sixteen panes. A glyph held still, or a system asking for less
        // motion, asks for nothing.
        if self.animate
            && !reduced_motion()
            && self.state.animates()
            && let Event::Window(window::Event::RedrawRequested(now)) = event
        {
            shell.request_redraw_at(*now + Duration::from_millis(40));
        }
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        draw(
            renderer,
            self.state,
            layout.bounds(),
            self.tint,
            self.animate,
        );
    }
}

impl<'a, Message: 'a, Theme: 'a> From<StateGlyph> for Element<'a, Message, Theme, iced::Renderer> {
    fn from(glyph: StateGlyph) -> Self {
        Element::new(glyph)
    }
}

impl StateGlyph {
    /// Draws in this colour instead of the state's own: on the accent of a
    /// chosen row, the glyph keeps its shape in the colour that reads there.
    pub fn tint(mut self, color: Color) -> Self {
        self.tint.fill = color;
        self
    }

    /// Draws the glyph at rest, for a mark in a log or a list of what
    /// happened, where a turning arc would say something is still going on.
    pub fn still(mut self) -> Self {
        self.animate = false;
        self
    }
}

/// The glyph of `state`, `side` pixels across (8 in the pane header, 7 in the
/// navigator and the menus), the one drawing of a state wherever it shows.
pub fn state(t: &Theme, state: GlyphState, side: f32) -> StateGlyph {
    StateGlyph {
        state,
        side,
        tint: Tint::of(t, state),
        animate: true,
    }
}

// ---- The context gauge -----------------------------------------------------------------------

/// Amber from here, in percent: the agent compacts on its own near the limit,
/// and these say how near that is.
pub const GAUGE_WARN: u8 = 70;
/// Red from here.
pub const GAUGE_HOT: u8 = 90;

/// How worrying a gauge's reading is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GaugeLevel {
    Ok,
    Warn,
    Hot,
}

/// The level of a context window `pct` percent full (`gaugeLevel`).
pub fn gauge_level(pct: u8) -> GaugeLevel {
    if pct >= GAUGE_HOT {
        GaugeLevel::Hot
    } else if pct >= GAUGE_WARN {
        GaugeLevel::Warn
    } else {
        GaugeLevel::Ok
    }
}

/// The ring on its 12-unit box: the radius, and the thickness of its stroke.
const RING_RADIUS: f32 = 4.6 / 12.0;
const RING_STROKE: f32 = 2.2 / 12.0;

/// The ring of a gauge, drawn on a canvas.
struct Ring {
    pct: u8,
    track: Color,
    value: Color,
}

impl<Message> canvas::Program<Message> for Ring {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let centre = frame.center();
        let radius = bounds.width * RING_RADIUS;
        let width = bounds.width * RING_STROKE;
        frame.stroke(
            &canvas::Path::circle(centre, radius),
            canvas::Stroke::default()
                .with_width(width)
                .with_color(self.track),
        );
        let fraction = f32::from(self.pct.min(100)) / 100.0;
        if fraction >= 1.0 {
            frame.stroke(
                &canvas::Path::circle(centre, radius),
                canvas::Stroke::default()
                    .with_width(width)
                    .with_color(self.value),
            );
        } else if fraction > 0.0 {
            // The ring starts at twelve o'clock and fills clockwise.
            let start = -std::f32::consts::FRAC_PI_2;
            let arc = canvas::Path::new(|path| {
                path.arc(canvas::path::Arc {
                    center: centre,
                    radius,
                    start_angle: Radians(start),
                    end_angle: Radians(start + std::f32::consts::TAU * fraction),
                });
            });
            frame.stroke(
                &arc,
                canvas::Stroke::default()
                    .with_width(width)
                    .with_color(self.value)
                    .with_line_cap(canvas::LineCap::Round),
            );
        }
        vec![frame.into_geometry()]
    }
}

/// A pane's context gauge.
#[must_use]
pub struct Gauge<'a> {
    theme: &'a Theme,
    pct: u8,
    tip: Option<String>,
    /// The words beside the ring: the percentage by default, the caller's
    /// own (a translated `{pct} %`), or nothing in a narrow header.
    label: Option<Option<String>>,
}

/// How full the agent's context window is: a ring and a percentage, sized to
/// survive an 8-pane layout. Amber from 70 %, red from 90 %.
pub fn gauge(t: &Theme, pct: u8) -> Gauge<'_> {
    Gauge {
        theme: t,
        pct,
        tip: None,
        label: None,
    }
}

impl Gauge<'_> {
    /// The numbers behind the ring, in a tooltip.
    pub fn tip(mut self, words: impl Into<String>) -> Self {
        self.tip = Some(words.into());
        self
    }

    /// What the ring is labelled with instead of the bare percentage: the
    /// app's translated figure, or `None` for the ring alone, which a narrow
    /// pane header keeps once the figure no longer fits.
    pub fn label(mut self, words: Option<String>) -> Self {
        self.label = Some(words);
        self
    }
}

impl<'a, M: 'a> From<Gauge<'a>> for Element<'a, M> {
    fn from(g: Gauge<'a>) -> Self {
        let (ink, ring) = match gauge_level(g.pct) {
            GaugeLevel::Ok => (g.theme.inks.text.muted, g.theme.inks.text.muted),
            GaugeLevel::Warn => (g.theme.inks.warning(), g.theme.inks.warning()),
            GaugeLevel::Hot => (g.theme.inks.danger(), g.theme.inks.danger()),
        };
        let ring = canvas(Ring {
            pct: g.pct,
            track: g.theme.palette.border_strong,
            value: ring,
        })
        .width(12.0)
        .height(12.0);
        let words = match g.label {
            None => Some(format!("{} %", g.pct)),
            Some(label) => label,
        };
        let mut content = row![ring].spacing(space::XS).align_y(Alignment::Center);
        if let Some(words) = words {
            content = content.push(
                text(words)
                    .size(font_size::CAPTION_LG)
                    .style(move |_: &iced::Theme| text::Style { color: Some(ink) }),
            );
        }
        match g.tip {
            Some(words) => tip::bottom(g.theme, content, words),
            None => content.into(),
        }
    }
}

#[cfg(test)]
mod tests;
