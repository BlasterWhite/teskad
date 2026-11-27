//! Popovers: the small raised surface that opens beside a control and holds
//! more than a list of lines: the session's "Details" under a pane's header,
//! a question with buttons, a few settings.
//!
//! The surface is `surfaceRaised` with a `border` hairline, corners of 8 and
//! the popover shadow (`style::container::popover`), the same one a menu
//! sits on. A popover is a [`Layer`] like every floating thing: open while it
//! is given content, asking to be closed (Escape, a press elsewhere) and never
//! closing itself. Tab stays inside it, wrapping round, so the keyboard
//! reaches each control of a popover and nothing under it.
//!
//! ```ignore
//! use hangar_ui::popover;
//!
//! let details = popover::popover(t, column![
//!     popover::row(t, "Shell", text::mono(t, "bash")),
//!     popover::row(t, "Session", copy_button),
//! ])
//! .width(264.0);
//! popover::under(t, details_button, self.details_open.then_some(details))
//!     .on_dismiss(Msg::CloseDetails)
//! ```

use hangar_theme::tokens::{font_size, space};
use hangar_theme::{Theme, style};
use iced::widget::{container, row as iced_row, space as iced_space};
use iced::{Alignment, Element, Length, Padding};

use crate::layer::{self, Layer};
use crate::text as typography;

/// The width of the session's Details popover, the reference's 264.
pub const WIDTH: f32 = 264.0;

/// A popover's contents on its surface. Becomes an [`Element`] with `.into()`.
#[must_use]
pub struct Popover<'a, M> {
    theme: &'a Theme,
    content: Element<'a, M>,
    width: Length,
    padding: Padding,
}

/// `content` on a popover's surface.
pub fn popover<'a, M: 'a>(t: &'a Theme, content: impl Into<Element<'a, M>>) -> Popover<'a, M> {
    Popover {
        theme: t,
        content: content.into(),
        width: Length::Shrink,
        padding: Padding::new(space::LG),
    }
}

impl<M> Popover<'_, M> {
    /// A fixed width in pixels ([`WIDTH`] for the Details popover).
    pub fn width(mut self, width: f32) -> Self {
        self.width = Length::Fixed(width);
        self
    }

    /// The space between the edge and the content, in pixels.
    pub fn padding(mut self, padding: f32) -> Self {
        self.padding = Padding::new(padding);
        self
    }
}

impl<'a, M: 'a> From<Popover<'a, M>> for Element<'a, M> {
    fn from(p: Popover<'a, M>) -> Self {
        container(p.content)
            .padding(p.padding)
            .width(p.width)
            .style(style::container::popover(p.theme))
            .into()
    }
}

/// A popover under `anchor`, open while `popover` is `Some`, with its left
/// edge on the anchor's ([`Layer::align_end`] for one at the right end of a
/// bar). Escape and a press elsewhere ask for
/// [`on_dismiss`](Layer::on_dismiss); Tab stays inside, and the first field
/// has the keyboard when it opens.
pub fn under<'a, M: Clone + 'a>(
    anchor: impl Into<Element<'a, M>>,
    popover: Option<Popover<'a, M>>,
) -> Layer<'a, M> {
    layer::dropdown(anchor, popover.map(Element::from))
        .tab_inside()
        .autofocus()
}

/// A line of a popover: a muted label at the left and its control or value at
/// the right.
pub fn row<'a, M: 'a>(
    t: &'a Theme,
    label: impl Into<String>,
    value: impl Into<Element<'a, M>>,
) -> Element<'a, M> {
    iced_row![
        typography::label(t, label.into()).size(font_size::LABEL),
        iced_space::horizontal(),
        value.into(),
    ]
    .spacing(space::MD)
    .align_y(Alignment::Center)
    .width(Length::Fill)
    .into()
}

#[cfg(test)]
mod tests;
