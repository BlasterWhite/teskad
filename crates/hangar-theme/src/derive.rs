//! The window's colours, from a theme file.
//!
//! Three steps, all here:
//!
//! 1. [`derive_palette`]: the interface's semantic colours ([`Palette`]) from
//!    the theme's terminal colours, by the rules of
//!    `docs/teskad-brand/docs/themes.md`. A key the theme's `ui` sets is used
//!    as it is (Teskad Dark and Teskad Light set them all, so nothing is
//!    computed for them); every other key is derived, then held to the
//!    contrast the same document requires.
//! 2. [`derive_inks`]: [`Inks`], the colours the palette has no token for (a
//!    state's ink, warning, the shadow, a chart's data colours), computed
//!    from the palette.
//! 3. [`derive_colors`]: [`Colors`], the finer tokens views are drawn with
//!    (a pane header, a hairline), computed from the palette and the inks. It
//!    is an alias of both, not a second source of colour.

use iced::Color;

use crate::contrast::{
    Rgb, contrast_ratio, ensure_contrast, mix, perceived_lightness, worst_contrast,
};
use crate::inks::{Inks, TextInks, Viz};
use crate::palette::{Palette, Status};
use crate::spec::UiOverrides;
use crate::theme::{Appearance, Colors, StateColors, TerminalColors};

/// Text on the window reads at this ratio at least: WCAG's AA for body text.
pub const TEXT_CONTRAST: f64 = 4.5;
/// A state's icon, a bar, a frame: WCAG's 3:1 for what is not text.
pub const FILL_CONTRAST: f64 = 3.0;
/// A border is at least this visible on the window.
pub const BORDER_CONTRAST: f64 = 1.3;

/// A colour with an alpha, from an opaque colour.
fn with_alpha(color: Rgb, alpha: f32) -> Color {
    Color::from_rgba8(color.0, color.1, color.2, alpha)
}

/// `color` brought to `min`:1 on all of `against`: toward `toward` first, so
/// a grey keeps the theme's tint, and toward white or black when even
/// `toward` is not enough.
fn reach(color: Rgb, against: &[Rgb], min: f64, toward: Rgb) -> Rgb {
    let nearer = ensure_contrast(color, against, min, Some(toward));
    if worst_contrast(nearer, against) >= min {
        nearer
    } else {
        ensure_contrast(nearer, against, min, None)
    }
}

