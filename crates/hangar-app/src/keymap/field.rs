//! The keyboard between the text fields and everything else.
//!
//! The app has two focuses. iced keeps one of its own for its text fields
//! (a click, Tab or `operation::focus` gives it), and the model keeps
//! `Model.ui.focus`, which says whether the panes have the keyboard: the
//! panes' sync gives it to the focused pane's terminal while that focus is
//! the panes' (`panes::focus_on_panes`). Nothing told either about the
//! other. A click in the broadcast field took iced's focus and left the model
//! on the pane, so its terminal stayed focused; and a focused terminal types
//! every key it is handed, the grid being drawn (so handed each key) before
//! the status bar. The pane got what was typed in the field.
//!
//! This module keeps the two in step, both ways, for every field that goes
//! through [`super::guard`] (every text field of the app does):
//!
//! - a field that takes the keyboard says so, and the model names it
//!   (`Focus::Widget`, or the region the field stands for): no terminal is
//!   focused while it has it. When it lets go (Escape, a press elsewhere), or
//!   leaves the screen with it, the keyboard goes back to what had it;
//! - when the model gives the keyboard to something else (a pane clicked or
//!   picked by a shortcut, F6, a dialog's close), the field that held it is
//!   blurred ([`reconcile`], from the panes' sync), so it neither keeps a
//!   caret nor types what is now the terminal's.
//!
//! A field says so through the keymap's own channel, read by its
//! subscription, not among the messages of the event that moved the
//! keyboard: what a field's own package hears is as it was, and a field
//! dropped from the screen (which no event tells) says so the same way.
//!
//! Inside a dialog nothing is told: while an overlay is open the panes do not
//! have the keyboard anyway (`panes::keyboard_free`), and the shell gives it
//! back to what had it when the overlay closes (`overlay_openers`).

use std::any::Any;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::operation::{Focusable, Outcome, Scrollable, TextInput};
use iced::advanced::widget::{Id, Operation, Tree, tree};
use iced::advanced::{Clipboard, Shell, Widget};
use iced::futures::stream;
use iced::{Element, Event, Length, Rectangle, Size, Subscription, Task, Vector, mouse, window};
use tokio::sync::mpsc;

use crate::ctx::Ctx;
use crate::message::Message;
use crate::shell::{Focus, Region, View};
use crate::state::Model;

/// What a field said of the keyboard (`keymap::Msg::Field`).
#[derive(Debug, Clone)]
pub enum FieldFocus {
    /// It took the keyboard. `id` is the field's own id, or the one its
    /// guard gave it; `region`, the region the field stands for when it has
    /// one (the broadcast field is region `Status`, as F6 enters it).
    Took {
        window: window::Id,
        id: Id,
        serial: u64,
        region: Option<Region>,
    },
    /// It let go of the keyboard, or left the screen with it.
    LetGo {
        window: window::Id,
        id: Id,
        serial: u64,
    },
}

/// The field that has the keyboard in a window, as the model was told.
#[derive(Debug, Clone)]
struct Hold {
    id: Id,
    /// The watch that told it: a field drawn again under the same id is
    /// another watch, whose word is the one that counts.
    serial: u64,
    /// What the window's focus was set to for the field.
    focus: Focus,
    /// What the keyboard goes back to when the field lets go of it; `None`
    /// when the focus already was the field's when it took it (F6 had
    /// entered its region), so its letting go changes nothing.
    opener: Option<Option<Focus>>,
}

/// The fields' side of the keyboard; lives in `Model.live.keymap.fields`.
pub struct Fields {
    /// Each window's field with the keyboard.
    holds: HashMap<window::Id, Hold>,
    /// Where the watches say what their field did with the keyboard.
    words: mpsc::UnboundedSender<FieldFocus>,
    heard: Heard,
}

/// Each channel's own number: the subscription of a model made since is
/// another one.
static CHANNELS: AtomicU64 = AtomicU64::new(1);

