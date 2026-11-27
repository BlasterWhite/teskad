//! The style functions against the iced widgets they are for.

use iced::widget::button::Status;
use iced::widget::{
    button, checkbox, container, pick_list, progress_bar, scrollable, slider, text, text_editor,
    text_input, toggler, tooltip,
};
use iced::{Background, Color, Element, Shadow};

use super::Tone;
use super::toggler as toggler_style;
use super::{brightness, faded};
use super::{button as button_style, checkbox as checkbox_style, container as container_style};
use super::{pick_list as pick_list_style, progress_bar as progress_bar_style, rule};
use super::{scrollable as scrollable_style, text as text_style, text_input as text_input_style};
use super::{slider as slider_style, text_editor as text_editor_style};
use crate::{Rgb, TEXT_CONTRAST, contrast_ratio, default_theme, themes};

/// Every style function has to fit the `.style(..)` of the widget it is for;
/// a signature iced would not take fails to compile here.
#[test]
fn the_functions_plug_into_the_widgets_they_are_for() {
    let t = default_theme();
    let _: Element<'_, ()> = button("x").style(button_style::primary(t)).into();
    let _: Element<'_, ()> = button("x").style(button_style::tab(t, true)).into();
    let _: Element<'_, ()> = container(text("x").style(text_style::muted(t)))
        .style(container_style::card(t))
        .into();
    let _: Element<'_, ()> = text_input("", "").style(text_input_style::field(t)).into();
    let _: Element<'_, u8> = pick_list(vec![1_u8, 2], Some(1_u8), |picked| picked)
        .style(pick_list_style::field(t))
        .menu_style(pick_list_style::menu(t))
        .into();
    let _: Element<'_, ()> = scrollable(text("x"))
        .style(scrollable_style::default(t))
        .into();
    let _: Element<'_, ()> = tooltip(text("x"), text("y"), tooltip::Position::Top)
        .style(container_style::tooltip(t))
        .into();
    let _: Element<'_, ()> = checkbox(true).style(checkbox_style::default(t)).into();
    let _: Element<'_, ()> = toggler(true).style(toggler_style::default(t)).into();
    let _: Element<'_, ()> = progress_bar(0.0..=1.0, 0.5)
        .style(progress_bar_style::accent(t))
        .into();
    let _: Element<'_, ()> = iced::widget::rule::horizontal(1).style(rule::sep(t)).into();
    let _: Element<'_, u8> = slider(0..=10, 5_u8, |value| value)
        .style(slider_style::default(t))
        .into();
    let content = text_editor::Content::new();
    let _: Element<'_, ()> = text_editor(&content)
        .style(text_editor_style::field(t))
        .into();
    let _: Element<'_, ()> = button("x").style(button_style::tab_strip(t, true)).into();
    let _: Element<'_, ()> = button("x").style(button_style::toggle(t, true)).into();
    let _: Element<'_, ()> = button("x").style(button_style::plain(t)).into();
    let _: Element<'_, ()> = button("x").style(button_style::palette_row(t, true)).into();
    let _: Element<'_, ()> = container("x").style(container_style::kbd(t)).into();
    let _: Element<'_, ()> = container("x")
        .style(container_style::toast(t, Tone::Danger))
        .into();
}

#[test]
fn a_primary_buttons_label_stays_readable_in_every_state_on_every_theme() {
    for theme in themes() {
        let style = button_style::primary(theme);
        for status in [Status::Active, Status::Hovered, Status::Pressed] {
            let look = style(&iced::Theme::Dark, status);
            let Some(Background::Color(background)) = look.background else {
                panic!("{}: a primary button without a fill", theme.id);
            };
            let ratio = contrast_ratio(Rgb::from(look.text_color), Rgb::from(background));
            // A hover or a press moves the accent a step and costs a little of
            // the 4.5:1 the resting state has: half a point at most. The two
            // Teskad themes use the pack's own hover and pressed fills, which
            // keep white at 3.6:1 and up, the 3:1 of a large label.
            let least = if theme.id.starts_with("teskad-") {
                3.0
            } else {
                TEXT_CONTRAST - 0.5
            };
            assert!(ratio >= least, "{}: {status:?} {ratio:.2}", theme.id);
        }
    }
}

