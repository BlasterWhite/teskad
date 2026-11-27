//! The gauges of the mockups: a 4 px bar that says how full something is,
//! and always says it in figures too.
//!
//! - [`bar`]: the bar alone.
//! - [`context`]: a pane's context window, a 28 px bar and the percentage
//!   (the header of a pane).
//! - [`plan_row`]: one line of the plan card, `5 h  ▬▬▬▬  20%`.
//!
//! The bar is the palette's `status.active` over a `border` track; from 70 %
//! it turns the warning colour and from 90 % the failure one, and the figure
//! beside it is written in the same colour from then on, so the level never
//! rests on a colour alone.
//!
//! ```ignore
//! meter::context(t, 69).tip("Context used: 69 %")
//! meter::plan_row(t, "5 h", 20)
//! ```

use hangar_theme::tokens::space;
use hangar_theme::{Theme, fonts, style};
use iced::widget::text::Wrapping;
use iced::widget::{container, progress_bar, row, text};
use iced::{Alignment, Element, Length};

use crate::glyph::{GaugeLevel, gauge_level};
use crate::tip;

/// The thickness of a bar.
pub const THICKNESS: f32 = 4.0;
/// The width of the context bar of a pane's header.
pub const CONTEXT_WIDTH: f32 = 28.0;
/// The width the percentage of a row is given: `100 %`.
const FIGURE_WIDTH: f32 = 34.0;
/// The width of the label of a plan row (`5 h`, `7 d`).
const LABEL_WIDTH: f32 = 22.0;

/// The bar for a reading of `pct` percent (clamped to 100), filling the
/// width it is given.
pub fn bar<'a, M: 'a>(t: &'a Theme, pct: u8) -> Element<'a, M> {
    let value = f32::from(pct.min(100));
    let built = progress_bar(0.0..=100.0, value).girth(THICKNESS);
    match gauge_level(pct) {
        GaugeLevel::Ok => built.style(style::progress_bar::gauge(t)),
        GaugeLevel::Warn => built.style(style::progress_bar::warning(t)),
        GaugeLevel::Hot => built.style(style::progress_bar::gauge_danger(t)),
    }
    .into()
}

fn figure<'a, M: 'a>(
    t: &'a Theme,
    words: String,
    pct: u8,
    quiet: iced::Color,
    width: Length,
) -> Element<'a, M> {
    let ink = match gauge_level(pct) {
        GaugeLevel::Ok => quiet,
        GaugeLevel::Warn => t.colors.warning,
        GaugeLevel::Hot => t.palette.status.failed,
    };
    container(
        text(words)
            .size(11.0)
            .font(fonts::mono())
            .color(ink)
            .wrapping(Wrapping::None),
    )
    .align_x(iced::alignment::Horizontal::Right)
    .width(width)
    .into()
}

/// A pane's context window: a 28 px bar and the percentage in mono 11.
#[must_use]
pub struct Context<'a> {
    theme: &'a Theme,
    pct: u8,
    tip: Option<String>,
}

pub fn context(t: &Theme, pct: u8) -> Context<'_> {
    Context {
        theme: t,
        pct,
        tip: None,
    }
}

impl Context<'_> {
    /// The numbers behind the bar, in a tooltip.
    pub fn tip(mut self, words: impl Into<String>) -> Self {
        self.tip = Some(words.into());
        self
    }
}

impl<'a, M: 'a> From<Context<'a>> for Element<'a, M> {
    fn from(c: Context<'a>) -> Self {
        let t = c.theme;
        let content = row![
            container(bar(t, c.pct)).width(CONTEXT_WIDTH),
            figure(
                t,
                format!("{}%", c.pct),
                c.pct,
                t.palette.text_muted,
                Length::Shrink
            ),
        ]
        .spacing(space::SM)
        .align_y(Alignment::Center);
        match c.tip {
            Some(words) => tip::bottom(t, content, words),
            None => content.into(),
        }
    }
}

/// One line of the plan card: the window's name, the bar filling the line,
/// and the percentage.
pub fn plan_row<'a, M: 'a>(t: &'a Theme, label: impl Into<String>, pct: u8) -> Element<'a, M> {
    plan_row_with(t, label, format!("{pct}%"), pct)
}

/// [`plan_row`] with the percentage written by the caller, in its own
/// language (`20 %` in French).
pub fn plan_row_with<'a, M: 'a>(
    t: &'a Theme,
    label: impl Into<String>,
    words: impl Into<String>,
    pct: u8,
) -> Element<'a, M> {
    row![
        container(
            text(label.into())
                .size(11.0)
                .font(fonts::mono())
                .color(t.palette.text_secondary)
                .wrapping(Wrapping::None),
        )
        .width(LABEL_WIDTH),
        container(bar(t, pct)).width(Length::Fill),
        figure(
            t,
            words.into(),
            pct,
            t.palette.text_secondary,
            Length::Fixed(FIGURE_WIDTH)
        ),
    ]
    .spacing(space::MD)
    .align_y(Alignment::Center)
    .into()
}

#[cfg(test)]
mod tests;
