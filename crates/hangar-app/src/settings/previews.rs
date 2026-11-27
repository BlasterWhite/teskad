//! The settings' own pictures: the theme cards, the font cards, the terminal
//! preview and the projects folder field (the `ThemeGrid`, `FontGrid`,
//! `TerminalPreview` and picker of `schema.tsx`).
//!
//! A swatch alone says which colours a palette holds, not what working in it
//! looks like, so a theme is drawn as the window it would give: a navigator
//! column, a few lines of terminal output in the theme's own colours, and its
//! accent. These are the one place a view takes colours from a theme other
//! than the one the window wears, which is the point of a preview.

use hangar_i18n::APP_NAME;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use hangar_i18n::Key;
use iced::font::Family;
use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Background, Border, Color, Element, Font, Length, Padding};

use hangar_theme::style;
use hangar_theme::tokens::{font_size, radius, space};
use hangar_ui::{button as buttons, focus, input};

use super::Msg;
use super::change::{Change, Target};
use super::schema::FONT_STACKS;
use crate::ctx::Ctx;
use crate::message::Message;
use crate::state::types::CursorStyle;

fn change(target: Target, value: &str) -> Message {
    Message::Settings(Msg::Change(Change::text(target, value)))
}

// ---- Fonts -------------------------------------------------------------------------------------------

/// A `'static` copy of a family name, which `iced::Font` wants. Each distinct
/// name is leaked once; the set of fonts a person tries out is small, so this
/// stays bounded (hangar-term does the same for its own).
fn intern(name: &str) -> &'static str {
    static NAMES: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
    let mut names = NAMES.get_or_init(Default::default).lock().expect("names");
    if let Some(known) = names.iter().find(|known| **known == name) {
        return known;
    }
    let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
    names.push(leaked);
    leaked
}

/// Whether the font system has a family called `name`.
fn installed(name: &str) -> bool {
    let mut system = iced::advanced::graphics::text::font_system()
        .write()
        .expect("font system lock");
    system.raw().db().faces().any(|face| {
        face.families
            .iter()
            .any(|(family, _)| family.eq_ignore_ascii_case(name))
    })
}

/// The font a CSS stack gives: the first of its families the machine has, else
/// the system's monospace one, which is how the terminal itself resolves it
/// (`hangar_term::Font`). Asked of the font system once per stack, since
/// listing its faces is not free and a page draws several cards.
pub fn font_of(stack: &str) -> Font {
    static CACHE: OnceLock<Mutex<HashMap<String, Font>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(font) = cache.lock().expect("fonts").get(stack) {
        return *font;
    }
    let families = hangar_term::Font::from_css_stack(stack, 13.0, 1.3).families;
    let family = families
        .iter()
        .find(|name| installed(name))
        .map_or(Family::Monospace, |name| Family::Name(intern(name)));
    let font = Font {
        family,
        ..Font::DEFAULT
    };
    cache.lock().expect("fonts").insert(stack.to_string(), font);
    font
}

/// The cards, plus one for a stack that is none of them.
pub fn font_cards(current: &str, custom_label: &str) -> Vec<(String, String)> {
    let mut cards: Vec<(String, String)> = FONT_STACKS
        .iter()
        .map(|(name, stack)| (name.to_string(), stack.to_string()))
        .collect();
    if !FONT_STACKS.iter().any(|(_, stack)| *stack == current) {
        cards.push((custom_label.to_string(), current.to_string()));
    }
    cards
}

pub fn font_grid<'a>(ctx: Ctx<'a>) -> Element<'a, Message> {
    let t = ctx.theme;
    let c = &t.colors;
    let current = ctx.model.settings().font_family.clone();
    let cards = font_cards(&current, &ctx.t(Key::SettingsFontCustom));
    let (accent, line, bg) = (c.accent, c.line_strong, c.bg);
    let mut grid = column![].spacing(space::LG);
    for chunk in cards.chunks(3) {
        let mut line_row = row![].spacing(space::LG);
        for (name, stack) in chunk {
            let active = *stack == current;
            let label = text(name.clone())
                .size(font_size::CAPTION)
                .color(if active { c.accent_hi } else { c.fg_secondary });
            let sample = text("Aa Bb 0123 => != ->")
                .size(font_size::BODY)
                .font(font_of(stack));
            let face = container(column![label, sample].spacing(3))
                .padding(Padding::from([space::LG, space::XL - 2.0]))
                .width(Length::Fill)
                .style(move |_: &iced::Theme| container::Style {
                    background: Some(Background::Color(bg)),
                    border: Border {
                        radius: radius::MD.into(),
                        width: if active { 2.0 } else { 1.0 },
                        color: if active { accent } else { line },
                    },
                    ..container::Style::default()
                });
            let pick = change(Target::FontFamily, stack);
            let card = button(face)
                .padding(0)
                .width(Length::FillPortion(1))
                .style(style::button::plain(t))
                .on_press(pick.clone());
            line_row = line_row.push(focus::ring(t, card).radius(radius::MD).on_activate(pick));
        }
        // A short last row keeps its cards the width of the others.
        for _ in chunk.len()..3 {
            line_row = line_row.push(Space::new().width(Length::FillPortion(1)));
        }
        grid = grid.push(line_row);
    }
    grid.into()
}

