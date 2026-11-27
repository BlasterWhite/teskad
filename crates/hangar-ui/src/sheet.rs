//! The component sheet: every shared control in each of its states, on one
//! page, in whichever theme it is given.
//!
//! The gallery example shows it as its "Controls" page (`cargo run -p
//! hangar-ui --example gallery`, or `-- --snapshot --page controls --theme
//! teskad-dark`), and the tests render it in Teskad Dark and Teskad Light.
//!
//! A state that only the pointer or the keyboard brings about (hover,
//! pressed, focus) is drawn by asking the control for it (`shown_as`), so
//! that all of them are on the page at once.

use hangar_theme::{Theme, fonts, style};
use iced::widget::button::Status;
use iced::widget::{column, container, pick_list, row, slider as iced_slider, text_input};
use iced::{Alignment, Element, Length};

use crate::badge::{self, Role};
use crate::button::{self, Side};
use crate::focus;
use crate::form;
use crate::icon::Icon;
use crate::input::{self, Segment};
use crate::keycap;
use crate::meter;
use crate::text as typography;

type View<'a> = Element<'a, ()>;
type MakeButton = for<'a> fn(&'a Theme, String) -> button::Btn<'a, ()>;

/// The states of a button, in the order of the page.
const STATES: [(&str, Status, bool); 5] = [
    ("Normal", Status::Active, false),
    ("Hover", Status::Hovered, false),
    ("Pressed", Status::Pressed, false),
    ("Focus", Status::Active, true),
    ("Disabled", Status::Disabled, false),
];

/// The page.
pub fn view(t: &Theme) -> View<'_> {
    let page = column![
        section(t, "Buttons", buttons(t)),
        section(t, "Icon buttons: 26, 28, 32 px", icon_buttons(t)),
        section(t, "Segmented control", segments(t)),
        section(
            t,
            "Switch",
            column![switches(t), switches_more(t)].spacing(14).into()
        ),
        section(t, "Slider", sliders(t)),
        section(t, "Fields and drop-down", fields(t)),
        section(t, "Settings block", block(t)),
        section(t, "Badges and keys", badges(t)),
        section(t, "Gauges", gauges(t)),
    ]
    .spacing(24)
    .padding(24)
    .width(Length::Fill);
    container(page)
        .style(style::container::page(t))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn section<'a>(t: &'a Theme, title: &'static str, body: View<'a>) -> View<'a> {
    column![typography::label(t, title), body]
        .spacing(10)
        .into()
}

fn caption<'a>(t: &'a Theme, words: &'static str) -> View<'a> {
    container(typography::hint(t, words)).width(72).into()
}

fn line<'a>(items: Vec<View<'a>>) -> View<'a> {
    row(items).spacing(16).align_y(Alignment::Center).into()
}

fn buttons(t: &Theme) -> View<'_> {
    let kinds: [(&str, MakeButton); 4] = [
        ("Primary", |t, l| button::primary(t, l)),
        ("Secondary", |t, l| button::secondary(t, l)),
        ("Text", |t, l| button::text_button(t, l)),
        ("Danger", |t, l| button::danger(t, l)),
    ];
    let mut rows = column![].spacing(14);
    for (name, make) in kinds {
        let mut items = vec![container(typography::hint(t, name)).width(72).into()];
        for (state, status, ring) in STATES {
            let built = make(t, state.to_string()).size(button::Size::Large);
            items.push(match status {
                Status::Disabled => built.into(),
                _ => built.on_press(()).shown_as(status, ring).into(),
            });
        }
        rows = rows.push(line(items));
    }
    rows.into()
}

fn icon_buttons(t: &Theme) -> View<'_> {
    let mut rows = column![].spacing(14);
    for (name, side) in [("26", Side::Xs), ("28", Side::Sm), ("32", Side::Md)] {
        let mut items = vec![container(typography::hint(t, name)).width(72).into()];
        for (_, status, ring) in STATES {
            let built = button::icon_button(t, Icon::Gear).sized(side);
            items.push(match status {
                Status::Disabled => built.into(),
                _ => built.on_press(()).shown_as(status, ring).into(),
            });
        }
        // Selected: a toggle that is on.
        items.push(
            button::toggle_button(t, Icon::Gear, true)
                .sized(side)
                .on_press(())
                .into(),
        );
        rows = rows.push(line(items));
    }
    rows.push(line(vec![
        caption(t, "Selected"),
        typography::hint(t, "last of each row: the toggle that is on").into(),
    ]))
    .into()
}

fn segments(t: &Theme) -> View<'_> {
    let options = [("a", "System"), ("b", "English"), ("c", "Français")];
    line(vec![
        caption(t, "Normal"),
        input::segmented(t, &options, &"a", |_| ()),
        caption(t, "Focus"),
        focus::ring(t, input::segmented(t, &options, &"b", |_| ()))
            .radius(hangar_theme::tokens::radius::LG)
            .shown(true)
            .into(),
        caption(t, "Unavailable"),
        input::segmented_icons(
            t,
            vec![
                Segment::new("a", Some(Icon::Terminal), "Local"),
                Segment::new("b", Some(Icon::Cloud), "Remote").unavailable("offline"),
            ],
            &"a",
            |_| (),
        ),
    ])
}

