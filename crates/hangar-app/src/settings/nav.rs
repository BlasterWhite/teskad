//! The list of categories at the left of the sheet: the search field, the
//! categories under the heading of their [`Group`], and at the foot, set apart
//! by a hairline, the two that belong to none.
//!
//! Every entry takes the keyboard (Tab, then Enter or Space), and once the
//! list has been used Up and Down walk it.

use hangar_theme::tokens::{BORDER_WIDTH, font_size, radius, space};
use hangar_theme::{Theme, fonts, style};
use hangar_ui::menu_nav::arrow_step;
use hangar_ui::{Icon, focus, form, icon, keys};
use iced::widget::{button, column, container, row, rule, scrollable, text};
use iced::{Alignment, Background, Border, Element, Length, Padding, Shadow};

use super::schema::Group;
use super::{INNER_RADIUS, Msg, State};
use crate::ctx::Ctx;
use crate::message::Message;
use crate::shell::SettingsCategory;

/// The column's width.
const WIDTH: f32 = 240.0;
/// An entry's height.
const ENTRY_HEIGHT: f32 = 30.0;
/// The room at both sides of the column.
const PADDING_X: f32 = 10.0;
const PADDING_Y: f32 = 14.0;
/// A group's heading: mono, in capitals.
const HEADING_SIZE: f32 = 10.5;
/// How far from `bg` towards `surface` the column is filled: a step darker
/// than the page beside it.
const FILL_LIFT: f32 = 0.5;

/// A category the search left: its name, icon and number of rows.
pub type Entry = (SettingsCategory, String, Icon, usize);

/// The whole column. `searching` adds each entry's count of rows.
pub fn side<'a>(
    ctx: Ctx<'a>,
    st: &'a State,
    search: Element<'a, Message>,
    entries: Vec<Entry>,
    active: Option<SettingsCategory>,
    searching: bool,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let mut top = column![].spacing(space::XS);
    let mut foot = column![].spacing(space::XS);
    let mut headed = None;
    let mut footed = false;
    for entry in entries {
        let group = Group::of(entry.0);
        let current = Some(entry.0) == active;
        let line = line(t, entry, current, searching);
        if group == Group::Foot {
            footed = true;
            foot = foot.push(line);
            continue;
        }
        if headed != Some(group) {
            let first = headed.is_none();
            headed = Some(group);
            if let Some(key) = group.heading() {
                top = top.push(heading(t, ctx.t(key).to_uppercase(), first));
            }
        }
        top = top.push(line);
    }

    // The ring of a focused entry reaches this far out of it: room the
    // scrolling part keeps, so as not to cut it.
    let ring = space::XS;
    let top = scrollable(container(top).padding(Padding::from([ring, PADDING_X])))
        .height(Length::Fill)
        .style(style::scrollable::default(t));
    let mut list = column![top].height(Length::Fill);
    if footed {
        list = list.push(
            container(
                column![
                    rule::horizontal(BORDER_WIDTH).style(style::rule::hairline(t)),
                    foot
                ]
                .spacing(PADDING_X),
            )
            .padding(Padding {
                top: ring,
                right: PADDING_X,
                bottom: PADDING_Y,
                left: PADDING_X,
            }),
        );
    }
    let nav_active = st.nav_active;
    let list = keys::on_key(list, move |key, _| {
        arrow_step(key)
            .filter(|_| nav_active)
            .map(|step| Message::Settings(Msg::Step(step)))
    });

    let fill = form::between(t.palette.bg, t.palette.surface, FILL_LIFT);
    container(column![
        container(search).padding(Padding {
            top: PADDING_Y,
            right: PADDING_X,
            bottom: space::MD,
            left: PADDING_X,
        }),
        list,
    ])
    .width(WIDTH)
    .height(Length::Fill)
    .style(move |_: &iced::Theme| container::Style {
        background: Some(Background::Color(fill)),
        text_color: Some(t.palette.text),
        // Its own corners round with the sheet's: the sheet clips to its
        // bounds, not to its radius, so a square fill would cover them.
        border: Border {
            radius: iced::border::left(INNER_RADIUS),
            ..Border::default()
        },
        ..container::Style::default()
    })
    .into()
}

/// The hairline between the column and the page.
pub fn divider<'a>(t: &'a Theme) -> Element<'a, Message> {
    rule::vertical(BORDER_WIDTH)
        .style(style::rule::hairline(t))
        .into()
}

fn heading<'a>(t: &'a Theme, words: String, first: bool) -> Element<'a, Message> {
    container(
        text(words)
            .size(HEADING_SIZE)
            .font(fonts::mono())
            .color(t.palette.text_muted),
    )
    .padding(Padding {
        top: if first { space::MD } else { space::XL },
        right: PADDING_X,
        bottom: space::XS,
        left: PADDING_X,
    })
    .into()
}

fn line<'a>(
    t: &'a Theme,
    (id, label, icon_name, count): Entry,
    current: bool,
    searching: bool,
) -> Element<'a, Message> {
    let p = &t.palette;
    let glyph = icon(icon_name, 15.0);
    let glyph = if current {
        glyph.color(p.status.active)
    } else {
        glyph
    };
    let name = text(label)
        .size(font_size::UI)
        .width(Length::Fill)
        .wrapping(text::Wrapping::None);
    // The chosen one is told by its weight and its frame as well as its fill.
    let name = if current {
        name.font(fonts::ui_medium())
    } else {
        name
    };
    let mut content = row![glyph, name]
        .spacing(PADDING_X)
        .align_y(Alignment::Center);
    if searching {
        content = content.push(
            text(count.to_string())
                .size(font_size::LABEL)
                .font(fonts::mono())
                .color(p.text_muted),
        );
    }
    let pick = Message::Settings(Msg::Pick(id));
    let entry = button(container(content).center_y(Length::Fill))
        .width(Length::Fill)
        .height(ENTRY_HEIGHT)
        .padding(Padding::from([0.0, PADDING_X]))
        .style(entry_style(t, current))
        .on_press(pick.clone());
    focus::ring(t, entry)
        .radius(radius::SM)
        .on_activate(pick)
        .into()
}

/// The chosen entry on the selection's colour with a hairline, the one under
/// the pointer on the overlay.
fn entry_style(
    t: &Theme,
    current: bool,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style + '_ {
    move |_, status| {
        let p = &t.palette;
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let fill = if current {
            Some(p.selection_bg)
        } else if hovered {
            Some(p.hover_overlay)
        } else {
            None
        };
        button::Style {
            background: fill.map(Background::Color),
            text_color: if current || hovered {
                p.text
            } else {
                p.text_secondary
            },
            border: Border {
                color: if current {
                    p.border_strong
                } else {
                    iced::Color::TRANSPARENT
                },
                width: BORDER_WIDTH,
                radius: radius::SM.into(),
            },
            shadow: Shadow::default(),
            ..button::Style::default()
        }
    }
}
