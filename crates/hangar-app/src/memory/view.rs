//! The memory view (`ecrans.md` §5, `05-memory` and `05b-memory-new-entry`):
//! the tab's bar with the count, the search, the sort and "New entry", then
//! the facts one per row (what each says on the left, where it came from on
//! the right), and a window over the page to write or edit one.

use std::time::{SystemTime, UNIX_EPOCH};

use iced::keyboard::{Key, key::Named};
use iced::widget::markdown::{HeadingLevel, Highlight, Settings, Style as MarkdownStyle};
use iced::widget::{
    button, column, container, markdown, mouse_area, row, rule, scrollable, space, text,
    text_editor::Content,
};
use iced::{Alignment, Background, Border, Element, Length, Padding, border, padding};

use hangar_core::memory::MemoryEntry;
use hangar_i18n::{Key as K, args};
use hangar_theme::tokens::{BORDER_WIDTH, scale};
use hangar_theme::{Theme, fonts, style, tokens};
use hangar_ui::{Icon, button as btn, dialog, empty, focus, icon, input, keys, tab_toolbar, tip};

use super::format::{ShortDay, compact_path, full_date, short_day, signer_name, was_updated};
use super::guards::{
    MEMORY_CAP, MemorySort, find_title_clash, matches_haystack, near_cap, search_terms,
    sort_entries,
};
use super::{Draft, Msg, State, composer_id, search_id};
use crate::ctx::Ctx;
use crate::message::Message;

/// The width under which a row's origin folds under its text, on one line
/// (`ecrans.md` §5, « Place qui manque »).
const NARROW: f32 = 1100.0;
/// The list is centred and no wider than this.
const LIST_MAX_WIDTH: f32 = 1120.0;
/// The width of the column that says where a fact came from.
const ORIGIN_WIDTH: f32 = 210.0;
/// The chevron's column.
const CHEVRON: f32 = 24.0;
/// How long a path may be in that column.
const PATH_CHARS: usize = 30;
/// The search field's width.
const SEARCH_WIDTH: f32 = 300.0;
/// A collapsed entry shows two lines of its text (13 px on 19).
const EXCERPT_LINE: f32 = 19.0;
const CLAMP_HEIGHT: f32 = 2.0 * EXCERPT_LINE + 1.0;
/// The window that writes or edits an entry.
const FORM_WIDTH: f32 = 640.0;
/// The content field's height.
const CONTENT_HEIGHT: f32 = 180.0;

fn msg(msg: Msg) -> Message {
    Message::Memory(msg)
}

