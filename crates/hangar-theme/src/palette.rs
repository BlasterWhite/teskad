//! The semantic colours of the whole interface: `tokens/theme.rs › Palette`
//! of the Teskad brand pack (`docs/teskad-brand`), with iced colours.
//!
//! This is the one set of colours a theme defines for the interface. Every
//! other colour the app draws with ([`crate::Colors`]) is computed from it,
//! and nothing outside this crate writes a colour of its own.

use iced::Color;

/// The five states a thing can be in. Each has a colour here and a shape in
/// the interface (`docs/teskad-brand/icons/status`): never a colour alone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Status {
    /// An arc that turns.
    pub running: Color,
    /// A disc with a tick.
    pub completed: Color,
    /// A disc with a cross.
    pub failed: Color,
    /// A plain dot.
    pub active: Color,
    /// An empty circle.
    pub inactive: Color,
}

/// Every colour token of the interface, by the name the pack gives it
/// (`surfaceRaised` is `surface_raised`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    /// The window.
    pub bg: Color,
    /// Headers and cards.
    pub surface: Color,
    /// Menus and popovers.
    pub surface_raised: Color,
    /// Terminals and fields.
    pub surface_sunken: Color,
    pub border: Color,
    pub border_strong: Color,
    pub text: Color,
    pub text_secondary: Color,
    pub text_muted: Color,
    /// A button's fill.
    pub accent: Color,
    pub accent_hover: Color,
    pub accent_pressed: Color,
    /// What is written on the accent.
    pub on_accent: Color,
    /// The accent as text or an icon: a link.
    pub accent_text: Color,
    pub focus_ring: Color,
    pub selection_bg: Color,
    pub selection_border: Color,
    /// Laid over what is under the pointer. Translucent.
    pub hover_overlay: Color,
    /// Laid over the window behind a modal. Translucent.
    pub scrim: Color,
    pub status: Status,
}

impl Palette {
    /// Every token with the name the pack gives it, in the pack's order.
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, Color)> {
        [
            ("bg", self.bg),
            ("surface", self.surface),
            ("surfaceRaised", self.surface_raised),
            ("surfaceSunken", self.surface_sunken),
            ("border", self.border),
            ("borderStrong", self.border_strong),
            ("text", self.text),
            ("textSecondary", self.text_secondary),
            ("textMuted", self.text_muted),
            ("accent", self.accent),
            ("accentHover", self.accent_hover),
            ("accentPressed", self.accent_pressed),
            ("onAccent", self.on_accent),
            ("accentText", self.accent_text),
            ("focusRing", self.focus_ring),
            ("selectionBg", self.selection_bg),
            ("selectionBorder", self.selection_border),
            ("hoverOverlay", self.hover_overlay),
            ("scrim", self.scrim),
            ("status.running", self.status.running),
            ("status.completed", self.status.completed),
            ("status.failed", self.status.failed),
            ("status.active", self.status.active),
            ("status.inactive", self.status.inactive),
        ]
        .into_iter()
    }
}
