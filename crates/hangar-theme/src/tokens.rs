//! The design tokens that are not colours, taken from `src/styles.css` and
//! `src/styles/`.
//!
//! The stylesheets name only the radii and the transition (`:root`); the
//! rest were repeated numbers, and the ones that repeat are named here once.
//! All lengths are in logical pixels at an interface size of 100 %: the web
//! app multiplied each by `--ui-scale`, and the app does the same by handing
//! iced a scale factor, so nothing here is ever multiplied again.

/// Corner radii, as the Teskad brand pack has them (`docs/teskad-brand`):
/// controls 6, cards and fields 8, panels and modals 12, and no pill but a
/// badge. Panes themselves stay square and share 1px separators.
pub mod radius {
    /// `--radius-xs`: a segment of a segmented control, a checkbox.
    pub const XS: f32 = 4.0;
    /// `--radius-sm`: a menu item, a tab. A control: 6.
    pub const SM: f32 = 6.0;
    /// `--radius`: a button.
    pub const MD: f32 = 6.0;
    /// `--radius-lg`: a card, a field, a context menu.
    pub const LG: f32 = 8.0;
    /// `--radius-xl`: a dialog, the command palette: a panel or a modal, 12.
    pub const XL: f32 = 12.0;
    /// The track of a switch, a badge, a progress bar: any radius larger than
    /// half the shortest side, which iced clamps. Nothing else is a pill.
    pub const FULL: f32 = 999.0;
    /// A pane, a column: the panes are square.
    pub const NONE: f32 = 0.0;
}

/// The spacing scale: the gaps and paddings that recur in the stylesheets.
/// Others occur (1, 3, 5, 7, 9, 14), nearly always to nudge a glyph into line
/// with its text; those are written where they are needed.
pub mod space {
    /// A hairline between two things, and the width of a border.
    pub const HAIRLINE: f32 = 1.0;
    /// The gap in a segmented control, an inner padding.
    pub const XXS: f32 = 2.0;
    pub const XS: f32 = 4.0;
    /// The gap between an icon and its label.
    pub const SM: f32 = 6.0;
    /// The usual gap and padding: 68 of the stylesheets' gaps.
    pub const MD: f32 = 8.0;
    pub const LG: f32 = 10.0;
    pub const XL: f32 = 12.0;
    pub const XXL: f32 = 16.0;
    /// The padding of a dialog's body.
    pub const XXXL: f32 = 24.0;
}

/// Font sizes. The window reads in 12 and 13; the small ones carry metadata.
pub mod font_size {
    // Every text size is the stylesheet's plus a pixel (the three smallest a
    // pixel and a half, the 11.5 px step a pixel and a quarter, to keep the
    // scale strictly rising): a browser drew them with the system's subpixel
    // smoothing, and smoothed in grey the same sizes read too thin. The icon
    // size is the stylesheet's own.
    /// A mono counter or a version (`.nav-top__version`).
    pub const MICRO: f32 = 11.5;
    /// A group label in small capitals (`.nav-section`), a key hint.
    pub const LABEL: f32 = 12.0;
    /// A caption, a metadata line.
    pub const CAPTION: f32 = 12.5;
    /// A tiny button, a row's secondary line.
    pub const CAPTION_LG: f32 = 12.75;
    /// The toolbar, the navigator, the status bar, fields, the palette.
    pub const UI: f32 = 13.0;
    /// The navigator's title.
    pub const UI_LG: f32 = 13.5;
    /// The page's text: `body`, menu items.
    pub const BODY: f32 = 14.0;
    /// A dialog's or a page's title.
    pub const TITLE: f32 = 16.0;
    /// The palette's search glyph.
    pub const ICON_LG: f32 = 17.0;
    /// The big number of a statistic.
    pub const DISPLAY: f32 = 27.0;
}

/// Font weights, as the numbers of CSS. The stylesheets also use 550 and 650
/// once each, which iced's four weights cannot tell from 500 and 600.
pub mod font_weight {
    pub const REGULAR: u16 = 400;
    /// Buttons, tabs, segments.
    pub const MEDIUM: u16 = 500;
    /// Titles, labels, a primary button: the most used after 500.
    pub const SEMIBOLD: u16 = 600;
    pub const BOLD: u16 = 700;
}

