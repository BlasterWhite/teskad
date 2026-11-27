use hangar_theme::default_theme;
use iced::Element;

use super::*;
use crate::testing::simulate;

#[test]
fn the_context_gauge_writes_its_figure() {
    let element: Element<'_, ()> = context(default_theme(), 69).into();
    let mut ui = simulate(element);
    assert!(ui.find("69%").is_ok());
}

#[test]
fn a_plan_row_has_its_name_its_bar_and_its_figure() {
    let element: Element<'_, ()> = plan_row(default_theme(), "5 h", 20);
    let mut ui = simulate(element);
    let name = ui.find("5 h").expect("the name").bounds();
    let figure = ui.find("20%").expect("the figure").bounds();
    assert!(figure.x > name.x + name.width, "the bar lies between them");
}

#[test]
fn a_reading_over_100_is_drawn_full_and_not_past_its_track() {
    let element: Element<'_, ()> = context(default_theme(), 200).into();
    let mut ui = simulate(element);
    assert!(ui.find("200%").is_ok(), "the figure is the reading's");
}
