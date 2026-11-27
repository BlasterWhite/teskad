//! The board's three filters (`BoardFilter.tsx`): a compact button reading
//! "Label All ▾" that opens a list to pick from; in a narrow window the three
//! fold into one "Filters ▾" button.
//!
//! The list is hangar-ui's menu: the arrows walk it, Enter picks, Escape and
//! a click outside close it, and the pick is ticked, as the listbox of the web
//! app was.

use hangar_i18n::Key;
use hangar_theme::fonts;
use hangar_theme::tokens::scale::{radius, size as scale_size, type_scale};
use hangar_theme::tokens::space;
use hangar_ui::icon::{Icon, icon};
use hangar_ui::menu::{self, Entry};
use hangar_ui::{focus, tab_toolbar, tip};
use iced::widget::{button, row, text};
use iced::{Alignment, Element, Padding};

use crate::message::Message;

use super::select::{self, ANY, NOBODY, STATE_BLOCKED, STATE_FREE};
use super::view::View;
use super::{FilterKind, Msg};

/// A choice of a filter: `FilterOption` of the TS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opt {
    pub value: String,
    pub label: String,
    /// What the button says when this is picked, if shorter: "All".
    pub short: Option<String>,
    /// Said after the label, quieter: the pane an agent's name leads to.
    pub hint: Option<String>,
}

fn opt(value: &str, label: impl Into<String>) -> Opt {
    Opt {
        value: value.to_owned(),
        label: label.into(),
        short: None,
        hint: None,
    }
}

/// The options of a filter, for the tasks on the board.
pub(super) fn options(v: &View<'_>, kind: FilterKind) -> Vec<Opt> {
    let tr = v.ctx.i18n;
    let all = |label: Key| Opt {
        short: Some(tr.t(Key::BoardFilterAll).into_owned()),
        ..opt(ANY, tr.t(label))
    };
    match kind {
        FilterKind::Label => std::iter::once(all(Key::BoardAllLabels))
            .chain(
                select::labels(v.all, &v.st.filters.label)
                    .into_iter()
                    .map(|label| opt(&label, label.clone())),
            )
            .collect(),
        FilterKind::Assignee => {
            let named = select::assignees(v.all, &v.st.filters.assignee)
                .into_iter()
                .map(|name| {
                    // The pane's name, when it is not the name itself.
                    let hint = v
                        .pane(&name)
                        .filter(|pane| pane.name != name)
                        .map(|pane| pane.name.clone());
                    Opt {
                        hint,
                        ..opt(&name, name.clone())
                    }
                });
            std::iter::once(all(Key::BoardAllAssignees))
                .chain(std::iter::once(opt(NOBODY, tr.t(Key::BoardUnassigned))))
                .chain(named)
                .collect()
        }
        FilterKind::State => vec![
            all(Key::BoardStateAny),
            opt(STATE_BLOCKED, tr.t(Key::BoardStateBlocked)),
            opt(STATE_FREE, tr.t(Key::BoardStateFree)),
        ],
    }
}

/// The three filters.
const KINDS: [FilterKind; 3] = [FilterKind::Label, FilterKind::Assignee, FilterKind::State];

/// The three filters, and the button that clears them while any is on.
pub(super) fn group<'a>(v: &View<'a>) -> Element<'a, Message> {
    let mut filters = row(KINDS.map(|kind| control(v, kind)))
        .spacing(space::MD)
        .align_y(Alignment::Center);
    if v.st.filters.active() {
        let t = v.t();
        let tr = v.ctx.i18n;
        filters = filters.push(tip::bottom(
            t,
            Element::from(
                hangar_ui::button::ghost(t, tr.t(Key::BoardClearFiltersShort))
                    .icon(Icon::Close)
                    .on_press(Message::Board(Msg::ClearFilters)),
            ),
            tr.t(Key::BoardClearFilters),
        ));
    }
    filters.into()
}