impl Default for Fields {
    fn default() -> Self {
        let (words, rx) = mpsc::unbounded_channel();
        Fields {
            holds: HashMap::new(),
            words,
            heard: Heard(
                CHANNELS.fetch_add(1, Ordering::Relaxed),
                Arc::new(Mutex::new(Some(rx))),
            ),
        }
    }
}

impl std::fmt::Debug for Fields {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fields")
            .field("holds", &self.holds)
            .finish_non_exhaustive()
    }
}

/// The receiving half of the watches' word, taken once by the subscription,
/// and the channel's number.
#[derive(Clone)]
struct Heard(u64, Arc<Mutex<Option<mpsc::UnboundedReceiver<FieldFocus>>>>);

// The subscription's identity: its channel.
impl Hash for Heard {
    fn hash<H: Hasher>(&self, state: &mut H) {
        ("hangar fields", self.0).hash(state);
    }
}

/// What the watches say, as messages.
pub fn subscription(fields: &Fields) -> Subscription<Message> {
    Subscription::run_with(fields.heard.clone(), |heard| {
        let rx = heard.1.lock().ok().and_then(|mut slot| slot.take());
        stream::unfold(rx, |rx| async move {
            let mut rx = rx?;
            let said = rx.recv().await?;
            Some((Message::Keymap(super::Msg::Field(said)), Some(rx)))
        })
    })
}

