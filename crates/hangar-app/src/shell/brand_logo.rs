//! The Teskad mark in the interface: the pack's symbol (docs/teskad-brand,
//! `logo/mark/`), in the variant that reads on the theme's appearance, and in
//! its `-small` form under 24 px, where strokes and dots are thicker.
//!
//! Decorative wherever it ships: the name is always written beside it.

use hangar_theme::Theme;
use iced::widget::svg;
use iced::{Color, Element, Length};
use std::sync::LazyLock;

/// Under this side, in pixels, the pack asks for the `-small` files.
const SMALL_BELOW: f32 = 24.0;

macro_rules! mark {
    ($name:literal) => {
        LazyLock::new(|| {
            svg::Handle::from_memory(
                include_bytes!(concat!("brand/teskad-mark-", $name, ".svg")).as_slice(),
            )
        })
    };
}

static ON_DARK: LazyLock<svg::Handle> = mark!("color-on-dark");
static ON_DARK_SMALL: LazyLock<svg::Handle> = mark!("color-on-dark-small");
static ON_LIGHT: LazyLock<svg::Handle> = mark!("color-on-light");
static ON_LIGHT_SMALL: LazyLock<svg::Handle> = mark!("color-on-light-small");

static MONO_SMALL: LazyLock<svg::Handle> = mark!("mono-white-small");

macro_rules! lockup {
    ($name:literal) => {
        LazyLock::new(|| {
            svg::Handle::from_memory(
                include_bytes!(concat!("brand/teskad-lockup-stacked-", $name, ".svg")).as_slice(),
            )
        })
    };
}

static LOCKUP_ON_DARK: LazyLock<svg::Handle> = lockup!("color-on-dark");
static LOCKUP_ON_LIGHT: LazyLock<svg::Handle> = lockup!("color-on-light");

/// The stacked lockup's own size, in the pack's file.
const LOCKUP: (f32, f32) = (92.0, 105.0);

/// The small mark in one colour, for a place where it only says what a thing
/// is (a workspace that orchestrates, not the one in front).
pub fn mono<'a, M: 'a>(side: f32, color: Color) -> Element<'a, M> {
    svg(MONO_SMALL.clone())
        .width(Length::Fixed(side))
        .height(Length::Fixed(side))
        .style(move |_: &iced::Theme, _| svg::Style { color: Some(color) })
        .into()
}

fn lockup_handle(dark: bool) -> svg::Handle {
    if dark {
        LOCKUP_ON_DARK.clone()
    } else {
        LOCKUP_ON_LIGHT.clone()
    }
}

/// The mark over the name, as the pack draws them together: the About page's.
pub fn stacked_lockup<'a, M: 'a>(t: &Theme) -> Element<'a, M> {
    svg(lockup_handle(t.is_dark()))
        .width(Length::Fixed(LOCKUP.0))
        .height(Length::Fixed(LOCKUP.1))
        .into()
}

/// The mark's file for a theme and a size.
fn handle(dark: bool, side: f32) -> svg::Handle {
    match (dark, side < SMALL_BELOW) {
        (true, false) => ON_DARK.clone(),
        (true, true) => ON_DARK_SMALL.clone(),
        (false, false) => ON_LIGHT.clone(),
        (false, true) => ON_LIGHT_SMALL.clone(),
    }
}

/// The mark, `side` pixels across.
pub fn logo<'a, M: 'a>(t: &Theme, side: f32) -> Element<'a, M> {
    svg(handle(t.is_dark(), side))
        .width(Length::Fixed(side))
        .height(Length::Fixed(side))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_sizes_take_the_small_files_and_the_appearance_picks_the_colours() {
        for (dark, side, name) in [
            (true, 15.0, "color-on-dark-small"),
            (true, 23.9, "color-on-dark-small"),
            (true, 24.0, "color-on-dark"),
            (false, 15.0, "color-on-light-small"),
            (false, 64.0, "color-on-light"),
        ] {
            let expected = match name {
                "color-on-dark-small" => ON_DARK_SMALL.clone(),
                "color-on-dark" => ON_DARK.clone(),
                "color-on-light-small" => ON_LIGHT_SMALL.clone(),
                _ => ON_LIGHT.clone(),
            };
            assert_eq!(handle(dark, side).id(), expected.id(), "{name} at {side}");
        }
    }

    #[test]
    fn the_about_pages_lockup_follows_the_appearance() {
        assert_eq!(lockup_handle(true).id(), LOCKUP_ON_DARK.id());
        assert_eq!(lockup_handle(false).id(), LOCKUP_ON_LIGHT.id());
        assert_ne!(LOCKUP_ON_DARK.id(), LOCKUP_ON_LIGHT.id());
    }

    #[test]
    fn the_bundled_marks_are_the_teskad_ones() {
        for bytes in [
            include_str!("brand/teskad-mark-color-on-dark.svg"),
            include_str!("brand/teskad-mark-color-on-dark-small.svg"),
            include_str!("brand/teskad-mark-color-on-light.svg"),
            include_str!("brand/teskad-mark-color-on-light-small.svg"),
            include_str!("brand/teskad-mark-mono-white-small.svg"),
            include_str!("brand/teskad-lockup-stacked-color-on-dark.svg"),
            include_str!("brand/teskad-lockup-stacked-color-on-light.svg"),
        ] {
            assert!(bytes.contains("aria-label=\"Teskad\""));
        }
    }
}