/// The interface's colours for this terminal palette: each key of `ui` as the
/// theme set it, the others derived from `terminal` (`themes.md`, "Dérivation")
/// and then held to the contrast the document requires ("Contrôle de
/// contraste").
pub fn derive_palette(
    appearance: Appearance,
    terminal: &TerminalColors,
    ui: &UiOverrides,
) -> Palette {
    let is_dark = appearance.is_dark();
    let b = Rgb::from(terminal.background);
    let f = Rgb::from(terminal.foreground);
    let [green, red, magenta] = [2usize, 1, 5].map(|slot| Rgb::from(terminal.ansi[slot]));
    let rgb = |color: Option<Color>| color.map(Rgb::from);
    let c = |color: Rgb| Color::from(color);
    let pole = if is_dark { Rgb::WHITE } else { Rgb::BLACK };

    let bg = rgb(ui.bg).unwrap_or(b);
    let surface = rgb(ui.surface).unwrap_or_else(|| mix(b, f, 0.04));
    let readable_on = [bg, surface];

    let surface_sunken = rgb(ui.surface_sunken).unwrap_or_else(|| {
        if is_dark {
            mix(b, Rgb::BLACK, 0.20)
        } else {
            mix(b, f, 0.02)
        }
    });
    let surface_raised = rgb(ui.surface_raised).unwrap_or_else(|| mix(b, f, 0.07));
    let border =
        rgb(ui.border).unwrap_or_else(|| reach(mix(b, f, 0.12), &[bg], BORDER_CONTRAST, f));
    let border_strong =
        rgb(ui.border_strong).unwrap_or_else(|| reach(mix(b, f, 0.18), &[bg], BORDER_CONTRAST, f));

    // Text: the foreground, unless it is too faint for the window (Solarized):
    // then it leans out to white or black until it reads. The two quieter
    // levels are mixes toward it, brought nearer to it until they read too.
    let text = rgb(ui.text).unwrap_or_else(|| reach(f, &readable_on, TEXT_CONTRAST, f));
    let text_secondary = rgb(ui.text_secondary)
        .unwrap_or_else(|| reach(mix(b, text, 0.75), &readable_on, TEXT_CONTRAST, text));
    let text_muted = rgb(ui.text_muted)
        .unwrap_or_else(|| reach(mix(b, text, 0.60), &readable_on, TEXT_CONTRAST, text));

    let accent = rgb(ui.accent).unwrap_or(magenta);
    // White or the terminal background, whichever contrasts more with the
    // accent; black when an accent in between leaves neither at 4.5:1.
    let on_accent = rgb(ui.on_accent).unwrap_or_else(|| {
        let best = if contrast_ratio(Rgb::WHITE, accent) >= contrast_ratio(b, accent) {
            Rgb::WHITE
        } else {
            b
        };
        if contrast_ratio(best, accent) >= TEXT_CONTRAST {
            best
        } else if contrast_ratio(Rgb::BLACK, accent) >= contrast_ratio(Rgb::WHITE, accent) {
            Rgb::BLACK
        } else {
            Rgb::WHITE
        }
    });
    // The accent a step toward the foreground (hovered) or the background
    // (pressed): a step that would leave the label under 4.5:1 is shortened.
    let accent_step = |toward: Rgb| {
        let mut amount = 0.12;
        while amount > 0.0 && contrast_ratio(on_accent, mix(accent, toward, amount)) < TEXT_CONTRAST
        {
            amount -= 0.01;
        }
        mix(accent, toward, amount.max(0.0))
    };
    let accent_hover = rgb(ui.accent_hover).unwrap_or_else(|| accent_step(f));
    let accent_pressed = rgb(ui.accent_pressed).unwrap_or_else(|| accent_step(b));
    let accent_text = rgb(ui.accent_text)
        .unwrap_or_else(|| ensure_contrast(accent, &readable_on, TEXT_CONTRAST, Some(pole)));
    let focus_ring = rgb(ui.focus_ring).unwrap_or(accent_text);
    let selection_bg = rgb(ui.selection_bg).unwrap_or_else(|| mix(b, accent, 0.14));
    let selection_border = rgb(ui.selection_border).unwrap_or(accent);

    // The states: each one the pack's own colour when the derived one does not
    // reach 3:1 on the surface it is drawn on.
    let pack = crate::bundled::pack_status(appearance);
    let status_of = |set: Option<Color>, derived: Rgb, pack: Color| -> Color {
        match set {
            Some(set) => set,
            None if contrast_ratio(derived, surface) >= FILL_CONTRAST => c(derived),
            // The pack's colour is made for the pack's surfaces: on a theme's
            // own it is brought to 3:1 too.
            None => c(reach(
                rgb(Some(pack)).unwrap_or(derived),
                &[surface],
                FILL_CONTRAST,
                f,
            )),
        }
    };
    let status = Status {
        running: status_of(ui.running, accent_text, pack.running),
        completed: status_of(ui.completed, green, pack.completed),
        failed: status_of(ui.failed, red, pack.failed),
        active: status_of(ui.active, mix(accent, f, 0.40), pack.active),
        inactive: status_of(ui.inactive, mix(b, f, 0.45), pack.inactive),
    };

    // The pack gives no rule for these two: a veil of the theme's own
    // foreground over what is under the pointer, and a shade of its
    // background (of its foreground on a light theme) over the window behind
    // a modal.
    let hover_overlay = ui
        .hover_overlay
        .unwrap_or_else(|| with_alpha(f, if is_dark { 0.06 } else { 0.05 }));
    let scrim = ui.scrim.unwrap_or_else(|| {
        if is_dark {
            with_alpha(mix(b, Rgb::BLACK, 0.5), 0.72)
        } else {
            with_alpha(mix(f, Rgb::BLACK, 0.3), 0.40)
        }
    });

    Palette {
        bg: c(bg),
        surface: c(surface),
        surface_raised: c(surface_raised),
        surface_sunken: c(surface_sunken),
        border: c(border),
        border_strong: c(border_strong),
        text: c(text),
        text_secondary: c(text_secondary),
        text_muted: c(text_muted),
        accent: c(accent),
        accent_hover: c(accent_hover),
        accent_pressed: c(accent_pressed),
        on_accent: c(on_accent),
        accent_text: c(accent_text),
        focus_ring: c(focus_ring),
        selection_bg: c(selection_bg),
        selection_border: c(selection_border),
        hover_overlay,
        scrim,
        status,
    }
}

