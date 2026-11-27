//! Drop-downs (`.set-select`). A pick list has two styles: the field that
//! shows the choice, and the menu that opens under it.
//!
//! ```ignore
//! pick_list(options, selected, Message::Picked)
//!     .style(style::pick_list::field(t))
//!     .menu_style(style::pick_list::menu(t))
//! ```

use iced::overlay::menu;
use iced::widget::pick_list::{Status, Style};
use iced::{Background, Border};

use super::{drop_shadow, focus_border, hairline, lit};
use crate::Theme;
use crate::tokens::radius;

/// The closed field: dressed like a text field, with its chevron in
/// `text_muted`, the ring while it is open.
pub fn field(t: &Theme) -> impl Fn(&iced::Theme, Status) -> Style + '_ {
    move |_, status| {
        let p = &t.palette;
        let (background, border) = match status {
            Status::Active => (p.surface_sunken, hairline(p.border_strong, radius::LG)),
            Status::Hovered => (
                lit(p.surface_sunken, p.hover_overlay, 1),
                hairline(p.border_strong, radius::LG),
            ),
            Status::Opened { .. } => (p.surface_sunken, focus_border(t, radius::LG)),
        };
        Style {
            text_color: p.text,
            placeholder_color: p.text_muted,
            handle_color: p.text_muted,
            background: Background::Color(background),
            border,
        }
    }
}

/// The open list: `surface_raised`, a strong hairline and the shadow of what
/// floats; the highlighted line is `selection_bg` (a fill and the text, not a
/// colour alone: the line under the pointer also reads in `text`).
pub fn menu(t: &Theme) -> impl Fn(&iced::Theme) -> menu::Style + '_ {
    move |_| {
        let p = &t.palette;
        menu::Style {
            background: Background::Color(p.surface_raised),
            border: Border {
                radius: radius::LG.into(),
                ..hairline(p.border_strong, radius::LG)
            },
            text_color: p.text,
            selected_text_color: p.text,
            selected_background: Background::Color(p.selection_bg),
            shadow: drop_shadow(t.colors.shadow, 8.0, 24.0),
        }
    }
}
