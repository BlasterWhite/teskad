//! The colours the window is drawn with that the brand pack has no token for.
//!
//! [`crate::Palette`] is the pack's tokens and nothing else. The interface
//! also needs colours the pack does not name: an agent state's text and fill
//! held to a contrast, "waiting" (the palette has five states, not six),
//! warning, the soft red behind a destructive row, the drop shadow and the
//! statistics' data colours. They live here, derived from the palette by
//! [`crate::derive_inks`], one field each, named and documented.
//!
//! # Where each field of the former `Colors` went
//!
//! [`crate::Colors`] stays for now, computed from the palette and from these
//! inks, so that nothing which uses it changes. This is the table the ticket
//! that removes it follows (`colors.old` → `new`). `palette.*` is
//! [`crate::Palette`], `inks.*` is [`Inks`], and `theme.*` a method of
//! [`crate::Theme`].
//!
//! ```text
//! state_working        → inks.running.text      state_working_fill → inks.running.fill
//! state_waiting        → inks.waiting.text      state_waiting_fill → inks.waiting.fill
//! state_yours          → inks.yours.text        state_yours_fill   → inks.yours.fill
//! state_idle           → inks.idle.text         state_idle_fill    → inks.idle.fill
//! (none)               → inks.failed.{text,fill}, inks.completed.{text,fill}
//! colors.state(state)  → inks.state(state)
//! danger               → inks.danger()  (= inks.failed.text)
//! success              → inks.success() (= inks.completed.text)
//! warning              → inks.warning() (= inks.waiting.text)
//! danger_soft          → inks.danger_soft
//! shadow               → inks.shadow
//! viz_1 .. viz_4       → inks.viz.series[0..4]
//! viz_heat_0 .. _5     → inks.viz.heat[0..6]
//! viz_mute             → inks.viz.mute
//! viz_line             → inks.viz.line
//!
//! bg                   → palette.bg             bg_sunken       → palette.surface_sunken
//! chrome, pane_head, surface → palette.surface
//! chrome_bar, pane_head_focus, raised → palette.surface_raised
//! sep, seg_track       → palette.surface_sunken
//! line, border         → palette.border         line_strong, border_strong → palette.border_strong
//! surface_active       → palette.selection_bg
//! accent               → palette.accent         accent_fg       → palette.on_accent
//! accent_hi            → palette.accent_text
//! fg, fg_secondary     → palette.text, palette.text_secondary
//! fg_muted, fg_subtle  → palette.text_muted
//! accent_soft          → palette.accent at alpha 0.14
//! accent_ring          → palette.focus_ring at alpha 0.45
//! surface_hover        → palette.hover_overlay over palette.surface
//! card                 → palette.surface_raised on a dark theme, palette.bg on a light one
//! seg_thumb            → palette.surface_raised on a dark theme, palette.surface on a light one
//! float_ring           → black at 0.55 (dark) or 0.05 (light); stays a view detail
//! ```
//!
//! The computed ones have their own place:
//!
//! ```text
//! fg → inks.text.primary   fg_secondary → inks.text.secondary
//! fg_muted, fg_subtle → inks.text.muted   accent_hi → inks.accent_text
//! card → inks.card   surface_hover → inks.hover   seg_thumb → inks.seg_thumb
//! float_ring → inks.float_ring   accent_soft → inks.accent_soft
//! accent_ring → inks.accent_ring
//! ```
//!
//! `fg`, `fg_secondary`, `fg_muted`, `fg_subtle` and `accent_hi` are the
//! palette's text and accent text held to 4.5:1 on every surface (the palette's
//! own check covers only the window and the surface), so they can differ from
//! their palette token. The same goes for `card`, `surface_hover`, `seg_thumb`
//! and `float_ring`, which are computed. `Colors` keeps them until the
//! ticket that removes it picks, for each use, the palette token or the held
//! colour.

use iced::Color;

use crate::theme::{AgentState, StateColors};

/// The colours of the window the palette has no token for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Inks {
    /// The text, held to 4.5:1 on every surface (see [`TextInks`]).
    pub text: TextInks,
    /// The accent's text (a link, a chosen label) held to 4.5:1 on every
    /// surface; `palette.accent_text` itself only on the window and surface.
    pub accent_text: Color,
    /// A card laid on the window: `palette.surface_raised` on a dark theme,
    /// `palette.bg` on a light one.
    pub card: Color,
    /// A row under the pointer: `palette.hover_overlay` over `palette.surface`.
    pub hover: Color,
    /// The chosen segment of a segmented control: `palette.surface_raised` on
    /// a dark theme, `palette.surface` on a light one.
    pub seg_thumb: Color,
    /// The outer edge of what floats over the window: black at 0.55 (dark) or
    /// 0.05 (light). Translucent.
    pub float_ring: Color,
    /// The accent at alpha 0.14, behind a selected or hovered accent item.
    pub accent_soft: Color,
    /// The focus ring at alpha 0.45, drawn round a focused field.
    pub accent_ring: Color,
    /// An agent at work (`status.running`): a word at 4.5:1, a fill at 3:1.
    pub running: StateColors,
    /// An agent waiting for an answer. The palette has no colour for it: it is
    /// the terminal's yellow (`ansi[3]`), always apart from [`Inks::yours`].
    pub waiting: StateColors,
    /// "Your turn" (`status.active`): a hue of its own, so it never reads as a
    /// link or as the focus, and never as [`Inks::waiting`].
    pub yours: StateColors,
    /// At rest (`status.inactive`); its word is the muted text.
    pub idle: StateColors,
    /// A failure (`status.failed`).
    pub failed: StateColors,
    /// A success (`status.completed`).
    pub completed: StateColors,
    /// Behind something destructive under the pointer, or a badge that warns:
    /// the failure colour at 14 %. Translucent.
    pub danger_soft: Color,
    /// The colour of a drop shadow. Translucent.
    pub shadow: Color,
    /// The statistics' charts.
    pub viz: Viz,
}

impl Inks {
    /// An agent state's two inks.
    pub fn state(&self, state: AgentState) -> StateColors {
        match state {
            AgentState::Working => self.running,
            AgentState::Waiting => self.waiting,
            AgentState::Yours => self.yours,
            AgentState::Idle => self.idle,
        }
    }

    /// Text that says something is wrong or destructive, 4.5:1.
    pub fn danger(&self) -> Color {
        self.failed.text
    }

    /// Text that says something went well, 4.5:1.
    pub fn success(&self) -> Color {
        self.completed.text
    }

    /// Text that asks for attention without being an error, 4.5:1: the
    /// waiting ink.
    pub fn warning(&self) -> Color {
        self.waiting.text
    }
}

/// The text, each held to 4.5:1 on every surface where the palette's own check
/// covers only the window and the surface, so each can differ from its token.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextInks {
    /// `palette.text`, held.
    pub primary: Color,
    /// `palette.text_secondary`, held.
    pub secondary: Color,
    /// `palette.text_muted`, held to 4.5:1 on the plain surfaces.
    pub muted: Color,
}

/// The statistics' data colours.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viz {
    /// Four ordered data colours of the accent's hue, the cheapest nearest
    /// the card.
    pub series: [Color; 4],
    /// The activity heatmap: an empty cell, then five steps up.
    pub heat: [Color; 6],
    /// What steps back while something else is picked out.
    pub mute: Color,
    /// The line of a sparkline.
    pub line: Color,
}
