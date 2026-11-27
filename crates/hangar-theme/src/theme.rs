//! What a theme is: an identity, a terminal palette and every colour token of
//! the window.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, PoisonError};

use iced::Color;

use crate::bundled;
use crate::derive::{derive_inks, derive_palette};
use crate::inks::Inks;
use crate::palette::Palette;
use crate::spec::UiOverrides;

/// The theme the app starts with, and falls back on for an id it does not know.
pub const DEFAULT_ID: &str = "teskad-dark";

/// Whether a theme sits on a dark or a light background. It decides which way
/// the chrome's surfaces step away from the terminal background, and the
/// scheme kits are told.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Appearance {
    Dark,
    Light,
}

impl Appearance {
    pub fn is_dark(self) -> bool {
        self == Appearance::Dark
    }

    /// `"dark"` or `"light"`, the `scheme` of the kits' theme event.
    pub fn as_str(self) -> &'static str {
        match self {
            Appearance::Dark => "dark",
            Appearance::Light => "light",
        }
    }
}

/// One theme: how the window and its terminals are coloured.
///
/// A theme is a file (`themes/*.json`, the format of the brand pack's
/// `theme.schema.json`) that holds the terminal's colours and, for the
/// interface, only what it sets itself: the rest of the [`Palette`] is derived
/// from the terminal colours (`docs/teskad-brand/docs/themes.md`) and checked
/// for contrast when the theme is built. Drawing never computes a colour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    /// Stable and unique, what settings and workspaces store (`"tokyo-night"`).
    pub id: &'static str,
    /// What the theme picker shows (`"Tokyo Night"`).
    pub name: &'static str,
    pub appearance: Appearance,
    /// The colours terminals are drawn with.
    pub terminal: TerminalColors,
    /// The semantic colours of the interface: the one set a theme defines.
    pub palette: Palette,
    /// The colours the palette has no token for: a state's inks, warning,
    /// the shadow, the charts'. Computed from the palette.
    pub inks: Inks,
    /// How opaque the window's own background is, from 0 to 1: 1 for a solid
    /// window, less for one the desktop shows through, behind the terminals'
    /// default background and the chrome's bars. Text, cards, menus, dialogs
    /// and popovers stay opaque whatever it is. Only Linux, where the window
    /// is transparent already, honours it: see [`Theme::window_alpha`].
    ///
    /// 1 for every bundled theme: the person's setting (Settings ›
    /// Appearance › Window opacity) makes it less, for whatever theme is
    /// shown, through [`Theme::with_window_opacity`].
    pub window_opacity: f32,
}

/// The least opaque a window is drawn, whatever is asked: below it, text on
/// the desktop's own colours stops reading.
pub const MIN_WINDOW_OPACITY: f32 = 0.5;

impl Theme {
    /// The accent, which marks links, focus and the selection.
    pub fn accent(&self) -> Color {
        self.palette.accent
    }

    /// A theme the app does not bundle, from its terminal colours and the
    /// keys of `ui` it sets (the others derived, as for a bundled one), with
    /// the same contrast promises.
    pub fn derived(
        id: &'static str,
        name: &'static str,
        appearance: Appearance,
        terminal: TerminalColors,
        ui: &UiOverrides,
    ) -> Theme {
        let palette = derive_palette(appearance, &terminal, ui);
        let inks = derive_inks(appearance, &palette, &terminal);
        Theme {
            id,
            name,
            appearance,
            terminal,
            palette,
            inks,
            window_opacity: 1.0,
        }
    }

    pub fn is_dark(&self) -> bool {
        self.appearance.is_dark()
    }

    /// What a column of the board is filled with (`ecrans.md`, the token
    /// `deep`: a step off the window, under the cards laid on it). It is not a
    /// colour a theme writes: halfway from `bg` to `surface` on a dark theme
    /// (Teskad Dark's `#120F18`, the mockups' `#120F17`), and
    /// `surfaceRaised` on a light one, whose surface is lighter than its
    /// window (Teskad Light's `#F3EFF8`, as its mockup).
    pub fn deep(&self) -> Color {
        let p = &self.palette;
        if !self.is_dark() {
            return p.surface_raised;
        }
        let half = |a: f32, b: f32| (a + b) / 2.0;
        Color {
            r: half(p.bg.r, p.surface.r),
            g: half(p.bg.g, p.surface.g),
            b: half(p.bg.b, p.surface.b),
            a: p.bg.a,
        }
    }

    /// How opaque the window's background is drawn here: the theme's
    /// [`window_opacity`](Theme::window_opacity) on Linux, where the window is
    /// created transparent (for the corners of the title bar Hangar draws),
    /// and 1 everywhere else, where it is not and a translucent fill would
    /// only darken.
    ///
    /// Whether the desktop shows blurred through it is the compositor's
    /// business: KDE blurs what asks for it, GNOME only with an extension
    /// such as Blur my Shell.
    pub fn window_alpha(&self) -> f32 {
        if cfg!(target_os = "linux") {
            window_alpha_of(self.window_opacity)
        } else {
            1.0
        }
    }