/// The watches' word read without the subscription, for the tests that run
/// the app's loop by hand.
#[cfg(test)]
pub fn drain(fields: &Fields) -> Vec<FieldFocus> {
    let Ok(mut slot) = fields.heard.1.lock() else {
        return Vec::new();
    };
    let Some(rx) = slot.as_mut() else {
        return Vec::new();
    };
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

// ---- The model's side --------------------------------------------------------------------------

/// Window `window`'s focus, the focused window's or a parked one's; `None`
/// for a window that is gone.
fn focus_of(m: &Model, window: window::Id) -> Option<Option<Focus>> {
    if m.is_focused_window(window) {
        Some(m.ui.focus.clone())
    } else if m.live.windows.contains_key(&window) {
        Some(m.ui.parked.get(&window).and_then(|ui| ui.focus.clone()))
    } else {
        None
    }
}

fn set_focus(m: &mut Model, window: window::Id, focus: Option<Focus>) {
    if m.is_focused_window(window) {
        m.ui.focus = focus;
    } else if m.live.windows.contains_key(&window) {
        m.ui.parked.entry(window).or_default().focus = focus;
    }
}

/// What a field said, applied to the model.
pub fn apply(m: &mut Model, said: FieldFocus) {
    match said {
        FieldFocus::Took {
            window,
            id,
            serial,
            region,
        } => took(m, window, id, serial, region),
        FieldFocus::LetGo { window, id, serial } => let_go(m, window, &id, serial),
    }
}

/// A field took the keyboard: the window's focus names it, which takes the
/// keyboard from the panes, and what had it is kept to give it back. A region
/// the shell gave the keyboard to (F6 entered the dock, whose add field then
/// took it) keeps it: it keeps the terminals from the keys as well, and F6
/// goes on from there. From another field, the keyboard is the new field's.
fn took(m: &mut Model, window: window::Id, id: Id, serial: u64, region: Option<Region>) {
    let Some(current) = focus_of(m, window) else {
        return;
    };
    let held = m.live.keymap.fields.holds.get(&window);
    let wanted = match (region, &current) {
        (Some(region), _) => Focus::Region(region),
        (None, Some(Focus::Region(entered)))
            if held.is_none() && keeps_terminals_off(m, window, *entered) =>
        {
            Focus::Region(*entered)
        }
        (None, _) => Focus::Widget(id.clone()),
    };
    // A field taking over from another hands on what that one would have
    // given the keyboard back to.
    let inherited = held
        .filter(|hold| current.as_ref() == Some(&hold.focus))
        .and_then(|hold| hold.opener.clone());
    let opener = match (inherited, &current) {
        (Some(opener), _) => Some(opener),
        // Already its own: F6 entered the field's region, which keeps it.
        (None, Some(Focus::Region(_))) if current.as_ref() == Some(&wanted) => None,
        // A field named by the shell (an overlay closed and gave it back by
        // its id), or one that went: the keyboard goes to the panes after.
        (None, Some(Focus::Widget(_))) => Some(None),
        (None, _) => Some(current.clone()),
    };
    set_focus(m, window, Some(wanted.clone()));
    m.live.keymap.fields.holds.insert(
        window,
        Hold {
            id,
            serial,
            focus: wanted,
            opener,
        },
    );
}

/// Whether region `region` having the keyboard keeps it from the terminals:
/// every region but the panes' own, which is theirs while they are on screen.
fn keeps_terminals_off(m: &Model, window: window::Id, region: Region) -> bool {
    region != Region::Main || m.view_in(window) != View::Terminals
}

/// A field let go of the keyboard, or left the screen with it: if nothing
/// took the keyboard meanwhile, it goes back to what had it.
fn let_go(m: &mut Model, window: window::Id, id: &Id, serial: u64) {
    let holds = &mut m.live.keymap.fields.holds;
    if !holds
        .get(&window)
        .is_some_and(|hold| hold.id == *id && hold.serial == serial)
    {
        return;
    }
    let Some(hold) = holds.remove(&window) else {
        return;
    };
    let Some(opener) = hold.opener else {
        return;
    };
    if focus_of(m, window) == Some(Some(hold.focus)) {
        set_focus(m, window, opener);
    }
}

/// The model gave the keyboard to something else than the field that held
/// it (a pane clicked or picked by a shortcut, F6, a dialog's close): that
/// field is blurred, so it does not type what is now someone else's. Called
/// by the panes' sync, which gives the terminals the keyboard from the same
/// focus.
pub fn reconcile(m: &mut Model) -> Task<Message> {
    let windows: Vec<window::Id> = m.live.keymap.fields.holds.keys().copied().collect();
    let mut blurs = Vec::new();
    for window in windows {
        let current = focus_of(m, window);
        let holds = &mut m.live.keymap.fields.holds;
        let Some(held) = holds.get(&window).map(|hold| hold.focus.clone()) else {
            continue;
        };
        match current {
            Some(Some(focus)) if focus == held => {}
            // The window closed: its fields went with it.
            None => {
                holds.remove(&window);
            }
            Some(_) => {
                if let Some(hold) = holds.remove(&window) {
                    blurs.push(blur(hold.id));
                }
            }
        }
    }
    Task::batch(blurs)
}

/// Takes the keyboard from the field `id`, and from no other.
fn blur(id: Id) -> Task<Message> {
    iced::advanced::widget::operate(Blur(id))
}

struct Blur(Id);

impl<T> Operation<T> for Blur {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<T>)) {
        operate(self);
    }

    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if id == Some(&self.0) {
            state.unfocus();
        }
    }
}

// ---- The fields' side --------------------------------------------------------------------------

/// Each watch's own number, for a word of an older watch to be told apart.
static SERIALS: AtomicU64 = AtomicU64::new(1);

/// `content` watched: when a field in it takes the keyboard or lets go of
/// it, the model is told. `region`: the region the field stands for, if any.
/// `frames`: whether to look at every frame too (`Watched::frames`).
pub fn watch<'a>(
    ctx: Ctx<'a>,
    region: Option<Region>,
    frames: bool,
    content: Element<'a, Message>,
) -> Element<'a, Message> {
    let m = ctx.model;
    Element::new(Watched {
        content,
        window: ctx.window,
        region,
        tells: m.ui.overlays.is_empty() && m.overlays_in(ctx.window).is_empty(),
        frames,
        words: &m.live.keymap.fields.words,
    })
}

