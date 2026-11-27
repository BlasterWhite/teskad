//! The bundled themes: files in the brand pack's format, each held to the
//! contrast `docs/teskad-brand/docs/themes.md` requires, the two Teskad themes
//! to the exact values of their files.

use std::collections::HashSet;

use iced::Color;

use crate::{
    DEFAULT_ID, MIN_WINDOW_OPACITY, Rgb, Theme, ThemeSpec, UiOverrides, by_id, by_id_or_default,
    contrast_failures, contrast_ratio, default_theme, derive_palette, mix, perceived_lightness,
    theme_files, themes,
};

/// A pack file, read from the brand pack itself when the checkout has it.
fn pack_file(name: &str) -> Option<String> {
    let path = format!(
        "{}/../../docs/teskad-brand/themes/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(path).ok()
}

fn hex(color: Color) -> String {
    let [r, g, b, a] = color.into_rgba8();
    if a == 255 {
        format!("#{r:02X}{g:02X}{b:02X}")
    } else {
        format!(
            "rgba({r}, {g}, {b}, {})",
            (f32::from(a) / 255.0 * 100.0).round() / 100.0
        )
    }
}

mod the_theme_files {
    use super::*;

    #[test]
    fn are_35_and_every_one_is_read_strictly() {
        assert_eq!(theme_files().len(), 35);
        for json in theme_files() {
            let spec = ThemeSpec::parse(json).unwrap_or_else(|error| panic!("{error}"));
            let theme = by_id(&spec.id).expect("a file is a bundled theme");
            assert_eq!(theme.name, spec.name);
            assert_eq!(theme.appearance, spec.appearance);
            assert_eq!(theme.terminal, spec.terminal);
        }
    }

    #[test]
    fn refuse_what_the_schema_refuses() {
        let good = theme_files()[2];
        assert!(ThemeSpec::parse(good).is_ok());
        let broken: [(&str, String); 7] = [
            (
                "an unknown key",
                good.replacen("\"ui\"", "\"extra\": 1, \"ui\"", 1),
            ),
            ("a bad colour", good.replacen("\"#1A1B26\"", "\"blue\"", 1)),
            (
                "a short colour",
                good.replacen("\"#1A1B26\"", "\"#1A1B2\"", 1),
            ),
            (
                "a bad id",
                good.replacen("\"tokyo-night\"", "\"Tokyo Night\"", 1),
            ),
            ("a bad appearance", good.replacen("\"dark\"", "\"grey\"", 1)),
            ("a missing terminal", {
                let mut json: serde_json::Value = serde_json::from_str(good).unwrap();
                json.as_object_mut().unwrap().remove("terminal");
                json.to_string()
            }),
            ("a missing ansi colour", {
                let mut json: serde_json::Value = serde_json::from_str(good).unwrap();
                json["terminal"]["ansi"]
                    .as_object_mut()
                    .unwrap()
                    .remove("cyan");
                json.to_string()
            }),
        ];
        for (what, json) in broken {
            assert!(ThemeSpec::parse(&json).is_err(), "{what} is accepted");
        }
        let alpha = good.replacen(
            "\"ui\": {",
            "\"ui\": {\"hoverOverlay\": \"rgba(1, 2, 3, 1.5)\",",
            1,
        );
        assert!(
            ThemeSpec::parse(&alpha).is_err(),
            "an alpha above 1 is accepted"
        );
        let rgba = good.replacen(
            "\"ui\": {",
            "\"ui\": {\"scrim\": \"rgba(1, 2, 3, 0.5)\",",
            1,
        );
        assert!(ThemeSpec::parse(&rgba).is_ok());
    }

    #[test]
    fn leave_the_interface_to_the_derivation_but_for_an_accent() {
        for json in theme_files() {
            let spec = ThemeSpec::parse(json).unwrap();
            if spec.id.starts_with("teskad-") {
                continue;
            }
            let set = UiOverrides {
                accent: None,
                ..spec.ui
            };
            assert_eq!(
                set,
                UiOverrides::default(),
                "{} sets more than its accent",
                spec.id
            );
        }
        // Tokyo Night's own blue, not its magenta.
        let tokyo = ThemeSpec::parse(theme_files()[2]).unwrap();
        assert_eq!(tokyo.id, "tokyo-night");
        assert_eq!(tokyo.ui.accent, Some(Rgb::hex("#7AA2F7").to_color()));
    }

    #[test]
    fn teskad_files_are_the_packs_own_when_the_pack_is_there() {
        for (name, index) in [("teskad-dark.json", 0), ("teskad-light.json", 26)] {
            if let Some(pack) = pack_file(name) {
                assert_eq!(
                    theme_files()[index],
                    pack,
                    "{name} drifted from docs/teskad-brand"
                );
            }
        }
        if let Some(pack) = pack_file("theme.schema.json") {
            let ours = include_str!("../themes/theme.schema.json");
            assert_eq!(
                ours, pack,
                "theme.schema.json drifted from docs/teskad-brand"
            );
        }
    }
}

mod the_contrast_of_every_theme {
    use super::*;

    /// The one finding the check has on a bundled theme: the pack's own
    /// `border` is 1.27:1 on Teskad Dark's `bg` (#2A2135 on #0E0C12) and 1.22:1
    /// on Teskad Light's (#E6E0EE on #FAF8FC), under the 1.3:1 `themes.md`
    /// asks for. Both files are used as they are, so their border is held to
    /// what it is (a hairline: `borderStrong` is the visible one).
    const PACK_BORDERS: [&str; 2] = ["teskad-dark", "teskad-light"];

    #[test]
    fn every_bundled_theme_passes_the_check() {
        for theme in themes() {
            let failures = contrast_failures(&theme.palette);
            if PACK_BORDERS.contains(&theme.id) {
                assert_eq!(failures.len(), 1, "{failures:?}");
                assert!(failures[0].starts_with("border on bg"), "{failures:?}");
            } else {
                assert!(failures.is_empty(), "{}: {failures:?}", theme.id);
            }
        }
    }

    #[test]
    fn the_numbers_of_themes_md_hold_one_by_one() {
        for theme in themes() {
            let p = &theme.palette;
            let (bg, surface) = (Rgb::from(p.bg), Rgb::from(p.surface));
            let id = theme.id;
            for (name, color) in [
                ("text", p.text),
                ("textSecondary", p.text_secondary),
                ("textMuted", p.text_muted),
                ("accentText", p.accent_text),
            ] {
                for (on, background) in [("bg", bg), ("surface", surface)] {
                    let ratio = contrast_ratio(Rgb::from(color), background);
                    assert!(ratio >= 4.495, "{id}: {name} on {on} is {ratio:.2}");
                }
            }
            let on_accent = contrast_ratio(Rgb::from(p.on_accent), Rgb::from(p.accent));
            assert!(
                on_accent >= 4.495,
                "{id}: onAccent on accent is {on_accent:.2}"
            );
            for (name, color) in [
                ("running", p.status.running),
                ("completed", p.status.completed),
                ("failed", p.status.failed),
                ("active", p.status.active),
                ("inactive", p.status.inactive),
            ] {
                let ratio = contrast_ratio(Rgb::from(color), surface);
                assert!(
                    ratio >= 2.995,
                    "{id}: status.{name} on surface is {ratio:.2}"
                );
            }
            let border = contrast_ratio(Rgb::from(p.border), bg);
            let least = if PACK_BORDERS.contains(&id) {
                1.2
            } else {
                1.295
            };
            assert!(border >= least, "{id}: border on bg is {border:.2}");
        }
    }

    #[test]
    fn a_palette_the_derivation_gets_wrong_is_corrected_not_sent_as_it_is() {
        let base = by_id("graphite").unwrap();
        let custom = |appearance, background: &str, foreground: &str| {
            let mut terminal = base.terminal;
            terminal.background = Rgb::hex(background).to_color();
            terminal.foreground = Rgb::hex(foreground).to_color();
            // A red and a green that vanish into the background.
            for slot in [1, 2] {
                terminal.ansi[slot] = terminal.background;
            }
            Theme::derived(
                "custom",
                "Not bundled",
                appearance,
                terminal,
                &UiOverrides::default(),
            )
        };
        for theme in [
            custom(crate::Appearance::Dark, "#101014", "#1c1c20"),
            custom(crate::Appearance::Light, "#fafafa", "#eeeeee"),
        ] {
            assert_ne!(theme.palette.text, theme.terminal.foreground);
            assert_ne!(theme.palette.status.failed, theme.terminal.ansi[1]);
            let failures = contrast_failures(&theme.palette);
            assert!(failures.is_empty(), "{failures:?}");
        }
    }

    #[test]
    fn what_a_theme_sets_is_kept_as_it_is_and_the_rest_follows_from_the_terminal() {
        let tokyo = ThemeSpec::parse(theme_files()[2]).unwrap();
        let mut ui = tokyo.ui;
        ui.text = Some(Rgb::hex("#123456").to_color());
        let p = derive_palette(tokyo.appearance, &tokyo.terminal, &ui);
        assert_eq!(hex(p.text), "#123456");
        assert_eq!(p.bg, tokyo.terminal.background);
        assert_eq!(p.accent, tokyo.ui.accent.unwrap());
        assert_eq!(p.selection_border, p.accent);
        assert_eq!(p.focus_ring, p.accent_text);
        assert_eq!(p.status.running, p.accent_text);
        assert_eq!(p.status.completed, tokyo.terminal.ansi[2]);
        assert_eq!(p.status.failed, tokyo.terminal.ansi[1]);
    }
}

mod the_teskad_themes {
    use super::*;

    /// A token of a pack file, as the pack writes it.
    fn ui(json: &str, key: &str) -> String {
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        let mut at = &value["ui"];
        for part in key.split('.') {
            at = &at[part];
        }
        at.as_str().unwrap_or_else(|| panic!("{key}")).to_owned()
    }

    #[test]
    fn are_their_files_exactly_with_nothing_computed() {
        for (id, index) in [("teskad-dark", 0), ("teskad-light", 26)] {
            let theme = by_id(id).unwrap();
            let json = theme_files()[index];
            for (name, color) in theme.palette.iter() {
                let wanted = crate::spec::parse_color(&ui(json, name)).unwrap();
                assert_eq!(color, wanted, "{id}: {name}");
            }
            let file: serde_json::Value = serde_json::from_str(json).unwrap();
            let t = &theme.terminal;
            assert_eq!(hex(t.background), file["terminal"]["background"]);
            assert_eq!(hex(t.foreground), file["terminal"]["foreground"]);
            assert_eq!(hex(t.cursor), file["terminal"]["cursor"]);
            assert_eq!(hex(t.selection), file["terminal"]["selection"]);
            assert_eq!(hex(t.ansi[5]), file["terminal"]["ansi"]["magenta"]);
        }
    }

    #[test]
    fn dark_has_the_packs_surfaces_and_white_on_its_violet() {
        let p = &by_id("teskad-dark").unwrap().palette;
        assert_eq!(hex(p.bg), "#0E0C12");
        assert_eq!(hex(p.surface), "#17121E");
        assert_eq!(hex(p.surface_raised), "#1F1828");
        assert_eq!(hex(p.surface_sunken), "#0B0A0E");
        assert_eq!(hex(p.accent), "#8A4DFF");
        assert_eq!(hex(p.on_accent), "#FFFFFF");
        assert_eq!(hex(p.status.failed), "#FF6A55");
        assert_eq!(hex(p.status.completed), "#4FD8A4");
    }

    #[test]
    fn light_has_the_packs_surfaces_and_states() {
        let p = &by_id("teskad-light").unwrap().palette;
        assert_eq!(hex(p.bg), "#FAF8FC");
        assert_eq!(hex(p.surface), "#FFFFFF");
        assert_eq!(hex(p.surface_sunken), "#F1EDF6");
        assert_eq!(hex(p.accent_text), "#5B2FC9");
        assert_eq!(hex(p.status.running), "#6D3BE0");
        assert_eq!(hex(p.status.completed), "#0F7A55");
    }

    #[test]
    fn are_the_first_of_their_appearance_and_the_default_is_the_dark_one() {
        assert_eq!(themes()[0].id, "teskad-dark");
        let first_light = themes().iter().find(|theme| !theme.is_dark()).unwrap();
        assert_eq!(first_light.id, "teskad-light");
        assert_eq!(DEFAULT_ID, "teskad-dark");
        // Nothing was removed: the old default is still there to be chosen.
        assert!(by_id("graphite").is_some());
    }

    #[test]
    fn never_use_the_brand_magenta_for_a_failure() {
        for id in ["teskad-dark", "teskad-light"] {
            let theme = by_id(id).unwrap();
            let magenta = Rgb::hex("#E040B0").to_color();
            assert_ne!(theme.palette.status.failed, magenta, "{id}");
            assert_ne!(theme.inks.danger(), magenta, "{id}");
        }
    }
}

mod the_bundled_themes {
    use super::*;

    #[test]
    fn cover_every_theme() {
        assert_eq!(themes().len(), 35);
    }

    #[test]
    fn have_unique_ids_and_names() {
        let ids: HashSet<_> = themes().iter().map(|theme| theme.id).collect();
        let names: HashSet<_> = themes().iter().map(|theme| theme.name).collect();
        assert_eq!(ids.len(), themes().len());
        assert_eq!(names.len(), themes().len());
    }

    #[test]
    fn have_ids_that_are_safe_to_store() {
        for theme in themes() {
            assert!(!theme.id.is_empty(), "an empty id");
            assert!(
                theme
                    .id
                    .chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-'),
                "{}",
                theme.id
            );
            assert!(!theme.name.trim().is_empty(), "{}", theme.id);
        }
    }

    #[test]
    fn start_with_teskad_dark_which_is_the_default() {
        assert_eq!(DEFAULT_ID, "teskad-dark");
        assert_eq!(themes()[0].id, DEFAULT_ID);
        assert_eq!(default_theme().id, DEFAULT_ID);
    }

    #[test]
    fn are_found_by_id_and_fall_back_on_the_default() {
        for theme in themes() {
            assert_eq!(by_id(theme.id).map(|found| found.id), Some(theme.id));
        }
        assert!(by_id("no-such-theme").is_none());
        assert_eq!(by_id_or_default("no-such-theme").id, DEFAULT_ID);
        assert_eq!(by_id_or_default("nord").id, "nord");
    }

    #[test]
    fn put_the_dark_themes_before_the_light_ones() {
        let first_light = themes().iter().position(|theme| !theme.is_dark()).unwrap();
        assert!(themes()[first_light..].iter().all(|theme| !theme.is_dark()));
        assert!(themes()[..first_light].iter().all(Theme::is_dark));
    }

    #[test]
    fn are_complete() {
        for theme in themes() {
            let t = &theme.terminal;
            let opaque = [
                t.foreground,
                t.background,
                t.cursor,
                t.cursor_text,
                t.selection,
            ]
            .into_iter()
            .chain(t.ansi);
            for color in opaque {
                assert_eq!(
                    color.a, 1.0,
                    "{}: a terminal colour is translucent",
                    theme.id
                );
            }
            // The 16 slots are not all one colour, and the background differs
            // from the foreground.
            assert_ne!(t.foreground, t.background, "{}", theme.id);
            let distinct: HashSet<_> = t.ansi.iter().map(|c| c.into_rgba8()).collect();
            assert!(distinct.len() >= 8, "{}", theme.id);

            assert_eq!(theme.palette.accent, theme.accent());
        }
    }

    #[test]
    fn have_the_appearance_their_background_has() {
        for theme in themes() {
            let lightness = perceived_lightness(Rgb::from(theme.terminal.background));
            assert_eq!(theme.is_dark(), lightness < 0.5, "{}", theme.id);
        }
    }
}

mod window_opacity {
    use super::*;
    use crate::theme::window_alpha_of;

    fn tokyo() -> &'static Theme {
        by_id("tokyo-night").unwrap()
    }

    #[test]
    fn every_bundled_theme_is_a_solid_window_until_the_person_says_otherwise() {
        for theme in themes() {
            assert_eq!(theme.window_opacity, 1.0, "{}", theme.id);
            assert!(!theme.is_translucent(), "{}", theme.id);
            assert_eq!(theme.window_alpha(), 1.0, "{}", theme.id);
            assert_eq!(theme.see_through(theme.palette.bg), theme.palette.bg);
        }
    }

    #[test]
    fn is_held_between_the_floor_and_solid() {
        assert_eq!(window_alpha_of(0.92), 0.92);
        assert_eq!(window_alpha_of(1.0), 1.0);
        assert_eq!(window_alpha_of(1.7), 1.0);
        assert_eq!(window_alpha_of(0.1), MIN_WINDOW_OPACITY);
        assert_eq!(window_alpha_of(f32::NAN), 1.0);
        assert_eq!(window_alpha_of(f32::INFINITY), 1.0);
    }

    #[test]
    fn at_1_a_theme_is_the_bundled_one() {
        let theme = default_theme();
        assert!(std::ptr::eq(theme.with_window_opacity(1.0), theme));
        assert!(std::ptr::eq(theme.with_window_opacity(f64::NAN), theme));
        assert!(std::ptr::eq(theme.with_window_opacity(7.0), theme));
    }

    #[test]
    fn applies_to_any_theme_and_changes_nothing_but_the_alpha() {
        for theme in [default_theme(), tokyo()] {
            let solid = theme.with_window_opacity(1.0);
            let glass = theme.with_window_opacity(0.85);
            assert_eq!(glass.window_opacity, 0.85, "{}", theme.id);
            // The same theme at 1 and at 0.85: its accent, its text, its
            // borders, its terminal's palette.
            assert_eq!(glass.id, solid.id);
            assert_eq!(glass.accent(), solid.accent(), "{}", theme.id);
            assert_eq!(
                glass.inks.text.primary, solid.inks.text.primary,
                "{}",
                theme.id
            );
            assert_eq!(glass.palette, solid.palette, "{}", theme.id);
            assert_eq!(glass.inks, solid.inks, "{}", theme.id);
            assert_eq!(glass.terminal, solid.terminal, "{}", theme.id);
            if cfg!(target_os = "linux") {
                assert!(glass.is_translucent());
                assert_eq!(glass.window_alpha(), 0.85, "{}", theme.id);
                // Only the alpha of a background goes down.
                assert_eq!(
                    glass.see_through(glass.palette.surface),
                    solid.palette.surface.scale_alpha(0.85)
                );
            } else {
                assert!(!glass.is_translucent());
                assert_eq!(glass.window_alpha(), 1.0, "{}", theme.id);
            }
        }
        // Tokyo Night keeps its own accent, not Graphite's.
        assert_ne!(
            tokyo().with_window_opacity(0.85).accent(),
            default_theme().with_window_opacity(0.85).accent()
        );
    }

    #[test]
    fn is_held_to_whole_percents_from_the_floor_and_made_once() {
        let theme = tokyo();
        let once = theme.with_window_opacity(0.854);
        assert_eq!(once.window_opacity, 0.85);
        assert!(std::ptr::eq(once, theme.with_window_opacity(0.85)));
        assert_eq!(
            theme.with_window_opacity(0.1).window_opacity,
            MIN_WINDOW_OPACITY
        );
    }
}

