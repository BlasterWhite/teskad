//! A view pane drawn: a pane of the grid that shows a preview (a file, a 3D
//! model, a kit's tab) rather than a terminal.
//!
//! Its header is a terminal pane's in shape and in what it answers: the same
//! height and look, the drag handle the grid reads off the header's band
//! (`grid.rs`), a double click that zooms, the pin of a pinned pane, and the
//! pin, zoom and close buttons (a view has no « … » menu to pin it from). What it says (the mark
//! of the entry's kind, its title, who opened it or the kit page's word for
//! its state) is the preview's (`preview::head`), and so is its body
//! (`preview::view_entry`): the viewers, the 3D view, the kit tab, or the
//! card of a file a helper window shows.

use hangar_i18n::{Key, args};
use hangar_theme::{fonts, style, tokens};
use hangar_ui::{Icon, button, tip};
use iced::widget::{Space, column, container, row, text};
use iced::{Alignment, Element, Length, Padding};

use crate::ctx::Ctx;
use crate::message::Message;
use crate::panes::terminal_pane::{Placing, slot_id};
use crate::panes::{Msg, header};
use crate::preview::store::Entry;
use crate::state::types::Workspace;

fn wrap(msg: Msg) -> Message {
    Message::Panes(msg)
}

/// The entry a view pane shows, once the preview has it.
pub fn entry_of<'a>(ctx: Ctx<'a>, ws: &str, id: &str) -> Option<&'a Entry> {
    ctx.model.live.preview.scope(ws)?.entry(id)
}

/// What the grid calls a view pane (the ghost of a drag, the zoom bar, a
/// boundary's label): its entry's title, or nothing before it is read.
pub fn name_of<'a>(ctx: Ctx<'a>, ws: &str, id: &str) -> &'a str {
    entry_of(ctx, ws, id).map_or("", |entry| entry.title.as_str())
}

/// The title's tooltip: the shortcut that jumps to the pane, for the first
/// nine in reading order while `pane.focusN` is bound, else what the entry
/// is (its path, or its kit and params).
fn title_tip(ctx: Ctx<'_>, ws: &str, entry: Option<&Entry>, index: usize) -> String {
    let keys = crate::commands::run::FOCUS
        .get(index)
        .map_or("", |id| ctx.keymap.label(*id));
    let what = entry.map(|entry| crate::preview::head::hint(ctx, ws, entry));
    if keys.is_empty() {
        return what.unwrap_or_default();
    }
    let jump = ctx
        .i18n
        .t_with(Key::PaneJump, &args!("keys" => keys))
        .into_owned();
    match what {
        Some(what) => format!("{jump}\n{what}"),
        None => jump,
    }
}

pub fn view<'a>(
    ctx: Ctx<'a>,
    ws: &'a Workspace,
    id: &'a str,
    placing: Placing,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let entry = entry_of(ctx, &ws.id, id);
    let focused = placing.focused;

    let mut bar = row![].spacing(tokens::space::SM).align_y(Alignment::Center);
    if let Some(entry) = entry {
        bar = bar.push(container(crate::preview::head::mark(ctx, entry, focused)).center_x(14));
    }
    let title = entry.map_or("", |entry| entry.title.as_str());
    bar = bar.push(tip::bottom(
        t,
        text(title)
            .size(tokens::font_size::UI)
            .font(fonts::ui_semibold())
            .wrapping(text::Wrapping::None)
            .style(move |theme| {
                if focused {
                    style::text::primary(t)(theme)
                } else {
                    style::text::muted(t)(theme)
                }
            }),
        title_tip(ctx, &ws.id, entry, placing.index),
    ));
    // Pinned to an edge of the grid, as a terminal can be.
    if let Some(pin) = ws.pin_of(id) {
        bar = bar.push(header::pin_glyph(ctx, &ws.id, id, pin.edge));
    }
    if let Some(aside) = entry.and_then(|entry| crate::preview::head::aside(ctx, &ws.id, entry)) {
        bar = bar.push(
            text(aside)
                .size(tokens::font_size::LABEL)
                .font(fonts::mono())
                .style(style::text::subtle(t))
                .wrapping(text::Wrapping::None),
        );
    }
    bar = bar.push(Space::new().width(Length::Fill));
    bar = bar.push(actions(ctx, ws, id, entry, &placing));

    let header = header::frame(t, bar);

    // What the preview draws may hold fields of its own (a kit's tab): they
    // take the keyboard from the terminals as the app's own fields do.
    let body = crate::keymap::watch(
        ctx,
        crate::preview::view_entry(&ctx.pkg.preview, ctx, &ws.id, id),
    );
    container(column![
        header,
        container(body).width(Length::Fill).height(Length::Fill)
    ])
    .id(slot_id(id))
    .width(Length::Fill)
    .height(Length::Fill)
    .style(style::container::window(t))
    .into()
}

/// The id of the button that pins a view pane or lets it go.
pub fn pin_toggle_id(id: &str) -> iced::widget::Id {
    iced::widget::Id::from(format!("pane-pin-toggle-{id}"))
}

/// The pin, zoom and close buttons, as a terminal pane's: a view pane may
/// always close, whatever is left beside it.
fn actions<'a>(
    ctx: Ctx<'a>,
    ws: &'a Workspace,
    id: &'a str,
    entry: Option<&'a Entry>,
    placing: &Placing,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let side =
        header::action_side(header::Fit::of(placing.width - header::PADDING_X).narrow_actions);
    let mut group = row![].spacing(1).align_y(Alignment::Center);
    // A view has no « … » menu: it pins and lets go from here, as a
    // terminal does from its menu.
    let pinned = ws.pin_of(id).is_some();
    group = group.push(
        container(
            button::toggle_button(t, Icon::Pin, pinned)
                .side(side)
                .tip(
                    ctx.t(if pinned { Key::PaneUnpin } else { Key::PanePin })
                        .into_owned(),
                )
                .on_press(wrap(Msg::Pin(ws.id.clone(), id.to_owned(), !pinned))),
        )
        .id(pin_toggle_id(id)),
    );
    if placing.can_zoom {
        let words = ctx
            .t(if placing.zoomed {
                Key::ZoomRestore
            } else {
                Key::PaneZoomIn
            })
            .into_owned();
        group = group.push(
            button::toggle_button(
                t,
                if placing.zoomed {
                    Icon::Collapse
                } else {
                    Icon::Expand
                },
                placing.zoomed,
            )
            .side(side)
            .tip(words)
            .on_press(wrap(Msg::ToggleZoom(ws.id.clone(), id.to_owned()))),
        );
    }
    let close = ctx
        .i18n
        .t_with(
            Key::ModelClose,
            &args!("title" => entry.map_or("", |entry| entry.title.as_str())),
        )
        .into_owned();
    group = group.push(
        button::icon_button(t, Icon::Close)
            .side(side)
            .danger()
            .tip(close)
            .on_press(wrap(Msg::Close(ws.id.clone(), id.to_owned()))),
    );
    container(group)
        .padding(Padding {
            left: 4.0,
            ..Padding::ZERO
        })
        .into()
}
