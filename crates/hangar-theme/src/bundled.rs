//! The bundled themes: the JSON files of `themes/`, in the format of the brand
//! pack's `theme.schema.json`, embedded in the binary and read once.

use std::sync::OnceLock;

use crate::derive::{derive_colors, derive_palette};
use crate::palette::{Palette, Status};
use crate::spec::ThemeSpec;
use crate::theme::{Appearance, Theme};

/// Every bundled theme file, in the order the picker shows them: Teskad Dark
/// first, the dark themes, then Teskad Light and the light ones. A theme's
/// `id` is what settings store, so it never changes once released.
const FILES: [&str; 35] = [
    include_str!("../themes/teskad-dark.json"),
    include_str!("../themes/graphite.json"),
    include_str!("../themes/tokyo-night.json"),
    include_str!("../themes/catppuccin-mocha.json"),
    include_str!("../themes/dracula.json"),
    include_str!("../themes/one-dark.json"),
    include_str!("../themes/nord.json"),
    include_str!("../themes/gruvbox-dark.json"),
    include_str!("../themes/github-dark.json"),
    include_str!("../themes/ayu-dark.json"),
    include_str!("../themes/rose-pine.json"),
    include_str!("../themes/solarized-dark.json"),
    include_str!("../themes/vscode-dark.json"),
    include_str!("../themes/night-owl.json"),
    include_str!("../themes/monokai-pro.json"),
    include_str!("../themes/material-palenight.json"),
    include_str!("../themes/synthwave-84.json"),
    include_str!("../themes/cobalt2.json"),
    include_str!("../themes/shades-of-purple.json"),
    include_str!("../themes/andromeda.json"),
    include_str!("../themes/ayu-mirage.json"),
    include_str!("../themes/horizon.json"),
    include_str!("../themes/panda.json"),
    include_str!("../themes/kanagawa.json"),
    include_str!("../themes/everforest-dark.json"),
    include_str!("../themes/poimandres.json"),
    include_str!("../themes/teskad-light.json"),
    include_str!("../themes/catppuccin-latte.json"),
    include_str!("../themes/github-light.json"),
    include_str!("../themes/one-light.json"),
    include_str!("../themes/solarized-light.json"),
    include_str!("../themes/quiet-light.json"),
    include_str!("../themes/ayu-light.json"),
    include_str!("../themes/rose-pine-dawn.json"),
    include_str!("../themes/night-owl-light.json"),
];

/// The raw text of every bundled theme file, for tests and tools.
pub fn theme_files() -> &'static [&'static str] {
    &FILES
}

/// The window's opacity of a theme as it ships: solid. The person's setting
/// makes it less (`Theme::with_window_opacity`).
const SOLID: f32 = 1.0;

/// A theme from its parsed file: the palette is what the file sets, with what
/// it leaves out derived, and the finer [`crate::Colors`] are computed from
/// that palette.
fn build(spec: ThemeSpec) -> Theme {
    let palette = derive_palette(spec.appearance, &spec.terminal, &spec.ui);
    Theme {
        id: Box::leak(spec.id.into_boxed_str()),
        name: Box::leak(spec.name.into_boxed_str()),
        appearance: spec.appearance,
        terminal: spec.terminal,
        palette,
        colors: derive_colors(spec.appearance, &palette, &spec.terminal),
        window_opacity: SOLID,
    }
}

pub(crate) fn themes() -> &'static [Theme] {
    static THEMES: OnceLock<Vec<Theme>> = OnceLock::new();
    THEMES.get_or_init(|| {
        FILES
            .iter()
            .enumerate()
            .map(|(index, json)| {
                let spec = ThemeSpec::parse(json)
                    .unwrap_or_else(|error| panic!("bundled theme #{index} is invalid: {error}"));
                build(spec)
            })
            .collect()
    })
}

/// The Teskad palette of an appearance, exactly as its file gives it: what a
/// derived state colour falls back on when it does not read.
pub(crate) fn teskad_palette(appearance: Appearance) -> Palette {
    static PACK: OnceLock<[Palette; 2]> = OnceLock::new();
    let both = PACK.get_or_init(|| {
        [FILES[0], FILES[26]].map(|json| {
            let spec = ThemeSpec::parse(json).expect("the Teskad themes are valid");
            // Every `ui` key is set, so nothing is derived (and nothing
            // reaches back here).
            derive_palette_complete(&spec)
        })
    });
    both[usize::from(!appearance.is_dark())]
}

pub(crate) fn pack_status(appearance: Appearance) -> Status {
    teskad_palette(appearance).status
}

/// The palette of a file that sets every key, which is read as it is.
fn derive_palette_complete(spec: &ThemeSpec) -> Palette {
    let ui = &spec.ui;
    let key = |value: Option<iced::Color>| value.expect("a Teskad theme sets every ui key");
    Palette {
        bg: key(ui.bg),
        surface: key(ui.surface),
        surface_raised: key(ui.surface_raised),
        surface_sunken: key(ui.surface_sunken),
        border: key(ui.border),
        border_strong: key(ui.border_strong),
        text: key(ui.text),
        text_secondary: key(ui.text_secondary),
        text_muted: key(ui.text_muted),
        accent: key(ui.accent),
        accent_hover: key(ui.accent_hover),
        accent_pressed: key(ui.accent_pressed),
        on_accent: key(ui.on_accent),
        accent_text: key(ui.accent_text),
        focus_ring: key(ui.focus_ring),
        selection_bg: key(ui.selection_bg),
        selection_border: key(ui.selection_border),
        hover_overlay: key(ui.hover_overlay),
        scrim: key(ui.scrim),
        status: Status {
            running: key(ui.running),
            completed: key(ui.completed),
            failed: key(ui.failed),
            active: key(ui.active),
            inactive: key(ui.inactive),
        },
    }
}