    /// This theme with its window drawn at `opacity` (the person's setting,
    /// held within [`MIN_WINDOW_OPACITY`] and 1; a value that is not a number
    /// is 1). Nothing but the alpha of the window's backgrounds changes: the
    /// accent, the text, the borders and the terminal's palette stay the
    /// theme's.
    ///
    /// Themes are handed out as `&'static`, so a theme at an opacity of its
    /// own is made once and kept: in whole percents, at most 51 of each.
    pub fn with_window_opacity(&'static self, opacity: f64) -> &'static Theme {
        let opacity = if opacity.is_finite() { opacity } else { 1.0 };
        let percent = (opacity.clamp(f64::from(MIN_WINDOW_OPACITY), 1.0) * 100.0).round() as u8;
        let window_opacity = f32::from(percent) / 100.0;
        if window_opacity == self.window_opacity {
            return self;
        }
        static COPIES: OnceLock<Mutex<HashMap<(usize, u8), &'static Theme>>> = OnceLock::new();
        let key = (std::ptr::from_ref(self) as usize, percent);
        let mut copies = COPIES
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let &mut copy = copies.entry(key).or_insert_with(|| {
            Box::leak(Box::new(Theme {
                window_opacity,
                ..*self
            }))
        });
        copy
    }

    /// Whether the desktop shows through the window here.
    pub fn is_translucent(&self) -> bool {
        self.window_alpha() < 1.0
    }

    /// `color` as a surface laid straight on the desktop: at the window's
    /// alpha. Only for what nothing else of the window is drawn under (the
    /// window's page, the chrome's bars): two such surfaces on top of each
    /// other add up to an opaque one.
    pub fn see_through(&self, color: Color) -> Color {
        color.scale_alpha(self.window_alpha())
    }

    /// Matches and the current match of a search in a terminal: the theme's
    /// own yellow, the current one in its accent, both laid over the
    /// background so the text on top stays legible in light themes too.
    pub fn search_colors(&self) -> SearchColors {
        let bg = crate::Rgb::from(self.terminal.background);
        let match_color = crate::Rgb::from(self.terminal.ansi[3]);
        let accent = crate::Rgb::from(self.palette.accent);
        SearchColors {
            match_background: crate::mix(bg, match_color, 0.3).into(),
            match_border: crate::mix(bg, match_color, 0.65).into(),
            match_marker: match_color.into(),
            active_background: crate::mix(bg, accent, 0.45).into(),
            active_border: accent.into(),
        }
    }
}

/// A theme's opacity as it is drawn: within [`MIN_WINDOW_OPACITY`] and 1, and 1
/// for a value that is not a number.
pub(crate) fn window_alpha_of(opacity: f32) -> f32 {
    if opacity.is_finite() {
        opacity.clamp(MIN_WINDOW_OPACITY, 1.0)
    } else {
        1.0
    }
}

/// How a terminal search marks what it found.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SearchColors {
    pub match_background: Color,
    pub match_border: Color,
    /// On the scrollbar, where the matches show.
    pub match_marker: Color,
    pub active_background: Color,
    pub active_border: Color,
}

/// What a terminal is drawn with, in the terms of a terminal: a plain value,
/// so the crate that renders terminals does not have to depend on this one
/// and the app maps it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerminalColors {
    pub foreground: Color,
    pub background: Color,
    pub cursor: Color,
    /// The colour of the character under a block cursor.
    pub cursor_text: Color,
    /// The background of selected text. Opaque: the text keeps its colour.
    pub selection: Color,
    /// The 16 ANSI colours in the order of the escape codes: black, red,
    /// green, yellow, blue, magenta, cyan, white, then the same eight bright.
    pub ansi: [Color; 16],
}

impl TerminalColors {
    /// Names of the 16 colours in the order of [`TerminalColors::ansi`].
    pub const ANSI_NAMES: [&'static str; 16] = [
        "black",
        "red",
        "green",
        "yellow",
        "blue",
        "magenta",
        "cyan",
        "white",
        "bright black",
        "bright red",
        "bright green",
        "bright yellow",
        "bright blue",
        "bright magenta",
        "bright cyan",
        "bright white",
    ];
}

/// The four states of an agent. The chrome always shows them in a colour and
/// a shape, never in a colour alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentState {
    /// Busy on its own.
    Working,
    /// Blocked until the person answers.
    Waiting,
    /// Done, and the turn is the person's.
    Yours,
    /// Nothing going on.
    Idle,
}

impl AgentState {
    pub const ALL: [AgentState; 4] = [
        AgentState::Working,
        AgentState::Waiting,
        AgentState::Yours,
        AgentState::Idle,
    ];
}

/// The two colours of one agent state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StateColors {
    pub text: Color,
    pub fill: Color,
}

/// Every bundled theme, Graphite first, the dark ones then the light ones, in
/// the order the picker shows them.
pub fn themes() -> &'static [Theme] {
    bundled::themes()
}

/// The theme with this id.
pub fn by_id(id: &str) -> Option<&'static Theme> {
    bundled::themes().iter().find(|theme| theme.id == id)
}

/// The theme with this id, or the default one for an id that is not (or no
/// longer) bundled, as `getTheme` does.
pub fn by_id_or_default(id: &str) -> &'static Theme {
    by_id(id).unwrap_or_else(default_theme)
}

/// Teskad Dark, the default.
pub fn default_theme() -> &'static Theme {
    by_id(DEFAULT_ID).expect("the default theme is bundled")
}