#[test]
fn a_disabled_button_fades_and_loses_its_shadow() {
    let t = default_theme();
    let style = button_style::primary(t);
    let at_rest = style(&iced::Theme::Dark, Status::Active);
    let disabled = style(&iced::Theme::Dark, Status::Disabled);
    assert!(disabled.text_color.a < at_rest.text_color.a);
    assert_eq!(disabled.shadow, Shadow::default());
}

#[test]
fn the_menu_line_under_the_pointer_is_lit_not_filled_and_keeps_its_text() {
    let t = default_theme();
    let rest = button_style::menu_item(t)(&iced::Theme::Dark, Status::Active);
    let hovered = button_style::menu_item(t)(&iced::Theme::Dark, Status::Hovered);
    let pressed = button_style::menu_item(t)(&iced::Theme::Dark, Status::Pressed);
    assert_eq!(rest.background, None);
    let lit = |s: &iced::widget::button::Style| match s.background {
        Some(Background::Color(c)) => c.a,
        _ => 0.0,
    };
    assert!(lit(&hovered) > 0.0 && lit(&pressed) > lit(&hovered));
    assert_eq!(hovered.text_color, t.palette.text);
    let danger = button_style::menu_item_danger(t)(&iced::Theme::Dark, Status::Hovered);
    assert_eq!(danger.text_color, t.inks.danger());
}

#[test]
fn floating_surfaces_take_their_colours_from_the_palette() {
    for t in crate::themes() {
        let dark = &iced::Theme::Dark;
        let pop = container_style::popover(t)(dark);
        assert_eq!(
            pop.background,
            Some(Background::Color(t.palette.surface_raised))
        );
        assert_eq!(pop.border.color, t.palette.border);
        assert_eq!(pop.border.radius, 8.0.into());
        let modal = container_style::modal(t)(dark);
        assert_eq!(modal.background, Some(Background::Color(t.palette.surface)));
        assert_eq!(modal.border.color, t.palette.border_strong);
        assert_eq!(modal.border.radius, 12.0.into());
        assert_eq!(
            container_style::scrim(t)(dark).background,
            Some(Background::Color(t.palette.scrim))
        );
    }
}

#[test]
fn the_chosen_tab_is_raised_and_the_others_are_not() {
    let t = default_theme();
    let chosen = button_style::tab(t, true)(&iced::Theme::Dark, Status::Active);
    let other = button_style::tab(t, false)(&iced::Theme::Dark, Status::Active);
    assert_eq!(chosen.background, Some(Background::Color(t.palette.border)));
    assert_eq!(other.background, None);
}

#[test]
fn a_focused_field_takes_the_focus_ring_for_its_edge() {
    let t = default_theme();
    let focused = text_input_style::field(t)(
        &iced::Theme::Dark,
        text_input::Status::Focused { is_hovered: false },
    );
    assert_eq!(focused.border.color, t.palette.focus_ring);
    assert_eq!(focused.border.width, crate::tokens::FOCUS_RING_WIDTH);
}

#[test]
fn brightness_scales_the_channels_and_stops_at_white() {
    let lighter = brightness(Color::from_rgb(0.5, 0.9, 0.2), 1.08);
    assert!((lighter.r - 0.54).abs() < 1e-6);
    assert!((lighter.g - 0.972).abs() < 1e-6);
    assert_eq!(brightness(Color::WHITE, 2.0), Color::WHITE);
}

#[test]
fn a_disabled_colour_keeps_45_percent_of_its_alpha() {
    assert!((faded(Color::from_rgba(1.0, 1.0, 1.0, 0.5)).a - 0.225).abs() < 1e-6);
}

#[test]
fn an_invalid_field_keeps_the_danger_edge_even_while_it_has_the_keyboard() {
    let t = default_theme();
    for status in [
        text_input::Status::Active,
        text_input::Status::Hovered,
        text_input::Status::Focused { is_hovered: false },
    ] {
        let look = text_input_style::field_invalid(t)(&iced::Theme::Dark, status);
        assert_eq!(look.border.color, t.inks.danger(), "{status:?}");
    }
}

