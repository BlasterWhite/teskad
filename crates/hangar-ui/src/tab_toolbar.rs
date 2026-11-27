//! The bar every tab of a workspace has under the top bar (Board, Memory,
//! Statistics; `ecrans.md`, « Cadre commun aux écrans 04 à 11 »): 48 px, a
//! rule under it; on the left the count and then the filters or the search,
//! on the right the MCP note and the tab's main action.
//!
//! ```ignore
//! use hangar_ui::tab_toolbar;
//!
//! responsive(|size| {
//!     let compact = tab_toolbar::compact(size.width);
//!     tab_toolbar::bar(
//!         t,
//!         row![tab_toolbar::count(t, "23 tasks"), filters(compact)],
//!         row![tab_toolbar::note(t, note, tip, compact, Msg::OpenMcp), add],
//!     )
//! })
//! ```
//!
//! In a window under 1280 px ([`compact`]) the MCP note is its icon alone,
//! with the words in a tooltip, and a tab's filters go into one button with
//! the number of those that are on ([`filters_button`]).

use hangar_theme::tokens::{BORDER_WIDTH, font_size, scale, space};
use hangar_theme::{Theme, fonts, style};
use iced::widget::text::Wrapping;
use iced::widget::{Space, button, column, container, row, rule, text};
use iced::{Alignment, Background, Border, Element, Length, Padding};

use crate::focus;
use crate::icon::{Icon, icon};
use crate::tip;

/// The bar's height, the rule under it included.
pub const HEIGHT: f32 = 48.0;

/// The window width under which the bar makes room.
const NARROW_WINDOW: f32 = 1280.0;

/// The room under which a tab's bar makes room: what a window of 1280 px
/// leaves beside the navigator and its hairline. A docked side panel takes
/// from the same room, so it counts as a narrower window.
pub const COMPACT_BELOW: f32 = NARROW_WINDOW - scale::size::SIDEBAR - BORDER_WIDTH;

/// Whether a bar `room` wide makes room.
pub fn compact(room: f32) -> bool {
    room < COMPACT_BELOW
}

/// The bar: `leading` on the left, `trailing` at the right end.
pub fn bar<'a, M: 'a>(
    t: &'a Theme,
    leading: impl Into<Element<'a, M>>,
    trailing: impl Into<Element<'a, M>>,
) -> Element<'a, M> {
    let line = row![
        leading.into(),
        Space::new().width(Length::Fill),
        trailing.into()
    ]
    .spacing(space::MD)
    .align_y(Alignment::Center);
    let under = t.palette.surface_raised;
    column![
        container(line)
            .padding(Padding::from([0.0, scale::space::S4]))
            .height(HEIGHT - BORDER_WIDTH)
            .width(Length::Fill)
            .align_y(Alignment::Center),
        rule::horizontal(BORDER_WIDTH).style(move |theme: &iced::Theme| rule::Style {
            color: under,
            ..style::rule::sep(t)(theme)
        }),
    ]
    .into()
}

/// A count's number and the words after it: "23" and "tasks". The words are
/// everything when they do not start with a number.
pub fn split_count(words: &str) -> (&str, &str) {
    let end = words
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(words.len());
    let (number, rest) = words.split_at(end);
    (number, rest.trim_start())
}

