//! Surfaces: what the window's areas, cards and floating panels are filled
//! with. Use as `container(..).style(style::container::sidebar(t))`.
//!
//! Every function sets the text colour too, so text inside draws in `fg`
//! without naming it.

use iced::widget::container::Style;
use iced::{Background, Border, Color, Shadow};

use super::{Tone, drop_shadow, hairline, rounded, with_alpha};
use crate::Theme;
use crate::tokens::radius;

fn fill(color: Color, text: Color) -> Style {
    Style {
        background: Some(Background::Color(color)),
        text_color: Some(text),
        border: Border::default(),
        shadow: Shadow::default(),
        ..Style::default()
    }
}

/// Nothing but the theme's text colour: for a container that only lays
/// things out over an area that already has its own surface.
pub fn plain(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| Style {
        text_color: Some(t.colors.fg),
        ..Style::default()
    }
}

/// The page under everything (`body`): the terminal's own background.
pub fn window(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| fill(t.palette.bg, t.colors.fg)
}

/// The page laid straight on the desktop: [`window`] at the window's alpha
/// ([`Theme::see_through`]). For the regions of the window that show the
/// page and nothing is drawn under: the work area, a placeholder, the whole
/// window while it starts.
pub fn backdrop(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| fill(t.see_through(t.palette.bg), t.colors.fg)
}

/// The page under a pane that lies on a [`backdrop`]: the page's colour on a
/// solid window, nothing on a translucent one, where the backdrop under it
/// already is that colour and a second coat would make it opaque.
pub fn page(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |theme| {
        if t.is_translucent() {
            plain(t)(theme)
        } else {
            window(t)(theme)
        }
    }
}

/// `style` laid straight on the desktop: its fill at the window's alpha
/// ([`Theme::see_through`]), so that the desktop shows through it on a
/// see-through window, and as it is on a solid one. Only for the window's own
/// regions, which nothing else is drawn under: the navigator, the toolbar,
/// the status bar, a docked side panel. What floats (menus, dialogs, cards)
/// stays opaque.
pub fn see_through<'a>(
    t: &'a Theme,
    style: impl Fn(&iced::Theme) -> Style + 'a,
) -> impl Fn(&iced::Theme) -> Style + 'a {
    move |theme| {
        let mut look = style(theme);
        if let Some(Background::Color(color)) = look.background {
            look.background = Some(Background::Color(t.see_through(color)));
        }
        look
    }
}

/// The navigator (`.sidebar`). Its right edge is a [`super::rule::sep`].
pub fn sidebar(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| fill(t.palette.surface, t.colors.fg)
}

/// The inspector and the other side panels: the same chrome as the navigator.
pub fn panel(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    sidebar(t)
}

/// The bar over the workspace (`.toolbar`), a step above the navigator.
pub fn toolbar(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| fill(t.palette.surface_raised, t.colors.fg)
}

/// The bar along the bottom (`.status`).
pub fn statusbar(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| fill(t.palette.surface, t.colors.fg)
}

/// A pane's header (`.pane__bar`), a little lighter while the pane has the
/// focus.
pub fn pane_head(t: &Theme, focused: bool) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| {
        let color = if focused {
            t.palette.surface_raised
        } else {
            t.palette.surface
        };
        fill(color, t.colors.fg)
    }
}

/// A card or a group of settings: lifted off the page on a dark theme, the
/// page's own colour on a light one, with a hairline around it.
pub fn card(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| Style {
        border: hairline(t.palette.border, radius::LG),
        ..fill(t.colors.card, t.colors.fg)
    }
}

/// A well inside a dialog (`--group-bg`): lists and grouped boxes sitting
/// back in the panel.
pub fn well(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| Style {
        border: hairline(t.palette.border, radius::MD),
        ..fill(t.palette.bg, t.colors.fg)
    }
}

/// A chip or a capsule: a hairline's tint over whatever it sits on.
pub fn chip(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| Style {
        border: hairline(t.palette.border, radius::SM),
        ..fill(t.palette.border, t.colors.fg_secondary)
    }
}

/// The track of a segmented control in the toolbar (`.segmented`); its chosen
/// segment is a [`super::button::tab`].
pub fn segmented_track(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| Style {
        background: Some(Background::Color(t.palette.border)),
        border: rounded(7.0),
        ..Style::default()
    }
}

/// The track of a segmented control: recessed (`surface_sunken`), a hairline
/// around it, corners of a field (8); its chosen segment is a
/// [`super::button::tab`].
pub fn segmented(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| Style {
        border: hairline(t.palette.border, radius::LG),
        ..fill(t.palette.surface_sunken, t.palette.text)
    }
}