#[test]
fn the_palette_row_is_the_accent_only_when_chosen() {
    let t = default_theme();
    let chosen = button_style::palette_row(t, true)(&iced::Theme::Dark, Status::Active);
    let other = button_style::palette_row(t, false)(&iced::Theme::Dark, Status::Hovered);
    assert_eq!(chosen.background, Some(Background::Color(t.palette.accent)));
    assert_eq!(chosen.text_color, t.palette.on_accent);
    assert_eq!(other.background, None);
}

#[test]
fn the_slider_knob_stands_out_from_its_track_in_every_theme() {
    for theme in themes() {
        let look = slider_style::default(theme)(&iced::Theme::Dark, slider::Status::Active);
        let Background::Color(knob) = look.handle.background else {
            panic!("a colour")
        };
        let track = Rgb::from(theme.palette.border);
        assert!(
            contrast_ratio(Rgb::from(knob), track) >= 1.5,
            "{}: the knob is lost on the track",
            theme.id
        );
        assert_eq!(
            look.rail.backgrounds.0,
            Background::Color(theme.palette.accent)
        );
    }
}

#[test]
fn a_front_tab_takes_the_panels_surface_in_every_state() {
    let t = default_theme();
    for status in [Status::Active, Status::Hovered, Status::Pressed] {
        let look = button_style::tab_strip(t, true)(&iced::Theme::Dark, status);
        assert_eq!(look.background, Some(Background::Color(t.palette.surface)));
    }
}

#[test]
fn on_a_solid_theme_what_lies_on_the_desktop_is_drawn_as_before() {
    for theme in themes() {
        let dark = &iced::Theme::Dark;
        assert_eq!(
            container_style::backdrop(theme)(dark),
            container_style::window(theme)(dark),
            "{}",
            theme.id
        );
        assert_eq!(
            container_style::page(theme)(dark),
            container_style::window(theme)(dark),
            "{}",
            theme.id
        );
        assert_eq!(
            container_style::see_through(theme, container_style::sidebar(theme))(dark),
            container_style::sidebar(theme)(dark),
            "{}",
            theme.id
        );
        assert_eq!(
            rule::sep_see_through(theme)(dark),
            rule::sep(theme)(dark),
            "{}",
            theme.id
        );
    }
}

#[test]
fn on_a_translucent_window_the_desktop_shows_through_the_page_and_the_bars_only() {
    let theme = default_theme().with_window_opacity(0.9);
    if !theme.is_translucent() {
        // Not on Linux: every window is solid there.
        assert_eq!(theme.window_alpha(), 1.0);
        return;
    }
    let dark = &iced::Theme::Dark;
    let alpha = theme.window_alpha();
    assert!(alpha < 1.0);
    let fill_of = |style: container::Style| match style.background {
        Some(Background::Color(color)) => Some(color),
        _ => None,
    };
    // The page under everything, and the bars, at the window's alpha.
    let backdrop = fill_of(container_style::backdrop(theme)(dark)).unwrap();
    assert_eq!(backdrop, theme.palette.bg.scale_alpha(alpha));
    let bar = fill_of(container_style::see_through(
        theme,
        container_style::toolbar(theme),
    )(dark));
    assert_eq!(bar, Some(theme.palette.surface_raised.scale_alpha(alpha)));
    assert_eq!(rule::sep_see_through(theme)(dark).color.a, alpha);
    // A pane on the backdrop adds nothing to it, or the two coats would make
    // it opaque; its text keeps the theme's colour.
    let page = container_style::page(theme)(dark);
    assert_eq!(fill_of(page), None);
    assert_eq!(page.text_color, Some(theme.colors.fg));
    // What floats stays opaque.
    for style in [
        container_style::card(theme)(dark),
        container_style::popover(theme)(dark),
        container_style::modal(theme)(dark),
        container_style::tooltip(theme)(dark),
        container_style::window(theme)(dark),
    ] {
        assert_eq!(fill_of(style).map(|color| color.a), Some(1.0));
    }
}
