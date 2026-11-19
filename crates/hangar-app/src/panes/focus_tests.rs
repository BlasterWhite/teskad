//! The keyboard between the terminals and the text fields, through the whole
//! main window (`shell::frame::main`) driven with real events, the app's loop
//! run by hand between them: each message goes to the package that handles
//! it, the panes give the keyboard out as their sync does (`give_keyboard`),
//! and the window is built again over its widgets' state (iced's cache), then
//! drawn, as the runtime does after every update. A window built from scratch
//! at every step (as `iced_test`'s simulator is) would never see a field lose
//! the keyboard as it is drawn again, which was half of the bug.
//!
//! The shell itself is not run (it needs the core): where a message asks it
//! to move the keyboard, the harness does what `App::focus_pane`,
//! `App::open_overlay` and `App::close_overlay` do with the focus, and
//! nothing else.

use std::borrow::Cow;
use std::time::Instant;

use hangar_i18n::{Key, Locale, Translator, args};
use iced::advanced::clipboard;
use iced::advanced::renderer::Headless;
use iced::advanced::widget::Operation;
use iced::advanced::widget::operation::{self, Outcome};
use iced::keyboard::key::Named;
use iced::widget::Id;
use iced::{Event, Point, Rectangle, mouse, window};
use iced_runtime::Action;
use iced_runtime::user_interface::{Cache, UserInterface};
use iced_test::selector::{Candidate, Selector};

use super::*;
use crate::app::Packages;
use crate::mobile::desk::ChatView;
use crate::navigator::testing::{WINDOW, ctx, model_of, pane, workspace};
use crate::shell::{Overlay, Region, View};
use crate::state::types::{AgentId, AppState, PaneRole};
use crate::state::{Patch, Settings, StoreAction};

const WS: &str = "w";
const AVA: &str = "ava";

/// The main window, drawn and driven.
struct Window {
    model: Model,
    pkg: Packages,
    renderer: iced::Renderer,
    /// The widgets' state between two builds, as the runtime keeps it.
    cache: Option<Cache>,
    cursor: mouse::Cursor,
    /// What the terminals sent their programs, in order.
    typed: Vec<(PaneId, Vec<u8>)>,
    /// Every message the window sent, as words.
    said: Vec<String>,
    /// The pane selected at the start.
    pane: PaneId,
}

impl Window {
    /// A workspace of one shell, Ava, on screen in the main window, its pane
    /// selected: its terminal has the keyboard.
    fn new() -> Window {
        Window::of(
            AppState {
                workspaces: vec![workspace(
                    WS,
                    "hangar",
                    None,
                    vec![pane(AVA, "Ava", AgentId::Shell)],
                )],
                active_workspace_id: Some(WS.into()),
                ..AppState::default()
            },
            AVA,
        )
    }

    /// Workspace `WS` of `state` on screen in the main window, its pane
    /// `selected` the focused one, its terminal with the keyboard.
    fn of(state: AppState, selected: &str) -> Window {
        let mut model = model_of(state);
        model.ui.view = View::Terminals;
        model
            .ui
            .focus_by_workspace
            .insert(WS.into(), selected.into());
        model.ui.focus = Some(Focus::Terminal(selected.into()));
        let mut pkg = Packages::default();
        pkg.panes
            .mount_for_test(model.workspace(WS).expect("the workspace"));
        iced::advanced::graphics::text::font_system()
            .write()
            .expect("the font system")
            .load_font(Cow::Borrowed(hangar_theme::fonts::INTER_VARIABLE));
        let backend = std::env::var("ICED_TEST_BACKEND").ok();
        let renderer = iced::futures::executor::block_on(<iced::Renderer as Headless>::new(
            hangar_theme::fonts::ui(),
            iced::Settings::default().default_text_size,
            backend.as_deref(),
        ))
        .expect("a renderer");
        let mut window = Window {
            model,
            pkg,
            renderer,
            cache: None,
            cursor: mouse::Cursor::Unavailable,
            typed: Vec::new(),
            said: Vec::new(),
            pane: selected.into(),
        };
        window.settle();
        assert!(
            window.terminal_has_keyboard(),
            "a pane selected: its terminal has the keyboard"
        );
        window
    }