/// A context menu or any popover (`ecrans.md`: `surfaceRaised`, radius 8,
/// the popover shadow): the raised surface, a `border` hairline (the pack's
/// ring) and a shadow that lifts it off the window.
pub fn popover(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| Style {
        border: hairline(t.palette.border, radius::LG),
        shadow: drop_shadow(t.inks.shadow, 8.0, 24.0),
        ..fill(t.palette.surface_raised, t.palette.text)
    }
}

/// A dialog (`ecrans.md`: `surface`, `borderStrong`, radius 12, the modal
/// shadow).
pub fn modal(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| Style {
        border: hairline(t.palette.border_strong, radius::XL),
        shadow: drop_shadow(t.inks.shadow, 24.0, 64.0),
        ..fill(t.palette.surface, t.palette.text)
    }
}

/// A dialog's foot, a step apart from its body: the sunken surface
/// (`ecrans.md`: "un cran plus sombre"). Only the bottom corners, which meet
/// the dialog's edge, are round.
pub fn modal_foot(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| Style {
        border: Border {
            // Inside the dialog's 1 px hairline.
            radius: iced::border::bottom(radius::XL - 1.0),
            ..Border::default()
        },
        ..fill(t.palette.surface_sunken, t.palette.text)
    }
}

/// The command palette's panel (`.palette__panel`). The web app blurred what
/// is behind it through 90 % of the toolbar's colour; this is opaque.
pub fn palette(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| Style {
        border: hairline(t.palette.border_strong, radius::XL),
        shadow: drop_shadow(t.inks.shadow, 30.0, 80.0),
        ..fill(t.palette.surface_raised, t.colors.fg)
    }
}

/// A tooltip: a small popover.
pub fn tooltip(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| Style {
        border: hairline(t.palette.border_strong, radius::SM),
        shadow: drop_shadow(t.inks.shadow, 4.0, 12.0),
        ..fill(t.palette.surface_raised, t.colors.fg)
    }
}

/// The veil behind a dialog: the palette's `scrim`, which is already
/// lighter on a light theme.
pub fn scrim(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| Style {
        background: Some(Background::Color(t.palette.scrim)),
        ..Style::default()
    }
}

/// The veil behind the command palette (`.palette`): the separator colour at
/// 42 %, so the window stays readable and it is clear nothing has moved yet.
pub fn palette_scrim(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| Style {
        background: Some(Background::Color(
            t.palette.surface_sunken.scale_alpha(0.42),
        )),
        ..Style::default()
    }
}

/// A badge that warns (`--danger-soft` behind `--danger`).
pub fn badge_danger(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| Style {
        background: Some(Background::Color(t.inks.danger_soft)),
        text_color: Some(t.inks.danger()),
        border: rounded(radius::XS),
        ..Style::default()
    }
}

/// A tint of the accent: a selection, a drop target (`--accent-soft`).
pub fn accent_soft(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| Style {
        background: Some(Background::Color(t.colors.accent_soft)),
        text_color: Some(t.colors.fg),
        border: rounded(radius::SM),
        ..Style::default()
    }
}

/// A key of a keyboard shortcut (`kbd`): a 4 px cap with a `border_strong`
/// hairline, nothing inside, in `text_muted`.
pub fn kbd(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| Style {
        border: hairline(t.palette.border_strong, radius::XS),
        text_color: Some(t.palette.text_muted),
        ..Style::default()
    }
}

/// A key of a shortcut another command answers to as well: the failure
/// colour on the edge and the text.
pub fn kbd_conflict(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| Style {
        border: hairline(t.palette.status.failed, radius::XS),
        text_color: Some(t.palette.status.failed),
        ..Style::default()
    }
}

/// The box a shortcut is recorded in (`.key-rec`): the accent for its edge
/// and its tint behind, the way a field with the keyboard looks.
pub fn recorder(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| Style {
        border: hairline(t.palette.accent, radius::MD),
        ..fill(t.colors.accent_soft, t.colors.fg)
    }
}

/// The square an icon sits in at the start of a palette row
/// (`.palette__icon`), turned over to the accent's foreground on the chosen
/// row.
pub fn palette_icon(t: &Theme, chosen: bool) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| {
        let c = &t.colors;
        if chosen {
            Style {
                background: Some(Background::Color(with_alpha(t.palette.on_accent, 0.18))),
                text_color: Some(t.palette.on_accent),
                border: rounded(radius::SM),
                ..Style::default()
            }
        } else {
            Style {
                background: Some(Background::Color(t.palette.border)),
                text_color: Some(c.fg_muted),
                border: rounded(radius::SM),
                ..Style::default()
            }
        }
    }
}

/// A hairline box with nothing in it (`.palette__esc`): the cap of the key
/// that closes the palette, on a surface of its own.
pub fn outline(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| Style {
        border: hairline(t.palette.border_strong, radius::XS),
        text_color: Some(t.colors.fg_secondary),
        ..Style::default()
    }
}

