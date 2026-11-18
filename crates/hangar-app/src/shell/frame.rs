//! The window's layout (App.tsx:1516, inventory §1.11):
//!
//! ```text
//! navigator │ toolbar
//!           │ ───────────────────────────────────────────────────────────
//!           │ main: terminals · board · memory · stats │ preview │ dock │ inspector
//!           │ ───────────────────────────────────────────────────────────
//!           │ status bar
//! + drawers over the right of the work area, the preview expanded over all of it
//! + floating: update banner, consent card, voice HUD, notice, chord hint
//! + overlays, top one last
//! ```
//!
//! The frame gives each region its size and its surface (the theme's chrome
//! colours, the hairlines between them); the region's content is its owner's
//! view. A workspace window is the same frame without the navigator. Where
//! each side panel goes (docked, drawer, hidden) is `ctx.panels()`
//! (side_panels.rs).
//!
//! A window the desktop shows through (the window opacity of the settings
//! under 1, on Linux: `Theme::is_translucent`) changes who paints the page: two translucent coats on top of each other
//! add up to an opaque one, so the window's surface paints nothing and each
//! region lays its own surface straight on the desktop, at the window's
//! alpha (the navigator, the toolbar, the status bar, the separators, a
//! docked side panel, the work area under the panes). What floats over them
//! stays opaque. On a solid window all of it is drawn as it always was.

use iced::widget::{Space, column, container, mouse_area, opaque, row, rule, stack};
use iced::{Alignment, Element, Length};

use hangar_theme::{Theme, style, tokens};
use hangar_ui::layer;

use crate::ctx::Ctx;
use crate::message::Message;
use crate::shell::title_bar::{self, Corners};
use crate::shell::{Action, BoardVariant, View, WindowKind, overlay, placeholder};
use crate::state::Workspace;

/// The main window.
pub fn main<'a>(ctx: Ctx<'a>) -> Element<'a, Message> {
    if !ctx.model.live.hydrated {
        return window_surface(ctx, placeholder::boot(ctx));
    }
    let t = ctx.theme;
    let p = ctx.pkg;
    let body: Element<'a, Message> = match (ctx.workspace(), ctx.model.active_workspace()) {
        (Some(ws), _) => workspace_body(ctx, ws),
        (None, Some(ws)) if ws.detached == Some(true) => {
            with_bare_band(ctx, placeholder::elsewhere(ctx, ws))
        }
        _ => with_bare_band(ctx, placeholder::no_workspace(ctx)),
    };
    let mut frame = row![].height(Length::Fill);
    if ctx.panels().navigator {
        frame = frame
            .push(
                container(crate::navigator::view(&p.navigator, ctx))
                    .width(tokens::size::SIDEBAR_WIDTH)
                    .height(Length::Fill)
                    .style(title_bar::rounded_style(
                        ctx,
                        Corners::LEFT,
                        style::container::see_through(t, style::container::sidebar(t)),
                    )),
            )
            .push(vertical_sep(ctx));
    }
    frame = frame.push(container(body).width(Length::Fill).height(Length::Fill));
    title_bar::outline(ctx, layered(ctx, frame_surface(ctx, frame.into())))
}

/// A placeholder under the band Hangar's title bar needs where there is no
/// toolbar to be it, on the page.
fn with_bare_band<'a>(ctx: Ctx<'a>, body: Element<'a, Message>) -> Element<'a, Message> {
    let bottom = Corners::bottom(ctx);
    match title_bar::bare_band(ctx) {
        Some(band) => column![band, horizontal_sep(ctx), on_page(ctx, body, bottom)].into(),
        None => {
            let top = Corners::top(ctx);
            let corners = Corners {
                top_left: top.top_left,
                top_right: top.top_right,
                ..bottom
            };
            on_page(ctx, body, corners)
        }
    }
}

/// `content` on the page, which it paints itself on a window the desktop
/// shows through (rounded at `corners` of the window), and leaves to the
/// window's surface on a solid one.
fn on_page<'a>(
    ctx: Ctx<'a>,
    content: Element<'a, Message>,
    corners: Corners,
) -> Element<'a, Message> {
    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(title_bar::rounded_style(ctx, corners, page(ctx.theme)))
        .into()
}

