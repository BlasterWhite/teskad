//! A dashed rounded frame, which iced's borders cannot draw: the queue's.

use iced::widget::canvas;
use iced::{Color, Point, Rectangle, Size, mouse};

use hangar_theme::tokens::BORDER_WIDTH;

/// A dash, then the gap after it.
const DASHES: [f32; 2] = [4.0, 3.0];

/// The outline of the whole room it is given, dashed. It takes no event.
pub struct Dashed {
    pub colour: Color,
    pub radius: f32,
}

impl<Message> canvas::Program<Message> for Dashed {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        // Half the line in from the edge, so none of it is clipped.
        let inset = BORDER_WIDTH / 2.0;
        let outline = canvas::Path::rounded_rectangle(
            Point::new(inset, inset),
            Size::new(
                (bounds.width - BORDER_WIDTH).max(0.0),
                (bounds.height - BORDER_WIDTH).max(0.0),
            ),
            self.radius.into(),
        );
        let mut stroke = canvas::Stroke::default()
            .with_color(self.colour)
            .with_width(BORDER_WIDTH);
        stroke.line_dash = canvas::LineDash {
            segments: &DASHES,
            offset: 0,
        };
        frame.stroke(&outline, stroke);
        vec![frame.into_geometry()]
    }
}