/// What a palette fails of the contrast `themes.md` requires, one line per
/// failure; empty when it passes. Text on the window and the surface at
/// 4.5:1, the text on the accent at 4.5:1, the states on the surface at 3:1,
/// the border on the window at 1.3:1.
pub fn contrast_failures(palette: &Palette) -> Vec<String> {
    let rgb = Rgb::from;
    let (bg, surface) = (rgb(palette.bg), rgb(palette.surface));
    let mut failures = Vec::new();
    let mut check = |name: &str, color: Color, on: &str, background: Rgb, min: f64| {
        let ratio = contrast_ratio(rgb(color), background);
        // Colours are 8 bits: half a hundredth of slack for the rounding.
        if ratio < min - 0.005 {
            failures.push(format!("{name} on {on}: {ratio:.2}:1, needs {min}:1"));
        }
    };
    for (name, color) in [
        ("text", palette.text),
        ("textSecondary", palette.text_secondary),
        ("textMuted", palette.text_muted),
        ("accentText", palette.accent_text),
    ] {
        check(name, color, "bg", bg, TEXT_CONTRAST);
        check(name, color, "surface", surface, TEXT_CONTRAST);
    }
    check(
        "onAccent",
        palette.on_accent,
        "accent",
        rgb(palette.accent),
        TEXT_CONTRAST,
    );
    let s = palette.status;
    for (name, color) in [
        ("status.running", s.running),
        ("status.completed", s.completed),
        ("status.failed", s.failed),
        ("status.active", s.active),
        ("status.inactive", s.inactive),
    ] {
        check(name, color, "surface", surface, FILL_CONTRAST);
    }
    check("border", palette.border, "bg", bg, BORDER_CONTRAST);
    failures
}

/// The surfaces a colour is held against, as [`derive_inks`] holds them.
struct Surfaces {
    /// Every surface text is written on. A colour is held to the least
    /// favourable of them: the plain ones, the selected and the hovered row,
    /// a chip (filled with the border's colour) and a filter that is on (the
    /// accent's tint over the surface).
    anywhere: Vec<Rgb>,
    /// The three plain surfaces: the window, the surface, the raised one.
    plain: [Rgb; 3],
    /// A card is lifted off the window on a dark theme, and is the window's
    /// own colour on a light one, where the window is the grey.
    card: Rgb,
}

impl Surfaces {
    fn of(appearance: Appearance, palette: &Palette) -> Surfaces {
        let rgb = Rgb::from;
        let p = palette;
        let [bg, surface, raised, sunken] =
            [p.bg, p.surface, p.surface_raised, p.surface_sunken].map(rgb);
        let hover = crate::contrast::over(p.hover_overlay, surface);
        let lit = mix(surface, rgb(p.accent), 0.14);
        Surfaces {
            anywhere: vec![
                bg,
                surface,
                raised,
                sunken,
                rgb(p.selection_bg),
                hover,
                rgb(p.border),
                lit,
            ],
            plain: [bg, surface, raised],
            card: if appearance.is_dark() { raised } else { bg },
        }
    }

    /// `color` as text: 4.5:1 on every surface.
    fn text(&self, toward: Rgb, color: Rgb) -> Rgb {
        reach(color, &self.anywhere, TEXT_CONTRAST, toward)
    }

    /// `color` as a glyph, a bar or a frame: 3:1 on every surface.
    fn fill(&self, color: Rgb) -> Rgb {
        ensure_contrast(color, &self.anywhere, FILL_CONTRAST, None)
    }
}

/// The surfaces a state's ink is held to (text at 4.5:1, fill at 3:1), for
/// tests and for a view that wants to check an ink of its own.
pub fn ink_surfaces(appearance: Appearance, palette: &Palette) -> Vec<Color> {
    Surfaces::of(appearance, palette)
        .anywhere
        .into_iter()
        .map(Color::from)
        .collect()
}