/// What a region that shows the page is filled with: the page at the
/// window's alpha on a translucent window, where the window's surface paints
/// nothing; nothing on a solid one, where it does.
fn page(t: &Theme) -> impl Fn(&iced::Theme) -> container::Style + '_ {
    move |theme| {
        if t.is_translucent() {
            style::container::backdrop(t)(theme)
        } else {
            container::Style::default()
        }
    }
}

/// A workspace's own window: the main window's frame for that workspace,
/// without the navigator.
pub fn workspace_window<'a>(ctx: Ctx<'a>, ws: &'a Workspace) -> Element<'a, Message> {
    if !ctx.model.live.hydrated {
        return window_surface(ctx, placeholder::boot(ctx));
    }
    title_bar::outline(
        ctx,
        layered(ctx, frame_surface(ctx, workspace_body(ctx, ws))),
    )
}

/// The width of the row the terminals share with the side panels, in a
/// window `width` wide: what `side_panels::place` places in.
pub fn body_width(width: f32, navigator: bool) -> f32 {
    if navigator {
        (width - tokens::size::SIDEBAR_WIDTH - tokens::BORDER_WIDTH).max(0.0)
    } else {
        width
    }
}

/// Toolbar, main area with its side panels, status bar.
fn workspace_body<'a>(ctx: Ctx<'a>, ws: &'a Workspace) -> Element<'a, Message> {
    let t = ctx.theme;
    let p = ctx.pkg;
    let panels = ctx.panels();
    // The toolbar's buttons open and close the drawers themselves: the
    // pointer over it is not "outside" a drawer (shell/drawer.rs). It is
    // the title bar too, when Hangar draws that.
    let toolbar = mouse_area(title_bar::band(
        ctx,
        container(crate::toolbar::view(&p.toolbar, ctx))
            .width(Length::Fill)
            .height(tokens::size::TOOLBAR_HEIGHT)
            .style(title_bar::rounded_style(
                ctx,
                Corners::top(ctx),
                style::container::see_through(t, style::container::toolbar(t)),
            )),
    ))
    .on_enter(Message::Shell(Action::PointerOverToolbar(true)))
    .on_exit(Message::Shell(Action::PointerOverToolbar(false)));
    let status = container(crate::status_bar::view(&p.status_bar, ctx))
        .width(Length::Fill)
        .height(tokens::size::STATUS_BAR_HEIGHT)
        .style(title_bar::rounded_style(
            ctx,
            Corners::bottom(ctx),
            style::container::see_through(t, style::container::statusbar(t)),
        ));

    let mut body = row![work_area(ctx, ws)].height(Length::Fill);
    let inspector = panels.inspector;
    if inspector.shown && !inspector.drawer {
        body = body.push(vertical_sep(ctx)).push(side(
            ctx,
            crate::inspector::view(&p.inspector, ctx),
            inspector.width,
        ));
    }
    let mut layers = stack![body].width(Length::Fill).height(Length::Fill);
    if inspector.shown && inspector.drawer {
        layers = layers.push(drawer(
            ctx,
            crate::inspector::view(&p.inspector, ctx),
            inspector.width,
        ));
    }

    column![
        toolbar,
        horizontal_sep(ctx),
        layers,
        horizontal_sep(ctx),
        status
    ]
    .into()
}