/// Line heights, as a multiple of the font size (what iced's
/// `LineHeight::Relative` takes).
pub mod line_height {
    /// A single line in a control, an icon.
    pub const NONE: f32 = 1.0;
    pub const TIGHT: f32 = 1.25;
    /// A title that may wrap.
    pub const TITLE: f32 = 1.3;
    /// Fields and most text: the one used most.
    pub const UI: f32 = 1.4;
    pub const BODY: f32 = 1.5;
    /// A paragraph of prose (the markdown preview).
    pub const PROSE: f32 = 1.6;
}

/// Letter spacing in em. iced has no letter spacing; kept so the one place
/// that needs it (small capitals) can be approximated, and for the day it has.
pub mod letter_spacing {
    pub const TIGHT: f32 = -0.01;
    pub const NORMAL: f32 = 0.0;
    pub const CAPITALS: f32 = 0.06;
}

/// How long things take to change.
pub mod duration {
    use std::time::Duration;

    /// `--dur`: a hover, a focus, most transitions.
    pub const BASE: Duration = Duration::from_millis(140);
    /// A context menu appearing.
    pub const FAST: Duration = Duration::from_millis(100);
    /// A dialog or a pill rising into place.
    pub const MODAL: Duration = Duration::from_millis(180);
    /// The command palette: `--dur` and 30 ms.
    pub const PALETTE: Duration = Duration::from_millis(170);
    /// The update banner sliding in.
    pub const BANNER: Duration = Duration::from_millis(220);
    /// A board revealing itself.
    pub const REVEAL: Duration = Duration::from_millis(240);
    /// The pulse of a working agent's glyph.
    pub const PULSE: Duration = Duration::from_millis(1400);

    /// `--ease`: `cubic-bezier(0.2, 0.7, 0.2, 1)`, as the control points
    /// `[x1, y1, x2, y2]`.
    pub const EASE: [f32; 4] = [0.2, 0.7, 0.2, 1.0];
}

/// Sizes of the window's parts and of its controls.
pub mod size {
    /// The navigator's width (`--sidebar-w`).
    pub const SIDEBAR_WIDTH: f32 = 252.0;
    /// The toolbar and the navigator's top row, level so the window reads as
    /// one band.
    pub const TOOLBAR_HEIGHT: f32 = 44.0;
    /// The status bar, as tall as the navigator's bottom bar.
    pub const STATUS_BAR_HEIGHT: f32 = 36.0;
    /// A workspace in the navigator (`--nav-row`).
    pub const NAV_ROW_HEIGHT: f32 = 28.0;
    /// A button, a field, a segmented control.
    pub const CONTROL_HEIGHT: f32 = 26.0;
    /// A tiny button, a segment in the toolbar.
    pub const CONTROL_HEIGHT_SM: f32 = 22.0;
    /// A line of a menu.
    pub const MENU_ITEM_HEIGHT: f32 = 26.0;
    /// A pane's header.
    pub const PANE_HEAD_HEIGHT: f32 = 30.0;
    /// The palette's search field.
    pub const PALETTE_SEARCH_HEIGHT: f32 = 50.0;
    /// Most icons.
    pub const ICON: f32 = 14.0;
    pub const ICON_LG: f32 = 16.0;
    /// A scrollbar, rail and thumb.
    pub const SCROLLBAR: f32 = 9.0;
    /// A switch: its track, and its knob.
    pub const SWITCH_WIDTH: f32 = 36.0;
    pub const SWITCH_HEIGHT: f32 = 20.0;
    pub const SWITCH_KNOB: f32 = 16.0;
    /// A dialog's width, and a wider list dialog's.
    pub const DIALOG_WIDTH: f32 = 520.0;
    pub const DIALOG_LIST_WIDTH: f32 = 560.0;
    /// The command palette's width.
    pub const PALETTE_WIDTH: f32 = 640.0;
    /// A context menu's bounds.
    pub const MENU_MIN_WIDTH: f32 = 200.0;
    pub const MENU_MAX_WIDTH: f32 = 280.0;
}

/// The brand pack's scales, as `docs/teskad-brand/tokens/tokens.json` (and its
/// generated `theme.rs`) has them, under the pack's own names. They do not
/// depend on the theme: a theme changes colours and nothing else. A test
/// reads `tokens.json` and fails when one of these drifts from it.
///
/// The older modules above keep the names the window was built with; a new
/// component takes its sizes from here.
pub mod scale {
    /// `space`: the 4 px grid.
    pub mod space {
        pub const S0: f32 = 0.0;
        pub const S1: f32 = 4.0;
        pub const S2: f32 = 8.0;
        pub const S3: f32 = 12.0;
        pub const S4: f32 = 16.0;
        pub const S5: f32 = 20.0;
        pub const S6: f32 = 24.0;
        pub const S8: f32 = 32.0;
        pub const S10: f32 = 40.0;
        pub const S12: f32 = 48.0;
    }