struct Watched<'a> {
    content: Element<'a, Message>,
    window: window::Id,
    region: Option<Region>,
    /// No dialog is open: a field taking the keyboard is told (see the
    /// module's notes). One that already told goes on saying when it lets
    /// go, whatever opened since.
    tells: bool,
    /// The keyboard is looked for at every frame too, not only after a
    /// press or a key: an operation (`operation::focus`, the `view.broadcast`
    /// command's) gives a field the keyboard between two events, and the
    /// frame after it is the first to show it, before any key is typed. Off
    /// for what may be a long document (a preview), looked through on
    /// presses and keys only.
    frames: bool,
    words: &'a mpsc::UnboundedSender<FieldFocus>,
}

/// What a watch keeps between frames.
struct Watch {
    /// The id a field without one of its own goes by, for the model to name
    /// it and for an operation to reach it.
    own: Id,
    serial: u64,
    window: window::Id,
    /// The id the model was told has the keyboard, until it lets go.
    told: Option<Id>,
    words: mpsc::UnboundedSender<FieldFocus>,
}

impl Drop for Watch {
    fn drop(&mut self) {
        // Gone from the screen with the keyboard (a rename settled, a view
        // left): nothing else will say the field let go.
        if let Some(id) = self.told.take() {
            let _ = self.words.send(FieldFocus::LetGo {
                window: self.window,
                id,
                serial: self.serial,
            });
        }
    }
}

impl Watched<'_> {
    /// The id of the field in `content` that has the keyboard, if one does.
    fn focused(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
    ) -> Option<Id> {
        let own = tree.state.downcast_ref::<Watch>().own.clone();
        let mut found = Focused(None);
        self.content.as_widget_mut().operate(
            &mut tree.children[0],
            layout,
            renderer,
            &mut Named {
                inner: &mut found,
                own: &own,
            },
        );
        found.0
    }
}

impl Widget<Message, iced::Theme, iced::Renderer> for Watched<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Watch>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(Watch {
            own: Id::unique(),
            serial: SERIALS.fetch_add(1, Ordering::Relaxed),
            window: self.window,
            told: None,
            words: self.words.clone(),
        })
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&[self.content.as_widget()]);
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
        let told = tree.state.downcast_ref::<Watch>().told.clone();
        // The pointer moving gives no field the keyboard, and it moves often.
        let frame = matches!(event, Event::Window(window::Event::RedrawRequested(_)));
        if (told.is_none() && !self.tells)
            || matches!(event, Event::Mouse(mouse::Event::CursorMoved { .. }))
            || (frame && !self.frames)
        {
            return;
        }
        // After the event: a press, Tab, Escape, or an operation since the
        // last one may have moved the keyboard.
        let focused = self.focused(tree, layout, renderer);
        if focused == told {
            return;
        }
        let watch = tree.state.downcast_mut::<Watch>();
        let (window, serial) = (watch.window, watch.serial);
        if let Some(id) = told {
            watch.told = None;
            let _ = self.words.send(FieldFocus::LetGo { window, id, serial });
        }
        if let Some(id) = focused.filter(|_| self.tells) {
            watch.told = Some(id.clone());
            let _ = self.words.send(FieldFocus::Took {
                window,
                id,
                serial,
                region: self.region,
            });
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        // A field without an id goes by the watch's, so that the model can
        // name it and `operation::focus` or a blur can reach it.
        let own = tree.state.downcast_ref::<Watch>().own.clone();
        self.content.as_widget_mut().operate(
            &mut tree.children[0],
            layout,
            renderer,
            &mut Named {
                inner: operation,
                own: &own,
            },
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

/// The first field found with the keyboard, by its id.
struct Focused(Option<Id>);

impl Operation for Focused {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }

    fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if self.0.is_none() && state.is_focused() {
            self.0 = id.cloned();
        }
    }
}