/// Region `main`, with the board dock beside it, or over it as a drawer.
/// What the previews show is in the grid, as panes (`preview::panes`).
fn work_area<'a>(ctx: Ctx<'a>, ws: &'a Workspace) -> Element<'a, Message> {
    let p = ctx.pkg;
    let panels = ctx.panels();
    let terminals = ctx.view() == View::Terminals;

    // Every workspace opened once stays drawn, hidden, whatever the view:
    // the grids keep their place in the tree, so their widgets keep their
    // state, and their terminals keep consuming output (PACKAGES.md, P5). A
    // workspace window draws its own workspace's alone; the main window
    // never draws one shown in a window of its own.
    let own_window = matches!(
        ctx.model.live.windows.get(&ctx.window),
        Some(WindowKind::Workspace(_))
    );
    // A workspace shown as the chat view keeps its grid in the tree, hidden
    // like any other off screen, and the chat view goes over it.
    let chat = terminals && crate::chat_view::shown(ctx.model, ws);
    let mut main = stack![].width(Length::Fill).height(Length::Fill);
    if own_window {
        main = main.push(crate::panes::view_workspace(
            &p.panes,
            ctx,
            &ws.id,
            !terminals || chat,
        ));
    } else {
        for id in &ctx.model.ui.opened_ids {
            let Some(opened) = ctx.model.workspace(id) else {
                continue;
            };
            if opened.detached == Some(true) {
                continue;
            }
            let hidden = opened.id != ws.id || !terminals || chat;
            main = main.push(crate::panes::view_workspace(
                &p.panes, ctx, &opened.id, hidden,
            ));
        }
    }
    match ctx.view() {
        View::Terminals if chat => main = main.push(crate::chat_view::view(ctx, ws)),
        View::Terminals => {}
        View::Board => main = main.push(crate::board::view(&p.board, ctx, BoardVariant::Full)),
        View::Memory => main = main.push(crate::memory::view(&p.memory, ctx)),
        View::Stats => main = main.push(crate::stats::view(&p.stats, ctx)),
    }

    // The panes' terminals leave their default background to this, on a
    // window the desktop shows through.
    let mut area = row![
        container(main)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(page(ctx.theme))
    ];
    let dock = panels.dock;
    if dock.shown && !dock.drawer {
        let board = crate::board::view(
            &p.board,
            ctx,
            BoardVariant::Dock {
                width: dock.width,
                drawer: false,
            },
        );
        area = area
            .push(vertical_sep(ctx))
            .push(side(ctx, board, dock.width));
    }

    let mut layers = stack![area].width(Length::Fill).height(Length::Fill);
    if dock.shown && dock.drawer {
        let board = crate::board::view(
            &p.board,
            ctx,
            BoardVariant::Dock {
                width: dock.width,
                drawer: true,
            },
        );
        layers = layers.push(drawer(ctx, board, dock.width));
    }
    layers.into()
}

/// A docked side panel, `width` wide.
fn side<'a>(ctx: Ctx<'a>, content: Element<'a, Message>, width: f32) -> Element<'a, Message> {
    let t = ctx.theme;
    container(content)
        .width(width)
        .height(Length::Fill)
        .style(style::container::see_through(t, style::container::panel(t)))
        .into()
}

/// A side panel as a drawer over the right of what is under it. Opaque, so a
/// press on it never reaches the terminal underneath; its mouse area tells
/// the shell when the pointer is over it, for "a press outside closes it".
fn drawer<'a>(ctx: Ctx<'a>, content: Element<'a, Message>, width: f32) -> Element<'a, Message> {
    let panel = mouse_area(
        container(content)
            .width(width)
            .height(Length::Fill)
            .style(style::container::popover(ctx.theme)),
    )
    .on_enter(Message::Shell(Action::PointerOverDrawer(true)))
    .on_exit(Message::Shell(Action::PointerOverDrawer(false)));
    container(opaque(panel))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::End)
        .into()
}

/// The floating pieces and the overlays over a window's frame.
fn layered<'a>(ctx: Ctx<'a>, frame: Element<'a, Message>) -> Element<'a, Message> {
    let p = ctx.pkg;
    let mut layers = stack![frame];
    let focused = ctx.model.is_focused_window(ctx.window);

    // The update banner and the consent card are the main window's only.
    if ctx.model.live.windows.get(&ctx.window) == Some(&WindowKind::Main) {
        layers = layers
            .push(
                container(crate::update_banner::view(&p.update_banner, ctx))
                    .width(Length::Fill)
                    .align_x(Alignment::Center),
            )
            .push(
                container(crate::consent_card::view(&p.consent_card, ctx))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .padding(tokens::space::XXL)
                    .align_x(Alignment::End)
                    .align_y(Alignment::End),
            );
    }

    // The voice HUD, the notice and the chord hint share the bottom centre,
    // above the status bar, in the window the person is in.
    if focused {
        let mut bottom = column![crate::voice::hud(&p.voice, ctx)]
            .spacing(tokens::space::SM)
            .align_x(Alignment::Center);
        if let Some(notice) = crate::shell::notice::view(ctx) {
            bottom = bottom.push(notice);
        }
        if let Some(hint) = crate::keymap::chord_hint(ctx) {
            bottom = bottom.push(hint);
        }
        layers = layers.push(
            container(column![
                bottom,
                Space::new().height(tokens::size::STATUS_BAR_HEIGHT + 12.0)
            ])
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::Center)
            .align_y(Alignment::End),
        );
    }

    // Only the overlay in front takes the pointer and the keys. iced hands an
    // event to every overlay of the window, the lowest first: a dialog under
    // another would answer a click meant for the one in front (its veil
    // closing the front one instead, `CloseOverlay` closing the top, or a
    // widget of its own under the click taking it). The ones under are drawn
    // as they are, inert; each overlay keeps the wrapper, so that the widget
    // tree keeps its shape as overlays open and close over it.
    let overlays = ctx.model.overlays_in(ctx.window);
    let front = overlays.len().saturating_sub(1);
    for (at, overlay) in overlays.iter().enumerate() {
        layers = layers.push(layer::inert(overlay::view(ctx, overlay)).when(at < front));
    }
    // Over everything, an open sheet's backdrop included: a window is
    // resized from its edges whatever it shows.
    if let Some(edges) = title_bar::edges(ctx) {
        layers = layers.push(edges);
    }
    layers.into()
}