    /// `radius`: 4 for badges and keys, 6 for buttons, menu items and tabs,
    /// 8 for fields, cards and menus, 12 for panels, modals and blocks of
    /// settings, a pill for role badges only.
    pub mod radius {
        pub const XS: f32 = 4.0;
        pub const SM: f32 = 6.0;
        pub const MD: f32 = 8.0;
        pub const LG: f32 = 12.0;
        pub const XL: f32 = 16.0;
        pub const PILL: f32 = 999.0;
    }

    /// `size`: the heights of controls and icons, and of the big bars.
    pub mod size {
        pub const CONTROL_SM: f32 = 28.0;
        pub const CONTROL_MD: f32 = 32.0;
        pub const CONTROL_LG: f32 = 40.0;
        pub const ICON_SM: f32 = 14.0;
        pub const ICON_MD: f32 = 16.0;
        pub const ICON_LG: f32 = 20.0;
        pub const TITLE_BAR: f32 = 44.0;
        pub const SIDEBAR: f32 = 248.0;
        /// The smallest icon-only button of the mockups (a pane header's).
        pub const ICON_BUTTON_XS: f32 = 26.0;
    }

    /// A step of the type scale: size, line height, weight.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct TypeStep {
        pub size: f32,
        pub line_height: f32,
        pub weight: u16,
    }

    const fn step(size: f32, line_height: f32, weight: u16) -> TypeStep {
        TypeStep {
            size,
            line_height,
            weight,
        }
    }

    impl TypeStep {
        /// The line height as the multiple of the size iced's
        /// `LineHeight::Relative` takes.
        pub fn relative_line_height(self) -> f32 {
            self.line_height / self.size
        }
    }

    /// `type`: the typographic scale. Sizes are in logical pixels, like all
    /// the others.
    pub mod type_scale {
        use super::{TypeStep, step};

        pub const CAPTION: TypeStep = step(12.0, 16.0, 500);
        pub const SMALL: TypeStep = step(13.0, 18.0, 400);
        pub const BODY: TypeStep = step(14.0, 20.0, 400);
        pub const BODY_STRONG: TypeStep = step(14.0, 20.0, 600);
        pub const SUBTITLE: TypeStep = step(16.0, 22.0, 600);
        pub const TITLE: TypeStep = step(20.0, 26.0, 600);
        pub const HEADLINE: TypeStep = step(28.0, 34.0, 600);
        pub const CODE: TypeStep = step(13.0, 20.0, 400);
        pub const LABEL: TypeStep = step(11.0, 14.0, 500);
    }

    /// `borderWidth`.
    pub const BORDER_DEFAULT: f32 = 1.0;
    pub const BORDER_FOCUS: f32 = 2.0;

    /// How far from the control the focus ring is drawn (`charte §6`: 2 px).
    pub const FOCUS_OFFSET: f32 = 2.0;

    /// `font.fallback`: what a platform without one of the three faces
    /// reads in. iced hands text to cosmic-text, which falls back to the
    /// system's fonts by itself; these are kept so the window and the kits
    /// name the same ones.
    pub mod fallback {
        pub const DISPLAY: &str = "Inter, system-ui, sans-serif";
        pub const UI: &str = "system-ui, -apple-system, 'Segoe UI', sans-serif";
        pub const MONO: &str = "ui-monospace, 'SF Mono', Menlo, Consolas, monospace";
    }
}

/// The focus ring: a 2 px outline in `accent_ring`, in the web app drawn
/// outside the control and here as the control's border, since iced has no
/// outline.
pub const FOCUS_RING_WIDTH: f32 = 2.0;

/// How far a disabled control fades.
pub const DISABLED_OPACITY: f32 = 0.45;

/// The width of every hairline border.
pub const BORDER_WIDTH: f32 = 1.0;

