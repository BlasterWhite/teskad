//! The settings form: rows of a label and its control, grouped in cards under
//! a heading (`.set-row`, `.set-group`, `.set-section`).
//!
//! ```ignore
//! use hangar_ui::{form, input, Icon};
//!
//! form::section(t, "Appearance")
//!     .hint("How the window looks.")
//!     .group(vec![
//!         form::row(t, "Interface size", input::slider(t, 90.0..=130.0, s, Msg::Scale).display("100 %"))
//!             .hint("Scales every size in the window.")
//!             .modified(s != 100.0)
//!             .on_reset(Msg::ResetScale)
//!             .wide()
//!             .into(),
//!         form::row(t, "Reduce motion", input::switch(t, on).on_toggle(Msg::Motion)).into(),
//!     ])
//! ```
//!
//! A row has the label and an optional hint at the left and its control at
//! the right. A row whose value differs from its default shows a reset
//! button, in place, so that undoing a change never means finding the
//! setting's default somewhere else. Controls that take a width (a field, a
//! drop-down, a slider) ask for `.wide()` so that a page of them lines up
//! down its right edge.
//!
//! Also here: [`block`], for a control that is too big for a row, such as a
//! preview or a grid of themes, and [`group`], for the card around rows.

use hangar_theme::tokens::{font_size, space};
use hangar_theme::{Theme, fonts, style};
use iced::widget::text::Text;
use iced::widget::{column, container, rule, text};
use iced::{Alignment, Background, Color, Element, Length, Padding};

use crate::badge;
use crate::button::icon_button;
use crate::icon::Icon;

/// The room above and below a row's text: 14 px, as in the mockups.
const ROW_PADDING_Y: f32 = 14.0;

/// The least height of a row.
const MIN_ROW_HEIGHT: f32 = 48.0;

/// The width a row's control takes when it asks for [`Row::wide`]: a slider's
/// 180 px track, its gap and its value.
const WIDE: f32 = 248.0;

/// The widest a row's description runs before it wraps.
const HINT_MAX_WIDTH: f32 = 560.0;

/// Between two sections of a page.
const SECTION_GAP: f32 = 20.0;

/// The room at both sides of a page.
const PAGE_PADDING_X: f32 = 32.0;

/// How far from `surface` towards `surface_raised` a block is filled: a step
/// lighter than the window it lies in.
const BLOCK_LIFT: f32 = 0.4;

/// `from` taken `amount` of the way (0 to 1) to `to`: the steps between two
/// colours of the palette that the mockups use for a block or a side column.
pub fn between(from: Color, to: Color, amount: f32) -> Color {
    let mix = |a: f32, b: f32| a + (b - a) * amount;
    Color {
        r: mix(from.r, to.r),
        g: mix(from.g, to.g),
        b: mix(from.b, to.b),
        a: mix(from.a, to.a),
    }
}

/// What a block of settings is filled with.
pub fn block_fill(t: &Theme) -> Color {
    between(t.palette.surface, t.palette.surface_raised, BLOCK_LIFT)
}

/// What a row is called: 13 px, medium.
pub fn label<'a>(t: &'a Theme, content: impl text::IntoFragment<'a>) -> Text<'a> {
    Text::new(content)
        .size(font_size::UI)
        .font(fonts::ui_medium())
        .color(t.palette.text)
}

/// What a row or a section does: 12 px, muted.
pub fn hint<'a>(t: &'a Theme, content: impl text::IntoFragment<'a>) -> Text<'a> {
    Text::new(content)
        .size(font_size::LABEL)
        .color(t.palette.text_muted)
}

/// A section's small title: 12 px, semibold, muted.
pub fn title<'a>(t: &'a Theme, content: impl text::IntoFragment<'a>) -> Text<'a> {
    Text::new(content)
        .size(font_size::LABEL)
        .font(fonts::ui_semibold())
        .color(t.palette.text_muted)
}

/// A row of the form.
#[must_use]
pub struct Row<'a, M> {
    theme: &'a Theme,
    label: String,
    hint: Option<String>,
    control: Element<'a, M>,
    wide: bool,
    modified: bool,
    on_reset: Option<M>,
    reset_tip: String,
    badge: Option<String>,
}

/// A row labelled `label` with `control` at its right.
pub fn row<'a, M: 'a>(
    t: &'a Theme,
    label: impl Into<String>,
    control: impl Into<Element<'a, M>>,
) -> Row<'a, M> {
    Row {
        theme: t,
        label: label.into(),
        hint: None,
        control: control.into(),
        wide: false,
        modified: false,
        on_reset: None,
        reset_tip: String::from("Reset to default"),
        badge: None,
    }
}

impl<'a, M: 'a> Row<'a, M> {
    /// A line under the label saying what the setting does.
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// The outlined badge after the label, of a setting that is not finished
    /// ("BETA").
    pub fn badge(mut self, words: impl Into<String>) -> Self {
        self.badge = Some(words.into());
        self
    }

    /// Gives the control the form's common width, 248 px.
    pub fn wide(mut self) -> Self {
        self.wide = true;
        self
    }

    /// The value differs from its default: the reset button shows.
    pub fn modified(mut self, modified: bool) -> Self {
        self.modified = modified;
        self
    }

    /// Sent by the reset button, which only shows while the row is
    /// [`modified`](Row::modified).
    pub fn on_reset(mut self, message: M) -> Self {
        self.on_reset = Some(message);
        self
    }

    /// What the reset button's tip says, in the interface's language.
    pub fn reset_tip(mut self, words: impl Into<String>) -> Self {
        self.reset_tip = words.into();
        self
    }
}

