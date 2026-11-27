//! Surfaces and marks of the chrome that no widget of hangar-ui draws: the
//! rows of the navigator, the floating capsules of the update banner and the
//! dictation pill, the toolbar's well.
//!
//! These are written from the theme's colours, never a literal, and belong to
//! this package because they are the stylesheet's `.nav-ws`, `.update`,
//! `.voice-pill` and `.activity` rather than a widget anyone else reuses.

use hangar_theme::tokens::{BORDER_WIDTH, FOCUS_RING_WIDTH, radius, scale};
use hangar_theme::{Theme, style};
use iced::widget::container;
use iced::{Background, Border, Color, Shadow, Vector};

/// A hairline of `color` with these corners.
pub fn border(color: Color, radius: f32) -> Border {
    Border {
        color,
        width: BORDER_WIDTH,
        radius: radius.into(),
    }
}

/// A floating capsule (`.update`, `.voice-pill`): the toolbar's colour a
/// little see-through, a hairline, the float ring and a soft shadow. The web
/// app blurred what is behind it, which iced cannot, so it is kept nearly
/// opaque to stay legible over a busy terminal.
pub fn capsule(t: &Theme) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(Background::Color(t.palette.surface.scale_alpha(0.96))),
        text_color: Some(t.palette.text),
        border: border(t.palette.border_strong, radius::FULL),
        shadow: Shadow {
            color: t.inks.shadow,
            offset: Vector::new(0.0, 8.0),
            blur_radius: 24.0,
        },
        ..container::Style::default()
    }
}

/// The surface of the navigator and of the bar over the workspace: the
/// window's own (`bg`).
pub fn window_chrome(t: &Theme) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(Background::Color(t.palette.bg)),
        text_color: Some(t.palette.text),
        ..container::Style::default()
    }
}

/// The hairline under the navigator's top and the toolbar: `surface_raised`
/// on a dark theme, `border` on a light one, where the raised surface is too
/// close to the window's to be seen.
pub fn header_line(t: &Theme) -> Color {
    if t.is_dark() {
        t.palette.surface_raised
    } else {
        t.palette.border
    }
}

/// What a row of the navigator is drawn on: the selected one on the
/// selection's colour with a lighter hairline, the one under the pointer on the
/// overlay, the others on the navigator's own surface. `dragged` fades the
/// row being dragged, and `focused` rings the row the keyboard is on.
pub fn nav_row(
    t: &Theme,
    active: bool,
    hovered: bool,
    dragged: bool,
    focused: bool,
) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| {
        let p = &t.palette;
        let fill = if active {
            Some(p.selection_bg)
        } else if hovered {
            Some(p.hover_overlay)
        } else {
            None
        };
        let fade = if dragged { 0.45 } else { 1.0 };
        let (edge, width) = if focused {
            (p.focus_ring, FOCUS_RING_WIDTH)
        } else if active {
            (p.border_strong, BORDER_WIDTH)
        } else {
            (Color::TRANSPARENT, BORDER_WIDTH)
        };
        container::Style {
            background: fill.map(|c| Background::Color(c.scale_alpha(fade))),
            text_color: Some(
                if active || hovered {
                    p.text
                } else {
                    p.text_secondary
                }
                .scale_alpha(fade),
            ),
            border: Border {
                color: edge.scale_alpha(fade),
                width,
                radius: scale::radius::MD.into(),
            },
            ..container::Style::default()
        }
    }
}

/// The « Plan » card: `surface`, a `border` hairline, corners of 12.
pub fn card(t: &Theme) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(Background::Color(t.palette.surface)),
        text_color: Some(t.palette.text),
        border: border(t.palette.border, scale::radius::LG),
        ..container::Style::default()
    }
}

/// The badge of the project manager's door: the accent, filled.
pub fn hq_badge(t: &Theme) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(Background::Color(t.palette.accent)),
        text_color: Some(t.palette.on_accent),
        border: Border {
            radius: scale::radius::XS.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// The landing spot of a drag over a heading or a folder (`--into`,
/// `.nav-section--drop`, `.nav-root--drop`): the accent's tint with its ring.
pub fn drop_target(t: &Theme) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(Background::Color(t.palette.selection_bg)),
        text_color: Some(t.palette.text),
        border: border(t.palette.selection_border, radius::MD),
        ..container::Style::default()
    }
}

/// The insertion line a drag shows between two rows (`--drop-before`).
pub fn insertion(t: &Theme) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(Background::Color(t.palette.accent)),
        border: Border {
            radius: 1.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// The empty folder's dashed placeholder, as a solid hairline (iced has no
/// dashed border).
pub fn placeholder(t: &Theme) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| container::Style {
        border: border(t.palette.border_strong, radius::SM),
        text_color: Some(t.palette.text_muted),
        ..container::Style::default()
    }
}

/// The navigator's filter field (`.nav-field`): `surface_sunken` and a
/// hairline; edged in the accent while it narrows the list.
pub fn field(t: &Theme, narrowing: bool) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| {
        let p = &t.palette;
        container::Style {
            background: Some(Background::Color(p.surface_sunken)),
            text_color: Some(p.text_muted),
            border: border(if narrowing { p.accent } else { p.border }, radius::MD),
            ..container::Style::default()
        }
    }
}

/// The text of a state: its word colour.
pub fn state_text(
    t: &Theme,
    state: crate::state::AgentActivity,
) -> impl Fn(&iced::Theme) -> iced::widget::text::Style + '_ {
    style::text::state(t, crate::state::live::agent_state(state))
}