/// The page alone over the whole window (while the app starts), in the
/// theme's colours and at the window's alpha, rounded at the window's
/// corners when they are.
fn window_surface<'a>(ctx: Ctx<'a>, content: Element<'a, Message>) -> Element<'a, Message> {
    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(title_bar::rounded_style(
            ctx,
            Corners::ALL,
            style::container::backdrop(ctx.theme),
        ))
        .into()
}

/// What the window's regions lie on: the page on a solid window, rounded at
/// its corners; only the theme's text colour on a translucent one, whose
/// regions each paint their own surface (see the module's docs).
fn frame_surface<'a>(ctx: Ctx<'a>, content: Element<'a, Message>) -> Element<'a, Message> {
    let t = ctx.theme;
    let surface = move |theme: &iced::Theme| {
        if t.is_translucent() {
            style::container::plain(t)(theme)
        } else {
            style::container::window(t)(theme)
        }
    };
    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(title_bar::rounded_style(ctx, Corners::ALL, surface))
        .into()
}

fn vertical_sep<'a>(ctx: Ctx<'a>) -> Element<'a, Message> {
    rule::vertical(tokens::BORDER_WIDTH)
        .style(style::rule::sep_see_through(ctx.theme))
        .into()
}

fn horizontal_sep<'a>(ctx: Ctx<'a>) -> Element<'a, Message> {
    rule::horizontal(tokens::BORDER_WIDTH)
        .style(style::rule::sep_see_through(ctx.theme))
        .into()
}

#[cfg(test)]
mod tests {
    //! The window at a window opacity under 1, drawn over nothing at all:
    //! the alpha of each pixel says where the desktop would show.
    //! `HANGAR_P8_SNAPSHOTS=<folder>` keeps the pictures there, each twice:
    //! as on a dark desktop (`<name>-<renderer>.png`) and with the desktop
    //! left out (`<name>-desktop-<renderer>.png`, translucent).

    use std::borrow::Cow;
    use std::path::{Path, PathBuf};

    use hangar_i18n::Locale;
    use iced::theme::Palette;
    use iced::{Color, Rectangle, Settings};
    use iced_test::Simulator;

    use super::*;
    use crate::shell::windows::MAIN_SIZE;
    use crate::shell::{Overlay, SettingsCategory};
    use crate::workspace_dialog::testing::Harness;

    /// A prompt, a status, and the six colours as backgrounds of their own,
    /// which stay opaque.
    const SAMPLE: &[u8] = b"\x1b[1;35m~/dev/hangar\x1b[0m on \x1b[34mmain\x1b[0m\r\n\
        $ git status --short\r\n \x1b[31mM\x1b[0m crates/hangar-theme/src/data.rs\r\n\
        \x1b[32m??\x1b[0m notes.md\r\n\
        \x1b[30;41m red \x1b[42m green \x1b[43m yellow \x1b[44m blue \x1b[45m pink \x1b[46m cyan \x1b[0m\r\n$ ";

    /// The harness's model in `theme` at the window `opacity` of the
    /// settings, a few lines in each pane of the active workspace.
    fn glass(locale: Locale, theme: &str, opacity: f64) -> (Harness, &'static Theme) {
        let mut h =
            Harness::new(locale, theme).edit(|state| state.settings.window_opacity = opacity);
        let theme = h.model.theme();
        let ws = h.model.state().workspaces[0].clone();
        h.pkg.panes.mount_for_test(&ws);
        for pane in &ws.panes {
            h.pkg.panes.print_for_test(&pane.id, theme, SAMPLE);
        }
        (h, theme)
    }