/// A label without arguments, as the widgets that keep a `&str` for the whole
/// frame want it. The dictionary lends such a label as a `'static` borrow
/// (hangar-i18n, `t`), so nothing is allocated; the owned case, which only a
/// computed key naming nothing could reach, is leaked rather than shown wrong.
fn label(ctx: Ctx<'_>, key: K) -> &'static str {
    match ctx.t(key) {
        std::borrow::Cow::Borrowed(text) => text,
        std::borrow::Cow::Owned(text) => Box::leak(text.into_boxed_str()),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

pub(super) fn view<'a>(st: &'a State, ctx: Ctx<'a>) -> Element<'a, Message> {
    let t = ctx.theme;
    let loaded = st.entries.is_some();
    let all: &[MemoryEntry] = st.entries.as_deref().unwrap_or(&[]);
    let terms = search_terms(&st.query);
    let visible = sort_entries(
        all.iter()
            .zip(&st.haystacks)
            .filter(|(_, haystack)| matches_haystack(haystack, &terms))
            .map(|(entry, _)| entry),
        st.sort,
    );
    let width = ctx.metrics.body_width;
    let narrow = width < NARROW;

    let mut page = column![bar(
        st,
        ctx,
        &visible,
        all.len(),
        tab_toolbar::compact(width)
    )];

    if let Some(error) = &st.error {
        page = page.push(notice(
            t,
            Icon::Alert,
            t.palette.status.failed,
            text(error.as_str())
                .size(tokens::font_size::UI)
                .color(t.palette.text)
                .into(),
        ));
    }
    if loaded && near_cap(all.len()) {
        page = page.push(cap_note(st, ctx, all.len()));
    }

    let mut list = column![].width(Length::Fill);
    if !loaded {
        list = list.push(skeleton(t));
    }
    // Never under an error: a memory that could not be read is not one that
    // has nothing in it, and saying "empty" would invite rewriting facts that
    // are still on disk.
    if loaded && all.is_empty() && st.error.is_none() && !st.composing {
        list = list.push(
            container(
                empty::empty(t, Icon::Bookmark, ctx.t(K::MemoryEmpty))
                    .description(ctx.t(K::MemoryEmptyBody))
                    .action(
                        btn::primary(t, ctx.t(K::MemoryEmptyAction))
                            .icon(Icon::Plus)
                            .on_press(msg(Msg::StartComposing)),
                    ),
            )
            .height(360.0),
        );
    }
    if !all.is_empty() && visible.is_empty() {
        list = list.push(no_results(st, ctx));
    }
    for entry in &visible {
        list = list.push(entry_row(st, ctx, entry, narrow));
    }

    let body = scrollable(
        container(container(list).max_width(LIST_MAX_WIDTH))
            .width(Length::Fill)
            .center_x(Length::Fill)
            .padding(Padding {
                top: scale::space::S2,
                right: scale::space::S6,
                bottom: scale::space::S6,
                left: scale::space::S6,
            }),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(style::scrollable::default(t));

    let page = container(page.push(body))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_: &iced::Theme| container::Style {
            background: Some(Background::Color(t.palette.bg)),
            text_color: Some(t.palette.text),
            ..container::Style::default()
        });

    dialog::over(t, page, form(st, ctx, all)).into()
}

/// The tab's bar: the count, the search, the sort; the note that agents write
/// here, and "New entry".
fn bar<'a>(
    st: &'a State,
    ctx: Ctx<'a>,
    visible: &[&MemoryEntry],
    total: usize,
    compact: bool,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let heading = if st.entries.is_none() {
        ctx.t(K::MemoryTitle).to_string()
    } else if !search_terms(&st.query).is_empty() {
        ctx.i18n
            .tn(K::MemoryResults, visible.len(), &[])
            .to_string()
    } else {
        ctx.i18n.tn(K::MemoryEntries, total, &[]).to_string()
    };

    let search = input::search_field(t, &st.query, label(ctx, K::MemorySearch))
        .on_input(|query| msg(Msg::Query(query)))
        .on_clear(msg(Msg::ClearQuery))
        .id(search_id())
        .width(Length::Fill);
    let search = crate::keymap::guard(ctx, search);
    // Escape empties the field first, the way a search field does.
    let escapes = !st.query.is_empty();
    let search = keys::on_field_key(search, move |key, _| match key {
        Key::Named(Named::Escape) if escapes => Some(msg(Msg::ClearQuery)),
        _ => None,
    });
    let search = container(search).width(SEARCH_WIDTH);

    let sort = input::segmented_labels(
        t,
        vec![
            (MemorySort::Recent, ctx.t(K::MemorySegRecent).to_string()),
            (MemorySort::Oldest, ctx.t(K::MemorySegOldest).to_string()),
        ],
        &st.sort,
        |sort| msg(Msg::SetSort(sort)),
    );

    let new = btn::primary(t, ctx.t(K::MemoryNew))
        .icon(Icon::Plus)
        .size(btn::Size::Bar)
        .on_press(msg(Msg::ToggleComposer));

    tab_toolbar::bar(
        t,
        row![tab_toolbar::count(t, heading), search, sort]
            .spacing(tokens::space::XXL)
            .align_y(Alignment::Center),
        row![
            tab_toolbar::note_plain(t, ctx.t(K::MemoryHint).to_string(), compact),
            Element::from(new)
        ]
        .spacing(tokens::space::LG)
        .align_y(Alignment::Center),
    )
}

/// A line that says something, over the list: a hairline of `accent` round a
/// sunken ground, its glyph in the same colour.
fn notice<'a>(
    t: &'a Theme,
    glyph: Icon,
    accent: iced::Color,
    content: Element<'a, Message>,
) -> Element<'a, Message> {
    container(
        row![
            icon(glyph, tokens::size::ICON).color(accent),
            container(content).width(Length::Fill),
        ]
        .spacing(tokens::space::MD)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([tokens::space::MD, scale::space::S6]))
    .width(Length::Fill)
    .style(move |_: &iced::Theme| container::Style {
        background: Some(Background::Color(t.palette.surface_sunken)),
        border: Border {
            color: accent,
            width: BORDER_WIDTH,
            radius: scale::radius::MD.into(),
        },
        ..container::Style::default()
    })
    .into()
}

