//! The fonts of the window. Inter is the interface's face on every platform,
//! so that the window reads the same everywhere. Space Grotesk (600) sets the
//! titles of views and the heads of panels, and JetBrains Mono the terminals
//! and the technical texts: branches, hashes, paths, key hints. All three are
//! the Teskad brand pack's (`docs/teskad-brand/fonts`), embedded with their
//! licences.
//!
//! iced draws text with cosmic-text, which reads TrueType and OpenType and not
//! WOFF2, so the `inter.woff2` the kits are served cannot be loaded; that
//! file is also a subset made for the web. The window uses Inter Variable as
//! the Inter project ships it, a TTF, from `fonts/` (SIL Open Font License
//! 1.1, `fonts/Inter-LICENSE.txt`, which has to travel with the font). Its
//! weight axis is what gives the medium (500) and semibold (600) the
//! stylesheets use from a single file.
//!
//! The app loads it once, and makes it the default:
//!
//! ```ignore
//! iced::application(App::new, App::update, App::view)
//!     .font(hangar_theme::fonts::INTER_VARIABLE)
//!     .font(hangar_theme::fonts::SPACE_GROTESK_VARIABLE)
//!     .font(hangar_theme::fonts::JETBRAINS_MONO_VARIABLE)
//!     .default_font(hangar_theme::fonts::ui())
//! ```

use iced::Font;
use iced::font::{Family, Stretch, Style, Weight};

/// Inter Variable (TTF), for `Settings::fonts` or `Application::font`.
pub const INTER_VARIABLE: &[u8] = include_bytes!("../fonts/InterVariable.ttf");

/// The licence of the font above, to show in an about page.
pub const INTER_LICENSE: &str = include_str!("../fonts/Inter-LICENSE.txt");

/// Space Grotesk Variable (TTF), weights 300 to 700.
pub const SPACE_GROTESK_VARIABLE: &[u8] = include_bytes!("../fonts/SpaceGrotesk-Variable.ttf");

/// JetBrains Mono Variable (TTF), weights 100 to 800.
pub const JETBRAINS_MONO_VARIABLE: &[u8] = include_bytes!("../fonts/JetBrainsMono-Variable.ttf");

/// The licences of the two fonts above (SIL OFL 1.1), to show in an about page.
pub const SPACE_GROTESK_LICENSE: &str = include_str!("../fonts/SpaceGrotesk-LICENSE.txt");
pub const JETBRAINS_MONO_LICENSE: &str = include_str!("../fonts/JetBrainsMono-LICENSE.txt");

/// Every font file the window embeds: what the app loads at start.
pub const ALL: [&[u8]; 3] = [
    INTER_VARIABLE,
    SPACE_GROTESK_VARIABLE,
    JETBRAINS_MONO_VARIABLE,
];

/// Space Grotesk's family name as its file declares it.
pub const DISPLAY_FAMILY: &str = "Space Grotesk";

/// JetBrains Mono's family name as its file declares it.
pub const MONO_FAMILY: &str = "JetBrains Mono";

/// Inter's family name as the font file declares it, what `font-family`
/// said in the web app (`"Inter Variable"`).
pub const UI_FAMILY: &str = "Inter Variable";

const fn inter(weight: Weight) -> Font {
    Font {
        family: Family::Name(UI_FAMILY),
        weight,
        stretch: Stretch::Normal,
        style: Style::Normal,
    }
}

/// The interface font at regular weight: the window's default.
pub const fn ui() -> Font {
    inter(Weight::Normal)
}

/// Buttons, tabs, segments (weight 500).
pub const fn ui_medium() -> Font {
    inter(Weight::Medium)
}

/// Titles, labels, a primary button (weight 600).
pub const fn ui_semibold() -> Font {
    inter(Weight::Semibold)
}

/// The interface font at a weight of CSS, as the stylesheets name them
/// (400, 500, 600, 700). The 550 and 650 the stylesheets use once each go to
/// semibold, since iced has only those four weights.
pub const fn ui_weight(css_weight: u16) -> Font {
    inter(match css_weight {
        0..=449 => Weight::Normal,
        450..=549 => Weight::Medium,
        550..=650 => Weight::Semibold,
        _ => Weight::Bold,
    })
}

/// Titles of views and heads of panels: Space Grotesk at weight 600.
pub const fn display() -> Font {
    Font {
        family: Family::Name(DISPLAY_FAMILY),
        weight: Weight::Semibold,
        stretch: Stretch::Normal,
        style: Style::Normal,
    }
}

/// What the interface sets in mono: branches, hashes, paths, versions, key
/// hints. The terminals have a font of their own, from the settings, which
/// is JetBrains Mono until the person picks another.
pub const fn mono() -> Font {
    Font {
        family: Family::Name(MONO_FAMILY),
        weight: Weight::Normal,
        stretch: Stretch::Normal,
        style: Style::Normal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_font_is_a_truetype_file_with_the_family_the_stylesheets_name() {
        // TrueType, not WOFF2 (`wOF2`), which cosmic-text cannot read.
        assert_eq!(&INTER_VARIABLE[..4], &[0, 1, 0, 0]);
        let needle = UI_FAMILY
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect::<Vec<_>>();
        assert!(
            INTER_VARIABLE
                .windows(needle.len())
                .any(|window| window == needle)
        );
    }

    #[test]
    fn the_brand_fonts_are_truetype_files_with_the_families_the_window_asks_for() {
        for (bytes, family) in [
            (SPACE_GROTESK_VARIABLE, DISPLAY_FAMILY),
            (JETBRAINS_MONO_VARIABLE, MONO_FAMILY),
        ] {
            assert_eq!(&bytes[..4], &[0, 1, 0, 0], "{family}");
            let needle = family
                .encode_utf16()
                .flat_map(u16::to_be_bytes)
                .collect::<Vec<_>>();
            assert!(
                bytes.windows(needle.len()).any(|window| window == needle),
                "{family}"
            );
        }
        assert_eq!(display().weight, Weight::Semibold);
        assert_eq!(mono().family, Family::Name("JetBrains Mono"));
    }

    #[test]
    fn the_licence_travels_with_the_font() {
        for licence in [INTER_LICENSE, SPACE_GROTESK_LICENSE, JETBRAINS_MONO_LICENSE] {
            assert!(licence.contains("SIL OPEN FONT LICENSE Version 1.1"));
        }
    }

    #[test]
    fn weights_round_to_the_nearest_step() {
        assert_eq!(ui_weight(400).weight, Weight::Normal);
        assert_eq!(ui_weight(500).weight, Weight::Medium);
        assert_eq!(ui_weight(550).weight, Weight::Semibold);
        assert_eq!(ui_weight(600).weight, Weight::Semibold);
        assert_eq!(ui_weight(650).weight, Weight::Semibold);
        assert_eq!(ui_weight(700).weight, Weight::Bold);
    }
}