    /// `element` in a window the size the shell opens the main one at.
    fn show<'a>(element: Element<'a, Message>) -> Simulator<'a, Message> {
        let settings = Settings {
            fonts: vec![Cow::Borrowed(hangar_theme::fonts::INTER_VARIABLE)],
            default_font: hangar_theme::fonts::ui(),
            ..Settings::default()
        };
        Simulator::with_size(settings, MAIN_SIZE, element)
    }

    /// What `ui` draws over no desktop at all, read back: kept in the
    /// snapshot folder when there is one, else in a scratch one.
    fn over_nothing(
        ui: &mut Simulator<'_, Message>,
        theme: &Theme,
        name: &str,
    ) -> ::image::RgbaImage {
        // The simulator clears to the page of the extended palette.
        let iced = hangar_theme::to_iced(theme);
        let mut extended = *iced.extended_palette();
        extended.background.base.color = Color::TRANSPARENT;
        let palette = Palette {
            background: Color::TRANSPARENT,
            ..iced.palette()
        };
        let desktop = iced::Theme::custom_with_fn("desktop", palette, move |_| extended);
        let scratch = tempfile::tempdir().expect("a scratch folder");
        let folder = std::env::var_os("HANGAR_P8_SNAPSHOTS")
            .map_or_else(|| scratch.path().to_path_buf(), PathBuf::from);
        std::fs::create_dir_all(&folder).expect("the folder");
        let stem = format!("{name}-desktop");
        let written = |folder: &Path| -> Vec<PathBuf> {
            std::fs::read_dir(folder)
                .into_iter()
                .flatten()
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|file| file.to_str())
                        .is_some_and(|file| {
                            file.starts_with(&format!("{stem}-")) && file.ends_with(".png")
                        })
                })
                .collect()
        };
        // The simulator compares with a picture that is there already: this
        // one is wanted afresh.
        for old in written(&folder) {
            let _ = std::fs::remove_file(old);
        }
        ui.snapshot(&desktop)
            .expect("the window can be drawn")
            .matches_image(folder.join(&stem))
            .expect("the picture can be written");
        let path = written(&folder).pop().expect("the picture");
        ::image::open(path)
            .expect("the picture can be read")
            .into_rgba8()
    }

    /// The share of the pixels of `area` (in the window's units) whose alpha
    /// is within `alphas`.
    fn share(picture: &::image::RgbaImage, area: Rectangle, alphas: (u8, u8)) -> f32 {
        let scale = picture.width() as f32 / MAIN_SIZE.width;
        let (x0, y0) = ((area.x * scale) as u32, (area.y * scale) as u32);
        let (x1, y1) = (
            ((area.x + area.width) * scale) as u32,
            ((area.y + area.height) * scale) as u32,
        );
        let mut inside = 0_u32;
        let mut total = 0_u32;
        for y in y0..y1.min(picture.height()) {
            for x in x0..x1.min(picture.width()) {
                let alpha = picture.get_pixel(x, y).0[3];
                total += 1;
                if (alphas.0..=alphas.1).contains(&alpha) {
                    inside += 1;
                }
            }
        }
        inside as f32 / total.max(1) as f32
    }

    fn area(x: f32, y: f32, width: f32, height: f32) -> Rectangle {
        Rectangle::new(iced::Point::new(x, y), iced::Size::new(width, height))
    }

    /// The alpha of a surface drawn at `opacity`, give or take the rounding
    /// of eight bits and the veil over a pane without the focus.
    fn see_through(opacity: f32) -> (u8, u8) {
        let alpha = opacity * 255.0;
        ((alpha - 6.0) as u8, (alpha + 6.0).min(255.0) as u8)
    }
    const OPAQUE: (u8, u8) = (255, 255);

    /// The main window in `theme` at `opacity`: the desktop shows under the
    /// navigator and the terminals on Linux, not under a pane's header, and
    /// nowhere through a hole. Its colours are the theme's.
    fn main_window_lets_the_desktop_through(theme: &str, opacity: f64, name: &str) {
        let (h, theme) = glass(Locale::En, theme, opacity);
        assert_eq!(
            theme.colors,
            hangar_theme::by_id_or_default(theme.id).colors
        );
        let mut ui = show(main(h.ctx()));
        h.shot(&mut ui, name);
        let picture = over_nothing(&mut ui, theme, name);

        let navigator = area(0.0, 300.0, tokens::size::SIDEBAR_WIDTH - 2.0, 400.0);
        // The first pane's terminal, below the lines printed in it.
        let terminal = area(300.0, 300.0, 300.0, 100.0);
        // Its header, a bar of text that stays opaque.
        let head = area(
            300.0,
            tokens::size::TOOLBAR_HEIGHT + 4.0,
            300.0,
            tokens::size::PANE_HEAD_HEIGHT - 8.0,
        );
        let through = see_through(theme.window_alpha());
        // A solid window everywhere but on Linux.
        assert_eq!(theme.is_translucent(), cfg!(target_os = "linux"));
        for (part, region) in [("navigator", navigator), ("terminal", terminal)] {
            let got = share(&picture, region, through);
            assert!(got > 0.6, "{part}: {got}");
        }
        let part = share(&picture, head, OPAQUE);
        assert!(part > 0.9, "head: {part}");
        // No hole anywhere but the window's rounded corners, inside the
        // hairline Hangar's title bar outlines the window with on Linux (a
        // faint line over the desktop whatever the theme).
        let whole = area(2.0, 2.0, MAIN_SIZE.width - 4.0, MAIN_SIZE.height - 4.0);
        let holes = share(&picture, whole, (0, through.0 - 1));
        assert!(holes < 0.001, "holes: {holes}");
    }

    #[test]
    fn graphite_at_0_9_lets_the_desktop_through_the_page_and_the_bars() {
        main_window_lets_the_desktop_through("graphite", 0.9, "frame-main-en-graphite-90");
    }

    #[test]
    fn tokyo_night_at_0_85_lets_the_desktop_through_the_page_and_the_bars() {
        main_window_lets_the_desktop_through("tokyo-night", 0.85, "frame-main-en-tokyo-night-85");
    }

    #[test]
    fn by_default_the_window_is_as_opaque_as_it_always_was() {
        let (h, theme) = glass(Locale::En, "graphite", 1.0);
        assert!(!theme.is_translucent());
        let mut ui = show(main(h.ctx()));
        let picture = over_nothing(&mut ui, theme, "frame-main-en-graphite-100");
        let inside = area(2.0, 2.0, MAIN_SIZE.width - 4.0, MAIN_SIZE.height - 4.0);
        let part = share(&picture, inside, OPAQUE);
        assert!(part > 0.999, "opaque: {part}");
    }

    #[test]
    fn a_sheet_over_a_see_through_window_is_opaque_and_so_is_its_veil_almost() {
        let (mut h, theme) = glass(Locale::Fr, "tokyo-night", 0.85);
        h.model.ui.overlays = vec![Overlay::Settings(SettingsCategory::Appearance)];
        let mut ui = show(main(h.ctx()));
        let name = "frame-settings-appearance-fr-tokyo-night-85";
        h.shot(&mut ui, name);
        let picture = over_nothing(&mut ui, theme, name);
        // The middle of the sheet.
        let sheet = area(
            MAIN_SIZE.width / 2.0 - 100.0,
            MAIN_SIZE.height / 2.0 - 50.0,
            200.0,
            100.0,
        );
        let part = share(&picture, sheet, OPAQUE);
        assert!(part > 0.99, "sheet: {part}");
        // The veil darkens the window and lets less of the desktop through.
        let corner = area(4.0, MAIN_SIZE.height / 2.0, 20.0, 20.0);
        let part = share(&picture, corner, (see_through(theme.window_alpha()).0, 255));
        assert!(part > 0.99, "veil: {part}");
    }

    #[test]
    fn the_new_workspace_dialog_draws_over_a_see_through_window() {
        let (mut h, theme) = glass(Locale::En, "graphite", 0.9);
        h.model.ui.overlays = vec![Overlay::WorkspaceDialog];
        let mut ui = show(main(h.ctx()));
        let name = "frame-workspace-dialog-en-graphite-90";
        h.shot(&mut ui, name);
        let picture = over_nothing(&mut ui, theme, name);
        let middle = area(
            MAIN_SIZE.width / 2.0 - 50.0,
            MAIN_SIZE.height / 2.0 - 25.0,
            100.0,
            50.0,
        );
        let part = share(&picture, middle, OPAQUE);
        assert!(part > 0.95, "dialog: {part}");
    }
}
