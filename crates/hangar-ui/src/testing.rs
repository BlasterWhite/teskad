//! A headless window for the tests: widgets built, laid out and driven with
//! real events, without a display.
//!
//! The interaction tests of this crate (a menu's keys, a dialog's trap) go
//! through `iced_test`'s simulator, so that what they check is what the
//! runtime does with the widgets and not what a unit test imagines it does.

use std::borrow::Cow;

use hangar_theme::fonts;
use iced::keyboard::{self, Key, key::Named};
use iced::{Element, Event, Point, Settings, Size, mouse};
use iced_test::Simulator;

/// The window the tests run in.
pub const WINDOW: Size = Size::new(800.0, 600.0);

/// A simulator showing `element` in a window of [`WINDOW`], with the
/// interface font loaded so that text is measured as it is in the app.
pub fn simulate<'a, M>(element: impl Into<Element<'a, M>>) -> Simulator<'a, M> {
    let settings = Settings {
        fonts: hangar_theme::fonts::ALL.map(Cow::Borrowed).to_vec(),
        default_font: fonts::ui(),
        ..Settings::default()
    };
    Simulator::with_size(settings, WINDOW, element)
}

/// What a test does with the window.
pub trait Drive {
    /// Moves the pointer, the way a mouse does: the position and the event.
    fn move_to(&mut self, x: f32, y: f32);
    fn press(&mut self, button: mouse::Button);
    fn release(&mut self, button: mouse::Button);
    /// A press and a release of the left button where the pointer is.
    fn click_at(&mut self, x: f32, y: f32);
    fn key(&mut self, named: Named) -> iced::event::Status;
    fn key_with(&mut self, named: Named, modifiers: keyboard::Modifiers) -> iced::event::Status;
}

impl<M> Drive for Simulator<'_, M> {
    fn move_to(&mut self, x: f32, y: f32) {
        let position = Point::new(x, y);
        self.point_at(position);
        self.simulate([Event::Mouse(mouse::Event::CursorMoved { position })]);
    }

    fn press(&mut self, button: mouse::Button) {
        self.simulate([Event::Mouse(mouse::Event::ButtonPressed(button))]);
    }

    fn release(&mut self, button: mouse::Button) {
        self.simulate([Event::Mouse(mouse::Event::ButtonReleased(button))]);
    }

    fn click_at(&mut self, x: f32, y: f32) {
        self.move_to(x, y);
        self.press(mouse::Button::Left);
        self.release(mouse::Button::Left);
    }

    fn key(&mut self, named: Named) -> iced::event::Status {
        self.key_with(named, keyboard::Modifiers::empty())
    }

    fn key_with(&mut self, named: Named, modifiers: keyboard::Modifiers) -> iced::event::Status {
        let key = Key::Named(named);
        let press = Event::Keyboard(keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        });
        self.simulate([press])
            .first()
            .copied()
            .unwrap_or(iced::event::Status::Ignored)
    }
}

/// A window the tests can also run iced's focus operations in, which the
/// simulator cannot: what `widget::focus_next` does to a tree.
pub struct Keyboard<'a, M> {
    ui: iced_test::runtime::UserInterface<'a, M, iced::Theme, iced::Renderer>,
    renderer: iced::Renderer,
    cursor: mouse::Cursor,
    messages: Vec<M>,
}

impl<'a, M> Keyboard<'a, M> {
    pub fn new(element: impl Into<Element<'a, M>>) -> Self {
        use iced_test::core::renderer::Headless;
        for font in hangar_theme::fonts::ALL {
            iced_test::renderer::graphics::text::font_system()
                .write()
                .expect("the font system")
                .load_font(Cow::Borrowed(font));
        }
        let mut renderer = iced_test::futures::futures::executor::block_on(
            <iced::Renderer as Headless>::new(fonts::ui(), iced::Pixels(14.0), None),
        )
        .expect("a headless renderer");
        let ui = iced_test::runtime::UserInterface::build(
            element.into(),
            WINDOW,
            iced_test::runtime::user_interface::Cache::default(),
            &mut renderer,
        );
        Keyboard {
            ui,
            renderer,
            cursor: mouse::Cursor::Unavailable,
            messages: Vec::new(),
        }
    }

    /// Tab: the focus moves to the next control that can take it.
    pub fn focus_next(&mut self) {
        self.run(Box::new(
            iced_test::core::widget::operation::focusable::focus_next::<()>(),
        ));
    }

    /// Shift+Tab.
    pub fn focus_previous(&mut self) {
        self.run(Box::new(
            iced_test::core::widget::operation::focusable::focus_previous::<()>(),
        ));
    }

    /// Runs an operation as the runtime does: again for each one it chains.
    fn run(&mut self, mut operation: Box<dyn iced_test::core::widget::Operation<()>>) {
        use iced_test::core::widget::operation::Outcome;
        loop {
            self.ui.operate(&self.renderer, operation.as_mut());
            match operation.finish() {
                Outcome::Chain(next) => operation = next,
                _ => break,
            }
        }
    }

    /// A press and release of a named key, with no modifiers.
    pub fn key(&mut self, named: Named) {
        let key = Key::Named(named);
        let press = Event::Keyboard(keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::empty(),
            text: None,
            repeat: false,
        });
        self.send(press);
    }

    /// The pointer is pressed where it is.
    pub fn press_pointer(&mut self) {
        self.cursor = mouse::Cursor::Available(Point::new(1.0, 1.0));
        self.send(Event::Mouse(mouse::Event::ButtonPressed(
            mouse::Button::Left,
        )));
    }

    fn send(&mut self, event: Event) {
        let _ = self.ui.update(
            &[event],
            self.cursor,
            &mut self.renderer,
            &mut iced_test::core::clipboard::Null,
            &mut self.messages,
        );
    }

    pub fn into_messages(self) -> Vec<M> {
        self.messages
    }
}