/// "The memory holds 912 of 1000", with the toggle that sorts the oldest
/// first, to find what to delete.
fn cap_note<'a>(st: &'a State, ctx: Ctx<'a>, count: usize) -> Element<'a, Message> {
    let t = ctx.theme;
    let words = ctx
        .i18n
        .t_with(K::MemoryNearCap, &args!("n" => count, "max" => MEMORY_CAP));
    let toggle = btn::ghost(
        t,
        match st.sort {
            MemorySort::Oldest => ctx.t(K::MemorySortRecent),
            MemorySort::Recent => ctx.t(K::MemorySortOldest),
        },
    )
    .small()
    .on_press(msg(Msg::FlipSort));
    container(
        row![
            icon(Icon::Alert, tokens::size::ICON).color(t.palette.accent_text),
            text(words.into_owned())
                .size(tokens::font_size::UI)
                .color(t.palette.text_muted)
                .width(Length::Fill),
            toggle,
        ]
        .spacing(tokens::space::MD)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([tokens::space::SM, scale::space::S6]))
    .width(Length::Fill)
    .into()
}

/// A field's name above it.
fn field_label<'a>(ctx: Ctx<'a>, key: K) -> Element<'a, Message> {
    text(ctx.t(key).to_string())
        .size(tokens::font_size::UI)
        .font(fonts::ui_semibold())
        .color(ctx.theme.palette.text)
        .into()
}

/// The content field: the Markdown source, in the mono face.
fn editor<'a>(
    ctx: Ctx<'a>,
    content: &'a Content,
    placeholder: &'static str,
    on_action: fn(iced::widget::text_editor::Action) -> Msg,
) -> Element<'a, Message> {
    let field = iced::widget::text_editor(content)
        .placeholder(placeholder)
        .on_action(move |action| msg(on_action(action)))
        .font(fonts::mono())
        .size(tokens::font_size::LABEL)
        .padding(Padding::from([tokens::space::LG, tokens::space::XL]))
        .height(CONTENT_HEIGHT)
        .style(style::text_editor::field(ctx.theme));
    crate::keymap::guard(ctx, field)
}

