//! A window's three parts, one over the other: the head and the foot take
//! their height first, and the body what is left of the room. iced's column
//! lays its children out in order, each in what the ones before it left, so a
//! tall body would push the foot, and the buttons on it, out of a short
//! window.

use iced::advanced::layout::{self, Layout};
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::{Operation, Tree};
use iced::advanced::{Clipboard, Shell, Widget};
use iced::{Element, Event, Length, Point, Rectangle, Size, Vector, mouse};

const HEAD: usize = 0;
const BODY: usize = 1;
const FOOT: usize = 2;

pub(super) struct Frame<'a, M> {
    /// The head, the body and the foot, in the order they are drawn and Tab
    /// walks them.
    parts: [Element<'a, M>; 3],
}

impl<'a, M> Frame<'a, M> {
    pub(super) fn new(
        head: impl Into<Element<'a, M>>,
        body: impl Into<Element<'a, M>>,
        foot: impl Into<Element<'a, M>>,
    ) -> Self {
        Frame {
            parts: [head.into(), body.into(), foot.into()],
        }
    }
}

impl<M> Widget<M, iced::Theme, iced::Renderer> for Frame<'_, M> {
    fn children(&self) -> Vec<Tree> {
        self.parts.iter().map(Tree::new).collect()
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&self.parts);
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let width = limits.max().width;
        let mut room = limits.max().height;
        let mut nodes: [Option<layout::Node>; 3] = [None, None, None];
        for at in [HEAD, FOOT, BODY] {
            let within = layout::Limits::new(Size::ZERO, Size::new(width, room));
            let node =
                self.parts[at]
                    .as_widget_mut()
                    .layout(&mut tree.children[at], renderer, &within);
            room = (room - node.size().height).max(0.0);
            nodes[at] = Some(node);
        }
        let mut top = 0.0;
        let nodes = nodes
            .into_iter()
            .flatten()
            .map(|node| {
                let height = node.size().height;
                let node = node.move_to(Point::new(0.0, top));
                top += height;
                node
            })
            .collect();
        layout::Node::with_children(Size::new(width, top), nodes)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        operation.container(None, layout.bounds());
        operation.traverse(&mut |operation| {
            for ((part, state), layout) in self
                .parts
                .iter_mut()
                .zip(&mut tree.children)
                .zip(layout.children())
            {
                part.as_widget_mut()
                    .operate(state, layout, renderer, operation);
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
        for ((part, state), layout) in self
            .parts
            .iter_mut()
            .zip(&mut tree.children)
            .zip(layout.children())
        {
            part.as_widget_mut().update(
                state, event, layout, cursor, renderer, clipboard, shell, viewport,
            );
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
        self.parts
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
            .map(|((part, state), layout)| {
                part.as_widget()
                    .mouse_interaction(state, layout, cursor, viewport, renderer)
            })
            .max()
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
        for ((part, state), layout) in self.parts.iter().zip(&tree.children).zip(layout.children())
        {
            part.as_widget()
                .draw(state, renderer, theme, style, layout, cursor, viewport);
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
        overlay::from_children(
            &mut self.parts,
            tree,
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, M: 'a> From<Frame<'a, M>> for Element<'a, M> {
    fn from(frame: Frame<'a, M>) -> Self {
        Element::new(frame)
    }
}
