//! [`crate::Inks`]: the colours without a palette token, for the 35 themes.

use iced::Color;

use crate::{FILL_CONTRAST, Rgb, TEXT_CONTRAST, contrast_ratio, ink_surfaces, themes};

/// Colours are 8 bits: half a hundredth of slack for the rounding.
const SLACK: f64 = 0.005;

fn ratio(color: Color, background: Color) -> f64 {
    contrast_ratio(Rgb::from(color), Rgb::from(background))
}

#[test]
fn there_are_35_themes() {
    assert_eq!(themes().len(), 35);
}

/// Themes where lifting both words to 4.5:1 on every surface ends on the same
/// pale grey (their yellow and their "yours" hue are both too dark for it). The
/// fill, which only needs 3:1, still tells them apart, and so does the shape
/// the UI draws with it. Fixing the word would change the colour the UI has always drawn.
const SAME_WORD: &[&str] = &["solarized-dark", "solarized-light"];

#[test]
fn waiting_is_not_yours() {
    for t in themes() {
        let (waiting, yours) = (t.inks.waiting, t.inks.yours);
        assert_ne!(waiting.fill, yours.fill, "{}: the fills", t.id);
        assert_ne!(t.inks.waiting.fill, t.palette.status.active, "{}", t.id);
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
/// toolbar and secondary buttons, a menu). The palette is not changed here: a
/// later ticket decides, for each use on `surface_raised`,
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
