//! The switch: a pill track in `border_strong` that turns `accent` when on,
//! and a knob that is `on_accent` when on and `text_secondary` when off, so
//! the position and the colour of the knob say the state as much as the
//! track does.
//!
//! The switch of `hangar-ui` is not iced's `toggler` (whose track is always
//! twice its height) but a button of [`crate::tokens::size::SWITCH_WIDTH`] by
//! [`crate::tokens::size::SWITCH_HEIGHT`] styled with [`switch`] around a
//! [`knob`];
//! [`default`] stays for the places that still use iced's.

use iced::widget::button::{Status as ButtonStatus, Style as ButtonStyle};
use iced::widget::container::Style as ContainerStyle;
use iced::widget::toggler::{Status, Style};
use iced::{Background, Border, Color};

use super::{faded, lit, rounded};
use crate::Theme;
use crate::tokens::{radius, size};

fn colors(t: &Theme, on: bool) -> (Color, Color) {
    let p = &t.palette;
    if on {
        (p.accent, p.on_accent)
    } else {
        (p.border_strong, p.text_secondary)
    }
}

/// The switch as a button: the track is the button's own fill, the overlay
/// brightens it under the pointer and presses it, and a disabled one fades.
pub fn switch(t: &Theme, on: bool) -> impl Fn(&iced::Theme, ButtonStatus) -> ButtonStyle + '_ {
    move |_, status| {
        let (track, _) = colors(t, on);
        let overlay = t.palette.hover_overlay;
        let fill = match status {
            ButtonStatus::Active => track,
            ButtonStatus::Hovered => lit(track, overlay, 1),
            ButtonStatus::Pressed => lit(track, overlay, 2),
            ButtonStatus::Disabled => faded(track),
        };
        ButtonStyle {
            background: Some(Background::Color(fill)),
            text_color: t.palette.text,
            border: rounded(radius::FULL),
            ..ButtonStyle::default()
        }
    }
}

/// The knob of a switch.
pub fn knob(t: &Theme, on: bool, disabled: bool) -> impl Fn(&iced::Theme) -> ContainerStyle + '_ {
    move |_| {
        let (_, knob) = colors(t, on);
        ContainerStyle {
            background: Some(Background::Color(if disabled { faded(knob) } else { knob })),
            border: Border {
                radius: radius::FULL.into(),
                ..Border::default()
            },
            ..ContainerStyle::default()
        }
    }
}

pub fn default(t: &Theme) -> impl Fn(&iced::Theme, Status) -> Style + '_ {
    move |_, status| {
        let p = &t.palette;
        let (on, disabled) = match status {
            Status::Active { is_toggled } | Status::Hovered { is_toggled } => (is_toggled, false),
            Status::Disabled { is_toggled } => (is_toggled, true),
        };
        let (track, knob) = colors(t, on);
        let (track, knob, label) = if disabled {
            (faded(track), faded(knob), faded(p.text))
        } else {
            (track, knob, p.text)
        };
        Style {
            background: Background::Color(track),
            background_border_width: 0.0,
            background_border_color: Color::TRANSPARENT,
            foreground: Background::Color(knob),
            foreground_border_width: 0.0,
            foreground_border_color: Color::TRANSPARENT,
            text_color: Some(label),
            border_radius: Some(radius::FULL.into()),
            // The knob's 2 px inset in a track 20 px high.
            padding_ratio: 2.0 / size::SWITCH_HEIGHT,
        }
    }
}
