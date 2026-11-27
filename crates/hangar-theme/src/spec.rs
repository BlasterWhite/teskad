//! A theme file: the format of `themes/theme.schema.json` (the brand pack's),
//! read strictly. An unknown key, a missing `terminal` colour or a colour
//! that is not `#RRGGBB`, `#RRGGBBAA` or `rgba(r, g, b, a)` is an error, so a
//! theme the schema would refuse never gets in.

use iced::Color;
use serde::Deserialize;
use serde::de::{Deserializer, Error};

use crate::theme::{Appearance, TerminalColors};

/// A colour as a theme file writes it.
#[derive(Debug, Clone, Copy)]
struct Hex(Color);

/// Reads `#RRGGBB`, `#RRGGBBAA` or `rgba(r, g, b, a)`.
pub(crate) fn parse_color(text: &str) -> Result<Color, String> {
    let bad = || format!("{text:?} is not #RRGGBB, #RRGGBBAA or rgba(r, g, b, a)");
    if let Some(hex) = text.strip_prefix('#') {
        if !(hex.len() == 6 || hex.len() == 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(bad());
        }
        let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).map_err(|_| bad());
        let alpha = if hex.len() == 8 {
            f32::from(channel(6)?) / 255.0
        } else {
            1.0
        };
        return Ok(Color::from_rgba8(
            channel(0)?,
            channel(2)?,
            channel(4)?,
            alpha,
        ));
    }
    let inner = text
        .strip_prefix("rgba(")
        .and_then(|rest| rest.strip_suffix(')'))
        .ok_or_else(bad)?;
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    let [r, g, b, a] = parts[..] else {
        return Err(bad());
    };
    let channel = |part: &str| part.parse::<u8>().map_err(|_| bad());
    let alpha: f32 = a.parse().map_err(|_| bad())?;
    if !(0.0..=1.0).contains(&alpha) {
        return Err(bad());
    }
    Ok(Color::from_rgba8(
        channel(r)?,
        channel(g)?,
        channel(b)?,
        alpha,
    ))
}

