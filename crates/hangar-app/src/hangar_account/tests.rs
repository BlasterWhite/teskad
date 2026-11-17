//! The Hangar account card: its states, the sync switch (off by default), the
//! machines list, the sentences for errors, and the preference file.

use std::borrow::Cow;

use hangar_core::cloud::{Account, Device, Provider, StatsResponse, StatsRow};
use hangar_i18n::{Locale, Translator};
use iced::{Settings as WindowSettings, Size};
use iced_test::Simulator;

use super::view::{card, describe};
use super::*;
use crate::app::Packages;
use crate::ctx::{Ctx, Metrics};

fn account() -> Account {
    Account {
        id: "acc_1".into(),
        display_name: "Ada".into(),
        email: Some("ada@example.org".into()),
        avatar_url: None,
        providers: vec![Provider::Github],
    }
}

fn device(id: &str, name: &str, os: &str) -> Device {
    Device {
        id: id.into(),
        name: name.into(),
        os: os.into(),
        created_at: 1_794_000_000_000,
        last_seen_at: 1_794_000_600_000,
        revoked_at: None,
    }
}

fn signed_in() -> State {
    let mut st = State::signed_in_for_test(account(), device("d1", "Atelier", "windows"));
    st.machines = Some(vec![
        device("d1", "Atelier", "windows"),
        device("d2", "Portable", "macos"),
    ]);
    st
}

/// Whether the card, drawn for `st`, shows each of `words` (and none of
/// `absent`), in `locale`.
fn shows(st: State, locale: Locale, words: &[&str], absent: &[&str]) {
    let model = Model::default();
    let pkg = Packages {
        hangar_account: st,
        ..Packages::default()
    };
    let ctx = Ctx {
        model: &model,
        pkg: &pkg,
        i18n: Translator::new(locale),
        theme: hangar_theme::by_id_or_default("graphite"),
        metrics: Metrics::default(),
        keymap: &model.live.keymap.index,
        window: iced::window::Id::unique(),
        now: std::time::Instant::now(),
    };
    let window = WindowSettings {
        fonts: vec![Cow::Borrowed(hangar_theme::fonts::INTER_VARIABLE)],
        default_font: hangar_theme::fonts::ui(),
        ..WindowSettings::default()
    };
    let mut ui = Simulator::with_size(window, Size::new(760.0, 1400.0), card(ctx));
    for word in words {
        assert!(ui.find(*word).is_ok(), "{locale:?}: {word:?} is not shown");
    }
    for word in absent {
        assert!(ui.find(*word).is_err(), "{locale:?}: {word:?} is shown");
    }
}

#[test]
fn signed_out_the_card_offers_github_and_nothing_else() {
    let st = State {
        phase: Phase::SignedOut,
        ..State::default()
    };
    shows(
        st,
        Locale::En,
        &["Sign in with GitHub"],
        &["Sync statistics", "Sign out", "Delete my account"],
    );
    let st = State {
        phase: Phase::SignedOut,
        ..State::default()
    };
    shows(
        st,
        Locale::Fr,
        &["Se connecter avec GitHub"],
        &["Se déconnecter"],
    );
}

#[test]
fn waiting_for_the_browser_the_card_says_so_and_can_cancel() {
    let st = State {
        phase: Phase::Waiting,
        ..State::default()
    };
    shows(
        st,
        Locale::En,
        &["Waiting for the browser…", "Cancel"],
        &["Sign in with GitHub"],
    );
}

#[test]
fn signed_in_the_card_has_the_identity_the_switch_the_machines_and_the_two_exits() {
    shows(
        signed_in(),
        Locale::En,
        &[
            "Signed in as Ada",
            "ada@example.org",
            "Sync statistics",
            "What leaves this PC",
            "Your machines",
            "Atelier",
            "Portable",
            "This PC",
            "Sign out",
            "Delete my account",
            "Read the privacy page",
        ],
        // The sync is off: no "Sync now" yet.
        &["Sync now"],
    );
    shows(
        signed_in(),
        Locale::Fr,
        &[
            "Connecté en tant que Ada",
            "Synchroniser les statistiques",
            "Ce qui quitte ce PC",
            "Tes machines",
            "Supprimer mon compte",
        ],
        &[],
    );
}

#[test]
fn the_sync_switch_is_off_until_the_person_turns_it_on() {
    // Nothing signed in: off. Signed in: still off. Only the person's switch
    // turns it on, and signing in never does.
    assert!(!State::default().sync_on());
    let st = signed_in();
    assert!(!st.sync_on());
    assert!(st.all_machines().is_none());

    let mut on = signed_in();
    on.sync_on = true;
    shows(on, Locale::En, &["Sync now", "Not synced yet"], &[]);
}

