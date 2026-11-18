//! Rules: the one-pixel lines between areas. Draw them as
//! `rule::horizontal(1).style(style::rule::sep(t))`, which is how a border on
//! one side of a container is done here.

use iced::widget::rule::{FillMode, Style};

use super::rounded;
use crate::Theme;
use crate::tokens::radius;

fn line(color: iced::Color) -> Style {
    Style {
        color,
        radius: rounded(radius::NONE).radius,
        fill_mode: FillMode::Full,
        snap: true,
    }
}

/// Between panes, columns and the window's areas (`--sep`): darker than
/// anything around it.
pub fn sep(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| line(t.colors.sep)
}

/// A [`sep`] between two of the window's own regions (the navigator and the
/// rest, the toolbar and the panes), laid straight on the desktop: at the
/// window's alpha, so a translucent theme shows the desktop through it too.
pub fn sep_see_through(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| line(t.see_through(t.colors.sep))
}

/// A hairline inside a surface (`--line`): under a dialog's title, above its
/// buttons.
pub fn hairline(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| line(t.colors.line)
}

/// A stronger hairline (`--line-strong`): between a menu's groups.
pub fn strong(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| line(t.colors.line_strong)
}