impl<'de> Deserialize<'de> for Hex {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Hex, D::Error> {
        let text = String::deserialize(deserializer)?;
        parse_color(&text).map(Hex).map_err(D::Error::custom)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawAnsi {
    black: Hex,
    red: Hex,
    green: Hex,
    yellow: Hex,
    blue: Hex,
    magenta: Hex,
    cyan: Hex,
    white: Hex,
    bright_black: Hex,
    bright_red: Hex,
    bright_green: Hex,
    bright_yellow: Hex,
    bright_blue: Hex,
    bright_magenta: Hex,
    bright_cyan: Hex,
    bright_white: Hex,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawTerminal {
    background: Hex,
    foreground: Hex,
    cursor: Hex,
    cursor_text: Option<Hex>,
    selection: Hex,
    ansi: RawAnsi,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawStatus {
    running: Option<Hex>,
    completed: Option<Hex>,
    failed: Option<Hex>,
    active: Option<Hex>,
    inactive: Option<Hex>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawUi {
    bg: Option<Hex>,
    surface: Option<Hex>,
    surface_raised: Option<Hex>,
    surface_sunken: Option<Hex>,
    border: Option<Hex>,
    border_strong: Option<Hex>,
    text: Option<Hex>,
    text_secondary: Option<Hex>,
    text_muted: Option<Hex>,
    accent: Option<Hex>,
    accent_hover: Option<Hex>,
    accent_pressed: Option<Hex>,
    on_accent: Option<Hex>,
    accent_text: Option<Hex>,
    focus_ring: Option<Hex>,
    selection_bg: Option<Hex>,
    selection_border: Option<Hex>,
    hover_overlay: Option<Hex>,
    scrim: Option<Hex>,
    #[serde(default)]
    status: RawStatus,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RawTheme {
    #[serde(rename = "$schema")]
    _schema: Option<String>,
    id: String,
    name: String,
    appearance: RawAppearance,
    terminal: RawTerminal,
    #[serde(default)]
    ui: RawUi,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum RawAppearance {
    Dark,
    Light,
}

/// The `ui` keys a theme sets itself; every key left `None` is derived from
/// the terminal colours (see [`crate::derive`]).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct UiOverrides {
    pub bg: Option<Color>,
    pub surface: Option<Color>,
    pub surface_raised: Option<Color>,
    pub surface_sunken: Option<Color>,
    pub border: Option<Color>,
    pub border_strong: Option<Color>,
    pub text: Option<Color>,
    pub text_secondary: Option<Color>,
    pub text_muted: Option<Color>,
    pub accent: Option<Color>,
    pub accent_hover: Option<Color>,
    pub accent_pressed: Option<Color>,
    pub on_accent: Option<Color>,
    pub accent_text: Option<Color>,
    pub focus_ring: Option<Color>,
    pub selection_bg: Option<Color>,
    pub selection_border: Option<Color>,
    pub hover_overlay: Option<Color>,
    pub scrim: Option<Color>,
    pub running: Option<Color>,
    pub completed: Option<Color>,
    pub failed: Option<Color>,
    pub active: Option<Color>,
    pub inactive: Option<Color>,
}

/// A theme file, parsed.
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeSpec {
    pub id: String,
    pub name: String,
    pub appearance: Appearance,
    pub terminal: TerminalColors,
    pub ui: UiOverrides,
}

impl ThemeSpec {
    /// Reads a theme file.
    ///
    /// # Errors
    ///
    /// What the schema would refuse: bad JSON, an unknown or missing key, a
    /// colour in another notation, an id outside `[a-z0-9-]+`.
    pub fn parse(json: &str) -> Result<ThemeSpec, String> {
        let raw: RawTheme = serde_json::from_str(json).map_err(|error| error.to_string())?;
        if raw.id.is_empty()
            || !raw
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(format!("{:?} is not a theme id ([a-z0-9-]+)", raw.id));
        }
        let t = raw.terminal;
        let a = t.ansi;
        let ansi = [
            a.black,
            a.red,
            a.green,
            a.yellow,
            a.blue,
            a.magenta,
            a.cyan,
            a.white,
            a.bright_black,
            a.bright_red,
            a.bright_green,
            a.bright_yellow,
            a.bright_blue,
            a.bright_magenta,
            a.bright_cyan,
            a.bright_white,
        ]
        .map(|hex| hex.0);
        let u = raw.ui;
        let get = |hex: Option<Hex>| hex.map(|hex| hex.0);
        Ok(ThemeSpec {
            id: raw.id,
            name: raw.name,
            appearance: match raw.appearance {
                RawAppearance::Dark => Appearance::Dark,
                RawAppearance::Light => Appearance::Light,
            },
            terminal: TerminalColors {
                foreground: t.foreground.0,
                background: t.background.0,
                cursor: t.cursor.0,
                cursor_text: t.cursor_text.map_or(t.background.0, |hex| hex.0),
                selection: t.selection.0,
                ansi,
            },
            ui: UiOverrides {
                bg: get(u.bg),
                surface: get(u.surface),
                surface_raised: get(u.surface_raised),
                surface_sunken: get(u.surface_sunken),
                border: get(u.border),
                border_strong: get(u.border_strong),
                text: get(u.text),
                text_secondary: get(u.text_secondary),
                text_muted: get(u.text_muted),
                accent: get(u.accent),
                accent_hover: get(u.accent_hover),
                accent_pressed: get(u.accent_pressed),
                on_accent: get(u.on_accent),
                accent_text: get(u.accent_text),
                focus_ring: get(u.focus_ring),
                selection_bg: get(u.selection_bg),
                selection_border: get(u.selection_border),
                hover_overlay: get(u.hover_overlay),
                scrim: get(u.scrim),
                running: get(u.status.running),
                completed: get(u.status.completed),
                failed: get(u.status.failed),
                active: get(u.status.active),
                inactive: get(u.status.inactive),
            },
        })
    }
}