/// An operation handed on with the focusables that have no id named `own`.
struct Named<'a, 'b> {
    inner: &'a mut dyn Operation,
    own: &'b Id,
}

impl Operation for Named<'_, '_> {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        let own = self.own;
        self.inner
            .traverse(&mut |inner| operate(&mut Named { inner, own }));
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        self.inner.container(id, bounds);
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        content_bounds: Rectangle,
        translation: Vector,
        state: &mut dyn Scrollable,
    ) {
        self.inner
            .scrollable(id, bounds, content_bounds, translation, state);
    }

    fn focusable(&mut self, id: Option<&Id>, bounds: Rectangle, state: &mut dyn Focusable) {
        self.inner
            .focusable(Some(id.unwrap_or(self.own)), bounds, state);
    }

    fn text_input(&mut self, id: Option<&Id>, bounds: Rectangle, state: &mut dyn TextInput) {
        self.inner
            .text_input(Some(id.unwrap_or(self.own)), bounds, state);
    }

    fn text(&mut self, id: Option<&Id>, bounds: Rectangle, text: &str) {
        self.inner.text(id, bounds, text);
    }

    fn custom(&mut self, id: Option<&Id>, bounds: Rectangle, state: &mut dyn Any) {
        self.inner.custom(id, bounds, state);
    }

    fn finish(&self) -> Outcome<()> {
        self.inner.finish()
    }
}

#[cfg(test)]
mod tests {
    //! The model's side alone: what a field's word does to the focus. The
    //! fields driven in a window are the panes' `focus_tests`.

    use super::*;
    use crate::navigator::testing::model_of;
    use crate::state::types::AppState;

    fn model() -> (Model, window::Id) {
        let mut m = model_of(AppState::default());
        let main = m.main_window().expect("the main window");
        m.ui.focus = Some(Focus::Terminal("p1".into()));
        (m, main)
    }

    fn took(window: window::Id, id: &Id, serial: u64, region: Option<Region>) -> FieldFocus {
        FieldFocus::Took {
            window,
            id: id.clone(),
            serial,
            region,
        }
    }

    fn let_go(window: window::Id, id: &Id, serial: u64) -> FieldFocus {
        FieldFocus::LetGo {
            window,
            id: id.clone(),
            serial,
        }
    }

    /// Whether `task` operates on the widgets (a blur).
    fn operates(task: Task<Message>) -> bool {
        let Some(stream) = iced_runtime::task::into_stream(task) else {
            return false;
        };
        iced::futures::executor::block_on_stream(stream)
            .any(|action| matches!(action, iced_runtime::Action::Widget(_)))
    }

    #[test]
    fn a_field_takes_the_keyboard_from_the_pane_and_gives_it_back() {
        let (mut m, main) = model();
        let field = Id::new("field");
        apply(&mut m, took(main, &field, 1, None));
        assert_eq!(m.ui.focus, Some(Focus::Widget(field.clone())));
        apply(&mut m, let_go(main, &field, 1));
        assert_eq!(m.ui.focus, Some(Focus::Terminal("p1".into())));
    }

    #[test]
    fn a_field_that_is_a_region_names_the_region() {
        let (mut m, main) = model();
        let field = Id::new("broadcast");
        apply(&mut m, took(main, &field, 1, Some(Region::Status)));
        assert_eq!(m.ui.focus, Some(Focus::Region(Region::Status)));
        apply(&mut m, let_go(main, &field, 1));
        assert_eq!(m.ui.focus, Some(Focus::Terminal("p1".into())));
    }

    /// F6 had entered the region already: the field letting go leaves the
    /// keyboard in it, as before.
    #[test]
    fn a_field_entered_by_f6_leaves_its_region_as_it_found_it() {
        let (mut m, main) = model();
        m.ui.focus = Some(Focus::Region(Region::Status));
        let field = Id::new("broadcast");
        apply(&mut m, took(main, &field, 1, Some(Region::Status)));
        apply(&mut m, let_go(main, &field, 1));
        assert_eq!(m.ui.focus, Some(Focus::Region(Region::Status)));
    }