mod the_statistics_colours {
    use super::*;

    #[test]
    fn step_away_from_the_card_in_order_on_every_theme_the_faintest_still_at_2_to_1() {
        for theme in themes() {
            let card = Rgb::from(theme.inks.card);
            let steps = [
                theme.inks.viz.series[0],
                theme.inks.viz.series[1],
                theme.inks.viz.series[2],
                theme.inks.viz.series[3],
            ]
            .map(Rgb::from);
            let away =
                steps.map(|color| (perceived_lightness(color) - perceived_lightness(card)).abs());
            // Far enough apart to tell neighbours in a stacked column at a glance.
            for step in 1..steps.len() {
                assert!(
                    away[step] - away[step - 1] >= 0.06,
                    "{} step {step}",
                    theme.id
                );
            }
            assert!(contrast_ratio(steps[0], card) >= 2.0, "{}", theme.id);
            assert!(contrast_ratio(steps[3], card) >= 3.0, "{}", theme.id);
        }
    }

    #[test]
    fn darken_or_brighten_the_heatmap_with_the_value_from_an_empty_cell_up() {
        for theme in themes() {
            let card = Rgb::from(theme.inks.card);
            let levels = [
                theme.inks.viz.heat[1],
                theme.inks.viz.heat[2],
                theme.inks.viz.heat[3],
                theme.inks.viz.heat[4],
                theme.inks.viz.heat[5],
            ];
            let away = |color: Color| {
                (perceived_lightness(Rgb::from(color)) - perceived_lightness(card)).abs()
            };
            let mut previous = away(theme.inks.viz.heat[0]);
            for (level, color) in levels.into_iter().enumerate() {
                let now = away(color);
                assert!(now > previous, "{} level {}", theme.id, level + 1);
                previous = now;
            }
        }
    }
}

mod search_colours {
    use super::*;

    #[test]
    fn lay_the_themes_yellow_and_accent_over_its_background() {
        let theme = by_id("graphite").unwrap();
        let search = theme.search_colors();
        let bg = Rgb::from(theme.terminal.background);
        let yellow = Rgb::from(theme.terminal.ansi[3]);
        assert_eq!(Rgb::from(search.match_background), mix(bg, yellow, 0.3));
        assert_eq!(Rgb::from(search.match_border), mix(bg, yellow, 0.65));
        assert_eq!(search.match_marker, theme.terminal.ansi[3]);
        assert_eq!(search.active_border, theme.palette.accent);
    }
}
