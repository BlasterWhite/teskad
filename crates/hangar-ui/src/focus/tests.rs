use hangar_theme::default_theme;
use iced::keyboard::key::Named;
use iced::widget::{button, text};

use super::*;
use crate::testing::Keyboard;

#[derive(Debug, Clone, PartialEq)]
enum Msg {
    Save,
    Other,
}

fn two<'a>(first: Option<Msg>, second: Option<Msg>) -> Element<'a, Msg> {
    let t = default_theme();
    let one = |label: &'static str, message: Option<Msg>| {
        ring(t, button(text(label)).on_press_maybe(message.clone())).on_activate_maybe(message)
    };
    iced::widget::row![one("one", first), one("two", second)].into()
}

#[test]
fn a_control_nobody_focused_ignores_the_keys() {
    let mut ui = Keyboard::new(two(Some(Msg::Save), Some(Msg::Other)));
    ui.key(Named::Enter);
    assert!(ui.into_messages().is_empty());
}

#[test]
fn tab_focuses_the_first_and_enter_and_space_press_it() {
    let mut ui = Keyboard::new(two(Some(Msg::Save), Some(Msg::Other)));
    ui.focus_next();
    ui.key(Named::Enter);
    ui.key(Named::Space);
    assert_eq!(ui.into_messages(), vec![Msg::Save, Msg::Save]);
}

#[test]
fn tab_goes_on_to_the_next_and_shift_tab_comes_back() {
    let mut ui = Keyboard::new(two(Some(Msg::Save), Some(Msg::Other)));
    ui.focus_next();
    ui.focus_next();
    ui.key(Named::Enter);
    ui.focus_previous();
    ui.key(Named::Enter);
    assert_eq!(ui.into_messages(), vec![Msg::Other, Msg::Save]);
}

#[test]
fn a_disabled_control_is_skipped() {
    let mut ui = Keyboard::new(two(None, Some(Msg::Other)));
    ui.focus_next();
    ui.key(Named::Enter);
    assert_eq!(ui.into_messages(), vec![Msg::Other]);
}

#[test]
fn a_press_of_the_pointer_takes_the_ring_off() {
    let mut ui = Keyboard::new(two(Some(Msg::Save), Some(Msg::Other)));
    ui.focus_next();
    ui.press_pointer();
    ui.key(Named::Enter);
    assert!(ui.into_messages().is_empty());
}
