//! Teskad design tokens v1 — généré depuis tokens.json, ne pas éditer à la main.
//! Indépendant du framework : convertir `Rgba` vers le type couleur de l'UI (egui, iced, gpui, slint…).
#![allow(dead_code)]

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba { pub r: u8, pub g: u8, pub b: u8, pub a: f32 }
impl Rgba {
    pub const fn hex(v: u32) -> Self { Self { r: ((v >> 16) & 0xFF) as u8, g: ((v >> 8) & 0xFF) as u8, b: (v & 0xFF) as u8, a: 1.0 } }
    pub const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Self { Self { r, g, b, a } }
}

pub mod brand {
    use super::Rgba;
    pub const VIOLET: Rgba = Rgba::hex(0x8A4DFF);
    pub const MAGENTA: Rgba = Rgba::hex(0xE040B0);
    pub const PINK: Rgba = Rgba::hex(0xFF6FB0);
    pub const LAVENDER: Rgba = Rgba::hex(0xC9B6FF);
    pub const PLUM: Rgba = Rgba::hex(0x3A1F4D);
    pub const GRADIENT_FROM: Rgba = Rgba::hex(0x8A4DFF);
    pub const GRADIENT_TO: Rgba = Rgba::hex(0xE040B0);
}

#[derive(Clone, Copy, Debug)]
pub struct Status { pub running: Rgba, pub completed: Rgba, pub failed: Rgba, pub active: Rgba, pub inactive: Rgba }

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub bg: Rgba,
    pub surface: Rgba,
    pub surface_raised: Rgba,
    pub surface_sunken: Rgba,
    pub border: Rgba,
    pub border_strong: Rgba,
    pub text: Rgba,
    pub text_secondary: Rgba,
    pub text_muted: Rgba,
    pub accent: Rgba,
    pub accent_hover: Rgba,
    pub accent_pressed: Rgba,
    pub on_accent: Rgba,
    pub accent_text: Rgba,
    pub focus_ring: Rgba,
    pub selection_bg: Rgba,
    pub selection_border: Rgba,
    pub hover_overlay: Rgba,
    pub scrim: Rgba,
    pub status: Status,
}

pub const DARK: Palette = Palette {
    bg: Rgba::hex(0x0E0C12),
    surface: Rgba::hex(0x17121E),
    surface_raised: Rgba::hex(0x1F1828),
    surface_sunken: Rgba::hex(0x0B0A0E),
    border: Rgba::hex(0x2A2135),
    border_strong: Rgba::hex(0x3A2F47),
    text: Rgba::hex(0xF2EEF7),
    text_secondary: Rgba::hex(0xC8BFD4),
    text_muted: Rgba::hex(0xA79DB5),
    accent: Rgba::hex(0x8A4DFF),
    accent_hover: Rgba::hex(0x9B66FF),
    accent_pressed: Rgba::hex(0x7A3DF0),
    on_accent: Rgba::hex(0xFFFFFF),
    accent_text: Rgba::hex(0xA97BFF),
    focus_ring: Rgba::hex(0xC9B6FF),
    selection_bg: Rgba::hex(0x241A33),
    selection_border: Rgba::hex(0x8A4DFF),
    hover_overlay: Rgba::rgba(201, 182, 255, 0.06),
    scrim: Rgba::rgba(8, 6, 12, 0.72),
    status: Status { running: Rgba::hex(0xA97BFF), completed: Rgba::hex(0x4FD8A4), failed: Rgba::hex(0xFF6A55), active: Rgba::hex(0xC9B6FF), inactive: Rgba::hex(0x7D7290) },
};

pub const LIGHT: Palette = Palette {
    bg: Rgba::hex(0xFAF8FC),
    surface: Rgba::hex(0xFFFFFF),
    surface_raised: Rgba::hex(0xF3EFF8),
    surface_sunken: Rgba::hex(0xF1EDF6),
    border: Rgba::hex(0xE6E0EE),
    border_strong: Rgba::hex(0xD2C9DE),
    text: Rgba::hex(0x1A1422),
    text_secondary: Rgba::hex(0x4A3F58),
    text_muted: Rgba::hex(0x6E6380),
    accent: Rgba::hex(0x8A4DFF),
    accent_hover: Rgba::hex(0x7A3DF0),
    accent_pressed: Rgba::hex(0x6D3BE0),
    on_accent: Rgba::hex(0xFFFFFF),
    accent_text: Rgba::hex(0x5B2FC9),
    focus_ring: Rgba::hex(0x8A4DFF),
    selection_bg: Rgba::hex(0xEFE7FF),
    selection_border: Rgba::hex(0x8A4DFF),
    hover_overlay: Rgba::rgba(58, 31, 77, 0.05),
    scrim: Rgba::rgba(26, 20, 34, 0.40),
    status: Status { running: Rgba::hex(0x6D3BE0), completed: Rgba::hex(0x0F7A55), failed: Rgba::hex(0xC9382A), active: Rgba::hex(0x7A4FE0), inactive: Rgba::hex(0x8E84A0) },
};

pub mod font {
    pub const DISPLAY: &str = "Space Grotesk";
    pub const UI: &str = "Inter";
    pub const MONO: &str = "JetBrains Mono";
}

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

pub mod radius {
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 6.0;
    pub const MD: f32 = 8.0;
    pub const LG: f32 = 12.0;
    pub const XL: f32 = 16.0;
    pub const PILL: f32 = 999.0;
}

pub mod size {
    pub const CONTROL_SM: f32 = 28.0;
    pub const CONTROL_MD: f32 = 32.0;
    pub const CONTROL_LG: f32 = 40.0;
    pub const ICON_SM: f32 = 14.0;
    pub const ICON_MD: f32 = 16.0;
    pub const ICON_LG: f32 = 20.0;
    pub const TITLE_BAR: f32 = 44.0;
    pub const SIDEBAR: f32 = 248.0;
}

pub mod type_scale {
    /// (taille px, interligne px, graisse)
    pub const CAPTION: (f32, f32, u16) = (12.0, 16.0, 500);
    pub const SMALL: (f32, f32, u16) = (13.0, 18.0, 400);
    pub const BODY: (f32, f32, u16) = (14.0, 20.0, 400);
    pub const BODY_STRONG: (f32, f32, u16) = (14.0, 20.0, 600);
    pub const SUBTITLE: (f32, f32, u16) = (16.0, 22.0, 600);
    pub const TITLE: (f32, f32, u16) = (20.0, 26.0, 600);
    pub const HEADLINE: (f32, f32, u16) = (28.0, 34.0, 600);
    pub const CODE: (f32, f32, u16) = (13.0, 20.0, 400);
    pub const LABEL: (f32, f32, u16) = (11.0, 14.0, 500);
}

pub mod motion {
    pub const FAST_MS: u32 = 120;
    pub const BASE_MS: u32 = 180;
    pub const SLOW_MS: u32 = 280;
    pub const RUNNING_PULSE_MS: u32 = 1400;
}