// ---- The terminal -------------------------------------------------------------------------------------

/// `content` with `spacing` pixels after every character (the terminal's
/// `letterSpacing`; negative tightens). iced sets text with the font's own
/// advances and has no spacing to add to them, so a spaced line is a row of
/// its characters, which is also how the terminal draws them: one in each
/// cell. Without a spacing it is the one text it always was.
fn spaced_text<'a>(
    content: &str,
    size: f32,
    font: Font,
    line: iced::widget::text::LineHeight,
    color: Color,
    spacing: f32,
) -> Element<'a, Message> {
    let glyph = |content: String| {
        text(content)
            .size(size)
            .font(font)
            .line_height(line)
            .color(color)
    };
    if spacing == 0.0 {
        return glyph(content.to_owned()).into();
    }
    let glyphs: Vec<Element<'a, Message>> = content
        .chars()
        .map(|c| glyph(c.to_string()).into())
        .collect();
    // The spacing follows the last character too, as it does in a cell (a
    // tightening one is not given back: a pixel of a preview).
    container(iced::widget::Row::with_children(glyphs).spacing(spacing))
        .padding(Padding {
            right: spacing.max(0.0),
            ..Padding::ZERO
        })
        .into()
}

/// Typography, cursor and padding, shown on the live theme.
pub fn terminal_preview<'a>(ctx: Ctx<'a>) -> Element<'a, Message> {
    let t = ctx.theme;
    let c = &t.colors;
    let settings = ctx.model.settings();
    let font = font_of(&settings.font_family);
    let size = settings.font_size as f32;
    let line = iced::widget::text::LineHeight::Relative(settings.line_height as f32);
    // The terminal's own reading of the setting (a number of pixels, and a
    // value that is not one counts as none), so the picture spaces what the
    // panes will.
    let spacing = crate::panes::letter_spacing_of(settings);
    // A cell of a monospaced face is about 0.6 of its size wide, and the
    // spacing is part of it.
    let cell = size * 0.6 + spacing;
    let tall = size * 1.1;
    let accent = c.accent;

    let cursor = {
        let (width, height) = match settings.cursor_style {
            CursorStyle::Bar => (2.0, tall),
            CursorStyle::Block => (cell, tall),
            CursorStyle::Underline => (cell, 2.0),
        };
        container(Space::new())
            .width(width)
            .height(height)
            .style(move |_: &iced::Theme| container::Style {
                background: Some(Background::Color(accent)),
                ..container::Style::default()
            })
    };
    let run = |content: &str, color: Color| spaced_text(content, size, font, line, color, spacing);
    let prompt = row![run("❯", c.success), run(" claude --resume", c.fg)];
    let dim = run(
        &format!("{APP_NAME} · 4 panes · session restored"),
        c.fg_subtle,
    );
    let last = row![
        run("0123456789 => != -> |&", c.fg),
        container(cursor).padding(Padding {
            left: 1.0,
            top: size * 0.15,
            ..Padding::ZERO
        }),
    ]
    .align_y(Alignment::Start);

    let body = column![prompt, dim, last];
    let border = c.line_strong;
    let bg = c.bg;
    container(body)
        .padding(settings.padding as f32)
        .width(Length::Fill)
        .clip(true)
        .style(move |_: &iced::Theme| container::Style {
            background: Some(Background::Color(bg)),
            border: Border {
                radius: radius::LG.into(),
                width: 1.0,
                color: border,
            },
            ..container::Style::default()
        })
        .into()
}

// ---- The projects folder ------------------------------------------------------------------------------

/// The path field and its Browse button (`.picker`). The path is stored raw
/// on each keystroke.
pub fn projects_folder<'a>(ctx: Ctx<'a>) -> Element<'a, Message> {
    let t = ctx.theme;
    let path = ctx
        .model
        .settings()
        .projects_folder
        .as_deref()
        .unwrap_or("");
    let placeholder = match ctx.i18n.t(Key::SettingsProjectsFolderPlaceholder) {
        std::borrow::Cow::Borrowed(words) => words,
        std::borrow::Cow::Owned(_) => "",
    };
    row![
        crate::keymap::guard(
            ctx,
            input::text_field(t, path)
                .placeholder(placeholder)
                .mono()
                .on_input(|text| change(Target::ProjectsFolder, &text)),
        ),
        buttons::secondary(t, ctx.t(Key::CreateBrowse).into_owned())
            .size(buttons::Size::Large)
            .on_press(Message::Settings(Msg::BrowseProjects)),
    ]
    .spacing(space::MD)
    .align_y(Alignment::Center)
    .into()
}

/// Where the Browse dialog opens: the folder already chosen, else the home.
pub fn browse_start(current: Option<&str>) -> Option<std::path::PathBuf> {
    current
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_dir())
        .or_else(dirs_home)
}

fn dirs_home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_dir())
}

#[cfg(test)]
mod tests;