    /// Runs `f` over the window as the model and the packages draw it now,
    /// built over the widgets' state the last build left.
    fn with_ui<R>(
        &mut self,
        f: impl FnOnce(
            &mut UserInterface<'_, Message, iced::Theme, iced::Renderer>,
            &mut iced::Renderer,
        ) -> R,
    ) -> R {
        let Window {
            model,
            pkg,
            renderer,
            cache,
            ..
        } = self;
        let view = crate::shell::frame::main(ctx(model, pkg));
        let mut ui = UserInterface::build(view, WINDOW, cache.take().unwrap_or_default(), renderer);
        let result = f(&mut ui, renderer);
        *cache = Some(ui.into_cache());
        result
    }

    /// Hands `events` to the window, then runs the app's loop until nothing
    /// more happens.
    fn send(&mut self, events: impl IntoIterator<Item = Event>) {
        let events: Vec<Event> = events.into_iter().collect();
        for message in self.events(&events) {
            self.handle(message);
        }
        self.settle();
    }

    /// The messages the window sends for `events`.
    fn events(&mut self, events: &[Event]) -> Vec<Message> {
        let cursor = self.cursor;
        self.with_ui(|ui, renderer| {
            let mut messages = Vec::new();
            let _ = ui.update(
                events,
                cursor,
                renderer,
                &mut clipboard::Null,
                &mut messages,
            );
            messages
        })
    }

    /// A message, to the package that handles it.
    fn handle(&mut self, message: Message) {
        self.said.push(format!("{message:?}"));
        let task = match message {
            Message::Keymap(msg) => crate::keymap::apply(&mut self.model, msg),
            Message::StatusBar(msg) => {
                crate::status_bar::step(&mut self.pkg.status_bar, &mut self.model, msg)
            }
            Message::Panes(Msg::Term(pane, hangar_term::Action::Input(bytes))) => {
                self.typed.push((pane, bytes));
                Task::none()
            }
            Message::Panes(Msg::Focus(ws, pane)) => self.focus_pane(ws, pane, true),
            Message::Panes(Msg::Select(ws, pane)) => self.focus_pane(ws, pane, false),
            Message::Panes(Msg::Search(pane, msg)) => search_step(&mut self.pkg.panes, &pane, msg),
            _ => Task::none(),
        };
        self.run(task);
    }

    /// What the harness acts on: the rest (a grid that measured itself, a
    /// terminal asking for its size) changes nothing here.
    fn acts_on(message: &Message) -> bool {
        matches!(
            message,
            Message::Keymap(_)
                | Message::StatusBar(_)
                | Message::Panes(
                    Msg::Term(_, hangar_term::Action::Input(_))
                        | Msg::Focus(..)
                        | Msg::Select(..)
                        | Msg::Search(..)
                )
        )
    }

    /// What the shell does with the keyboard for `Intent::FocusPane`, which a
    /// press in a pane or a shortcut asks for (`App::focus_pane`): the pane
    /// becomes its workspace's focused one, and with `terminal` its terminal
    /// has the keyboard.
    fn focus_pane(&mut self, ws: WsId, pane: PaneId, terminal: bool) -> Task<Message> {
        self.model.ui.focus_by_workspace.insert(ws, pane.clone());
        if terminal {
            self.model.ui.focus = Some(Focus::Terminal(pane));
        }
        Task::none()
    }