    /// F6 entered the dock, whose add field took the keyboard: the dock
    /// keeps it, and F6 goes on from there.
    #[test]
    fn a_field_in_a_region_that_has_the_keyboard_leaves_it_the_regions() {
        let (mut m, main) = model();
        m.ui.focus = Some(Focus::Region(Region::Dock));
        let field = Id::new("add");
        apply(&mut m, took(main, &field, 1, None));
        assert_eq!(m.ui.focus, Some(Focus::Region(Region::Dock)));
        apply(&mut m, let_go(main, &field, 1));
        assert_eq!(m.ui.focus, Some(Focus::Region(Region::Dock)));
    }

    /// The panes' own region, over the terminals, would leave them the keys.
    #[test]
    fn the_panes_region_does_not_keep_the_keyboard_from_a_field() {
        let (mut m, main) = model();
        m.ui.view = View::Terminals;
        m.ui.focus = Some(Focus::Region(Region::Main));
        let field = Id::new("field");
        apply(&mut m, took(main, &field, 1, None));
        assert_eq!(m.ui.focus, Some(Focus::Widget(field.clone())));
        apply(&mut m, let_go(main, &field, 1));
        assert_eq!(m.ui.focus, Some(Focus::Region(Region::Main)));
    }

    #[test]
    fn a_field_taking_over_from_another_gives_the_keyboard_back_to_the_pane() {
        let (mut m, main) = model();
        let (a, b) = (Id::new("a"), Id::new("b"));
        apply(&mut m, took(main, &a, 1, None));
        apply(&mut m, took(main, &b, 2, None));
        assert_eq!(m.ui.focus, Some(Focus::Widget(b.clone())));
        // The first one's word comes after: it no longer holds anything.
        apply(&mut m, let_go(main, &a, 1));
        assert_eq!(m.ui.focus, Some(Focus::Widget(b.clone())));
        apply(&mut m, let_go(main, &b, 2));
        assert_eq!(m.ui.focus, Some(Focus::Terminal("p1".into())));
    }

    /// A field drawn again under the same id is another guard: what the one
    /// before says as it goes does not take the keyboard from the new one.
    #[test]
    fn the_word_of_a_guard_gone_since_changes_nothing() {
        let (mut m, main) = model();
        let field = Id::new("field");
        apply(&mut m, took(main, &field, 2, None));
        apply(&mut m, let_go(main, &field, 1));
        assert_eq!(m.ui.focus, Some(Focus::Widget(field)));
    }

    /// The keyboard moved on while the field held it (a pane picked by a
    /// shortcut): the field is blurred, and its letting go after that gives
    /// nothing back.
    #[test]
    fn the_keyboard_given_elsewhere_blurs_the_field() {
        let (mut m, main) = model();
        let field = Id::new("field");
        apply(&mut m, took(main, &field, 1, None));
        assert!(!operates(reconcile(&mut m)), "still the field's");

        m.ui.focus = Some(Focus::Terminal("p2".into()));
        assert!(operates(reconcile(&mut m)), "blurred");
        apply(&mut m, let_go(main, &field, 1));
        assert_eq!(m.ui.focus, Some(Focus::Terminal("p2".into())));
        assert!(!operates(reconcile(&mut m)), "once");
    }

    #[test]
    fn a_window_that_closed_takes_its_field_with_it() {
        let (mut m, _) = model();
        let gone = window::Id::unique();
        m.live.keymap.fields.holds.insert(
            gone,
            Hold {
                id: Id::new("field"),
                serial: 1,
                focus: Focus::Widget(Id::new("field")),
                opener: Some(None),
            },
        );
        assert!(!operates(reconcile(&mut m)));
        assert!(m.live.keymap.fields.holds.is_empty());
    }
}
