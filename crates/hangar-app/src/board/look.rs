//! The board's surfaces, from the theme's colours and the shared tokens
//! (`styles.css`, "Kanban board", "Board panel", "Task dialog").
//!
//! `hangar_theme::style` has the roles every view shares; the few here are the
//! board's own (a card that is being dragged, the column under the dragged
//! card, a filter that hides something). Each takes the theme and returns the
//! closure iced's `.style(..)` wants, and picks every colour from
//! `theme.palette` and `theme.inks`.

use hangar_theme::Theme;
use hangar_theme::tokens::{BORDER_WIDTH, radius, scale};
use iced::widget::{button, container, text_editor, text_input};
use iced::{Background, Border, Color, Shadow, Vector};

/// The dot that marks a column (`.column__mark`) and a task dialog's header.
pub const MARK: f32 = 12.0;
/// The mark in the task dialog's header.
pub const MARK_LARGE: f32 = 9.0;
/// Under this width the full board's columns scroll sideways (`ecrans.md` §4,
/// « Place qui manque »), none of them narrower than [`COLUMN_MIN`].
pub const SCROLL_BELOW: f32 = 1180.0;
pub const COLUMN_MIN: f32 = 260.0;
/// Between two columns, and the room around them.
pub const COLUMN_GAP: f32 = 12.0;
pub const PAGE_PADDING: (f32, f32) = (12.0, 16.0);
/// A task's sheet (`ecrans.md` §4, 04b): its width, the room under which it
/// takes the whole content, its head and the column of its properties' names.
pub const SHEET_WIDTH: f32 = 520.0;
pub const SHEET_FULL_BELOW: f32 = 960.0;
pub const SHEET_HEAD: f32 = 48.0;
pub const PROPERTY_LABEL: f32 = 136.0;
/// A card's padding, across.
pub const CARD_PADDING: f32 = 12.0;
/// The dock's bar (`.board__bar`).
pub const DOCK_BAR_HEIGHT: f32 = 40.0;
/// A column's header (`.column__head`), and the dock's.
pub const COLUMN_HEAD: f32 = 36.0;
pub const DOCK_COLUMN_HEAD: f32 = 28.0;
/// The dock's grip: the area that takes the pointer, and the line it draws.
pub const GRIP_WIDTH: f32 = 5.0;
pub const GRIP_INSET: f32 = 1.0;
/// The bars of a priority (`.prio__bars i`), their width and the tallest.
pub const PRIO_BAR_WIDTH: f32 = 2.5;
pub const PRIO_BAR_HEIGHT: f32 = 9.0;
/// The skeleton's cards (`.skeleton--card`), and its label.
pub const SKELETON_CARD: f32 = 76.0;
pub const SKELETON_CARD_SHORT: f32 = 58.0;
pub const SKELETON_LABEL: (f32, f32) = (72.0, 9.0);
/// The empty board's icon box (`.board__empty-icon`).
pub const EMPTY_ICON_BOX: f32 = 44.0;
/// The widest the empty board's paragraph runs (`50ch`, at 12.5 px).
pub const EMPTY_TEXT_WIDTH: f32 = 380.0;
/// A task dialog (`.modal--task`), the fields in it, its thread's height.
pub const DIALOG_WIDTH: f32 = 640.0;
pub const FIELD_HEIGHT: f32 = 30.0;
pub const THREAD_MAX_HEIGHT: f32 = 250.0;
/// A dialog's body is cut off here and scrolls (86 vh of a 900 px window,
/// less its head and foot).
pub const DIALOG_BODY_MAX: f32 = 560.0;

/// The content of a button of a fixed height, centred in it: iced puts a
/// button's content at its top left, which hangar-ui's buttons correct the
/// same way.
pub fn centred<'a, M: 'a>(content: impl Into<iced::Element<'a, M>>) -> iced::Element<'a, M> {
    container(content).center_y(iced::Length::Fill).into()
}

/// Text in one of the theme's inks, picked at run time.
pub fn ink(color: Color) -> impl Fn(&iced::Theme) -> iced::widget::text::Style {
    move |_| iced::widget::text::Style { color: Some(color) }
}

pub fn alpha(color: Color, alpha: f32) -> Color {
    Color { a: alpha, ..color }
}

fn hairline(color: Color, radius: f32) -> Border {
    Border {
        color,
        width: BORDER_WIDTH,
        radius: radius.into(),
    }
}

fn rounded(radius: f32) -> Border {
    Border {
        radius: radius.into(),
        ..Border::default()
    }
}

/// The board itself (`.board`): the page; in the dock, a panel of `deep`
/// with a `border` hairline and corners of 12 (`ecrans.md` §8).
pub fn surface(t: &Theme, in_dock: bool) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(Background::Color(if in_dock {
            t.deep()
        } else {
            t.palette.bg
        })),
        text_color: Some(t.palette.text),
        border: if in_dock {
            hairline(t.palette.border, scale::radius::LG)
        } else {
            Border::default()
        },
        ..container::Style::default()
    }
}

