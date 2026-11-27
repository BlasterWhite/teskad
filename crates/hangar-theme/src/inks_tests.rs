//! [`crate::Inks`]: the colours without a palette token, for the 35 themes.

use iced::Color;

use crate::{AgentState, FILL_CONTRAST, Rgb, TEXT_CONTRAST, contrast_ratio, ink_surfaces, themes};

/// Colours are 8 bits: half a hundredth of slack for the rounding.
const SLACK: f64 = 0.005;

fn ratio(color: Color, background: Color) -> f64 {
    contrast_ratio(Rgb::from(color), Rgb::from(background))
}

const STATES: [AgentState; 4] = [
    AgentState::Working,
    AgentState::Waiting,
    AgentState::Yours,
    AgentState::Idle,
];

#[test]
fn there_are_35_themes() {
    assert_eq!(themes().len(), 35);
}

/// Every field of `Colors` that has a place outside it is that place's value,
/// field by field: the table in `inks.rs` is what this checks.
#[test]
fn colors_are_computed_from_the_palette_and_the_inks() {
    for t in themes() {
        let (c, p, i) = (&t.colors, &t.palette, &t.inks);
        let id = t.id;
        let pairs: [(&str, Color, Color); 38] = [
            ("state_working", c.state_working, i.running.text),
            ("state_working_fill", c.state_working_fill, i.running.fill),
            ("state_waiting", c.state_waiting, i.waiting.text),
            ("state_waiting_fill", c.state_waiting_fill, i.waiting.fill),
            ("state_yours", c.state_yours, i.yours.text),
            ("state_yours_fill", c.state_yours_fill, i.yours.fill),
            ("state_idle", c.state_idle, i.idle.text),
            ("state_idle_fill", c.state_idle_fill, i.idle.fill),
            ("danger", c.danger, i.danger()),
            ("danger", c.danger, i.failed.text),
            ("success", c.success, i.success()),
            ("success", c.success, i.completed.text),
            ("warning", c.warning, i.warning()),
            ("warning", c.warning, i.waiting.text),
            ("danger_soft", c.danger_soft, i.danger_soft),
            ("shadow", c.shadow, i.shadow),
            ("viz_1", c.viz_1, i.viz.series[0]),
            ("viz_2", c.viz_2, i.viz.series[1]),
            ("viz_3", c.viz_3, i.viz.series[2]),
            ("viz_4", c.viz_4, i.viz.series[3]),
            ("viz_heat_0", c.viz_heat_0, i.viz.heat[0]),
            ("viz_heat_1", c.viz_heat_1, i.viz.heat[1]),
            ("viz_heat_2", c.viz_heat_2, i.viz.heat[2]),
            ("viz_heat_3", c.viz_heat_3, i.viz.heat[3]),
            ("viz_heat_4", c.viz_heat_4, i.viz.heat[4]),
            ("viz_heat_5", c.viz_heat_5, i.viz.heat[5]),
            ("viz_mute", c.viz_mute, i.viz.mute),
            ("viz_line", c.viz_line, i.viz.line),
            ("bg", c.bg, p.bg),
            ("bg_sunken", c.bg_sunken, p.surface_sunken),
            ("surface", c.surface, p.surface),
            ("chrome", c.chrome, p.surface),
            ("chrome_bar", c.chrome_bar, p.surface_raised),
            ("sep", c.sep, p.surface_sunken),
            ("line", c.line, p.border),
            ("line_strong", c.line_strong, p.border_strong),
            ("surface_active", c.surface_active, p.selection_bg),
            ("accent_fg", c.accent_fg, p.on_accent),
        ];
        for (name, colour, expected) in pairs {
            assert_eq!(colour, expected, "{id}: {name}");
        }
        let computed: [(&str, Color, Color); 11] = [
            ("fg", c.fg, i.text.primary),
            ("fg_secondary", c.fg_secondary, i.text.secondary),
            ("fg_muted", c.fg_muted, i.text.muted),
            ("fg_subtle", c.fg_subtle, i.text.muted),
            ("accent_hi", c.accent_hi, i.accent_text),
            ("card", c.card, i.card),
            ("surface_hover", c.surface_hover, i.hover),
            ("seg_thumb", c.seg_thumb, i.seg_thumb),
            ("float_ring", c.float_ring, i.float_ring),
            ("accent_soft", c.accent_soft, i.accent_soft),
            ("accent_ring", c.accent_ring, i.accent_ring),
        ];
        for (name, colour, expected) in computed {
            assert_eq!(colour, expected, "{id}: {name}");
        }
        assert_eq!(c.accent, p.accent, "{id}");
        assert_eq!(c.border, p.border, "{id}");
        assert_eq!(c.border_strong, p.border_strong, "{id}");
        assert_eq!(c.pane_head, p.surface, "{id}");
        assert_eq!(c.pane_head_focus, p.surface_raised, "{id}");
        assert_eq!(c.raised, p.surface_raised, "{id}");
        assert_eq!(c.seg_track, p.surface_sunken, "{id}");
        for state in STATES {
            assert_eq!(c.state(state), i.state(state), "{id}");
        }
    }
}

/// Themes where lifting both words to 4.5:1 on every surface ends on the same
/// pale grey (their yellow and their "yours" hue are both too dark for it). The
/// fill, which only needs 3:1, still tells them apart, and so does the shape
/// the UI draws with it. Fixing the word would change a value of `Colors`.
const SAME_WORD: &[&str] = &["solarized-dark", "solarized-light"];