    /// Runs a task as the runtime does: its operations on the window, its
    /// messages to their packages.
    fn run(&mut self, task: Task<Message>) {
        let Some(stream) = iced_runtime::task::into_stream(task) else {
            return;
        };
        let mut outputs = Vec::new();
        for action in iced::futures::executor::block_on_stream(stream) {
            match action {
                Action::Output(message) => outputs.push(message),
                Action::Widget(mut operation) => {
                    self.with_ui(|ui, renderer| ui.operate(renderer, operation.as_mut()));
                }
                _ => {}
            }
        }
        for message in outputs {
            self.handle(message);
        }
    }

    /// What follows an update: the panes' sync gives the keyboard out (and
    /// takes it from a field the model no longer says has it), the window is
    /// built again and drawn, and what that said is heard, with what the
    /// fields said through the keymap's channel (the subscription's messages).
    fn settle(&mut self) {
        for _ in 0..16 {
            let task = give_keyboard(&mut self.pkg.panes, &mut self.model);
            self.run(task);
            let drawn = self.events(&[Event::Window(window::Event::RedrawRequested(
                Instant::now(),
            ))]);
            let fields = crate::keymap::field_messages(&self.model)
                .into_iter()
                .map(Message::Keymap);
            let heard: Vec<Message> = drawn.into_iter().chain(fields).collect();
            if !heard.iter().any(Window::acts_on) {
                return;
            }
            for message in heard {
                self.handle(message);
            }
        }
        panic!("the window never settled: {:?}", self.said);
    }

    fn click_at(&mut self, at: Point) {
        self.cursor = mouse::Cursor::Available(at);
        self.send([
            Event::Mouse(mouse::Event::CursorMoved { position: at }),
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        ]);
    }

    fn click(&mut self, bounds: Rectangle) {
        self.click_at(bounds.center());
    }