/// The window over the page, while a fact is written (`05b`) or edited: a
/// title, the text, the tags, and in the foot who writes it and where.
fn form<'a>(
    st: &'a State,
    ctx: Ctx<'a>,
    all: &'a [MemoryEntry],
) -> Option<dialog::Dialog<'a, Message>> {
    let t = ctx.theme;
    // The entry being edited comes before a half-written new one.
    let (draft, editing): (&Draft, bool) = match &st.editing {
        Some(editing) => (&editing.draft, true),
        None if st.composing => (&st.draft, false),
        None => return None,
    };
    let clash = (!editing)
        .then(|| find_title_clash(all, &draft.title))
        .flatten();
    let replacing = st.confirming_replace && clash.is_some();

    let title = {
        let field = input::text_field(t, &draft.title)
            .placeholder(label(ctx, K::MemoryTitlePlaceholder))
            .on_input(move |title| {
                msg(if editing {
                    Msg::EditTitle(title)
                } else {
                    Msg::DraftTitle(title)
                })
            });
        let field = if editing {
            field
        } else {
            field.on_submit(msg(Msg::Create)).id(composer_id())
        };
        crate::keymap::guard(ctx, field)
    };

    let mut fields = column![column![field_label(ctx, K::MemoryFieldTitle), title].spacing(6)]
        .spacing(scale::space::S4);
    if clash.is_some() {
        fields = fields.push(
            container(
                row![
                    icon(Icon::Alert, tokens::size::ICON).color(t.palette.accent_text),
                    text(ctx.t(K::MemoryTitleTaken).to_string())
                        .size(tokens::font_size::UI)
                        .color(t.palette.text),
                ]
                .spacing(tokens::space::MD)
                .align_y(Alignment::Center),
            )
            .padding(Padding::from([tokens::space::MD, tokens::space::XL]))
            .width(Length::Fill)
            .style(move |_: &iced::Theme| container::Style {
                background: Some(Background::Color(t.palette.selection_bg)),
                border: Border {
                    radius: scale::radius::MD.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            }),
        );
    }
    let content_action: fn(_) -> Msg = if editing {
        Msg::EditContent
    } else {
        Msg::DraftContent
    };
    let tags_message: fn(String) -> Msg = if editing {
        Msg::EditTags
    } else {
        Msg::DraftTags
    };
    fields = fields
        .push(
            column![
                field_label(ctx, K::MemoryFieldContent),
                editor(
                    ctx,
                    &draft.content,
                    label(ctx, K::MemoryContentPlaceholder),
                    content_action
                ),
            ]
            .spacing(6),
        )
        .push(
            column![
                field_label(ctx, K::MemoryFieldTags),
                crate::keymap::guard(
                    ctx,
                    input::text_field(t, &draft.tags)
                        .placeholder(label(ctx, K::MemoryTagsAdd))
                        .on_input(move |tags| msg(tags_message(tags)))
                        .hint(ctx.t(K::MemoryTagsHint).to_string())
                        .mono(),
                ),
            ]
            .spacing(6),
        );

    let (title_key, save_message, cancel_message) = if editing {
        (K::MemoryEditTitle, Msg::SaveEdit, Msg::CancelEdit)
    } else {
        (K::MemoryNew, Msg::Create, Msg::CancelCompose)
    };
    let save_label = if editing {
        ctx.t(K::MemorySave)
    } else if replacing {
        ctx.t(K::MemoryReplaceConfirm)
    } else {
        ctx.t(K::MemorySave)
    };
    let workspace = ctx
        .model
        .active_workspace()
        .map(|ws| ws.name.clone())
        .unwrap_or_default();
    let by = ctx
        .i18n
        .t_with(K::MemoryWrittenByYouIn, &args!("workspace" => workspace))
        .into_owned();

    Some(
        dialog::dialog(t, ctx.t(title_key).to_string(), fields)
            .width(FORM_WIDTH)
            .body_max_height(560.0)
            .note(by)
            .action(btn::secondary(t, ctx.t(K::CreateCancel)).on_press(msg(cancel_message.clone())))
            .action(
                btn::primary(t, save_label)
                    .on_press_maybe(draft.is_complete().then(|| msg(save_message))),
            )
            .close_tip(ctx.t(K::ModalClose).to_string())
            .on_close(msg(cancel_message)),
    )
}

/// Four ghost rows while the first read is out.
fn skeleton<'a>(t: &'a Theme) -> Element<'a, Message> {
    let bar = |width: f32, height: f32| {
        container(space::horizontal())
            .width(width)
            .height(height)
            .style(move |_: &iced::Theme| container::Style {
                background: Some(Background::Color(t.palette.border)),
                border: Border {
                    radius: scale::radius::XS.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            })
    };
    let mut rows = column![];
    for _ in 0..4 {
        rows = rows.push(
            container(
                row![
                    column![bar(180.0, 10.0), bar(320.0, 8.0), bar(220.0, 8.0)]
                        .spacing(tokens::space::MD)
                        .width(Length::Fill),
                    bar(110.0, 8.0),
                ]
                .spacing(24),
            )
            .padding(Padding::from([14.0, 16.0])),
        );
    }
    rows.into()
}

/// A search that found nothing, and the way out of it.
fn no_results<'a>(st: &'a State, ctx: Ctx<'a>) -> Element<'a, Message> {
    let t = ctx.theme;
    container(
        empty::empty(
            t,
            Icon::Search,
            ctx.i18n
                .t_with(K::MemoryNoResults, &args!("q" => st.query.trim()))
                .into_owned(),
        )
        .description(ctx.t(K::MemoryNoResultsBody))
        .action(
            btn::secondary(t, ctx.t(K::MemoryClearSearch))
                .icon(Icon::Close)
                .on_press(msg(Msg::ClearQuery)),
        )
        .inline(),
    )
    .padding(Padding::from([tokens::space::XXXL, 0.0]))
    .into()
}