/// The colours the window is drawn with that the palette has no token for
/// ([`Inks`]), computed from the palette (and, for the one state it has no
/// colour for, from the terminal's yellow).
pub fn derive_inks(appearance: Appearance, palette: &Palette, terminal: &TerminalColors) -> Inks {
    let is_dark = appearance.is_dark();
    let rgb = Rgb::from;
    let c = |color: Rgb| Color::from(color);
    let p = palette;
    let accent = rgb(p.accent);
    let fg = rgb(p.text);
    let lift = if is_dark { Rgb::WHITE } else { Rgb::BLACK };
    let surfaces = Surfaces::of(appearance, palette);
    let card = surfaces.card;
    let text = |color: Rgb| surfaces.text(fg, color);
    let fill = |color: Rgb| surfaces.fill(color);
    // Labels and the quietest greys answer to the plain surfaces only, as the
    // palette's own check does; a rule that puts them on a fill takes the
    // secondary text.
    let quiet = |color: Color| c(reach(rgb(color), &surfaces.plain, TEXT_CONTRAST, fg));

    let waiting = rgb(terminal.ansi[3]);
    let status = p.status;
    // An agent's state, always with its shape in the UI. Each comes twice:
    // the word, at 4.5:1 like any text, and the fill of its glyph, a bar or a
    // frame, at 3:1.
    let ink = |base: Rgb| StateColors {
        text: c(text(base)),
        fill: c(fill(base)),
    };
    // Idle's word is the muted text, not the state's own colour.
    let idle = StateColors {
        text: c(text(rgb(p.text_muted))),
        fill: c(fill(rgb(status.inactive))),
    };

    // The statistics' data colours, all of one hue (the accent's) in steps
    // away from the card they are drawn on: brighter on a dark theme, deeper
    // on a light one. A chart there encodes "how much", and so does
    // lightness, so the page keeps the rest of the chrome's quiet without a
    // palette of its own. The strongest step is the accent held to 3:1 on the
    // card; the faintest still reads at 2:1, the floor for a mark a label or
    // a table also carries.
    let toward = |top: Rgb, amount: f64| mix(card, top, amount);
    // Where the ramp starts: 40 % of the way to the top, or further when a
    // pale accent needs it to reach 2:1.
    let viz_start_for = |top: Rgb| {
        let mut start = 0.4;
        while start < 0.7 && contrast_ratio(toward(top, start), card) < 2.0 {
            start += 0.01;
        }
        start
    };
    let span_of = |top: Rgb| {
        (perceived_lightness(top) - perceived_lightness(toward(top, viz_start_for(top)))).abs()
    };
    // A ramp too short to tell its four steps apart at a glance (0.065 of
    // perceived lightness each, with room to spare) has its top pushed out,
    // brighter on a dark theme and deeper on a light one, until it is not.
    let mut viz_top = fill(if is_dark {
        mix(accent, Rgb::WHITE, 0.12)
    } else {
        accent
    });
    let mut push = 0;
    while push < 8 && span_of(viz_top) < 0.2 {
        viz_top = mix(viz_top, if is_dark { Rgb::WHITE } else { Rgb::BLACK }, 0.06);
        push += 1;
    }
    // `count` steps from `start` of the way to the top up to the top itself,
    // at even distances of perceived lightness rather than of the mix: a mix
    // toward a vivid colour bunches its last steps together.
    let ramp = |start: f64, count: usize| -> Vec<Rgb> {
        let from = perceived_lightness(toward(viz_top, start));
        let to = perceived_lightness(viz_top);
        (0..count)
            .map(|index| {
                let target = from + (to - from) * index as f64 / (count - 1) as f64;
                let (mut low, mut high) = (start, 1.0);
                for _ in 0..20 {
                    let middle = (low + high) / 2.0;
                    if (perceived_lightness(toward(viz_top, middle)) - target) * (to - from) < 0.0 {
                        low = middle;
                    } else {
                        high = middle;
                    }
                }
                toward(viz_top, (low + high) / 2.0)
            })
            .collect()
    };
    let series: Vec<Rgb> = ramp(viz_start_for(viz_top), 4)
        .into_iter()
        .map(|color| ensure_contrast(color, &[card], 2.0, Some(viz_top)))
        .collect();
    // The heatmap starts fainter: a quiet hour is meant to be barely there,
    // and its cells carry their values in a tooltip and a table.
    let heat = ramp(0.24, 5);

    Inks {
        text: TextInks {
            primary: c(text(fg)),
            secondary: c(text(rgb(p.text_secondary))),
            muted: quiet(p.text_muted),
        },
        accent_text: c(text(rgb(p.accent_text))),
        card: c(card),
        hover: c(crate::contrast::over(p.hover_overlay, rgb(p.surface))),
        seg_thumb: if is_dark { p.surface_raised } else { p.surface },
        // What floats over the window gets a dark outer edge under its hairline
        // on a dark theme, so a panel never melts into what is behind it; on a
        // light theme the shadow does that almost alone.
        float_ring: if is_dark {
            Color::from_rgba8(0, 0, 0, 0.55)
        } else {
            Color::from_rgba8(0, 0, 0, 0.05)
        },
        accent_soft: with_alpha(accent, 0.14),
        accent_ring: with_alpha(rgb(p.focus_ring), 0.45),
        running: ink(rgb(status.running)),
        waiting: ink(waiting),
        yours: ink(rgb(status.active)),
        idle,
        failed: ink(rgb(status.failed)),
        completed: ink(rgb(status.completed)),
        danger_soft: with_alpha(rgb(status.failed), 0.14),
        shadow: if is_dark {
            Color::from_rgba8(0, 0, 0, 0.45)
        } else {
            Color::from_rgba8(0, 0, 0, 0.12)
        },
        viz: Viz {
            series: [c(series[0]), c(series[1]), c(series[2]), c(series[3])],
            heat: [
                c(mix(card, lift, if is_dark { 0.05 } else { 0.06 })),
                c(heat[0]),
                c(heat[1]),
                c(heat[2]),
                c(heat[3]),
                c(heat[4]),
            ],
            mute: c(ensure_contrast(mix(card, fg, 0.22), &[card], 1.6, Some(fg))),
            line: c(ensure_contrast(mix(card, fg, 0.42), &[card], 3.0, Some(fg))),
        },
    }
}