/// The dock's header: `surface`, its top corners inside the panel's hairline.
pub fn dock_head(t: &Theme) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(Background::Color(t.palette.surface)),
        text_color: Some(t.palette.text),
        border: Border {
            radius: iced::border::top(scale::radius::LG - BORDER_WIDTH),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// A column: `deep` in a hairline, corners of 12. The one a dragged card
/// would land in takes the selection's fill and the accent's hairline.
pub fn column(
    t: &Theme,
    over: bool,
    in_dock: bool,
) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| {
        let p = &t.palette;
        // The dock's columns are sections on the panel, not columns.
        let round = if in_dock {
            radius::NONE
        } else {
            scale::radius::LG
        };
        let (background, line) = if over {
            (Some(p.selection_bg), p.accent)
        } else if in_dock {
            (None, Color::TRANSPARENT)
        } else {
            (Some(t.deep()), p.border)
        };
        container::Style {
            background: background.map(Background::Color),
            border: hairline(line, round),
            text_color: Some(p.text),
            ..container::Style::default()
        }
    }
}

/// A card's states: under the pointer, open in the task's sheet, or being
/// dragged.
#[derive(Debug, Clone, Copy, Default)]
pub struct CardLook {
    pub hovered: bool,
    pub selected: bool,
    pub dragging: bool,
}

/// A card: `surface` in a hairline, corners of 8. Under the pointer it is
/// raised; the one whose sheet is open has the selection's fill and the
/// accent around it; the one being dragged is dimmed to 40 %.
pub fn card(t: &Theme, look: CardLook) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| {
        let p = &t.palette;
        let (fill, line) = if look.selected {
            (p.selection_bg, p.accent)
        } else if look.hovered {
            (p.surface_raised, p.border_strong)
        } else {
            (p.surface, p.border)
        };
        let fade = if look.dragging { 0.4 } else { 1.0 };
        container::Style {
            background: Some(Background::Color(alpha(fill, fill.a * fade))),
            border: hairline(alpha(line, line.a * fade), scale::radius::MD),
            shadow: Shadow::default(),
            text_color: Some(alpha(p.text, fade)),
            ..container::Style::default()
        }
    }
}

/// A label on a card (`.chip`): `surfaceRaised` in a hairline, corners of 4.
pub fn chip(t: &Theme) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(Background::Color(t.palette.surface_raised)),
        border: hairline(t.palette.border, scale::radius::XS),
        text_color: Some(t.palette.text_secondary),
        ..container::Style::default()
    }
}

/// Why a card waits for a person: an inset in `surfaceSunken`.
pub fn blocked_inset(t: &Theme) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(Background::Color(t.palette.surface_sunken)),
        border: hairline(t.palette.border, scale::radius::SM),
        text_color: Some(t.palette.text),
        ..container::Style::default()
    }
}

/// The agent's name on a card: a link with nothing around it until it is
/// pointed at.
pub fn name_link(t: &Theme) -> impl Fn(&iced::Theme, button::Status) -> button::Style + '_ {
    move |_, status| {
        let p = &t.palette;
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: hovered.then_some(Background::Color(p.hover_overlay)),
            text_color: p.text,
            border: rounded(scale::radius::XS),
            shadow: Shadow::default(),
            ..button::Style::default()
        }
    }
}

/// A task's sheet: `surface`, with the modal's shadow. Its left edge is a
/// rule of its own, which a container's border cannot be.
pub fn sheet(t: &Theme) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(Background::Color(t.palette.surface)),
        text_color: Some(t.palette.text),
        shadow: Shadow {
            color: t.inks.shadow,
            offset: Vector::new(0.0, 24.0),
            blur_radius: 64.0,
        },
        ..container::Style::default()
    }
}

/// The sheet's badge of the task's column: a pill on the selection's fill.
pub fn state_pill(t: &Theme) -> impl Fn(&iced::Theme, button::Status) -> button::Style + '_ {
    move |_, status| {
        let p = &t.palette;
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(Background::Color(p.selection_bg)),
            text_color: p.accent_text,
            border: hairline(
                if hovered {
                    p.selection_border
                } else {
                    Color::TRANSPARENT
                },
                scale::radius::PILL,
            ),
            shadow: Shadow::default(),
            ..button::Style::default()
        }
    }
}