/// A filter's name, the value it is on, and its name in full.
fn named<'s>(v: &View<'s>, kind: FilterKind) -> (Key, Key, &'s str) {
    let filters = &v.st.filters;
    match kind {
        FilterKind::Label => (
            Key::BoardFilterNameLabel,
            Key::BoardFilterLabel,
            &filters.label,
        ),
        FilterKind::Assignee => (
            Key::BoardFilterNameAgent,
            Key::BoardFilterAssignee,
            &filters.assignee,
        ),
        FilterKind::State => (
            Key::BoardFilterNameState,
            Key::BoardFilterState,
            &filters.state,
        ),
    }
}

/// The entries that pick a value of one filter, the one it is on ticked.
fn picks(v: &View<'_>, kind: FilterKind) -> Vec<Entry<Message>> {
    let (_, _, value) = named(v, kind);
    options(v, kind)
        .iter()
        .map(|option| {
            let label = match &option.hint {
                Some(hint) => format!("{}  ·  {hint}", option.label),
                None => option.label.clone(),
            };
            Entry::from(
                menu::item(label)
                    .checked(option.value == value)
                    .on_select(Message::Board(Msg::Filter(kind, option.value.clone()))),
            )
        })
        .collect()
}

/// One filter: its button, and its list while it is open.
fn control<'a>(v: &View<'a>, kind: FilterKind) -> Element<'a, Message> {
    let t = v.t();
    let tr = v.ctx.i18n;
    let (name, title, value) = named(v, kind);
    let options = options(v, kind);
    let active = value != ANY;
    let current = options
        .iter()
        .find(|option| option.value == value)
        .or(options.first());
    let shown = current
        .map(|option| option.short.clone().unwrap_or_else(|| option.label.clone()))
        .unwrap_or_default();

    let press = Message::Board(Msg::OpenFilter(Some(kind)));
    let face = button(super::look::centred(
        row![
            text(tr.t(name))
                .size(type_scale::CAPTION.size)
                .color(t.palette.text_muted)
                .wrapping(text::Wrapping::None),
            text(shown)
                .size(type_scale::CAPTION.size)
                .font(fonts::ui_medium())
                .wrapping(text::Wrapping::None),
            icon(Icon::ChevronDown, 10.0).color(t.palette.text_muted),
        ]
        .spacing(space::SM)
        .align_y(Alignment::Center),
    ))
    .height(scale_size::CONTROL_SM)
    .padding(Padding {
        top: 0.0,
        right: space::MD,
        bottom: 0.0,
        left: space::LG,
    })
    .style(tab_toolbar::filter_style(t, active))
    .on_press(press.clone());
    let face = focus::ring(t, face).radius(radius::SM).on_activate(press);

    let open = v.st.open_filter == Some(kind);
    let entries = open.then(|| picks(v, kind));
    menu::dropdown(t, tip::bottom(t, face, tr.t(title)), entries)
        .on_dismiss(Message::Board(Msg::OpenFilter(None)))
        .into()
}

/// The three filters as one button, "Filters ▾" with how many are on, for a
/// bar with no room: its menu lists the three one under another.
pub(super) fn folded<'a>(v: &View<'a>) -> Element<'a, Message> {
    let t = v.t();
    let tr = v.ctx.i18n;
    let on = KINDS
        .into_iter()
        .filter(|kind| named(v, *kind).2 != ANY)
        .count();
    let face = tab_toolbar::filters_button(
        t,
        tr.t(Key::BoardFilters).into_owned(),
        on,
        Message::Board(Msg::FiltersMenu(!v.st.filters_menu)),
    );
    let entries = v.st.filters_menu.then(|| {
        let mut entries = Vec::new();
        for kind in KINDS {
            if !entries.is_empty() {
                entries.push(menu::separator());
            }
            entries.push(menu::heading(tr.t(named(v, kind).0).into_owned()));
            entries.extend(picks(v, kind));
        }
        if v.st.filters.active() {
            entries.push(menu::separator());
            entries.push(Entry::from(
                menu::item(tr.t(Key::BoardClearFilters).into_owned())
                    .icon(Icon::Close)
                    .on_select(Message::Board(Msg::ClearFilters)),
            ));
        }
        entries
    });
    menu::dropdown(t, face, entries)
        .on_dismiss(Message::Board(Msg::FiltersMenu(false)))
        .into()
}
