use hangar_theme::{by_id, to_iced};
use iced::{Settings, Size};
use iced_test::Simulator;
use std::borrow::Cow;

use super::*;

const PAGE: Size = Size::new(1100.0, 1250.0);

fn simulator<'a>(t: &'a Theme) -> Simulator<'a, ()> {
    let settings = Settings {
        fonts: hangar_theme::fonts::ALL.map(Cow::Borrowed).to_vec(),
        default_font: fonts::ui(),
        ..Settings::default()
    };
    Simulator::with_size(settings, PAGE, view(t))
}

#[test]
fn every_state_is_on_the_sheet_in_both_teskad_themes() {
    for id in ["teskad-dark", "teskad-light"] {
        let t = by_id(id).expect("a bundled theme");
        let mut ui = simulator(t);
        for label in [
            "Normal",
            "Hover",
            "Pressed",
            "Focus",
            "Disabled",
            "Primary",
            "Secondary",
            "Text",
            "Danger",
            "Orchestrator",
            "Worker",
            "BETA",
            "Main",
            "700 ms",
            "5 h",
            "7 d",
            "69%",
        ] {
            assert!(ui.find(label).is_ok(), "{id}: {label}");
        }
    }
}

/// The ring is drawn in the palette's colour: its pixels are on the picture.
#[test]
fn the_focus_ring_is_drawn_in_the_themes_ring_colour() {
    for id in ["teskad-dark", "teskad-light"] {
        let t = by_id(id).expect("a bundled theme");
        let mut ui = simulator(t);
        let snapshot = ui.snapshot(&to_iced(t)).expect("a snapshot");
        let dir = crate::testdir::scratch("sheet");
        snapshot.matches_image(dir.join(id)).expect("written");
        let file = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .next()
            .expect("the png")
            .path();
        let mut reader =
            png::Decoder::new(std::io::BufReader::new(std::fs::File::open(file).unwrap()))
                .read_info()
                .unwrap();
        let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut bytes).unwrap();
        let channels = bytes.len() / (info.width as usize * info.height as usize);
        let p = t.palette.focus_ring;
        let want = [
            (p.r * 255.0).round() as u8,
            (p.g * 255.0).round() as u8,
            (p.b * 255.0).round() as u8,
        ];
        let hits = bytes
            .chunks_exact(channels)
            .filter(|px| px[..3] == want)
            .count();
        // Every control that shows its ring is a 2 px line around a box of
        // dozens of pixels, at twice the size.
        assert!(hits > 2000, "{id}: {hits} ring pixels");
    }
}
