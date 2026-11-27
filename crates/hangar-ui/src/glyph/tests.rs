use super::*;

mod gauge_level {
    use super::*;

    #[test]
    fn is_amber_from_70_and_red_from_90() {
        assert_eq!(gauge_level(0), GaugeLevel::Ok);
        assert_eq!(gauge_level(69), GaugeLevel::Ok);
        assert_eq!(gauge_level(70), GaugeLevel::Warn);
        assert_eq!(gauge_level(89), GaugeLevel::Warn);
        assert_eq!(gauge_level(90), GaugeLevel::Hot);
        assert_eq!(gauge_level(100), GaugeLevel::Hot);
    }
}

mod spin {
    use super::super::spin_angle;
    use std::f32::consts::TAU;

    #[test]
    fn turns_once_per_turn_in_a_straight_line() {
        assert_eq!(spin_angle(0.0), 0.0);
        assert!((spin_angle(0.25) - TAU / 4.0).abs() < 1e-6);
        assert!((spin_angle(0.5) - TAU / 2.0).abs() < 1e-6);
    }
}

mod reduced_motion {
    use super::*;

    #[test]
    fn is_a_switch_the_app_sets_from_the_system() {
        set_reduced_motion(true);
        assert!(reduced_motion());
        set_reduced_motion(false);
        assert!(!reduced_motion());
    }
}

mod animation {
    use super::*;

    #[test]
    fn only_a_working_or_starting_glyph_moves() {
        for state in [
            GlyphState::Waiting,
            GlyphState::Yours,
            GlyphState::Idle,
            GlyphState::Exited,
            GlyphState::Completed,
            GlyphState::Failed,
        ] {
            assert!(!state.animates(), "{state:?}");
        }
        assert!(GlyphState::Working.animates());
        assert!(GlyphState::Starting.animates());
    }

    #[test]
    fn a_starting_ring_is_lit_for_half_of_its_blink() {
        assert!(blink_lit(0.0));
        assert!(blink_lit(0.49));
        assert!(!blink_lit(0.5));
        assert!(!blink_lit(0.99));
    }

    #[test]
    fn the_app_states_map_one_for_one() {
        assert_eq!(GlyphState::from(AgentState::Working), GlyphState::Working);
        assert_eq!(GlyphState::from(AgentState::Waiting), GlyphState::Waiting);
        assert_eq!(GlyphState::from(AgentState::Yours), GlyphState::Yours);
        assert_eq!(GlyphState::from(AgentState::Idle), GlyphState::Idle);
    }
}

#[test]
fn a_glyph_takes_its_fill_from_the_themes_state_colours() {
    let t = hangar_theme::default_theme();
    assert_eq!(Tint::of(t, GlyphState::Working).fill, t.inks.running.fill);
    assert_eq!(Tint::of(t, GlyphState::Waiting).fill, t.inks.waiting.fill);
    assert_eq!(Tint::of(t, GlyphState::Exited).fill, t.inks.idle.fill);
}

#[test]
fn an_outcome_takes_the_themes_success_and_danger_and_draws_its_symbol_in_the_background() {
    let t = hangar_theme::default_theme();
    let done = Tint::of(t, GlyphState::Completed);
    assert_eq!(done.fill, t.inks.success());
    assert_eq!(done.ink, t.palette.bg);
    assert_eq!(Tint::of(t, GlyphState::Failed).fill, t.inks.danger());
}