/// One fact: the chevron, what it says with its tags and actions, and who
/// wrote it, when and from where on the right.
fn entry_row<'a>(
    st: &'a State,
    ctx: Ctx<'a>,
    entry: &'a MemoryEntry,
    narrow: bool,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let open = st.open.contains(&entry.id);
    let hovered = st.hover.as_deref() == Some(entry.id.as_str());

    let chevron = {
        let face = button(
            container(icon(
                if open {
                    Icon::ChevronDown
                } else {
                    Icon::ChevronRight
                },
                tokens::size::ICON,
            ))
            .center(Length::Fill),
        )
        .width(CHEVRON)
        .height(CHEVRON)
        .padding(0)
        .style(style::button::plain(t))
        .on_press(msg(Msg::Toggle(entry.id.clone())));
        // Right opens, Left folds, Enter edits, Delete asks to delete.
        let id = entry.id.clone();
        let toggle = || msg(Msg::Toggle(entry.id.clone()));
        let ring = focus::ring(
            t,
            tip::over(
                t,
                face,
                if open {
                    ctx.t(K::MemoryCollapse)
                } else {
                    ctx.t(K::MemoryExpand)
                },
            ),
        )
        .radius(scale::radius::SM)
        .on_activate(msg(Msg::StartEdit(entry.id.clone())))
        .on_arrows(open.then(toggle), (!open).then(toggle));
        keys::on_field_key(ring, move |key, _| match key {
            Key::Named(Named::Delete) => Some(msg(Msg::Arm(id.clone()))),
            _ => None,
        })
    };

    let editing_this = st
        .editing
        .as_ref()
        .is_some_and(|editing| editing.id == entry.id);
    let main = reading(st, ctx, entry, open, hovered || editing_this);
    let origin = origin(ctx, entry, narrow);

    let line: Element<'a, Message> = if narrow {
        row![
            container(chevron).width(CHEVRON),
            column![main, origin]
                .spacing(tokens::space::SM)
                .width(Length::Fill),
        ]
        .spacing(tokens::space::XL)
        .into()
    } else {
        row![
            container(chevron).width(CHEVRON),
            container(main).width(Length::Fill),
            container(origin).width(ORIGIN_WIDTH),
        ]
        .spacing(tokens::space::XL)
        .into()
    };
    let shown = container(line)
        .padding(Padding::from([14.0, 16.0]))
        .width(Length::Fill)
        .style(move |_: &iced::Theme| container::Style {
            background: hovered.then_some(Background::Color(t.palette.hover_overlay)),
            border: Border {
                radius: scale::radius::MD.into(),
                ..Border::default()
            },
            ..container::Style::default()
        });
    column![
        mouse_area(shown)
            .on_enter(msg(Msg::Hover(Some(entry.id.clone()))))
            .on_exit(msg(Msg::Hover(None))),
        rule::horizontal(1).style(move |theme: &iced::Theme| rule::Style {
            color: t.palette.border,
            ..style::rule::hairline(t)(theme)
        }),
    ]
    .into()
}

/// How an open entry's Markdown is drawn: the interface face at 13 px, headings
/// a step larger, code in the mono face.
fn markdown_settings(t: &Theme) -> Settings {
    let size = tokens::font_size::UI;
    let mut settings = Settings::with_text_size(
        size,
        MarkdownStyle {
            font: fonts::ui(),
            inline_code_highlight: Highlight {
                background: Background::Color(t.palette.hover_overlay),
                border: border::rounded(scale::radius::XS),
            },
            inline_code_padding: padding::left(3).right(3),
            inline_code_color: t.palette.text,
            inline_code_font: fonts::mono(),
            code_block_font: fonts::mono(),
            link_color: t.palette.accent_text,
        },
    );
    let step = |level: HeadingLevel| {
        size * match level {
            HeadingLevel::H1 => 1.4,
            HeadingLevel::H2 => 1.25,
            HeadingLevel::H3 => 1.12,
            _ => 1.0,
        }
    };
    settings.h1_size = step(HeadingLevel::H1).into();
    settings.h2_size = step(HeadingLevel::H2).into();
    settings.h3_size = step(HeadingLevel::H3).into();
    settings.h4_size = size.into();
    settings.h5_size = size.into();
    settings.h6_size = size.into();
    settings.code_size = (size * 0.88).into();
    settings
}