#[test]
fn waiting_is_not_yours() {
    for t in themes() {
        let (waiting, yours) = (t.inks.waiting, t.inks.yours);
        assert_ne!(waiting.fill, yours.fill, "{}: the fills", t.id);
        assert_ne!(
            t.colors.state_waiting_fill, t.palette.status.active,
            "{}",
            t.id
        );
        if !SAME_WORD.contains(&t.id) {
            assert_ne!(waiting.text, yours.text, "{}: the words", t.id);
        }
    }
    for id in SAME_WORD {
        let t = crate::by_id(id).unwrap();
        assert_eq!(
            t.inks.waiting.text, t.inks.yours.text,
            "{id}: the list is stale"
        );
    }
}

#[test]
fn every_state_reads_as_text_and_as_a_fill() {
    for t in themes() {
        let surfaces = ink_surfaces(t.appearance, &t.palette);
        let i = &t.inks;
        for (name, ink) in [
            ("running", i.running),
            ("waiting", i.waiting),
            ("yours", i.yours),
            ("idle", i.idle),
            ("failed", i.failed),
            ("completed", i.completed),
        ] {
            for &surface in &surfaces {
                let (text, fill) = (ratio(ink.text, surface), ratio(ink.fill, surface));
                assert!(
                    text >= TEXT_CONTRAST - SLACK,
                    "{} {name} text {text:.2}:1",
                    t.id
                );
                assert!(
                    fill >= FILL_CONTRAST - SLACK,
                    "{} {name} fill {fill:.2}:1",
                    t.id
                );
            }
        }
    }
}

#[test]
fn warning_danger_and_success_read_as_text() {
    for t in themes() {
        let surfaces = ink_surfaces(t.appearance, &t.palette);
        for (name, color) in [
            ("warning", t.inks.warning()),
            ("danger", t.inks.danger()),
            ("success", t.inks.success()),
        ] {
            for &surface in &surfaces {
                let text = ratio(color, surface);
                assert!(text >= TEXT_CONTRAST - SLACK, "{} {name} {text:.2}:1", t.id);
            }
        }
    }
}

/// The palette's own text tokens on the four surfaces. `themes.md` asks
/// 4.5:1 of all three, on `bg` and `surface` (what `contrast_failures`
/// checks); the views also lay them on `surface_raised` and `surface_sunken`.
fn text_token_failures() -> Vec<String> {
    let mut failures = Vec::new();
    for t in themes() {
        let p = &t.palette;
        let surfaces = [
            ("bg", p.bg),
            ("surface", p.surface),
            ("surface_raised", p.surface_raised),
            ("surface_sunken", p.surface_sunken),
        ];
        for (name, text) in [
            ("text", p.text),
            ("text_secondary", p.text_secondary),
            ("text_muted", p.text_muted),
        ] {
            for (on, surface) in surfaces {
                let r = ratio(text, surface);
                if r < TEXT_CONTRAST - SLACK {
                    failures.push(format!("{} {name} on {on}: {r:.2}:1", t.id));
                }
            }
        }
    }
    failures
}

/// The pairs below 4.5:1 today, all on `surface_raised` (a dark theme's card, the
/// toolbar and secondary buttons, a menu). The palette is not changed here: the
/// ticket that removes `Colors` decides, for each use on `surface_raised`,
/// between a lifted `inks.text.*` and a change of the theme's token. Nothing
/// fails on `bg`, `surface` or `surface_sunken`. `themes.md` asks 4.5:1 for
/// all three tokens, `text_muted` included (it has no 3:1 rule).
const BELOW_ON_RAISED: &[&str] = &[
    "graphite text_muted on surface_raised",
    "tokyo-night text_muted on surface_raised",
    "catppuccin-mocha text_muted on surface_raised",
    "one-dark text_secondary on surface_raised",
    "one-dark text_muted on surface_raised",
    "nord text_muted on surface_raised",
    "gruvbox-dark text_muted on surface_raised",
    "github-dark text_muted on surface_raised",
    "ayu-dark text_muted on surface_raised",
    "solarized-dark text on surface_raised",
    "solarized-dark text_secondary on surface_raised",
    "solarized-dark text_muted on surface_raised",
    "vscode-dark text_muted on surface_raised",
    "material-palenight text_secondary on surface_raised",
    "material-palenight text_muted on surface_raised",
    "andromeda text_muted on surface_raised",
    "ayu-mirage text_muted on surface_raised",
    "horizon text_muted on surface_raised",
    "panda text_muted on surface_raised",
    "kanagawa text_muted on surface_raised",
    "everforest-dark text_secondary on surface_raised",
    "everforest-dark text_muted on surface_raised",
    "poimandres text_secondary on surface_raised",
    "poimandres text_muted on surface_raised",
    "catppuccin-latte text_secondary on surface_raised",
    "catppuccin-latte text_muted on surface_raised",
    "github-light text_muted on surface_raised",
    "one-light text_muted on surface_raised",
    "solarized-light text on surface_raised",
    "solarized-light text_secondary on surface_raised",
    "solarized-light text_muted on surface_raised",
    "quiet-light text_muted on surface_raised",
    "ayu-light text_secondary on surface_raised",
    "ayu-light text_muted on surface_raised",
    "rose-pine-dawn text_secondary on surface_raised",
    "rose-pine-dawn text_muted on surface_raised",
    "night-owl-light text_secondary on surface_raised",
    "night-owl-light text_muted on surface_raised",
];

#[test]
fn text_tokens_read_on_every_surface_but_the_known_raised_cases() {
    let failures: Vec<String> = text_token_failures()
        .into_iter()
        .map(|f| f.split(": ").next().unwrap().to_owned())
        .collect();
    assert!(
        failures.iter().all(|f| f.ends_with("on surface_raised")),
        "a text token fell below 4.5:1 on bg, surface or surface_sunken: {failures:?}"
    );
    assert_eq!(failures, BELOW_ON_RAISED, "the list of known cases changed");
}