/// Every colour token of the window: [`Colors`], the finer names views are
/// drawn with, computed from the palette and from [`Inks`], the colours the
/// palette has no token for. Nothing here is a colour of its own.
pub fn derive_colors(appearance: Appearance, palette: &Palette, inks: &Inks) -> Colors {
    let _ = appearance;
    let p = palette;

    Colors {
        chrome: p.surface,
        chrome_bar: p.surface_raised,
        pane_head: p.surface,
        pane_head_focus: p.surface_raised,
        raised: p.surface_raised,
        card: inks.card,
        sep: p.surface_sunken,
        line: p.border,
        line_strong: p.border_strong,
        accent_hi: inks.accent_text,
        state_working: inks.running.text,
        state_working_fill: inks.running.fill,
        state_waiting: inks.waiting.text,
        state_waiting_fill: inks.waiting.fill,
        state_yours: inks.yours.text,
        state_yours_fill: inks.yours.fill,
        state_idle: inks.idle.text,
        state_idle_fill: inks.idle.fill,
        bg: p.bg,
        bg_sunken: p.surface_sunken,
        surface: p.surface,
        surface_hover: inks.hover,
        surface_active: p.selection_bg,
        border: p.border,
        border_strong: p.border_strong,
        fg: inks.text.primary,
        fg_secondary: inks.text.secondary,
        fg_muted: inks.text.muted,
        fg_subtle: inks.text.muted,
        accent: p.accent,
        accent_soft: inks.accent_soft,
        accent_ring: inks.accent_ring,
        accent_fg: p.on_accent,
        danger: inks.danger(),
        danger_soft: inks.danger_soft,
        success: inks.success(),
        warning: inks.warning(),
        shadow: inks.shadow,
        float_ring: inks.float_ring,
        // A segmented control: a recessed track, and the chosen segment
        // raised out of it.
        seg_track: p.surface_sunken,
        seg_thumb: inks.seg_thumb,
        viz_1: inks.viz.series[0],
        viz_2: inks.viz.series[1],
        viz_3: inks.viz.series[2],
        viz_4: inks.viz.series[3],
        viz_heat_0: inks.viz.heat[0],
        viz_heat_1: inks.viz.heat[1],
        viz_heat_2: inks.viz.heat[2],
        viz_heat_3: inks.viz.heat[3],
        viz_heat_4: inks.viz.heat[4],
        viz_heat_5: inks.viz.heat[5],
        viz_mute: inks.viz.mute,
        viz_line: inks.viz.line,
    }
}