/// An entry as it is read: its title, its text (cut at two lines, or whole
/// and in Markdown when open), its tags and, open or under the pointer, the
/// actions.
fn reading<'a>(
    st: &'a State,
    ctx: Ctx<'a>,
    entry: &'a MemoryEntry,
    open: bool,
    actions_shown: bool,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let title = button(
        text(entry.title.as_str())
            .size(tokens::font_size::BODY)
            .font(fonts::ui_semibold())
            .color(t.palette.text),
    )
    .padding(0)
    .style(style::button::plain(t))
    .on_press(msg(Msg::Toggle(entry.id.clone())));

    let mut main = column![title].spacing(6);
    let rendered = open.then(|| st.rendered.get(&entry.id)).flatten();
    let text_block: Element<'a, Message> = match rendered {
        Some(content) => container(
            markdown::view(content.items(), markdown_settings(t)).map(|_| msg(Msg::Ignore)),
        )
        .padding(Padding::from([tokens::space::XL, tokens::space::XXL]))
        .width(Length::Fill)
        .style(move |_: &iced::Theme| container::Style {
            background: Some(Background::Color(t.palette.surface_sunken)),
            text_color: Some(t.palette.text_secondary),
            border: Border {
                color: t.palette.border,
                width: BORDER_WIDTH,
                radius: scale::radius::MD.into(),
            },
            ..container::Style::default()
        })
        .into(),
        None => {
            // Collapsed entries keep the list scannable; the whole text is
            // one click away. iced has no line clamp, so the text is cut at
            // two lines.
            let excerpt: Element<'a, Message> = text(entry.content.as_str())
                .size(tokens::font_size::UI)
                .line_height(iced::widget::text::LineHeight::Absolute(
                    EXCERPT_LINE.into(),
                ))
                .color(t.palette.text_secondary)
                .into();
            container(excerpt)
                .max_height(CLAMP_HEIGHT)
                .clip(true)
                .into()
        }
    };
    main = main.push(text_block);

    let chips = row(entry.tags.iter().map(|tag| {
        container(
            text(tag.as_str())
                .size(tokens::font_size::LABEL)
                .font(fonts::mono())
                .color(t.palette.text_secondary),
        )
        .padding(Padding::from([2.0, tokens::space::MD]))
        .style(move |_: &iced::Theme| container::Style {
            background: Some(Background::Color(t.palette.surface_sunken)),
            border: Border {
                color: t.palette.border,
                width: BORDER_WIDTH,
                radius: scale::radius::XS.into(),
            },
            ..container::Style::default()
        })
        .into()
    }))
    .spacing(tokens::space::SM)
    .wrap();
    let mut line = row![container(chips).width(Length::Fill)]
        .spacing(tokens::space::XL)
        .align_y(Alignment::Center);
    if open || actions_shown {
        line = line.push(actions(ctx, entry));
    }
    let main = main.push(line);
    if st.armed.as_deref() == Some(entry.id.as_str()) {
        main.push(delete_alert(ctx, entry)).into()
    } else {
        main.into()
    }
}

/// The small alert under an entry whose Delete was pressed: what is lost, and
/// the two ways out (Cancel, Delete). The second step of deleting.
fn delete_alert<'a>(ctx: Ctx<'a>, entry: &'a MemoryEntry) -> Element<'a, Message> {
    let t = ctx.theme;
    let p = &t.palette;
    let (surface, border) = (p.surface_raised, p.border_strong);
    let card = container(
        column![
            text(
                ctx.i18n
                    .t_with(
                        K::MemoryDeleteTitle,
                        &args!("title" => entry.title.as_str())
                    )
                    .into_owned()
            )
            .size(tokens::font_size::BODY)
            .font(fonts::ui_semibold())
            .color(t.inks.text.primary),
            text(ctx.t(K::MemoryDeleteBody).to_string())
                .size(tokens::font_size::LABEL)
                .color(t.inks.text.muted),
            row![
                btn::secondary(t, ctx.t(K::CreateCancel)).on_press(msg(Msg::Disarm)),
                btn::danger(t, ctx.t(K::MemoryDeleteConfirm))
                    .icon(Icon::Trash)
                    .on_press(msg(Msg::Delete(entry.id.clone()))),
            ]
            .spacing(tokens::space::MD),
        ]
        .spacing(tokens::space::MD),
    )
    .padding(tokens::space::XXL)
    .max_width(420.0)
    .style(move |_: &iced::Theme| container::Style {
        background: Some(Background::Color(surface)),
        border: Border {
            color: border,
            width: BORDER_WIDTH,
            radius: scale::radius::LG.into(),
        },
        ..container::Style::default()
    });
    container(card)
        .padding(Padding {
            top: tokens::space::MD,
            ..Padding::ZERO
        })
        .into()
}

