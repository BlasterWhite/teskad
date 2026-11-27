//! The keyboard focus ring of a control that iced gives none: buttons,
//! switches, segments.
//!
//! iced 0.14's `button` is not focusable, so [`ring`] wraps one (or any
//! element) in a widget that is. It registers with iced's focus operations
//! (`widget::focus_next`, `focus_previous`, `focus`), draws the palette's
//! `focus_ring` around its content while it has the focus, 2 px wide and
//! 2 px off the control, and presses it for the keyboard: Space and Enter
//! send the message given to [`Ring::on_activate`], the one the button
//! sends when clicked.
//!
//! Like a browser's `:focus-visible`, the ring only shows for the keyboard:
//! a press of the pointer anywhere takes the focus off. A disabled control
//! (no message) is not focusable, and is skipped by `focus_next`.
//!
//! ```ignore
//! focus::ring(t, button(label).on_press(Msg::Save))
//!     .radius(radius::MD)
//!     .on_activate(Msg::Save)
//! ```

use hangar_theme::Theme;
use hangar_theme::tokens::{FOCUS_RING_WIDTH, scale};
use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer::{self, Quad};
use iced::advanced::widget::operation::Focusable;
use iced::advanced::widget::{Id, Operation, Tree, tree};
use iced::advanced::{Clipboard, Renderer as _, Shell, Widget};
use iced::keyboard::{self, Key, key::Named};
use iced::{Background, Border, Element, Event, Length, Rectangle, Size, Vector, mouse};

/// An element that can take the keyboard focus and shows a ring when it has it.
#[must_use]
pub struct Ring<'a, M> {
    theme: &'a Theme,
    content: Element<'a, M>,
    radius: f32,
    on_activate: Option<M>,
    id: Option<Id>,
    shown: bool,
}

/// `content` made focusable, with the ring around it.
pub fn ring<'a, M>(t: &'a Theme, content: impl Into<Element<'a, M>>) -> Ring<'a, M> {
    Ring {
        theme: t,
        content: content.into(),
        radius: scale::radius::SM,
        on_activate: None,
        id: None,
        shown: false,
    }
}

impl<'a, M> Ring<'a, M> {
    /// The corner radius of the control inside; the ring's is two more.
    pub fn radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    /// What Space and Enter send. Without it the control is disabled: it
    /// cannot take the focus.
    pub fn on_activate(mut self, message: M) -> Self {
        self.on_activate = Some(message);
        self
    }

    /// [`on_activate`](Self::on_activate) of something that may not be.
    pub fn on_activate_maybe(mut self, message: Option<M>) -> Self {
        self.on_activate = message;
        self
    }

    /// An id for `widget::focus(id)`.
    pub fn id(mut self, id: impl Into<Id>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Draws the ring as if the control had the keyboard: for the component
    /// sheet, which shows every state at once.
    pub fn shown(mut self, shown: bool) -> Self {
        self.shown = shown;
        self
    }
}

#[derive(Debug, Default)]
struct State {
    focused: bool,
}

impl Focusable for State {
    fn is_focused(&self) -> bool {
        self.focused
    }

    fn focus(&mut self) {
        self.focused = true;
    }

    fn unfocus(&mut self) {
        self.focused = false;
    }
}

impl<M: Clone> Widget<M, iced::Theme, iced::Renderer> for Ring<'_, M> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
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

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let inner = self
            .content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
        layout::Node::with_children(inner.size(), vec![inner])
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        let state = tree.state.downcast_mut::<State>();
        if self.on_activate.is_some() {
            operation.focusable(self.id.as_ref(), layout.bounds(), state);
        } else {
            // A control that cannot be used cannot keep the ring either.
            state.focused = false;
        }
        operation.traverse(&mut |operation| {
            if let Some(child) = layout.children().next() {
                self.content.as_widget_mut().operate(
                    &mut tree.children[0],
                    child,
                    renderer,
                    operation,
                );
            }
        });
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        if let Some(child) = layout.children().next() {
            self.content.as_widget_mut().update(
                &mut tree.children[0],
                event,
                child,
                cursor,
                renderer,
                clipboard,
                shell,
                viewport,
            );
        }
        let state = tree.state.downcast_mut::<State>();
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(_)) if state.focused => {
                state.focused = false;
                shell.request_redraw();
            }
            Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. })
                if state.focused
                    && !shell.is_event_captured()
                    && !modifiers.control()
                    && !modifiers.alt()
                    && !modifiers.logo() =>
            {
                let pressed = matches!(key, Key::Named(Named::Enter) | Key::Named(Named::Space))
                    || matches!(key, Key::Character(c) if c.as_str() == " ");
                if let (true, Some(message)) = (pressed, self.on_activate.clone()) {
                    shell.publish(message);
                    shell.capture_event();
                    shell.request_redraw();
                }
            }
            _ => {}
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
        layout
            .children()
            .next()
            .map(|child| {
                self.content.as_widget().mouse_interaction(
                    &tree.children[0],
                    child,
                    cursor,
                    viewport,
                    renderer,
                )
            })
            .unwrap_or_default()
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
        let Some(child) = layout.children().next() else {
            return;
        };
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            child,
            cursor,
            viewport,
        );
        let state = tree.state.downcast_ref::<State>();
        if state.focused || self.shown {
            let reach = scale::FOCUS_OFFSET + FOCUS_RING_WIDTH;
            let bounds = layout.bounds();
            renderer.fill_quad(
                Quad {
                    bounds: Rectangle {
                        x: bounds.x - reach,
                        y: bounds.y - reach,
                        width: bounds.width + 2.0 * reach,
                        height: bounds.height + 2.0 * reach,
                    },
                    border: Border {
                        color: self.theme.palette.focus_ring,
                        width: FOCUS_RING_WIDTH,
                        radius: (self.radius + reach).into(),
                    },
                    ..Quad::default()
                },
                Background::Color(iced::Color::TRANSPARENT),
            );
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, M, iced::Theme, iced::Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout.children().next()?,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, M: Clone + 'a> From<Ring<'a, M>> for Element<'a, M> {
    fn from(ring: Ring<'a, M>) -> Self {
        Element::new(ring)
    }
}

#[cfg(test)]
mod tests;
