//! The themes held to what the web app's `src/themes.test.ts` held them to,
//! plus the checks that make the port trustworthy: the Rust derivation must
//! give the tokens the table holds, which the TypeScript first wrote.

use std::collections::HashSet;

use iced::Color;

use crate::{
    AgentState, Colors, DEFAULT_ID, FILL_CONTRAST, MIN_WINDOW_OPACITY, Rgb, TEXT_CONTRAST, Theme,
    by_id, by_id_or_default, contrast_ratio, default_theme, derive_colors, hue_and_saturation,
    hue_distance, mix, over, perceived_lightness, themes,
};

/// The surfaces the chrome writes on; every token is held to all of them.
fn surfaces(theme: &Theme) -> [Rgb; 4] {
    let c = &theme.colors;
    [c.chrome, c.chrome_bar, c.raised, c.pane_head].map(Rgb::from)
}

/// Tokens that colour text: 4.5:1 on every surface.
fn text_tokens(c: &Colors) -> [(&'static str, Color); 12] {
    [
        ("--fg", c.fg),
        ("--fg-secondary", c.fg_secondary),
        ("--fg-muted", c.fg_muted),
        ("--fg-subtle", c.fg_subtle),
        ("--accent-hi", c.accent_hi),
        ("--state-working", c.state_working),
        ("--state-waiting", c.state_waiting),
        ("--state-yours", c.state_yours),
        ("--state-idle", c.state_idle),
        ("--danger", c.danger),
        ("--success", c.success),
        ("--warning", c.warning),
    ]
}

/// Everything that can also be written on a selected row, a chip or the
/// toolbar's capsule, all of them a step further out, at `surface_active`.
/// The two quietest greys are kept off those.
fn selection_text_tokens(c: &Colors) -> Vec<(&'static str, Color)> {
    text_tokens(c)
        .into_iter()
        .filter(|(name, _)| *name != "--fg-muted" && *name != "--fg-subtle")
        .collect()
}

/// A state's glyph, a bar, a frame: 3:1 on every surface.
fn fill_tokens(c: &Colors) -> [(&'static str, Color); 4] {
    [
        ("--state-working-fill", c.state_working_fill),
        ("--state-waiting-fill", c.state_waiting_fill),
        ("--state-yours-fill", c.state_yours_fill),
        ("--state-idle-fill", c.state_idle_fill),
    ]
}

/// The fills a label or state word can sit on beyond the plain surfaces.
fn fills(theme: &Theme) -> [Rgb; 2] {
    let c = &theme.colors;
    [
        Rgb::from(c.surface_active),
        // A hairline's tint over a raised group: a chip, the toolbar's capsule.
        over(c.line, Rgb::from(c.raised)),
    ]
}

fn hue(color: Color) -> f64 {
    hue_and_saturation(Rgb::from(color)).hue
}

mod the_bundled_themes {
    use super::*;

    #[test]
    fn cover_every_theme() {
        assert_eq!(themes().len(), 34);
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
    fn start_with_graphite_which_is_the_default() {
        assert_eq!(DEFAULT_ID, "graphite");
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

            assert_eq!(theme.colors.iter().count(), Colors::CSS_NAMES.len());
            for (name, color) in theme.colors.iter() {
                for channel in [color.r, color.g, color.b, color.a] {
                    assert!(
                        (0.0..=1.0).contains(&channel),
                        "{}: {name} is out of range",
                        theme.id
                    );
                }
            }
            assert_eq!(theme.colors.accent, theme.accent());
            assert_eq!(theme.colors.bg, theme.terminal.background, "{}", theme.id);
        }
    }

    #[test]
    fn name_every_css_token_once_and_find_it_again() {
        let names: HashSet<_> = Colors::CSS_NAMES.iter().collect();
        assert_eq!(names.len(), Colors::CSS_NAMES.len());
        let theme = default_theme();
        for (name, color) in theme.colors.iter() {
            assert!(name.starts_with("--"));
            assert_eq!(theme.colors.get(name), Some(color));
        }
        assert_eq!(theme.colors.get("--no-such-token"), None);
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

    #[test]
    fn every_theme_but_nightglass_is_a_solid_window() {
        for theme in themes() {
            if theme.id == "nightglass" {
                continue;
            }
            assert_eq!(theme.window_opacity, 1.0, "{}", theme.id);
            assert!(!theme.is_translucent(), "{}", theme.id);
            assert_eq!(theme.window_alpha(), 1.0, "{}", theme.id);
        }
    }

    #[test]
    fn every_opacity_is_one_the_window_is_drawn_at() {
        for theme in themes() {
            assert!(
                (MIN_WINDOW_OPACITY..=1.0).contains(&theme.window_opacity),
                "{}",
                theme.id
            );
            assert!(
                (0.0..=1.0).contains(&theme.unfocused_opacity) && theme.unfocused_opacity > 0.5,
                "{}",
                theme.id
            );
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
    fn shows_the_desktop_through_on_linux_only() {
        let glass = by_id("nightglass").unwrap();
        assert_eq!(glass.window_opacity, 0.92);
        if cfg!(target_os = "linux") {
            assert!(glass.is_translucent());
            assert_eq!(glass.window_alpha(), 0.92);
            let page = glass.see_through(glass.colors.bg);
            assert_eq!(page.a, 0.92);
            assert_eq!(
                Color { a: 1.0, ..page },
                glass.colors.bg,
                "the colour itself does not change"
            );
        } else {
            assert!(!glass.is_translucent());
            assert_eq!(glass.see_through(glass.colors.bg), glass.colors.bg);
        }
        // A solid theme's colours go through as they are.
        let solid = default_theme();
        assert_eq!(solid.see_through(solid.colors.chrome), solid.colors.chrome);
    }
}

mod nightglass {
    use super::*;

    fn hex(color: Color) -> String {
        Rgb::from(color).to_hex()
    }

    #[test]
    fn is_a_dark_theme_with_its_palette_and_a_pink_accent() {
        let theme = by_id("nightglass").unwrap();
        assert_eq!(theme.name, "Nightglass");
        assert!(theme.is_dark());
        let t = &theme.terminal;
        assert_eq!(hex(t.background), "#101016");
        assert_eq!(hex(t.foreground), "#dcdce4");
        assert_eq!(hex(t.cursor), "#f5a3c7");
        assert_eq!(hex(t.cursor_text), "#101016");
        assert_eq!(hex(t.selection), "#3a3a55");
        let ansi: Vec<String> = t.ansi.iter().map(|&c| hex(c)).collect();
        assert_eq!(
            ansi,
            [
                "#1c1c24", "#f07a8c", "#9fd8a4", "#f2d38a", "#8fb4f5", "#f5a3c7", "#8fd8e0",
                "#c8c8d4", "#5c5c6e", "#ff92a3", "#b5e8b9", "#f7e0a6", "#a8c6fa", "#fbbfd8",
                "#a9e6ec", "#f0f0f6",
            ]
        );
        assert_eq!(hex(theme.accent()), "#f5a3c7");
        // Dark text on the pink, as on every dark theme.
        assert!(
            perceived_lightness(Rgb::from(theme.colors.accent_fg)) < 0.3,
            "{}",
            hex(theme.colors.accent_fg)
        );
        assert_eq!(theme.unfocused_opacity, 0.75);
    }

    #[test]
    fn has_its_borders_near_the_split_divider_of_the_palette_it_comes_from() {
        // Ghostty's `split-divider-color = 2a2a3a`: the hairlines a theme
        // derives from its page land within a few steps of it.
        let c = &by_id("nightglass").unwrap().colors;
        let divider = Rgb(0x2a, 0x2a, 0x3a);
        let border = Rgb::from(c.border);
        let distance = [
            (border.0, divider.0),
            (border.1, divider.1),
            (border.2, divider.2),
        ]
        .iter()
        .map(|&(a, b)| a.abs_diff(b))
        .max()
        .unwrap();
        assert!(distance <= 12, "{} is {distance} away", hex(c.border));
    }
}

mod the_derivation_of_the_chrome_tokens {
    use super::*;

    /// A `colors` entry of `data.rs` as it is written there.
    fn rust_block(colors: &Colors) -> String {
        let lines: Vec<String> = colors
            .iter()
            .map(|(name, color)| {
                let field = name.trim_start_matches("--").replace('-', "_");
                let [r, g, b, _] = color.into_rgba8();
                if color.a >= 1.0 {
                    format!("            {field}: rgb(0x{r:02x}{g:02x}{b:02x}),")
                } else {
                    format!("            {field}: rgba({r}, {g}, {b}, {}),", color.a)
                }
            })
            .collect();
        format!(
            "        colors: Colors {{\n{}\n        }},",
            lines.join("\n")
        )
    }

    #[test]
    fn gives_on_every_theme_the_tokens_the_typescript_gives() {
        // The tables were first written by running the web app's
        // deriveChromeTokens; this is the port of it, which has to agree to
        // the last channel. A theme added to data.rs is held to it too: on a
        // mismatch the message is the whole `colors` entry the derivation
        // gives, ready to paste over the theme's.
        for theme in themes() {
            let derived = derive_colors(theme.appearance, theme.colors.accent, &theme.terminal);
            let agree = theme
                .colors
                .iter()
                .zip(derived.iter())
                .all(|((_, expected), (_, got))| {
                    got.into_rgba8() == expected.into_rgba8() && (got.a - expected.a).abs() < 1e-6
                });
            assert!(
                agree,
                "{}: the colours are not the ones its palette and accent give; they are\n{}",
                theme.id,
                rust_block(&derived)
            );
        }
    }

    #[test]
    fn prints_an_entry_the_way_the_table_writes_it() {
        // What the test above prints is pasted into data.rs: it has to read
        // exactly as the entries already there.
        // A Windows checkout may hold the file with CRLF line ends.
        let table = include_str!("data.rs").replace("\r\n", "\n");
        for theme in themes() {
            assert!(table.contains(&rust_block(&theme.colors)), "{}", theme.id);
        }
    }
}

mod on_every_bundled_theme {
    use super::*;

    #[test]
    fn every_text_token_is_written_at_4_5_to_1_on_every_surface() {
        for theme in themes() {
            for (name, color) in text_tokens(&theme.colors) {
                for surface in surfaces(theme) {
                    let ratio = contrast_ratio(Rgb::from(color), surface);
                    assert!(
                        ratio >= TEXT_CONTRAST,
                        "{}: {name} on {surface:?} is {ratio:.2}",
                        theme.id
                    );
                }
            }
        }
    }

    #[test]
    fn labels_state_words_and_links_read_at_4_5_to_1_on_the_selection_and_on_chips() {
        for theme in themes() {
            for (name, color) in selection_text_tokens(&theme.colors) {
                for fill in fills(theme) {
                    let ratio = contrast_ratio(Rgb::from(color), fill);
                    assert!(
                        ratio >= TEXT_CONTRAST,
                        "{}: {name} on {fill:?} is {ratio:.2}",
                        theme.id
                    );
                }
            }
        }
    }

    #[test]
    fn every_state_fill_is_drawn_at_3_to_1_on_every_surface_and_fill() {
        for theme in themes() {
            let on: Vec<Rgb> = surfaces(theme).into_iter().chain(fills(theme)).collect();
            for (name, color) in fill_tokens(&theme.colors) {
                for surface in &on {
                    let ratio = contrast_ratio(Rgb::from(color), *surface);
                    assert!(
                        ratio >= FILL_CONTRAST,
                        "{}: {name} on {surface:?} is {ratio:.2}",
                        theme.id
                    );
                }
            }
        }
    }

    #[test]
    fn the_chosen_segments_label_reads_at_4_5_to_1() {
        for theme in themes() {
            let c = &theme.colors;
            assert!(
                contrast_ratio(Rgb::from(c.fg), Rgb::from(c.seg_thumb)) >= TEXT_CONTRAST,
                "{}",
                theme.id
            );
        }
    }

    #[test]
    fn text_on_the_accent_reads_at_4_5_to_1() {
        for theme in themes() {
            let c = &theme.colors;
            assert!(
                contrast_ratio(Rgb::from(c.accent_fg), Rgb::from(c.accent)) >= TEXT_CONTRAST,
                "{}",
                theme.id
            );
        }
    }

    #[test]
    fn your_turn_has_a_hue_apart_from_the_accent_and_from_working() {
        for theme in themes() {
            let c = &theme.colors;
            // 30 degrees, give or take the rounding to 8-bit channels.
            let yours = hue(c.state_yours_fill);
            assert!(hue_distance(yours, hue(c.accent)) >= 29.0, "{}", theme.id);
            assert!(
                hue_distance(yours, hue(c.state_working_fill)) >= 29.0,
                "{}",
                theme.id
            );
        }
    }
}

mod on_the_themes_the_app_is_designed_in {
    use super::*;

    fn hex(color: Color) -> String {
        Rgb::from(color).to_hex()
    }

    #[test]
    fn graphite_keeps_its_own_state_colours_where_they_already_read() {
        let c = &by_id("graphite").unwrap().colors;
        assert_eq!(hex(c.state_working), "#5fd27f");
        assert_eq!(hex(c.state_working_fill), "#5fd27f");
        assert_eq!(hex(c.state_waiting), "#e7c06b");
        // Graphite's cyan, not its blue accent.
        assert!(hue_distance(hue(c.state_yours), hue(Color::from_rgb8(0x6f, 0xd3, 0xdf))) < 2.0);
    }

    #[test]
    fn graphite_and_github_light_keep_the_secondary_text_the_stylesheets_mixed() {
        for id in ["graphite", "github-light"] {
            let theme = by_id(id).unwrap();
            let fg = Rgb::from(theme.terminal.foreground);
            let bg = Rgb::from(theme.terminal.background);
            assert_eq!(Rgb::from(theme.colors.fg), fg, "{id}");
            assert_eq!(
                Rgb::from(theme.colors.fg_secondary),
                mix(fg, bg, 0.24),
                "{id}"
            );
        }
    }

    #[test]
    fn graphites_three_secondary_steps_are_in_order_brightest_first() {
        let c = &by_id("graphite").unwrap().colors;
        let on = |color: Color| contrast_ratio(Rgb::from(color), Rgb::from(c.raised));
        assert!(on(c.fg) > on(c.fg_secondary));
        assert!(on(c.fg_secondary) > on(c.fg_muted));
        assert!(on(c.fg_muted) >= on(c.fg_subtle));
    }

    #[test]
    fn a_light_themes_state_colours_darken_into_words_and_stay_vivid_as_fills() {
        let theme = by_id("ayu-light").unwrap();
        let c = &theme.colors;
        assert_ne!(c.state_working_fill, c.state_working);
        let green = theme.terminal.ansi[2];
        assert!(
            contrast_ratio(Rgb::from(c.state_working), Rgb::WHITE)
                > contrast_ratio(Rgb::from(green), Rgb::WHITE)
        );
    }

    #[test]
    fn a_card_sits_on_the_page_the_way_the_appearance_wants_it() {
        let dark = &by_id("graphite").unwrap().colors;
        assert_eq!(dark.card, dark.raised);
        let light = &by_id("github-light").unwrap().colors;
        assert_eq!(light.card, light.bg);
    }

    #[test]
    fn the_agent_states_resolve_to_their_tokens() {
        let c = &default_theme().colors;
        assert_eq!(c.state(AgentState::Working).text, c.state_working);
        assert_eq!(c.state(AgentState::Waiting).fill, c.state_waiting_fill);
        assert_eq!(c.state(AgentState::Yours).text, c.state_yours);
        assert_eq!(c.state(AgentState::Idle).fill, c.state_idle_fill);
        assert_eq!(AgentState::ALL.len(), 4);
    }
}

mod the_statistics_colours {
    use super::*;

    #[test]
    fn step_away_from_the_card_in_order_on_every_theme_the_faintest_still_at_2_to_1() {
        for theme in themes() {
            let c = &theme.colors;
            let card = Rgb::from(c.card);
            let steps = [c.viz_1, c.viz_2, c.viz_3, c.viz_4].map(Rgb::from);
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
            let c = &theme.colors;
            let card = Rgb::from(c.card);
            let levels = [
                c.viz_heat_1,
                c.viz_heat_2,
                c.viz_heat_3,
                c.viz_heat_4,
                c.viz_heat_5,
            ];
            let away = |color: Color| {
                (perceived_lightness(Rgb::from(color)) - perceived_lightness(card)).abs()
            };
            let mut previous = away(c.viz_heat_0);
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
        assert_eq!(search.active_border, theme.colors.accent);
    }
}