/// "23 tasks" in the display face: the number in the text's ink, what it
/// counts quieter.
pub fn count<'a, M: 'a>(t: &'a Theme, words: impl AsRef<str>) -> Element<'a, M> {
    let (number, rest) = split_count(words.as_ref());
    let piece = |content: &str, color: iced::Color| {
        text(content.to_string())
            .size(15.0)
            .font(fonts::display())
            .wrapping(Wrapping::None)
            .color(color)
    };
    let mut line = row![].spacing(space::XS).align_y(Alignment::Center);
    if !number.is_empty() {
        line = line.push(piece(number, t.palette.text));
    }
    if !rest.is_empty() {
        let ink = if number.is_empty() {
            t.palette.text
        } else {
            t.palette.text_muted
        };
        line = line.push(piece(rest, ink));
    }
    line.into()
}

/// The MCP note: a plug and `words`, or the plug alone when the bar is
/// `compact`; the tooltip is then the words too. Pressing it says `message`.
pub fn note<'a, M: Clone + 'a>(
    t: &'a Theme,
    words: String,
    action: String,
    compact: bool,
    message: M,
) -> Element<'a, M> {
    let mut content = row![icon(Icon::Plug, 12.0)]
        .spacing(space::SM)
        .align_y(Alignment::Center);
    let hint = if compact {
        format!("{words} · {action}")
    } else {
        content = content.push(text(words).size(12.0).wrapping(Wrapping::None));
        action
    };
    let face = button(container(content).center_y(Length::Fill))
        .padding([0.0, space::SM])
        .height(scale::size::CONTROL_SM)
        .style(move |_: &iced::Theme, status| {
            let p = &t.palette;
            let pointed = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: pointed.then_some(Background::Color(p.hover_overlay)),
                text_color: if pointed { p.text } else { p.text_muted },
                border: Border {
                    radius: scale::radius::SM.into(),
                    ..Border::default()
                },
                ..button::Style::default()
            }
        })
        .on_press(message.clone());
    let face = focus::ring(t, face)
        .radius(scale::radius::SM)
        .on_activate(message);
    tip::bottom(t, face, hint)
}

/// A filter's button: 28 px, `surface` in a `border` hairline; one that hides
/// something takes the accent's hairline on the selection's fill.
pub fn filter_style(
    t: &Theme,
    active: bool,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style + '_ {
    move |_, status| {
        let p = &t.palette;
        let pointed = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let rest = if active { p.selection_bg } else { p.surface };
        button::Style {
            background: Some(Background::Color(if pointed {
                over(p.hover_overlay, rest)
            } else {
                rest
            })),
            text_color: p.text,
            border: Border {
                color: if active { p.accent } else { p.border },
                width: BORDER_WIDTH,
                radius: scale::radius::SM.into(),
            },
            ..button::Style::default()
        }
    }
}

/// A translucent `top` laid over an opaque `base`.
fn over(top: iced::Color, base: iced::Color) -> iced::Color {
    let a = top.a;
    iced::Color {
        r: top.r * a + base.r * (1.0 - a),
        g: top.g * a + base.g * (1.0 - a),
        b: top.b * a + base.b * (1.0 - a),
        a: base.a,
    }
}

/// "Filters ▾" with the number of filters that are on: what a tab's filters
/// become in a narrow window. It opens a menu that is the caller's.
pub fn filters_button<'a, M: Clone + 'a>(
    t: &'a Theme,
    label: String,
    on: usize,
    message: M,
) -> Element<'a, M> {
    let mut content = row![
        icon(Icon::Filter, 12.0),
        text(label).size(12.0).wrapping(Wrapping::None)
    ]
    .spacing(space::SM)
    .align_y(Alignment::Center);
    if on > 0 {
        content = content.push(
            container(
                text(on.to_string())
                    .size(font_size::MICRO)
                    .font(fonts::mono())
                    .color(t.palette.on_accent),
            )
            .padding([0.0, space::XS + 1.0])
            .style(move |_: &iced::Theme| container::Style {
                background: Some(Background::Color(t.palette.accent)),
                border: Border {
                    radius: scale::radius::PILL.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            }),
        );
    }
    content = content.push(icon(Icon::ChevronDown, 10.0));
    let face = button(container(content).center_y(Length::Fill))
        .height(scale::size::CONTROL_SM)
        .padding([0.0, space::MD])
        .style(filter_style(t, on > 0))
        .on_press(message.clone());
    focus::ring(t, face)
        .radius(scale::radius::SM)
        .on_activate(message)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_number_of_a_count_is_told_from_what_it_counts() {
        assert_eq!(split_count("23 tasks"), ("23", "tasks"));
        assert_eq!(split_count("8 of 23 tasks"), ("8", "of 23 tasks"));
        assert_eq!(split_count("8 sur 23 tâches"), ("8", "sur 23 tâches"));
        assert_eq!(split_count("Board"), ("", "Board"));
    }

    #[test]
    fn a_window_under_1280_makes_room() {
        assert!(!compact(1280.0 - scale::size::SIDEBAR - BORDER_WIDTH));
        assert!(compact(1279.0 - scale::size::SIDEBAR - BORDER_WIDTH));
        assert!(!compact(1440.0 - 249.0));
    }
}
