use hangar_theme::default_theme;
use iced::keyboard::key::Named;
use iced::widget::{column, container, space};
use iced::{Element, Length};

use super::*;
use crate::button::secondary;
use crate::input::text_field;
use crate::testing::{Drive, simulate};

#[derive(Debug, Clone, PartialEq)]
enum Msg {
    Typed(String),
    Copy,
    Dismiss,
}

fn anchor<'a>() -> Element<'a, Msg> {
    container(space())
        .width(Length::Fixed(80.0))
        .height(Length::Fixed(26.0))
        .into()
}

fn details<'a>() -> Popover<'a, Msg> {
    let t = default_theme();
    popover(
        t,
        column![
            row(t, "Shell", text_field(t, "").on_input(Msg::Typed)),
            row(t, "Session", secondary(t, "Copy").on_press(Msg::Copy)),
        ]
        .spacing(8),
    )
    .width(WIDTH)
}

fn open<'a>() -> Element<'a, Msg> {
    under(anchor(), Some(details()))
        .on_dismiss(Msg::Dismiss)
        .into()
}

fn sent(ui: iced_test::Simulator<'_, Msg>) -> Vec<Msg> {
    ui.into_messages().collect()
}

#[test]
fn the_first_control_has_the_keyboard_when_it_opens() {
    let mut ui = simulate(open());
    ui.typewrite("a");
    assert_eq!(sent(ui), [Msg::Typed("a".into())]);
}

#[test]
fn tab_stays_inside_and_reaches_the_next_control() {
    let mut ui = simulate(open());
    ui.key(Named::Tab);
    ui.key(Named::Enter);
    assert_eq!(sent(ui), [Msg::Copy]);
}

#[test]
fn escape_asks_to_close_it() {
    let mut ui = simulate(open());
    ui.key(Named::Escape);
    assert_eq!(sent(ui), [Msg::Dismiss]);
}

#[test]
fn a_closed_popover_shows_only_its_anchor() {
    let mut ui = simulate(Element::from(
        under(anchor(), None).on_dismiss(Msg::Dismiss),
    ));
    ui.key(Named::Escape);
    assert_eq!(sent(ui), []);
}