/// A small word set on a surface (`.key-row__tag`): "custom", "new".
pub fn tag(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| Style {
        background: Some(Background::Color(t.palette.selection_bg)),
        text_color: Some(t.colors.fg_muted),
        border: rounded(radius::XS),
        ..Style::default()
    }
}

/// A count or a status word in a capsule. The tone is what it says: neutral
/// for a number, the accent for what is selected, success, warning and
/// danger for what they name.
pub fn badge(t: &Theme, tone: Tone) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| {
        let (background, text) = tone_colors(t, tone);
        Style {
            background: Some(Background::Color(background)),
            text_color: Some(text),
            border: rounded(radius::XS),
            ..Style::default()
        }
    }
}

/// A notice that floats over the window and goes by itself: a popover with
/// its tone in the edge.
pub fn toast(t: &Theme, tone: Tone) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| {
        let c = &t.colors;
        let edge = match tone {
            Tone::Neutral => t.palette.border_strong,
            Tone::Accent => t.palette.accent,
            Tone::Success => t.inks.success(),
            Tone::Warning => t.inks.warning(),
            Tone::Danger => t.inks.danger(),
        };
        Style {
            border: hairline(edge, radius::LG),
            shadow: drop_shadow(t.inks.shadow, 10.0, 28.0),
            ..fill(t.palette.surface_raised, c.fg)
        }
    }
}

/// The colour a tone's text and icons are drawn in (the glyph of a toast).
pub fn tone_text(t: &Theme, tone: Tone) -> Color {
    tone_colors(t, tone).1
}

/// A tone's tint and the colour that reads on it.
fn tone_colors(t: &Theme, tone: Tone) -> (Color, Color) {
    let c = &t.colors;
    match tone {
        Tone::Neutral => (t.palette.border, c.fg_secondary),
        Tone::Accent => (c.accent_soft, c.accent_hi),
        Tone::Success => (with_alpha(t.inks.success(), 0.16), t.inks.success()),
        Tone::Warning => (with_alpha(t.inks.warning(), 0.16), t.inks.warning()),
        Tone::Danger => (t.inks.danger_soft, t.inks.danger()),
    }
}

/// A group of rows with a hairline around it and nothing between them but the
/// rows' own separators (`.set-group`).
pub fn group(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    settings_block(t)
}

/// A row of a form or a list drawn under the pointer without being a button
/// (the shortcut list's rows): a faint wash of the text colour.
pub fn row_hover(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| Style {
        background: Some(Background::Color(with_alpha(t.colors.fg, 0.03))),
        border: rounded(radius::NONE),
        ..Style::default()
    }
}

/// A row that something is wrong with: a wash of the danger colour.
pub fn row_conflict(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| Style {
        background: Some(Background::Color(with_alpha(t.inks.danger(), 0.08))),
        border: rounded(radius::NONE),
        ..Style::default()
    }
}

/// A role badge, a pill (the one pill of the interface): the orchestrator is
/// `selection_bg` with `status.active` text, a worker a `border_strong`
/// outline in `text_secondary`. The words and the Fork symbol tell them apart
/// as much as the colours do.
pub fn role_pill(t: &Theme, orchestrator: bool) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| {
        let p = &t.palette;
        if orchestrator {
            Style {
                background: Some(Background::Color(p.selection_bg)),
                text_color: Some(p.status.active),
                border: rounded(radius::FULL),
                ..Style::default()
            }
        } else {
            Style {
                text_color: Some(p.text_secondary),
                border: hairline(p.border_strong, radius::FULL),
                ..Style::default()
            }
        }
    }
}

/// The badge of the Claude account of a workspace or a pane (a bolt and the
/// account's name): 4 px corners, `surface_raised`, `status.active` text.
/// `selected` is the badge on a selected row, which sits on `selection_bg`.
pub fn account_badge(t: &Theme, selected: bool) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| {
        let p = &t.palette;
        Style {
            background: Some(Background::Color(if selected {
                super::lit(p.selection_bg, p.status.active.scale_alpha(0.18), 1)
            } else {
                p.surface_raised
            })),
            text_color: Some(p.status.active),
            border: rounded(radius::XS),
            ..Style::default()
        }
    }
}

/// The BETA badge: a `border_strong` outline, 4 px corners, `status.active`
/// text.
pub fn beta(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| Style {
        text_color: Some(t.palette.status.active),
        border: hairline(t.palette.border_strong, radius::XS),
        ..Style::default()
    }
}

/// A block of settings: `surface`, a `border` hairline, corners of 12. Its
/// rows are separated by a [`super::rule::sep`] hairline.
pub fn settings_block(t: &Theme) -> impl Fn(&iced::Theme) -> Style + '_ {
    move |_| Style {
        border: hairline(t.palette.border, radius::XL),
        ..fill(t.palette.surface, t.palette.text)
    }
}
