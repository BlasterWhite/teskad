//! Text fields. Use as `text_input("..", &value).style(style::text_input::field(t))`.

use iced::widget::text_input::{Status, Style};
use iced::{Background, Border, Color};

use super::{faded, faded_border, focus_border, hairline, lit, rounded, with_alpha};
use crate::Theme;
use crate::tokens::radius;

/// A field in the settings or a dialog: `surface_sunken` with the strong
/// hairline, the overlay over it under the pointer, and the focus ring
/// (`focus_ring`, 2 px) while it has the keyboard.
pub fn field(t: &Theme) -> impl Fn(&iced::Theme, Status) -> Style + '_ {
    move |_, status| {
        let p = &t.palette;
        let base = |background: Color, border: Border| Style {
            background: Background::Color(background),
            border,
            icon: p.text_muted,
            placeholder: p.text_muted,
            value: p.text,
            selection: with_alpha(p.selection_border, 0.4),
        };
        let edge = hairline(p.border_strong, radius::LG);
        match status {
            Status::Active => base(p.surface_sunken, edge),
            Status::Hovered => base(lit(p.surface_sunken, p.hover_overlay, 1), edge),
            Status::Focused { .. } => base(p.surface_sunken, focus_border(t, radius::LG)),
            Status::Disabled => {
                let style = base(p.surface_sunken, edge);
                Style {
                    background: Background::Color(faded(p.surface_sunken)),
                    border: faded_border(style.border),
                    icon: faded(style.icon),
                    placeholder: faded(style.placeholder),
                    value: faded(style.value),
                    ..style
                }
            }
        }
    }
}

/// A field whose value is refused: the same field with the failure colour
/// for its edge, kept while it has the keyboard (the ring stays 2 px, so
/// the error is still told by a message beside it, not by colour alone).
pub fn field_invalid(t: &Theme) -> impl Fn(&iced::Theme, Status) -> Style + '_ {
    move |theme, status| {
        let style = field(t)(theme, status);
        match status {
            Status::Disabled => style,
            _ => Style {
                border: Border {
                    color: t.palette.status.failed,
                    ..style.border
                },
                ..style
            },
        }
    }
}

/// A field with no box of its own, for one that sits inside another surface:
/// the command palette's search, the status bar's broadcast field.
pub fn bare(t: &Theme) -> impl Fn(&iced::Theme, Status) -> Style + '_ {
    move |_, status| {
        let p = &t.palette;
        let value = match status {
            Status::Disabled => faded(p.text),
            _ => p.text,
        };
        Style {
            background: Background::Color(Color::TRANSPARENT),
            border: rounded(0.0),
            icon: p.text_muted,
            placeholder: p.text_muted,
            value,
            selection: with_alpha(p.selection_border, 0.4),
        }
    }
}
