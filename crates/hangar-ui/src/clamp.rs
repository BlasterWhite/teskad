//! Text that runs over a few lines and no more: past the last one it ends in
//! an ellipsis (`-webkit-line-clamp`, a task's title on its card).
//!
//! ```ignore
//! use hangar_ui::clamp::clamp;
//!
//! clamp(task.title.clone(), 3).size(13.0).line_height(18.0).font(fonts::ui_medium())
//! ```
//!
//! iced's `text` wraps for as long as it has words, so this widget lays the
//! words out in the room it is given and, when they take more lines than it
//! may, keeps the longest beginning that fits with the ellipsis after it. A
//! widget operation reads the whole text, as it reads iced's own `text`.

use iced::advanced::Widget;
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::text::{self, Paragraph as _, Renderer as _, Text};
use iced::advanced::widget::{Operation, Tree, tree};
use iced::{Color, Element, Font, Length, Pixels, Rectangle, Size, alignment, mouse};

type Paragraph = <iced::Renderer as text::Renderer>::Paragraph;

const ELLIPSIS: char = '…';

/// The longest beginning of `content` that `fits` with an ellipsis after it,
/// or all of it when it fits whole. `fits` is asked O(log n) times, and what
/// fits only shrinks as the text grows. Cuts after a whole character and
/// drops the space the cut would leave before the ellipsis.
pub fn fit_lines(content: &str, fits: impl Fn(&str) -> bool) -> String {
    if fits(content) {
        return content.to_string();
    }
    let ends: Vec<usize> = content.char_indices().map(|(at, _)| at).collect();
    let kept = |count: usize| {
        let upto = ends.get(count).copied().unwrap_or(content.len());
        let mut shown = content[..upto].trim_end().to_string();
        shown.push(ELLIPSIS);
        shown
    };
    let (mut low, mut high) = (0, ends.len());
    while low + 1 < high {
        let middle = low + (high - low) / 2;
        if fits(&kept(middle)) {
            low = middle;
        } else {
            high = middle;
        }
    }
    kept(low)
}

/// The widget [`clamp`] makes.
#[must_use]
pub struct Clamp {
    content: String,
    lines: usize,
    size: f32,
    line_height: f32,
    font: Font,
    color: Option<Color>,
}

/// `content` on `lines` lines at most.
pub fn clamp(content: impl Into<String>, lines: usize) -> Clamp {
    Clamp {
        content: content.into(),
        lines: lines.max(1),
        size: 13.0,
        line_height: 18.0,
        font: Font::DEFAULT,
        color: None,
    }
}

impl Clamp {
    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    /// The height of a line, in pixels.
    pub fn line_height(mut self, line_height: f32) -> Self {
        self.line_height = line_height;
        self
    }

    pub fn font(mut self, font: Font) -> Self {
        self.font = font;
        self
    }

    /// The colour of the text; the surrounding text colour when not given.
    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    fn paragraph(&self, content: &str, width: f32) -> Paragraph {
        Paragraph::with_text(Text {
            content,
            bounds: Size::new(width, f32::INFINITY),
            size: Pixels(self.size),
            line_height: text::LineHeight::Absolute(Pixels(self.line_height)),
            font: self.font,
            align_x: text::Alignment::Default,
            align_y: alignment::Vertical::Top,
            shaping: text::Shaping::Advanced,
            wrapping: text::Wrapping::WordOrGlyph,
        })
    }
}

/// What a [`Clamp`] laid out, kept in the widget tree between views.
#[derive(Default)]
struct Memo {
    /// (content, lines, size, line height, face, room) of the paragraph.
    key: Option<(String, usize, u32, u32, Font, u32)>,
    paragraph: Paragraph,
}

impl<Message, Theme> Widget<Message, Theme, iced::Renderer> for Clamp {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Memo>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(Memo::default())
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let width = limits.max().width;
        let memo = tree.state.downcast_mut::<Memo>();
        let key = (
            self.content.clone(),
            self.lines,
            self.size.to_bits(),
            self.line_height.to_bits(),
            self.font,
            width.to_bits(),
        );
        if memo.key.as_ref() != Some(&key) {
            // Half a line of slack: heights come out a fraction off.
            let room = (self.lines as f32 + 0.5) * self.line_height;
            let shown = fit_lines(&self.content, |candidate| {
                self.paragraph(candidate, width).min_height() <= room
            });
            memo.paragraph = self.paragraph(&shown, width);
            memo.key = Some(key);
        }
        let height = memo.paragraph.min_height();
        layout::Node::new(limits.resolve(Length::Fill, height, Size::new(width, height)))
    }

    fn operate(
        &mut self,
        _tree: &mut Tree,
        layout: Layout<'_>,
        _renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        operation.text(None, layout.bounds(), &self.content);
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let Some(clip) = bounds.intersection(viewport) else {
            return;
        };
        renderer.fill_paragraph(
            &tree.state.downcast_ref::<Memo>().paragraph,
            bounds.position(),
            self.color.unwrap_or(style.text_color),
            clip,
        );
    }
}

impl<'a, Message: 'a, Theme: 'a> From<Clamp> for Element<'a, Message, Theme, iced::Renderer> {
    fn from(clamp: Clamp) -> Self {
        Element::new(clamp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ten characters to a line, two lines.
    fn twenty(text: &str) -> bool {
        text.chars().count() <= 20
    }

    #[test]
    fn text_that_fits_is_left_whole() {
        assert_eq!(fit_lines("a short title", twenty), "a short title");
    }

    #[test]
    fn text_that_runs_over_ends_in_an_ellipsis() {
        let shown = fit_lines("a title that goes on for much too long", twenty);
        assert_eq!(shown, "a title that goes o…");
        assert!(twenty(&shown));
    }

    #[test]
    fn no_space_is_left_before_the_ellipsis() {
        assert_eq!(
            fit_lines("nineteen characte and more", twenty),
            "nineteen characte a…"
        );
        assert_eq!(fit_lines("abc def ghi", |t| t.chars().count() <= 5), "abc…");
    }

    #[test]
    fn it_cuts_between_characters_not_bytes() {
        assert_eq!(fit_lines("été été", |t| t.chars().count() <= 4), "été…");
    }
}
