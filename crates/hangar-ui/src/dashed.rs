//! A button in a dashed frame, which iced's borders cannot draw: the way to
//! put the first thing in a place that has nothing yet (an empty column of
//! the board).

use hangar_theme::Theme;
use hangar_theme::tokens::{BORDER_WIDTH, scale, space};
use iced::widget::{button, canvas, container, row, stack, text};
use iced::{Alignment, Color, Element, Length, Point, Rectangle, Size, mouse};

use crate::focus;
use crate::icon::{Icon, icon};

/// The button's height (`ecrans.md` §4).
pub const HEIGHT: f32 = 36.0;

/// A dash, then the gap after it.
const DASHES: [f32; 2] = [4.0, 3.0];

/// The outline of the whole room it is given, dashed. It takes no event.
struct Outline {
    colour: Color,
}

impl<M> canvas::Program<M> for Outline {
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
            scale::radius::MD.into(),
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

/// `label` after `lead`, in a dashed frame as wide as its place: pressed with
/// the pointer, or with Space and Enter once it has the keyboard, which
/// `id` lets a view send it to.
pub fn dashed<'a, M: Clone + 'a>(
    t: &'a Theme,
    lead: Icon,
    label: String,
    id: iced::advanced::widget::Id,
    message: M,
) -> Element<'a, M> {
    let content = row![icon(lead, 12.0), text(label).size(12.0)]
        .spacing(space::SM)
        .align_y(Alignment::Center);
    let face = button(container(content).center_y(Length::Fill))
        .width(Length::Fill)
        .height(HEIGHT)
        .padding([0.0, space::LG])
        .style(move |_: &iced::Theme, status| {
            let p = &t.palette;
            let pointed = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: pointed.then_some(iced::Background::Color(p.hover_overlay)),
                text_color: if pointed { p.text } else { p.text_muted },
                border: iced::Border {
                    radius: scale::radius::MD.into(),
                    ..iced::Border::default()
                },
                ..button::Style::default()
            }
        })
        .on_press(message.clone());
    let frame = canvas(Outline {
        colour: t.palette.border_strong,
    })
    .width(Length::Fill)
    .height(HEIGHT);
    focus::ring(t, stack![face, frame])
        .radius(scale::radius::MD)
        .id(id)
        .on_activate(message)
        .into()
}