/// The sheet's title: written as a heading, with a frame only once it has the
/// keyboard.
pub fn heading_field(
    t: &Theme,
) -> impl Fn(&iced::Theme, text_editor::Status) -> text_editor::Style + '_ {
    move |_, status| {
        let p = &t.palette;
        let line = match status {
            text_editor::Status::Focused { .. } => p.focus_ring,
            text_editor::Status::Hovered => p.border_strong,
            _ => Color::TRANSPARENT,
        };
        text_editor::Style {
            background: Background::Color(Color::TRANSPARENT),
            border: hairline(line, scale::radius::SM),
            placeholder: p.text_muted,
            value: p.text,
            selection: p.selection_border.scale_alpha(0.45),
        }
    }
}

/// A comment in the sheet: a card of its own.
pub fn comment(t: &Theme) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(Background::Color(t.palette.surface)),
        border: hairline(t.palette.border, scale::radius::MD),
        text_color: Some(t.palette.text),
        ..container::Style::default()
    }
}

/// The state a column's mark shows (`.column__mark`): an empty ring for what
/// waits, the working arc for what is under way, "your turn" for what waits on
/// the person, and a ticked disc for what is over.
pub fn mark(column: super::model::Column) -> hangar_ui::glyph::GlyphState {
    use super::model::Column;
    use hangar_ui::glyph::GlyphState;
    match column {
        Column::Todo => GlyphState::Idle,
        Column::Doing => GlyphState::Working,
        Column::Review => GlyphState::Yours,
        Column::Done => GlyphState::Completed,
    }
}

/// A button that shows a flag: a dot in the corner of the dock's filter tool
/// when a filter is on while its row is folded away.
pub fn flag(t: &Theme) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(Background::Color(t.palette.accent)),
        border: Border {
            color: t.palette.surface,
            width: 1.5,
            radius: 999.0.into(),
        },
        ..container::Style::default()
    }
}

/// The dock's section header, a button across the row (`.column__toggle`).
pub fn fold_button(t: &Theme) -> impl Fn(&iced::Theme, button::Status) -> button::Style + '_ {
    move |_, status| {
        let (pal, inks) = (&t.palette, &t.inks);
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: hovered.then_some(Background::Color(pal.border)),
            text_color: inks.text.primary,
            border: rounded(radius::SM),
            shadow: Shadow::default(),
            ..button::Style::default()
        }
    }
}

/// The row at the foot of a column that a card is typed into
/// (`.column__add`): nothing around it until it is pointed at, then the page
/// with the accent around it while it has the keyboard.
pub fn add_field(t: &Theme) -> impl Fn(&iced::Theme, text_input::Status) -> text_input::Style + '_ {
    move |_, status| {
        let (pal, inks) = (&t.palette, &t.inks);
        let (background, border) = match status {
            text_input::Status::Focused { .. } => (pal.surface, hairline(pal.accent, radius::LG)),
            text_input::Status::Hovered => (pal.border, rounded(radius::LG)),
            _ => (Color::TRANSPARENT, rounded(radius::LG)),
        };
        text_input::Style {
            background: Background::Color(background),
            border,
            icon: inks.text.muted,
            placeholder: inks.text.muted,
            value: inks.text.primary,
            selection: inks.accent_ring,
        }
    }
}

/// The outline of the first read (`.skeleton`).
pub fn skeleton(t: &Theme) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(Background::Color(t.palette.border)),
        border: rounded(radius::LG),
        ..container::Style::default()
    }
}

/// The icon in the empty board (`.board__empty-icon`).
pub fn empty_icon(t: &Theme) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(Background::Color(t.palette.surface_raised)),
        border: hairline(t.palette.border, radius::XL),
        text_color: Some(t.inks.text.muted),
        ..container::Style::default()
    }
}

/// Why a move was refused, over the board's foot (`.board__toast`).
pub fn toast(t: &Theme) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(Background::Color(t.palette.surface_raised)),
        border: hairline(t.palette.border_strong, radius::LG),
        text_color: Some(t.inks.text.primary),
        shadow: Shadow {
            color: t.inks.shadow,
            offset: Vector::new(0.0, 8.0),
            blur_radius: 24.0,
        },
        ..container::Style::default()
    }
}

/// A bar of lit and unlit priority (`.prio__bars i`).
pub fn prio_bar(t: &Theme, lit: bool) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| container::Style {
        background: Some(Background::Color(if lit {
            t.palette.text_muted
        } else {
            t.palette.border_strong
        })),
        border: rounded(1.0),
        ..container::Style::default()
    }
}

/// The list of a task's dependencies, a hairline around rows.
pub fn deps(t: &Theme) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |_| container::Style {
        border: hairline(t.palette.border, radius::LG),
        ..container::Style::default()
    }
}
