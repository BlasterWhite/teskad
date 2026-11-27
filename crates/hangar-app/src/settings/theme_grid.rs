//! The grid of themes of Appearance: one card per theme, each with a real
//! picture of the window it gives (the navigator, a pane's header with the
//! three states and the accent, a few lines of terminal), drawn from the
//! theme's own colours, and its name under it. Teskad Dark and Teskad Light
//! come first, pinned; the others follow in the order of the bundle.
//!
//! The card of a chosen theme has a 2 px accent border, a halo and a check.
//! Under System two are chosen, one for each appearance. The grid wraps: four
//! columns where there is room, then three, then two.
//!
//! The grid is a radio group: Tab brings the keyboard to a card, and the arrows
//! move the choice, and the keyboard with it, between the cards. Left and
//! Right go to the card before and after, from the last to the first; Up and
//! Down go to the card above and below, found where the cards are drawn since
//! the number of columns depends on the width. Space and Enter choose the card
//! that has the keyboard, as before.

use hangar_i18n::Key;
use hangar_theme::tokens::{font_size, space};
use hangar_theme::{Theme, fonts, style};
use hangar_ui::{Icon, focus, icon};
use iced::advanced::widget::operation::{Focusable, Outcome};
use iced::advanced::widget::{Id, Operation, operate};
use iced::widget::{Space, button, column, container, row, text, tooltip};
use iced::{
    Alignment, Background, Border, Color, Element, Length, Padding, Rectangle, Shadow, Task, Vector,
};

use super::Msg;
use super::change::{Change, Target};
use crate::ctx::Ctx;
use crate::message::Message;

/// The themes pinned at the top, in this order.
const PINNED: [&str; 2] = ["teskad-dark", "teskad-light"];

/// A card's width: four of them and their gaps fill the page at its usual
/// width, and the row wraps when it is narrower.
const CARD_WIDTH: f32 = 170.0;
const PICTURE_HEIGHT: f32 = 92.0;
/// A card's corner (`ecrans.md` §10).
const CARD_RADIUS: f32 = 10.0;
const GAP: f32 = 10.0;

/// The themes in the order of the grid: the pinned ones, then the rest as
/// they are bundled.
pub fn ordered() -> Vec<&'static Theme> {
    let all = hangar_theme::themes();
    PINNED
        .iter()
        .filter_map(|id| hangar_theme::by_id(id))
        .chain(all.iter().filter(|theme| !PINNED.contains(&theme.id)))
        .collect()
}

/// An arrow key of the keyboard on a card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrow {
    Left,
    Right,
    Up,
    Down,
}

/// The id of a card, for the keyboard to land on it.
fn card_id(theme_id: &str) -> Id {
    Id::from(format!("theme-card-{theme_id}"))
}

