//! The themes of Teskad: the 33 palettes the web app had (`src/themes.ts`, tag
//! v1.12.1), the colours of the whole window derived from them, the design
//! tokens of the stylesheets, and how all of it maps onto iced.
//!
//! See docs/native/PLAN.md, "hangar-theme".
//!
//! - [`themes`], [`by_id`], [`DEFAULT_ID`]: the bundled [`Theme`]s. A theme
//!   restyles the whole window, not only the terminals: [`Theme::palette`] is
//!   the one set of semantic colours of the interface ([`Palette`]),
//!   [`Theme::colors`] the finer tokens computed from it (surfaces, hairlines,
//!   the agent states, the statistics) and [`Theme::terminal`] the colours a
//!   terminal is drawn with. Any theme can let the desktop show through the
//!   window, at the opacity the person sets ([`Theme::with_window_opacity`]):
//!   on Linux only, behind the window's page, its bars and the terminals'
//!   default background, never behind text, cards, menus or dialogs, and
//!   with every colour of the theme as it is. Blurring what shows through is
//!   the compositor's (KDE does, GNOME with an extension such as Blur my
//!   Shell).
//! - [`tokens`]: what is not a colour: radii, spacing, font sizes, weights,
//!   durations, the sizes of the window's parts.
//! - [`to_iced`] and [`style`]: the theme for iced, and the style functions
//!   with which views name a role instead of a colour.
//! - [`fonts`]: Inter, Space Grotesk and JetBrains Mono, as bytes for iced to load and `Font`s to ask for.
//! - [`kit_tokens`]: what kits are told of the theme.
//! - [`contrast`], [`derive_palette`] and [`derive_inks`]: the colour
//!   arithmetic and the rules that turn a terminal palette into a window
//!   (`docs/teskad-brand/docs/themes.md`), and the contrast check
//!   ([`contrast_failures`]).
//! - [`fixed`]: the few colours no theme chooses.
//!
//! # Adding a theme
//!
//! A theme is a JSON file in `themes/`, in the format of the brand pack's
//! `theme.schema.json` (`docs/teskad-brand/themes`).
//!
//! 1. Write `themes/<id>.json`: `id` (lowercase letters, digits and `-`, never
//!    changed once released), `name`, `appearance`, the `terminal` colours and,
//!    only when the theme's main colour is not its ANSI magenta, `"ui":
//!    {"accent": "#..."}`. Every other interface colour is derived.
//! 2. List it in `FILES` of `bundled.rs`, the dark themes before the light
//!    ones, and raise the array's length and the count in `are_35_and_every_one_is_read_strictly`.
//! 3. Run `cargo test -p hangar-theme`: the contrast tests say whether the
//!    palette reads, and what the derivation corrected is already in it.
//! 4. If one of the two themes `crates/hangar-core/kit-sdk/tokens.json` names
//!    changed, run `node scripts/gen-hangar-ui.mjs` for the kits' stylesheet;
//!    the `kit` tests fail until it is.

mod bundled;
pub mod contrast;
mod derive;
pub mod fixed;
pub mod fonts;
mod iced_theme;
mod inks;
mod kit;
mod palette;
mod spec;
pub mod style;
mod theme;
pub mod tokens;

pub use bundled::theme_files;
pub use contrast::{
    HueAndSaturation, Rgb, contrast_ratio, ensure_contrast, farthest_pole, hue_and_saturation,
    hue_distance, mix, over, perceived_lightness, relative_luminance, with_hue, worst_contrast,
};
pub use derive::{
    BORDER_CONTRAST, FILL_CONTRAST, TEXT_CONTRAST, contrast_failures, derive_inks, derive_palette,
    ink_surfaces,
};
pub use iced_theme::to_iced;
pub use inks::{Inks, TextInks, Viz};
pub use kit::{CHROME_TOKEN_NAMES, chrome_token, css, kit_default_themes, kit_tokens};
pub use palette::{Palette, Status};
pub use spec::{ThemeSpec, UiOverrides};
pub use theme::{
    AgentState, Appearance, DEFAULT_ID, MIN_WINDOW_OPACITY, SearchColors, StateColors,
    TerminalColors, Theme, by_id, by_id_or_default, default_theme, themes,
};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod inks_tests;
