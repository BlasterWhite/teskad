//! The Discord card's own logic: the elapsed time and how the link reads.

use super::*;
use crate::settings::discord::build_presence;
use crate::state::types::Settings;
use crate::workspace_dialog::testing::Harness;
use hangar_i18n::{Locale, Translator};

#[test]
fn the_elapsed_time_counts_hours_minutes_and_seconds() {
    assert_eq!(elapsed_since(0, 0), "00:00:00");
    assert_eq!(elapsed_since(0, 59_999), "00:00:59");
    assert_eq!(elapsed_since(0, 61_000), "00:01:01");
    assert_eq!(elapsed_since(1_000, 3_661_000), "01:01:00");
    assert_eq!(elapsed_since(0, 100 * 3600 * 1000), "100:00:00");
}

#[test]
fn a_clock_that_stepped_back_does_not_show_a_negative_time() {
    assert_eq!(elapsed_since(10_000, 5_000), "00:00:00");
}

#[test]
fn the_link_reads_off_connected_or_waiting() {
    assert_eq!(tone(false, false), Tone::Off);
    // Off wins over a connection that has not closed yet.
    assert_eq!(tone(false, true), Tone::Off);
    assert_eq!(tone(true, true), Tone::On);
    assert_eq!(tone(true, false), Tone::Waiting);
}

#[test]
fn the_logo_is_bundled_and_decodes() {
    // The card shows the repository's own file, so a missing or broken one
    // would be a card with a hole in it.
    let bytes = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/logo.png"
    ));
    assert!(bytes.starts_with(b"\x89PNG"));
}

#[test]
fn without_a_usable_application_id_there_is_a_card_and_nothing_to_publish() {
    for id in ["", "1534592584045957282"] {
        let settings = Settings {
            discord_presence: true,
            discord_app_id: id.into(),
            ..Settings::default()
        };
        let input = PresenceInput {
            settings: &settings,
            workspace: None,
            focused_pane: None,
            t: Translator::new(Locale::En),
            started_at: 0,
        };
        let card = card_content(&input);
        assert_eq!(card.details, "No workspace open", "{id:?}");
        assert_eq!(card.app_id, "", "{id:?}");
        assert_eq!(build_presence(&input), None, "{id:?}");
    }
}

#[test]
fn the_preview_is_drawn_by_default_and_says_an_application_id_is_needed() {
    let h = Harness::new(Locale::En, "graphite");
    let st = State::default();
    let mut ui = h.show(view(&st, h.ctx(), 0));
    assert!(ui.find("Teskad").is_ok(), "the card");
    assert!(ui.find("View on GitHub").is_ok(), "its button");
    let note = Translator::new(Locale::En).t(Key::DiscordStatusNeedsAppId);
    assert!(ui.find(note.as_ref()).is_ok(), "{note}");
}

#[test]
fn a_missing_application_id_reads_off_and_not_waiting() {
    for enabled in [false, true] {
        assert_eq!(
            status(false, enabled, false),
            (Tone::Off, Key::DiscordStatusNeedsAppId)
        );
    }
    assert_eq!(
        status(true, false, false),
        (Tone::Off, Key::DiscordStatusOff)
    );
    assert_eq!(
        status(true, true, false),
        (Tone::Waiting, Key::DiscordStatusWaiting)
    );
    assert_eq!(
        status(true, true, true),
        (Tone::On, Key::DiscordStatusConnected)
    );
}