    /// Types `text` a key at a time, the app's loop run after each key as it
    /// runs after each in the app.
    fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            let key = c.to_string();
            let events: Vec<Event> = iced_test::simulator::typewrite(&key).collect();
            self.send(events);
        }
    }

    fn key(&mut self, named: Named) {
        self.send(iced_test::simulator::tap_key(named, None));
    }

    /// Every place a selector picks, in the order the window draws them.
    fn find_all(
        &mut self,
        select: impl FnMut(Candidate<'_>) -> Option<Rectangle> + Send,
    ) -> Vec<Rectangle> {
        let mut find = select.find_all();
        self.with_ui(|ui, renderer| ui.operate(renderer, &mut operation::black_box(&mut find)));
        match find.finish() {
            Outcome::Some(found) => found,
            _ => Vec::new(),
        }
    }

    /// Where the window draws `words`.
    fn texts(&mut self, words: &str) -> Vec<Rectangle> {
        let words = words.to_owned();
        self.find_all(move |candidate| match candidate {
            Candidate::Text {
                content,
                visible_bounds,
                ..
            } if content == words => visible_bounds,
            _ => None,
        })
    }

    /// Every text field the window draws.
    fn fields(&mut self) -> Vec<Rectangle> {
        self.find_all(|candidate| match candidate {
            Candidate::TextInput { visible_bounds, .. } => visible_bounds,
            _ => None,
        })
    }

    /// The field that goes by `id`.
    fn field_with(&mut self, id: Id) -> Rectangle {
        self.find_all(move |candidate| match candidate {
            Candidate::TextInput {
                id: Some(found),
                visible_bounds,
                ..
            } if *found == id => visible_bounds,
            _ => None,
        })
        .into_iter()
        .next()
        .expect("the field is on screen")
    }

    /// The focusable that goes by `id` (a text editor is one, not a text
    /// input).
    fn focusable_with(&mut self, id: Id) -> Rectangle {
        self.find_all(move |candidate| match candidate {
            Candidate::Focusable {
                id: Some(found),
                visible_bounds,
                ..
            } if *found == id => visible_bounds,
            _ => None,
        })
        .into_iter()
        .next()
        .expect("it is on screen")
    }

    /// Whatever has iced's keyboard focus: a field with a caret.
    fn carets(&mut self) -> Vec<Rectangle> {
        self.find_all(|candidate| match candidate {
            Candidate::Focusable { state, bounds, .. } if state.is_focused() => Some(bounds),
            _ => None,
        })
    }

    /// The broadcast field, while it is empty: its placeholder.
    fn broadcast_field(&mut self) -> Rectangle {
        let placeholder = Translator::new(Locale::En)
            .t_with(Key::BroadcastPlaceholder, &args!("n" => 1usize))
            .into_owned();
        *self
            .texts(&placeholder)
            .first()
            .expect("the broadcast field, empty")
    }

    fn broadcast(&self) -> &str {
        let main = self.model.main_window().expect("the main window");
        self.pkg.status_bar.broadcast_text(main)
    }

    fn terminal_has_keyboard(&self) -> bool {
        let rt = &self.pkg.panes.panes[&self.pane];
        rt.focused && rt.link.lock().is_focused()
    }

    /// What the selected pane's terminal sent its program, as text.
    fn typed_in_pane(&self) -> String {
        let bytes: Vec<u8> = self
            .typed
            .iter()
            .filter(|(pane, _)| *pane == self.pane)
            .flat_map(|(_, bytes)| bytes.iter().copied())
            .collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// The person's report: a pane selected, a click in « Send the same
/// instruction to… », and what was typed went to the pane. The keys are the
/// field's, all of them (the first character used to rebuild the field and
/// lose the keyboard), none reaches the terminal, and a click in the pane
/// takes them back.
#[test]
fn the_broadcast_field_clicked_takes_the_keys_and_a_click_in_the_pane_takes_them_back() {
    let mut w = Window::new();
    let field = w.broadcast_field();
    // Above the status bar, at the left of the work area: the pane.
    let in_pane = Point::new(field.x + 40.0, field.y - 160.0);

    w.click(field);
    assert_eq!(
        w.model.ui.focus,
        Some(Focus::Region(Region::Status)),
        "the model says where the keyboard is"
    );
    assert!(!w.terminal_has_keyboard(), "no terminal has it");
    w.type_text("ls -la");
    assert_eq!(w.broadcast(), "ls -la", "every key is the field's");
    assert_eq!(w.typed_in_pane(), "", "nothing is written to the pane");

    w.click_at(in_pane);
    assert_eq!(w.model.ui.focus, Some(Focus::Terminal(AVA.into())));
    assert!(w.terminal_has_keyboard());
    assert!(w.carets().is_empty(), "the field let go of the keyboard");
    w.type_text("pwd");
    assert_eq!(w.typed_in_pane(), "pwd");
    assert_eq!(w.broadcast(), "ls -la", "and the field got none of it");
}

/// `view.broadcast` and F6 give the field the keyboard by an operation. A
/// field focused that way, with nothing said to the model, takes the keys
/// from the pane all the same; its Escape gives them back.
#[test]
fn a_field_focused_by_the_view_broadcast_command_takes_the_keys_from_the_pane() {
    let mut w = Window::new();
    let task = crate::status_bar::focus_entry(&w.pkg.status_bar, &w.model, Region::Status)
        .expect("the field is over the terminals");
    w.run(task);
    w.settle();
    assert_eq!(w.model.ui.focus, Some(Focus::Region(Region::Status)));
    assert!(!w.terminal_has_keyboard());
    w.type_text("echo hi");
    assert_eq!(w.broadcast(), "echo hi");
    assert_eq!(w.typed_in_pane(), "");

    w.key(Named::Escape);
    assert_eq!(
        w.model.ui.focus,
        Some(Focus::Terminal(AVA.into())),
        "the keyboard goes back to what had it"
    );
    assert!(w.terminal_has_keyboard());
    w.type_text("q");
    assert_eq!(w.typed_in_pane(), "q");
    assert_eq!(w.broadcast(), "echo hi");
}

/// A shortcut that picks a pane (`pane.focus1`) gives its terminal the
/// keyboard: the field that had it is blurred, and types nothing more.
#[test]
fn a_pane_picked_by_a_shortcut_takes_the_keys_back_and_blurs_the_field() {
    let mut w = Window::new();
    let field = w.broadcast_field();
    w.click(field);
    w.type_text("a");
    assert_eq!(w.carets().len(), 1, "the field has the keyboard");

    let _ = w.focus_pane(WS.into(), AVA.into(), true);
    w.settle();
    assert!(w.terminal_has_keyboard());
    assert!(w.carets().is_empty(), "no field keeps a caret");
    w.type_text("b");
    assert_eq!(w.typed_in_pane(), "b");
    assert_eq!(w.broadcast(), "a");
}

/// A field that leaves the screen with the keyboard (the view changes, a
/// rename is settled) says nothing as it goes: its guard does, and the
/// keyboard goes back to the pane.
#[test]
fn a_field_gone_from_the_screen_with_the_keys_gives_them_back() {
    let mut w = Window::new();
    let field = w.broadcast_field();
    w.click(field);
    assert_eq!(w.model.ui.focus, Some(Focus::Region(Region::Status)));

    w.model.ui.view = View::Board;
    w.settle();
    assert_eq!(
        w.model.ui.focus,
        Some(Focus::Terminal(AVA.into())),
        "the keyboard is given back"
    );
    w.model.ui.view = View::Terminals;
    w.settle();
    assert!(w.terminal_has_keyboard());
    w.type_text("x");
    assert_eq!(w.typed_in_pane(), "x");
}

/// With a dialog open no terminal has the keyboard, whatever the focus it
/// will give back says: what is typed in the dialog's fields never reaches
/// the pane behind. Once it closes, the pane has the keys again.
#[test]
fn a_dialog_field_takes_the_keys_and_the_pane_behind_gets_none() {
    let mut w = Window::new();
    // What the shell does as it opens an overlay (`App::open_overlay`).
    let opener = w.model.ui.focus.clone();
    w.model.ui.overlays.push(Overlay::WorkspaceDialog);
    w.model.ui.overlay_openers.push(opener);
    w.settle();
    assert!(!w.terminal_has_keyboard(), "a dialog keeps the keyboard");

    let label = Translator::new(Locale::En).t(Key::CreateName).into_owned();
    let label = *w.texts(&label).last().expect("the name's label");
    // The dialog's labels sit at the left of their fields, on their line.
    let line = label.center().y;
    let name = w
        .fields()
        .into_iter()
        .find(|field| {
            field.x > label.x + label.width && field.y <= line && line <= field.y + field.height
        })
        .expect("the name field, beside its label");
    w.click(name);
    w.type_text("api");
    let named = w
        .said
        .iter()
        .filter(|said| said.starts_with("WorkspaceDialog(Name("))
        .count();
    assert_eq!(named, 3, "each key in the field: {:?}", w.said);
    assert_eq!(w.typed_in_pane(), "", "none in the pane behind");
    assert!(!w.terminal_has_keyboard());

    // What the shell does as it closes it (`App::close_overlay`).
    w.model.ui.overlays.pop();
    w.model.ui.focus = w.model.ui.overlay_openers.pop().flatten();
    w.settle();
    assert!(w.terminal_has_keyboard());
    w.type_text("ls");
    assert_eq!(w.typed_in_pane(), "ls");
}

/// The terminal's own search bar, over it: a press in it takes the keyboard
/// even when the keyboard was elsewhere (the press selects the pane without
/// giving its terminal the keys), every character is the bar's, and Escape
/// closes it and gives the terminal the keys back.
#[test]
fn the_search_bar_over_a_terminal_keeps_the_keys_it_was_clicked_for() {
    let mut w = Window::new();
    let broadcast = w.broadcast_field();
    let task = open_finder(&mut w.pkg.panes, &AVA.into());
    w.run(task);
    w.settle();
    let input = w.pkg.panes.panes[AVA]
        .search
        .as_ref()
        .expect("the bar is open")
        .input
        .clone();
    assert_eq!(w.model.ui.focus, Some(Focus::Widget(input.clone())));
    assert!(!w.terminal_has_keyboard());

    // The keyboard goes elsewhere, then back to the bar by a press in it.
    w.click(broadcast);
    assert_eq!(w.model.ui.focus, Some(Focus::Region(Region::Status)));
    let bar = w.field_with(input.clone());
    w.said.clear();
    w.click(bar);
    assert!(
        w.said.iter().any(|said| said.starts_with("Panes(Select(")),
        "{:?}",
        w.said
    );
    assert!(
        !w.said.iter().any(|said| said.starts_with("Panes(Focus(")),
        "the press does not give the terminal the keys: {:?}",
        w.said
    );
    assert_eq!(w.model.ui.focus, Some(Focus::Widget(input)));
    assert!(!w.terminal_has_keyboard());
    w.type_text("needle");
    let search = w.pkg.panes.panes[AVA].search.as_ref().expect("still open");
    assert_eq!(search.query, "needle");
    assert_eq!(w.typed_in_pane(), "");

    w.key(Named::Escape);
    assert!(w.pkg.panes.panes[AVA].search.is_none(), "Escape closes it");
    assert_eq!(w.model.ui.focus, Some(Focus::Terminal(AVA.into())));
    assert!(w.terminal_has_keyboard());
    w.type_text("n");
    assert_eq!(w.typed_in_pane(), "n");
}

/// The chat view's composer: a click in it takes the keyboard (the model
/// names it), each key is the composer's, and the grid shown again takes the
/// composer off the screen with the keyboard, which goes back to the pane.
#[test]
fn the_chat_composer_takes_the_keys_and_the_grid_back_gives_them_to_the_pane() {
    const LEO: &str = "leo";
    let mut leo = pane(LEO, "Leo", AgentId::Claude);
    leo.role = Some(PaneRole::Orchestrator);
    let mut w = Window::of(
        AppState {
            workspaces: vec![workspace(WS, "Shop", None, vec![leo])],
            active_workspace_id: Some(WS.into()),
            ..AppState::default()
        },
        LEO,
    );
    w.model.apply(StoreAction::UpdateSettings(Patch::new(
        |s: &mut Settings| s.experiments = vec![crate::settings::experiments::CHAT_VIEW.into()],
    )));
    w.model.apply(StoreAction::SetChatView {
        ws: WS.into(),
        on: true,
    });
    w.pkg.chat_view.conversation.read_for_test(
        &mut w.model,
        WS,
        LEO,
        ChatView {
            pane_id: LEO.into(),
            pane: "Leo".into(),
            workspace: "Shop".into(),
            agent: "claude".into(),
            activity: None,
            session_id: Some("s1".into()),
            items: Vec::new(),
            older: false,
            cursor: None,
            pending: None,
            reason: None,
        },
    );
    w.settle();
    assert!(!w.terminal_has_keyboard(), "the grid is off screen");

    let composer = w.focusable_with(crate::chat_view::conversation::input_id());
    w.click(composer);
    assert_eq!(
        w.model.ui.focus,
        Some(Focus::Widget(crate::chat_view::conversation::input_id()))
    );
    w.type_text("hi");
    let typed = w
        .said
        .iter()
        .filter(|said| said.starts_with("ChatView(Conversation(Edit(") && said.contains("Insert("))
        .count();
    assert_eq!(typed, 2, "each key in the composer: {:?}", w.said);
    assert_eq!(w.typed_in_pane(), "", "none in the pane");

    w.model.apply(StoreAction::SetChatView {
        ws: WS.into(),
        on: false,
    });
    w.settle();
    assert_eq!(
        w.model.ui.focus,
        Some(Focus::Terminal(LEO.into())),
        "the composer went with the keyboard, which came back"
    );
    assert!(w.terminal_has_keyboard());
    w.type_text("y");
    assert_eq!(w.typed_in_pane(), "y");
}
