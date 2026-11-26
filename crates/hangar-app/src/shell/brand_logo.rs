//! The Teskad mark in the interface: the pack's symbol (docs/teskad-brand,
//! `logo/mark/`), in the variant that reads on the theme's appearance, and in
//! its `-small` form under 24 px, where strokes and dots are thicker.
//!
//! Decorative wherever it ships: the name is always written beside it.

use hangar_theme::Theme;
use iced::widget::svg;
use iced::{Element, Length};
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
    fn the_bundled_marks_are_the_teskad_ones() {
        for bytes in [
            include_str!("brand/teskad-mark-color-on-dark.svg"),
            include_str!("brand/teskad-mark-color-on-dark-small.svg"),
            include_str!("brand/teskad-mark-color-on-light.svg"),
            include_str!("brand/teskad-mark-color-on-light-small.svg"),
        ] {
            assert!(bytes.contains("aria-label=\"Teskad\""));
        }
    }
}