#[test]
fn every_machine_is_listed_with_its_system_and_this_pc_cannot_be_removed() {
    let mut st = signed_in();
    // Another machine's removal is asked; this one's has no such button.
    st.removing = Some("d2".into());
    shows(
        st,
        Locale::En,
        &["Remove Portable from your account?", "Remove machine"],
        &[],
    );
    let renaming = {
        let mut st = signed_in();
        st.renaming = Some(("d1".into(), "Atelier".into()));
        st
    };
    shows(renaming, Locale::En, &["Save"], &[]);
}

#[test]
fn deleting_the_account_asks_for_the_word_to_be_typed() {
    let mut st = signed_in();
    st.deleting = Some(String::new());
    shows(
        st,
        Locale::En,
        &["Type DELETE to confirm", "Delete my account for good"],
        &[],
    );
    assert!(delete_word_typed(" delete ", "DELETE"));
    assert!(delete_word_typed("supprimer", "SUPPRIMER"));
    assert!(!delete_word_typed("", "DELETE"));
    assert!(!delete_word_typed("del", "DELETE"));
}

#[test]
fn the_merged_statistics_show_only_while_signed_in_with_the_sync_on() {
    let pulled = Arc::new(Pulled {
        answer: StatsResponse {
            devices: vec![device("d1", "Atelier", "windows")],
            rows: vec![hangar_core::cloud::DeviceStatsRow {
                device_id: "d1".into(),
                row: StatsRow {
                    day: "2026-11-16".into(),
                    model: "m".into(),
                    project: "hangar".into(),
                    input_tokens: 1,
                    output_tokens: 2,
                    cache_read_tokens: 0,
                    cache_write_tokens: 0,
                    cost_usd_micros: 3,
                    active_seconds: 4,
                    sessions: 1,
                },
            }],
        },
        at: 1,
        this: Some("d1".into()),
    });
    let mut st = signed_in();
    st.pulled = Some(pulled);
    assert!(st.all_machines().is_none(), "the sync is off");
    st.sync_on = true;
    assert!(st.all_machines().is_some());
    st.phase = Phase::SignedOut;
    assert!(st.all_machines().is_none(), "signed out");
}

#[test]
fn a_token_the_server_forgot_signs_the_card_out_and_turns_the_sync_off() {
    let mut st = signed_in();
    st.sync_on = true;
    fail(&mut st, Error::SignedOut);
    assert!(!st.signed_in());
    assert!(!st.sync_on());
    assert!(st.machines.is_none());
    // A server that is merely down changes nothing about who is signed in.
    let mut st = signed_in();
    fail(&mut st, Error::Unreachable("refused".into()));
    assert!(st.signed_in());
}

#[test]
fn errors_are_said_in_plain_words_in_both_languages_and_never_carry_a_token() {
    let en = Translator::new(Locale::En);
    let fr = Translator::new(Locale::Fr);
    let unreachable = Error::Unreachable("tcp connect error: hgr_secret".into());
    assert!(
        describe(en, &unreachable)
            .unwrap()
            .contains("could not be reached")
    );
    assert!(
        describe(fr, &unreachable)
            .unwrap()
            .contains("Impossible de joindre")
    );
    // What the network said stays out of the sentence.
    assert!(!describe(en, &unreachable).unwrap().contains("hgr_"));
    assert!(
        describe(en, &Error::SignedOut)
            .unwrap()
            .contains("signed out")
    );
    assert!(
        describe(fr, &Error::SignedOut)
            .unwrap()
            .contains("déconnecté")
    );
    assert!(describe(en, &Error::TimedOut).is_some());
    assert_eq!(describe(en, &Error::Cancelled), None);
}

#[test]
fn the_sync_preference_is_a_small_file_that_defaults_to_off() {
    let dir = std::env::temp_dir().join(format!("hangar-account-prefs-{}", std::process::id()));
    let path = dir.join(PREFS_FILE);
    assert!(!read_flag_at(&path), "no file: off");
    write_flag_at(&path, true).unwrap();
    assert!(read_flag_at(&path));
    write_flag_at(&path, false).unwrap();
    assert!(!read_flag_at(&path));
    std::fs::write(&path, "not json").unwrap();
    assert!(!read_flag_at(&path), "a damaged file: off");
    let _ = std::fs::remove_dir_all(dir);
}