/// Bounds of the interface size slider, in percent (`UI_SCALE_MIN` and
/// `UI_SCALE_MAX` of `src/types.ts`). The app turns the setting into the
/// scale factor it gives iced.
pub const UI_SCALE_MIN_PERCENT: u32 = 90;
pub const UI_SCALE_MAX_PERCENT: u32 = 130;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_radii_step_up() {
        let steps = [radius::XS, radius::SM, radius::MD, radius::LG, radius::XL];
        assert!(steps.windows(2).all(|pair| pair[0] <= pair[1]));
    }

    #[test]
    fn the_radii_are_the_brand_packs() {
        assert_eq!(
            [radius::XS, radius::SM, radius::MD, radius::LG, radius::XL],
            [4.0, 6.0, 6.0, 8.0, 12.0]
        );
    }

    #[test]
    fn the_spacing_scale_steps_up() {
        let steps = [
            space::HAIRLINE,
            space::XXS,
            space::XS,
            space::SM,
            space::MD,
            space::LG,
            space::XL,
            space::XXL,
            space::XXXL,
        ];
        assert!(steps.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn the_font_sizes_step_up() {
        let steps = [
            font_size::MICRO,
            font_size::LABEL,
            font_size::CAPTION,
            font_size::CAPTION_LG,
            font_size::UI,
            font_size::UI_LG,
            font_size::BODY,
            font_size::TITLE,
            font_size::ICON_LG,
            font_size::DISPLAY,
        ];
        assert!(steps.windows(2).all(|pair| pair[0] < pair[1]));
    }
    /// The scales are the pack's: read `tokens.json` and compare.
    #[test]
    fn the_scales_are_the_brand_packs_tokens_json() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/teskad-brand/tokens/tokens.json"
        );
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).expect("the pack's tokens"))
                .expect("valid JSON");
        let num = |value: &serde_json::Value| value.as_f64().expect("a number") as f32;

        let space = &json["space"];
        for (key, ours) in [
            ("0", scale::space::S0),
            ("1", scale::space::S1),
            ("2", scale::space::S2),
            ("3", scale::space::S3),
            ("4", scale::space::S4),
            ("5", scale::space::S5),
            ("6", scale::space::S6),
            ("8", scale::space::S8),
            ("10", scale::space::S10),
            ("12", scale::space::S12),
        ] {
            assert_eq!(num(&space[key]), ours, "space {key}");
        }
        let radius = &json["radius"];
        for (key, ours) in [
            ("xs", scale::radius::XS),
            ("sm", scale::radius::SM),
            ("md", scale::radius::MD),
            ("lg", scale::radius::LG),
            ("xl", scale::radius::XL),
            ("pill", scale::radius::PILL),
        ] {
            assert_eq!(num(&radius[key]), ours, "radius {key}");
        }
        let sizes = &json["size"];
        for (key, ours) in [
            ("controlSm", scale::size::CONTROL_SM),
            ("controlMd", scale::size::CONTROL_MD),
            ("controlLg", scale::size::CONTROL_LG),
            ("iconSm", scale::size::ICON_SM),
            ("iconMd", scale::size::ICON_MD),
            ("iconLg", scale::size::ICON_LG),
            ("titleBar", scale::size::TITLE_BAR),
            ("sidebar", scale::size::SIDEBAR),
        ] {
            assert_eq!(num(&sizes[key]), ours, "size {key}");
        }
        let types = &json["type"];
        for (key, ours) in [
            ("caption", scale::type_scale::CAPTION),
            ("small", scale::type_scale::SMALL),
            ("body", scale::type_scale::BODY),
            ("bodyStrong", scale::type_scale::BODY_STRONG),
            ("subtitle", scale::type_scale::SUBTITLE),
            ("title", scale::type_scale::TITLE),
            ("headline", scale::type_scale::HEADLINE),
            ("code", scale::type_scale::CODE),
            ("label", scale::type_scale::LABEL),
        ] {
            let theirs = &types[key];
            assert_eq!(num(&theirs["size"]), ours.size, "type {key} size");
            assert_eq!(num(&theirs["lineHeight"]), ours.line_height, "type {key}");
            assert_eq!(theirs["weight"].as_u64(), Some(u64::from(ours.weight)));
        }
        assert_eq!(num(&json["borderWidth"]["default"]), scale::BORDER_DEFAULT);
        assert_eq!(num(&json["borderWidth"]["focus"]), scale::BORDER_FOCUS);
        assert_eq!(num(&json["borderWidth"]["focus"]), FOCUS_RING_WIDTH);
        let fallback = &json["font"]["fallback"];
        assert_eq!(fallback["display"], scale::fallback::DISPLAY);
        assert_eq!(fallback["ui"], scale::fallback::UI);
        assert_eq!(fallback["mono"], scale::fallback::MONO);
    }
}
