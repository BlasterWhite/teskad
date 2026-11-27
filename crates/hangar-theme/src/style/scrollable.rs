//! Scrollbars (`::-webkit-scrollbar` in `src/styles.css`): a 9 px thumb in
//! `border_strong` that turns `fg_subtle` under the pointer, square, on a
//! transparent track.
//!
//! The width is the widget's, not the style's:
//! `scrollable(..).direction(Direction::Vertical(Scrollbar::new().width(SCROLLBAR)))`
//! with [`crate::tokens::size::SCROLLBAR`].

use iced::widget::container;
use iced::widget::scrollable::{AutoScroll, Rail, Scroller, Status, Style};
use iced::{Background, Border};

use super::{drop_shadow, hairline, rounded};
use crate::Theme;
use crate::tokens::radius;

pub fn default(t: &Theme) -> impl Fn(&iced::Theme, Status) -> Style + '_ {
    move |_, status| {
        let inks = &t.inks;
        let rail = |thumb| Rail {
            background: None,
            border: Border::default(),
            scroller: Scroller {
                background: Background::Color(thumb),
                border: rounded(radius::NONE),
            },
        };
        let (vertical, horizontal) = match status {
            Status::Active { .. } => (t.palette.border_strong, t.palette.border_strong),
            Status::Hovered {
                is_vertical_scrollbar_hovered,
                is_horizontal_scrollbar_hovered,
                ..
            } => (
                if is_vertical_scrollbar_hovered {
                    inks.text.muted
                } else {
                    t.palette.border_strong
                },
                if is_horizontal_scrollbar_hovered {
                    inks.text.muted
                } else {
                    t.palette.border_strong
                },
            ),
            Status::Dragged {
                is_vertical_scrollbar_dragged,
                is_horizontal_scrollbar_dragged,
                ..
            } => (
                if is_vertical_scrollbar_dragged {
                    inks.text.muted
                } else {
                    t.palette.border_strong
                },
                if is_horizontal_scrollbar_dragged {
                    inks.text.muted
                } else {
                    t.palette.border_strong
                },
            ),
        };
        Style {
            container: container::Style::default(),
            vertical_rail: rail(vertical),
            horizontal_rail: rail(horizontal),
            gap: None,
            auto_scroll: AutoScroll {
                background: Background::Color(t.palette.surface_raised),
                border: hairline(t.palette.border_strong, radius::FULL),
                shadow: drop_shadow(t.inks.shadow, 2.0, 6.0),
                icon: inks.text.primary,
            },
        }
    }
}