impl<'a, M: Clone + 'a> From<Row<'a, M>> for Element<'a, M> {
    fn from(r: Row<'a, M>) -> Self {
        let t = r.theme;
        let has_hint = r.hint.is_some();
        let mut name = iced::widget::row![label(t, r.label)]
            .spacing(space::MD)
            .align_y(Alignment::Center);
        if let Some(words) = r.badge {
            name = name.push(badge::beta(t, words));
        }
        let mut text = column![name].spacing(2);
        if let Some(words) = r.hint {
            text = text.push(container(hint(t, words)).max_width(HINT_MAX_WIDTH));
        }
        let mut control = iced::widget::row![]
            .spacing(space::MD)
            .align_y(Alignment::Center);
        if let (true, Some(reset)) = (r.modified, r.on_reset) {
            control = control.push(
                icon_button(t, Icon::Reset)
                    .small()
                    .icon_size(12.0)
                    .tip(r.reset_tip)
                    .on_press(reset),
            );
        }
        control = control.push(r.control);
        let control: Element<'a, M> = if r.wide {
            container(control)
                .width(WIDE)
                .align_x(Alignment::End)
                .into()
        } else {
            control.into()
        };
        // 48 px at least; a row with a hint is taller than that on its own.
        let line = container(
            iced::widget::row![text.width(Length::Fill), control]
                .spacing(space::XXXL)
                .align_y(Alignment::Center),
        )
        .padding(Padding::from([ROW_PADDING_Y, space::XXL]));
        if has_hint {
            line.into()
        } else {
            // A minimum, which iced has not for a height: a strut beside the
            // line holds it open without capping a taller control.
            iced::widget::row![iced::widget::Space::new().height(MIN_ROW_HEIGHT), line]
                .align_y(Alignment::Center)
                .into()
        }
    }
}

/// A block that does not fit a label and a control: a preview, a grid. It
/// takes the whole width of the group, with a label and a hint over it if
/// they are given.
pub fn block<'a, M: 'a>(
    t: &'a Theme,
    label: Option<&str>,
    hint: Option<&str>,
    content: impl Into<Element<'a, M>>,
) -> Element<'a, M> {
    let mut stack = column![].spacing(space::LG);
    if label.is_some() || hint.is_some() {
        let mut text = column![].spacing(2);
        if let Some(words) = label {
            text = text.push(self::label(t, words.to_string()));
        }
        if let Some(words) = hint {
            text = text.push(self::hint(t, words.to_string()));
        }
        stack = stack.push(text);
    }
    stack = stack.push(content.into());
    container(stack)
        .padding(Padding::from([ROW_PADDING_Y, space::XXL]))
        .width(Length::Fill)
        .into()
}

/// A block around rows, a hairline between each two.
pub fn group<'a, M: 'a>(t: &'a Theme, rows: Vec<Element<'a, M>>) -> Element<'a, M> {
    let mut stack = column![];
    for (at, one) in rows.into_iter().enumerate() {
        if at > 0 {
            stack = stack.push(rule::horizontal(1).style(style::rule::hairline(t)));
        }
        stack = stack.push(one);
    }
    let look = style::container::settings_block(t);
    let fill = block_fill(t);
    container(stack)
        .width(Length::Fill)
        // One pixel in, so a row's wash stays inside the block's hairline.
        .padding(1)
        .style(move |theme: &iced::Theme| container::Style {
            background: Some(Background::Color(fill)),
            ..look(theme)
        })
        .into()
}

/// A titled part of a settings page: a heading, a hint, and its group.
#[must_use]
pub struct Section<'a, M> {
    theme: &'a Theme,
    title: String,
    hint: Option<String>,
    body: Option<Element<'a, M>>,
}

pub fn section<'a, M: 'a>(t: &'a Theme, title: impl Into<String>) -> Section<'a, M> {
    Section {
        theme: t,
        title: title.into(),
        hint: None,
        body: None,
    }
}

impl<'a, M: 'a> Section<'a, M> {
    /// A sentence under the title.
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// The rows, in a card.
    pub fn group(mut self, rows: Vec<Element<'a, M>>) -> Self {
        self.body = Some(group(self.theme, rows));
        self
    }

    /// Something that is its own box and goes straight on the page.
    pub fn bare(mut self, body: impl Into<Element<'a, M>>) -> Self {
        self.body = Some(body.into());
        self
    }
}

impl<'a, M: 'a> From<Section<'a, M>> for Element<'a, M> {
    fn from(s: Section<'a, M>) -> Self {
        let t = s.theme;
        let mut head = column![title(t, s.title)].spacing(2);
        if let Some(words) = s.hint {
            head = head.push(hint(t, words));
        }
        let mut stack = column![head].spacing(space::LG);
        if let Some(body) = s.body {
            stack = stack.push(body);
        }
        stack.into()
    }
}

/// Sections one under the other, spaced the way the settings space theirs:
/// for a page's own parts, drawn inside another page.
pub fn stack<'a, M: 'a>(sections: Vec<Element<'a, M>>) -> Element<'a, M> {
    iced::widget::Column::with_children(sections)
        .spacing(SECTION_GAP)
        .width(Length::Fill)
        .into()
}

/// A page of sections, with the room a settings page keeps around them.
pub fn page<'a, M: 'a>(sections: Vec<Element<'a, M>>) -> Element<'a, M> {
    container(stack(sections))
        .padding(Padding::from([SECTION_GAP, PAGE_PADDING_X]))
        .width(Length::Fill)
        .into()
}
