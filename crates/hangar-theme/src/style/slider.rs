//! Sliders: a 4 px track filled with `accent` up to the knob (`border`
//! after it), and a 14 px knob in `on_accent` with a 3 px halo of the accent
//! at 35 % under the pointer and while it is dragged.

use iced::Background;
use iced::widget::slider::{Handle, HandleShape, Rail, Status, Style};

use super::{rounded, with_alpha};
use crate::Theme;

/// The height the widget is given so that the knob and its halo fit.
pub const HEIGHT: f32 = 20.0;

/// The thickness of the track.
const TRACK: f32 = 4.0;

/// The knob's radius, 14 px across.
const KNOB: f32 = 7.0;

pub fn default(t: &Theme) -> impl Fn(&iced::Theme, Status) -> Style + '_ {
    move |_, status| {
        let p = &t.palette;
        // Whichever of the two reads better on the track.
        let on_track = |color: iced::Color| {
            crate::contrast_ratio(crate::Rgb::from(color), crate::Rgb::from(p.border))
        };
        let knob = if on_track(p.text) >= on_track(p.surface) {
            p.text
        } else {
            p.surface
        };
        let (edge, width) = match status {
            Status::Active => (p.border_strong, 1.0),
            Status::Hovered | Status::Dragged => (with_alpha(p.accent, 0.35), 3.0),
        };
        Style {
            rail: Rail {
                backgrounds: (Background::Color(p.accent), Background::Color(p.border)),
                width: TRACK,
                border: rounded(TRACK / 2.0),
            },
            handle: Handle {
                shape: HandleShape::Circle { radius: KNOB },
                background: Background::Color(knob),
                border_width: width,
                border_color: edge,
            },
        }
    }
}