fn switches(t: &Theme) -> View<'_> {
    let one = |on: bool, status: Status, ring: bool, usable: bool| -> View<'_> {
        let built = input::switch(t, on).shown_as(status, ring);
        if usable {
            built.on_toggle(|_| ()).into()
        } else {
            built.into()
        }
    };
    line(vec![
        caption(t, "Off"),
        one(false, Status::Active, false, true),
        caption(t, "On"),
        one(true, Status::Active, false, true),
        caption(t, "Hover"),
        one(true, Status::Hovered, false, true),
        caption(t, "Pressed"),
        one(true, Status::Pressed, false, true),
    ])
}

fn switches_more(t: &Theme) -> View<'_> {
    let one = |on: bool, status: Status, ring: bool, usable: bool| -> View<'_> {
        let built = input::switch(t, on).shown_as(status, ring);
        if usable {
            built.on_toggle(|_| ()).into()
        } else {
            built.into()
        }
    };
    line(vec![
        caption(t, "Focus"),
        one(true, Status::Active, true, true),
        caption(t, "Disabled"),
        one(false, Status::Active, false, false),
        one(true, Status::Active, false, false),
    ])
}

fn sliders(t: &Theme) -> View<'_> {
    use iced::widget::slider::Status as Knob;
    let one = |status: Knob| -> View<'_> {
        container(
            iced_slider(0.0..=100.0, 35.0_f32, |_| ())
                .height(style::slider::HEIGHT)
                .style(move |theme: &iced::Theme, _| style::slider::default(t)(theme, status)),
        )
        .width(180)
        .into()
    };
    line(vec![
        caption(t, "Normal"),
        one(Knob::Active),
        caption(t, "Hover"),
        one(Knob::Hovered),
        caption(t, "Dragged"),
        one(Knob::Dragged),
        input::slider(t, 0.0..=1000.0, 700.0, |_| ())
            .display("700 ms")
            .into(),
    ])
}

fn fields(t: &Theme) -> View<'_> {
    use iced::widget::text_input::Status as Edge;
    let raw = |status: Edge, mono: bool, invalid: bool| -> View<'_> {
        let mut built = text_input("Placeholder", "Value")
            .padding([8.0, 10.0])
            .size(if mono { 12.0 } else { 13.0 })
            .width(130)
            .style(move |theme: &iced::Theme, _| {
                if invalid {
                    style::text_input::field_invalid(t)(theme, status)
                } else {
                    style::text_input::field(t)(theme, status)
                }
            });
        if mono {
            built = built.font(fonts::mono());
        }
        built.into()
    };
    let drop = |status: pick_list::Status| -> View<'_> {
        container(
            pick_list(vec!["zsh", "bash"], Some("zsh"), |_| ())
                .padding([8.0, 10.0])
                .text_size(13.0)
                .style(move |theme: &iced::Theme, _| style::pick_list::field(t)(theme, status)),
        )
        .width(150)
        .into()
    };
    column![
        line(vec![
            caption(t, "Normal"),
            raw(Edge::Active, false, false),
            caption(t, "Hover"),
            raw(Edge::Hovered, false, false),
            caption(t, "Focus"),
            raw(Edge::Focused { is_hovered: false }, false, false),
            caption(t, "Error"),
            raw(Edge::Active, false, true),
            caption(t, "Disabled"),
            raw(Edge::Disabled, false, false),
        ]),
        line(vec![
            caption(t, "Mono"),
            raw(Edge::Active, true, false),
            caption(t, "Drop-down"),
            drop(pick_list::Status::Active),
            drop(pick_list::Status::Hovered),
            drop(pick_list::Status::Opened { is_hovered: false }),
        ]),
        input::text_field(t, "")
            .label("With its label")
            .hint("And a hint under it.")
            .placeholder("Next to the workspace")
            .mono()
            .on_input(|_| ())
            .width(Length::Fixed(300.0)),
    ]
    .spacing(14)
    .into()
}

fn block(t: &Theme) -> View<'_> {
    form::group(
        t,
        vec![
            form::row(t, "Auto resume", input::switch(t, true).on_toggle(|_| ()))
                .hint("Relaunches agents on their last session.")
                .into(),
            form::row(
                t,
                "Interface",
                input::segmented(
                    t,
                    &[("a", "System"), ("b", "English"), ("c", "Français")],
                    &"a",
                    |_| (),
                ),
            )
            .into(),
            form::row(
                t,
                "Launch delay",
                input::slider(t, 0.0..=1000.0, 700.0, |_| ()).display("700 ms"),
            )
            .hint("Leaves the shell time to load its profile.")
            .into(),
        ],
    )
}

fn badges(t: &Theme) -> View<'_> {
    line(vec![
        badge::role(t, Role::Orchestrator, "Orchestrator"),
        badge::role(t, Role::Worker, "Worker"),
        badge::account(t, "Main", false),
        badge::account(t, "Main", true),
        badge::beta(t, "BETA"),
        badge::count(t, 12),
        keycap::binding(t, "Ctrl+Shift+B").into(),
        keycap::binding(t, "Ctrl+Shift+B").conflict().into(),
    ])
}

fn gauges(t: &Theme) -> View<'_> {
    line(vec![
        meter::context(t, 40).into(),
        meter::context(t, 69).into(),
        meter::context(t, 75).into(),
        meter::context(t, 95).into(),
        container(column![meter::plan_row(t, "5 h", 20), meter::plan_row(t, "7 d", 47)].spacing(8))
            .width(260)
            .into(),
    ])
}

#[cfg(test)]
mod tests;