/// What can be done to an entry: edit it, delete it (asking twice).
fn actions<'a>(ctx: Ctx<'a>, entry: &'a MemoryEntry) -> Element<'a, Message> {
    let t = ctx.theme;
    let mut line = row![
        btn::secondary(t, ctx.t(K::MemoryEdit))
            .icon(Icon::Pencil)
            .small()
            .on_press(msg(Msg::StartEdit(entry.id.clone()))),
    ]
    .spacing(tokens::space::SM)
    .align_y(Alignment::Center);
    let failed = t.palette.status.failed;
    let face = button(
        row![
            icon(Icon::Trash, tokens::size::ICON).color(failed),
            text(ctx.t(K::MemoryDelete).to_string())
                .size(tokens::font_size::UI)
                .font(fonts::ui_medium())
                .color(failed),
        ]
        .spacing(tokens::space::SM)
        .align_y(Alignment::Center),
    )
    .height(scale::size::CONTROL_SM)
    .padding(Padding::from([0.0, tokens::space::LG]))
    .style(move |_: &iced::Theme, status| {
        let pointed = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: pointed.then_some(Background::Color(t.palette.hover_overlay)),
            text_color: failed,
            border: Border {
                radius: scale::radius::SM.into(),
                ..Border::default()
            },
            ..button::Style::default()
        }
    })
    .on_press(msg(Msg::Arm(entry.id.clone())));
    line = line.push(
        focus::ring(t, face)
            .radius(scale::radius::SM)
            .on_activate(msg(Msg::Arm(entry.id.clone()))),
    );
    line.into()
}

/// "Written by Ava", "Updated Oct 21", "Created Oct 3", the folder: the
/// right column.
fn origin<'a>(ctx: Ctx<'a>, entry: &'a MemoryEntry, narrow: bool) -> Element<'a, Message> {
    let t = ctx.theme;
    let locale = ctx.i18n.locale();
    let line = |content: String| {
        text(content)
            .size(tokens::font_size::LABEL)
            .color(t.palette.text_muted)
            .wrapping(text::Wrapping::None)
    };
    let day = |at: u64, updated: bool| {
        let when = match short_day(at, now_ms(), locale) {
            ShortDay::Today => ctx
                .t(if updated {
                    K::MemoryUpdatedToday
                } else {
                    K::MemoryCreatedToday
                })
                .to_string(),
            ShortDay::On(day) => ctx
                .i18n
                .t_with(
                    if updated {
                        K::MemoryUpdatedAt
                    } else {
                        K::MemoryCreatedAt
                    },
                    &args!("date" => day),
                )
                .into_owned(),
        };
        let full = ctx
            .i18n
            .t_with(
                if updated {
                    K::MemoryUpdatedAt
                } else {
                    K::MemoryCreatedAt
                },
                &args!("date" => full_date(at, locale)),
            )
            .into_owned();
        tip::over(t, line(when), full)
    };

    let signer: Element<'a, Message> = match signer_name(&entry.agent) {
        None => line(ctx.t(K::MemoryByYou).to_string()).into(),
        Some(name) => row![
            line(ctx.t(K::MemoryWrittenBy).to_string()),
            text(name)
                .size(tokens::font_size::LABEL)
                .font(fonts::ui_medium())
                .color(t.palette.text)
                .wrapping(text::Wrapping::None),
        ]
        .spacing(tokens::space::XS)
        .into(),
    };
    let signer = tip::over(
        t,
        signer,
        ctx.i18n
            .t_with(K::MemoryByAgent, &args!("agent" => entry.agent.as_str()))
            .into_owned(),
    );

    let mut lines = column![signer].spacing(1);
    if was_updated(entry.created_at, entry.updated_at) {
        lines = lines.push(day(entry.updated_at, true));
    }
    lines = lines.push(day(entry.created_at, false));
    if !entry.workspace.is_empty() {
        lines = lines.push(tip::over(
            t,
            text(compact_path(&entry.workspace, PATH_CHARS))
                .font(fonts::mono())
                .size(tokens::font_size::MICRO - 0.5)
                .color(t.palette.text_muted)
                .wrapping(text::Wrapping::None),
            ctx.i18n
                .t_with(
                    K::MemoryFromWorkspace,
                    &args!("path" => entry.workspace.as_str()),
                )
                .into_owned(),
        ));
    }
    lines
        .align_x(if narrow {
            Alignment::Start
        } else {
            Alignment::End
        })
        .width(Length::Fill)
        .into()
}