/// The card before or after `from` in the order of the grid, from the last to
/// the first and the other way.
fn along(order: &[&'static str], from: &str, forward: bool) -> Option<&'static str> {
    let at = order.iter().position(|id| *id == from)?;
    let to = if forward {
        (at + 1) % order.len()
    } else {
        (at + order.len() - 1) % order.len()
    };
    Some(order[to])
}

/// The card above or below `from` among the drawn `cards`: in the nearest row
/// that way, the one whose middle is the closest to `from`'s.
fn across(cards: &[(&'static str, Rectangle)], from: &str, arrow: Arrow) -> Option<&'static str> {
    let (_, here) = cards.iter().find(|(id, _)| *id == from)?;
    let (cx, cy) = (here.center_x(), here.center_y());
    let half = here.height / 2.0;
    let way = |other: &Rectangle| match arrow {
        Arrow::Down => other.center_y() - cy > half,
        Arrow::Up => cy - other.center_y() > half,
        Arrow::Left | Arrow::Right => false,
    };
    let row_distance = |other: &Rectangle| (other.center_y() - cy).abs();
    let nearest_row = cards
        .iter()
        .filter(|(_, other)| way(other))
        .map(|(_, other)| row_distance(other))
        .min_by(f32::total_cmp)?;
    cards
        .iter()
        .filter(|(_, other)| way(other) && row_distance(other) - nearest_row <= half)
        .min_by(|(_, a), (_, b)| {
            (a.center_x() - cx)
                .abs()
                .total_cmp(&(b.center_x() - cx).abs())
        })
        .map(|(id, _)| *id)
}

/// Asks where the cards are drawn.
struct Where {
    ids: Vec<(Id, &'static str)>,
    found: Vec<(&'static str, Rectangle)>,
}

impl Operation<Vec<(&'static str, Rectangle)>> for Where {
    fn traverse(
        &mut self,
        operate: &mut dyn FnMut(&mut dyn Operation<Vec<(&'static str, Rectangle)>>),
    ) {
        operate(self);
    }

    fn focusable(&mut self, id: Option<&Id>, bounds: Rectangle, _state: &mut dyn Focusable) {
        let theme = id.and_then(|id| self.ids.iter().find(|(card, _)| card == id));
        if let Some((_, theme)) = theme {
            self.found.push((theme, bounds));
        }
    }

    fn finish(&self) -> Outcome<Vec<(&'static str, Rectangle)>> {
        Outcome::Some(self.found.clone())
    }
}

/// What an arrow on the card of `from` does: the choice moves to the card that
/// way, if there is one.
pub fn arrow(from: &'static str, arrow: Arrow) -> Task<Message> {
    let go = |to: &'static str| Message::Settings(Msg::ThemeGo(to));
    match arrow {
        Arrow::Left | Arrow::Right => {
            let order: Vec<&'static str> = ordered().iter().map(|theme| theme.id).collect();
            match along(&order, from, arrow == Arrow::Right) {
                Some(to) => Task::done(go(to)),
                None => Task::none(),
            }
        }
        Arrow::Up | Arrow::Down => {
            let ids = ordered()
                .iter()
                .map(|theme| (card_id(theme.id), theme.id))
                .collect();
            operate(Where {
                ids,
                found: Vec::new(),
            })
            .then(move |cards| match across(&cards, from, arrow) {
                Some(to) => Task::done(go(to)),
                None => Task::none(),
            })
        }
    }
}

/// The keyboard to the card of `theme_id`, once it is the chosen one.
pub fn focus_card(theme_id: &str) -> Task<Message> {
    iced::widget::operation::focus(card_id(theme_id))
}

/// A bar of the picture: `width` long, 3 high.
fn bar<'a>(color: Color, width: f32) -> Element<'a, Message> {
    container(Space::new())
        .width(width)
        .height(3)
        .style(move |_: &iced::Theme| container::Style {
            background: Some(Background::Color(color)),
            border: Border {
                radius: 2.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

/// A round dot of a state.
fn dot<'a>(color: Color) -> Element<'a, Message> {
    container(Space::new())
        .width(5)
        .height(5)
        .style(move |_: &iced::Theme| container::Style {
            background: Some(Background::Color(color)),
            border: Border {
                radius: 3.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

fn filled<'a>(
    content: impl Into<Element<'a, Message>>,
    background: Color,
    border: Option<Color>,
    radius: f32,
) -> container::Container<'a, Message> {
    container(content).style(move |_: &iced::Theme| container::Style {
        background: Some(Background::Color(background)),
        border: Border {
            radius: radius.into(),
            width: if border.is_some() { 1.0 } else { 0.0 },
            color: border.unwrap_or(Color::TRANSPARENT),
        },
        ..container::Style::default()
    })
}

/// A theme drawn as the window it gives, from its own colours: the page, the
/// navigator on the chrome, a pane with its header (the agent's letter, the
/// states working, done and failed, the accent) and the terminal lines.
fn picture<'a>(theme: &'static Theme) -> Element<'a, Message> {
    let ansi = &theme.terminal.ansi;
    let nav_width = CARD_WIDTH * 0.30;
    let nav_bars = nav_width - 16.0;
    let pane_width = CARD_WIDTH - nav_width - 16.0;
    let muted = Color {
        a: theme.inks.text.muted.a * 0.7,
        ..theme.inks.text.muted
    };

    let nav = container(
        column![
            bar(theme.palette.accent, nav_bars * 0.62),
            bar(muted, nav_bars * 0.9),
            bar(muted, nav_bars * 0.72),
            bar(muted, nav_bars * 0.5),
        ]
        .spacing(4),
    )
    .width(nav_width)
    .height(Length::Fill)
    .padding(Padding::from([9.0, 8.0]))
    .style(move |_: &iced::Theme| container::Style {
        background: Some(Background::Color(theme.palette.surface)),
        border: Border {
            width: 0.0,
            ..Border::default()
        },
        ..container::Style::default()
    });

    let header = filled(
        row![
            text("C")
                .size(8)
                .font(fonts::mono())
                .color(theme.inks.text.secondary),
            row![
                dot(theme.inks.running.fill),
                dot(theme.inks.success()),
                dot(theme.inks.danger())
            ]
            .spacing(3),
            Space::new().width(Length::Fill),
            filled(
                Space::new().width(20).height(5),
                theme.palette.accent,
                None,
                3.0
            ),
        ]
        .spacing(5)
        .align_y(Alignment::Center),
        theme.palette.surface,
        Some(theme.palette.border),
        4.0,
    )
    .padding(Padding::from([4.0, 6.0]))
    .width(Length::Fill);

    let lines = filled(
        column![
            bar(theme.terminal.foreground, pane_width * 0.60),
            bar(muted, pane_width * 0.42),
            bar(ansi[2], pane_width * 0.52),
            bar(theme.terminal.cursor, pane_width * 0.30),
        ]
        .spacing(4),
        theme.terminal.background,
        Some(theme.palette.border),
        4.0,
    )
    .padding(Padding::from([7.0, 6.0]))
    .width(Length::Fill)
    .height(Length::Fill);

    let pane = column![header, lines]
        .spacing(4)
        .width(Length::Fill)
        .height(Length::Fill);

    container(row![nav, container(pane).padding(6).width(Length::Fill)])
        .width(Length::Fill)
        .height(PICTURE_HEIGHT)
        .clip(true)
        .style(move |_: &iced::Theme| container::Style {
            background: Some(Background::Color(theme.palette.bg)),
            ..container::Style::default()
        })
        .into()
}

/// One card: the picture, then the name, with its pin and its check.
fn card<'a>(ctx: Ctx<'a>, theme: &'static Theme, chosen: bool) -> Element<'a, Message> {
    let t = ctx.theme;
    let p = &t.palette;
    let pinned = PINNED.contains(&theme.id);

    let mut name_row = row![
        text(theme.name)
            .size(font_size::LABEL)
            .font(if chosen {
                fonts::ui_semibold()
            } else {
                fonts::ui()
            })
            .color(if chosen { p.text } else { p.text_secondary })
            .width(Length::Fill),
    ]
    .spacing(space::SM)
    .align_y(Alignment::Center);
    if pinned {
        name_row = name_row.push(tooltip(
            icon(Icon::Pin, 12.0).color(p.text_muted),
            container(
                text(ctx.t(Key::SettingsThemePinned).into_owned()).size(font_size::CAPTION_LG),
            )
            .padding([space::XS - 1.0, space::MD - 1.0])
            .style(style::container::tooltip(t)),
            tooltip::Position::Top,
        ));
    }
    if chosen {
        name_row = name_row.push(icon(Icon::Check, 14.0).color(p.accent_text));
    }

    let body = column![
        picture(theme),
        container(name_row)
            .padding(Padding::from([space::MD, space::LG]))
            .width(Length::Fill),
    ];
    let (border_color, border_width) = if chosen {
        (p.accent, 2.0)
    } else {
        (p.border, 1.0)
    };
    let (surface, halo) = (p.surface, p.accent_text);
    let face = container(body)
        .width(CARD_WIDTH)
        .clip(true)
        .style(move |_: &iced::Theme| container::Style {
            background: Some(Background::Color(surface)),
            border: Border {
                radius: CARD_RADIUS.into(),
                width: border_width,
                color: border_color,
            },
            shadow: if chosen {
                Shadow {
                    color: Color { a: 0.35, ..halo },
                    offset: Vector::ZERO,
                    blur_radius: 10.0,
                }
            } else {
                Shadow::default()
            },
            ..container::Style::default()
        });

    let pick = Message::Settings(Msg::Change(Change::text(Target::Theme, theme.id)));
    let card = button(face)
        .padding(0)
        .style(style::button::plain(t))
        .on_press(pick.clone());
    let arrow = |arrow| Some(Message::Settings(Msg::ThemeArrow(theme.id, arrow)));
    focus::ring(t, card)
        .radius(CARD_RADIUS)
        .id(card_id(theme.id))
        .on_activate(pick)
        .on_arrows(arrow(Arrow::Left), arrow(Arrow::Right))
        .on_vertical(arrow(Arrow::Up), arrow(Arrow::Down))
        .into()
}

/// The whole grid. `active` are the themes chosen: the window's, or under
/// System the dark list's and the light list's.
pub fn view<'a>(ctx: Ctx<'a>, active: &[String]) -> Element<'a, Message> {
    let cards = ordered().into_iter().map(|theme| {
        let chosen = active.iter().any(|id| id == theme.id);
        card(ctx, theme, chosen)
    });
    row(cards).spacing(GAP).wrap().vertical_spacing(GAP).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn teskad_dark_and_light_lead_and_the_rest_keep_their_order() {
        let order: Vec<&str> = ordered().iter().map(|theme| theme.id).collect();
        assert_eq!(&order[..2], PINNED);
        let rest: Vec<&str> = hangar_theme::themes()
            .iter()
            .map(|theme| theme.id)
            .filter(|id| !PINNED.contains(id))
            .collect();
        assert_eq!(&order[2..], rest);
        assert_eq!(order.len(), hangar_theme::themes().len());
    }

    #[test]
    fn left_and_right_go_round_the_grid() {
        let order = ["a", "b", "c"];
        assert_eq!(along(&order, "a", true), Some("b"));
        assert_eq!(along(&order, "c", true), Some("a"));
        assert_eq!(along(&order, "a", false), Some("c"));
        assert_eq!(along(&order, "b", false), Some("a"));
        assert_eq!(along(&order, "nope", true), None);
    }

    /// Three columns, the last row with two cards.
    fn drawn() -> Vec<(&'static str, Rectangle)> {
        let at = |col: f32, row: f32| Rectangle {
            x: col * 180.0,
            y: row * 150.0,
            width: 170.0,
            height: 140.0,
        };
        vec![
            ("a", at(0.0, 0.0)),
            ("b", at(1.0, 0.0)),
            ("c", at(2.0, 0.0)),
            ("d", at(0.0, 1.0)),
            ("e", at(1.0, 1.0)),
        ]
    }

    #[test]
    fn up_and_down_keep_the_column_and_stop_at_the_edges() {
        let cards = drawn();
        assert_eq!(across(&cards, "a", Arrow::Down), Some("d"));
        assert_eq!(across(&cards, "e", Arrow::Up), Some("b"));
        assert_eq!(across(&cards, "d", Arrow::Up), Some("a"));
        assert_eq!(across(&cards, "a", Arrow::Up), None);
        assert_eq!(across(&cards, "d", Arrow::Down), None);
        // Nothing below the third column: the nearest card of the next row.
        assert_eq!(across(&cards, "c", Arrow::Down), Some("e"));
        // On one row, there is no row to go to.
        let one_row = &cards[..3];
        assert_eq!(across(one_row, "b", Arrow::Down), None);
        assert_eq!(across(one_row, "b", Arrow::Up), None);
    }
}
