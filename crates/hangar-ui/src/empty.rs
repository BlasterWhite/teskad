//! An empty state: what a list or a page says when it has nothing in it, and
//! what to do about it.
//!
//! ```ignore
//! use hangar_ui::{button, empty, Icon};
//!
//! empty::empty(t, Icon::Board, "No tasks yet")
//!     .description("Agents add tasks here as they plan the work.")
//!     .action(button::primary(t, "New task").on_press(Msg::NewTask))
//!
//! // An error: the failure mark, and always a way to try again.
//! empty::failed(t, "Couldn't read the board")
//!     .description("The board file could not be read.")
//!     .action(button::secondary(t, "Retry").on_press(Msg::Retry))
//!
//! // A read in progress: the turning arc and a sentence.
//! empty::loading(t, "Reading the transcripts…")
//! ```
//!
//! One format for every state (`ecrans.md` §11): a 36 px mark on a
//! `surfaceRaised` pastille of radius 10, a 14/600 title, one 12 `textMuted`
//! sentence, and at most two actions, a main one and a secondary one.
//! [`skeleton`] draws the still bars a loading state stands in the shape of
//! the content with: no glow, nothing moving but the arc.

use hangar_theme::tokens::{font_size, space};
use hangar_theme::{Theme, fonts};
use iced::widget::{column, container, row, text};
use iced::{Alignment, Background, Border, Element, Length, Padding};

use crate::glyph::{self, GlyphState};
use crate::icon::{Icon, icon};

/// The pastille behind the mark: 36 px square, radius 10.
const PASTILLE: f32 = 36.0;
const PASTILLE_RADIUS: f32 = 10.0;
/// The mark inside it.
const MARK: f32 = 18.0;

/// What the mark of a state shows.
enum Mark {
    /// A glyph of the icon set, in the muted colour.
    Icon(Icon),
    /// The failure indicator: its colour and its form.
    Failed,
    /// The arc of a read in progress.
    Working,
}

#[must_use]
pub struct EmptyState<'a, M> {
    theme: &'a Theme,
    mark: Mark,
    title: String,
    description: Option<String>,
    action: Option<Element<'a, M>>,
    secondary: Option<Element<'a, M>>,
    fill: bool,
}

fn state<'a, M: 'a>(t: &'a Theme, mark: Mark, title: impl Into<String>) -> EmptyState<'a, M> {
    EmptyState {
        theme: t,
        mark,
        title: title.into(),
        description: None,
        action: None,
        secondary: None,
        fill: true,
    }
}

/// An empty state with a glyph and a title.
pub fn empty<'a, M: 'a>(t: &'a Theme, icon: Icon, title: impl Into<String>) -> EmptyState<'a, M> {
    state(t, Mark::Icon(icon), title)
}

/// A read that went wrong: the failure indicator, and it is meant to carry an
/// action (Retry, Read again).
pub fn failed<'a, M: 'a>(t: &'a Theme, title: impl Into<String>) -> EmptyState<'a, M> {
    state(t, Mark::Failed, title)
}

/// Something is being read: the arc that turns, and a sentence.
pub fn loading<'a, M: 'a>(t: &'a Theme, title: impl Into<String>) -> EmptyState<'a, M> {
    state(t, Mark::Working, title)
}

/// A bar of a skeleton: the shape of a line or a block of the content to
/// come, still, in `surfaceRaised`.
pub fn skeleton<'a, M: 'a>(t: &'a Theme, width: Length, height: f32) -> Element<'a, M> {
    let color = t.palette.surface_raised;
    container(iced::widget::Space::new())
        .width(width)
        .height(height)
        .style(move |_: &iced::Theme| container::Style {
            background: Some(Background::Color(color)),
            border: Border {
                radius: hangar_theme::tokens::scale::radius::SM.into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

impl<'a, M: 'a> EmptyState<'a, M> {
    /// A sentence under the title.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// The main thing to do about it: a button.
    pub fn action(mut self, action: impl Into<Element<'a, M>>) -> Self {
        self.action = Some(action.into());
        self
    }

    /// The other thing to do, or a link: beside the main action. A state has
    /// no more than these two.
    pub fn secondary(mut self, action: impl Into<Element<'a, M>>) -> Self {
        self.secondary = Some(action.into());
        self
    }

    /// Takes the room its content needs and no more, for a state inside a
    /// list or a card instead of a whole page.
    pub fn inline(mut self) -> Self {
        self.fill = false;
        self
    }
}

impl<'a, M: 'a> From<EmptyState<'a, M>> for Element<'a, M> {
    fn from(e: EmptyState<'a, M>) -> Self {
        let t = e.theme;
        let p = &t.palette;
        let mark: Element<'a, M> = match e.mark {
            Mark::Icon(glyph) => icon(glyph, MARK).color(p.text_secondary).into(),
            Mark::Failed => glyph::state(t, GlyphState::Failed, MARK).still().into(),
            Mark::Working => glyph::state(t, GlyphState::Working, MARK).into(),
        };
        let (surface, text_color, muted) = (p.surface_raised, p.text, p.text_muted);
        let pastille = container(mark)
            .center(PASTILLE)
            .style(move |_: &iced::Theme| container::Style {
                background: Some(Background::Color(surface)),
                border: Border {
                    radius: PASTILLE_RADIUS.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            });
        let mut stack = column![
            pastille,
            text(e.title)
                .size(font_size::BODY)
                .font(fonts::ui_semibold())
                .color(text_color)
                .align_x(iced::alignment::Horizontal::Center),
        ]
        .spacing(space::LG)
        .align_x(Alignment::Center);
        if let Some(description) = e.description {
            stack = stack.push(
                text(description)
                    .size(font_size::LABEL)
                    .color(muted)
                    .align_x(iced::alignment::Horizontal::Center)
                    .width(Length::Shrink),
            );
        }
        if e.action.is_some() || e.secondary.is_some() {
            let actions = row![]
                .push(e.action)
                .push(e.secondary)
                .spacing(space::MD)
                .align_y(Alignment::Center);
            stack = stack.push(container(actions).padding(Padding {
                top: space::XS,
                ..Padding::ZERO
            }));
        }
        let body = container(stack).max_width(360.0);
        let page = container(body)
            .padding(space::XXXL)
            .style(move |_: &iced::Theme| container::Style {
                text_color: Some(text_color),
                ..container::Style::default()
            });
        if e.fill {
            page.width(Length::Fill)
                .height(Length::Fill)
                .center(Length::Fill)
                .into()
        } else {
            page.width(Length::Fill).center_x(Length::Fill).into()
        }
    }
}
