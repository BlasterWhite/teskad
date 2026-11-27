//! A foldable block of the column (§7 of the brand pack's screens): a 34 px
//! title row with its chevron, the body, and a hairline under it.
//!
//! The title row is one control: it lights up under the pointer, shows the
//! focus ring under the keyboard, and Space or Enter fold it. What sits at its
//! right end (a count, a shell) is part of it; an action ("Open", "View
//! diff") sits beside it, so each stays one control.

use std::f32::consts::FRAC_PI_2;

use hangar_theme::tokens::{BORDER_WIDTH, scale, scale::type_scale, space};
use hangar_theme::{Theme, fonts, style};
use hangar_ui::{Icon, focus, icon};
use iced::widget::{button, column, container, row, rule, space as filler, text};
use iced::{Alignment, Background, Border, Element, Length, Padding};

/// The height of the title row.
const HEAD_HEIGHT: f32 = 34.0;
/// What the title row is inset by on every side but the end, so that the focus
/// ring (2 px, 2 px off) is not cut by the column's edge.
const INSET: f32 = 4.0;

/// A block of the column, open or folded.
#[must_use]
pub struct Section<'a, M> {
    theme: &'a Theme,
    title: String,
    collapsed: bool,
    on_toggle: M,
    meta: Option<Element<'a, M>>,
    action: Option<Element<'a, M>>,
    body: Option<Element<'a, M>>,
    last: bool,
    padding: f32,
}

/// A block titled `title`, `padding` from both sides of the column.
/// `on_toggle` is sent by its title row.
pub fn section<'a, M: Clone + 'a>(
    t: &'a Theme,
    title: impl Into<String>,
    collapsed: bool,
    on_toggle: M,
    padding: f32,
) -> Section<'a, M> {
    Section {
        theme: t,
        title: title.into(),
        collapsed,
        on_toggle,
        meta: None,
        action: None,
        body: None,
        last: false,
        padding,
    }
}

impl<'a, M: Clone + 'a> Section<'a, M> {
    /// A count or a word at the right end of the title, read with it.
    pub fn meta(mut self, meta: impl Into<String>) -> Self {
        let color = self.theme.palette.text_muted;
        self.meta = Some(
            text(meta.into())
                .size(type_scale::LABEL.size)
                .font(fonts::mono())
                .color(color)
                .into(),
        );
        self
    }

    /// The same place, for a total that is more than a word.
    pub fn meta_element(mut self, meta: impl Into<Element<'a, M>>) -> Self {
        self.meta = Some(meta.into());
        self
    }

    /// A link at the far right of the title row. Hidden while folded.
    pub fn action(mut self, action: impl Into<Element<'a, M>>) -> Self {
        self.action = Some(action.into());
        self
    }

    /// What the block holds. Not built into the tree while it is folded.
    pub fn body(mut self, body: impl Into<Element<'a, M>>) -> Self {
        self.body = Some(body.into());
        self
    }

    /// No hairline under it: the last block of the column.
    pub fn last(mut self) -> Self {
        self.last = true;
        self
    }
}

impl<'a, M: Clone + 'a> From<Section<'a, M>> for Element<'a, M> {
    fn from(s: Section<'a, M>) -> Self {
        let t = s.theme;
        let p = t.palette;

        // The chevron points down while the block is open and turns a quarter
        // to point at the title when it is folded.
        let chevron = icon(Icon::ChevronDown, 12.0)
            .color(p.text_muted)
            .rotate(if s.collapsed { -FRAC_PI_2 } else { 0.0 });
        let mut title = row![
            chevron,
            text(s.title)
                .size(type_scale::CAPTION.size)
                .font(fonts::ui_semibold())
                .color(p.text),
        ]
        .spacing(space::SM)
        .align_y(Alignment::Center);
        if let Some(meta) = s.meta {
            title = title.push(filler::horizontal());
            title = title.push(meta);
        }
        let end = s.padding - INSET;
        let toggle = button(container(title).center_y(Length::Fill))
            .padding(Padding {
                left: end,
                right: end,
                ..Padding::ZERO
            })
            .width(Length::Fill)
            .height(HEAD_HEIGHT - 2.0 * INSET)
            .style(move |_: &iced::Theme, status| button::Style {
                background: matches!(status, button::Status::Hovered | button::Status::Pressed)
                    .then_some(Background::Color(p.hover_overlay)),
                text_color: p.text,
                border: Border {
                    radius: scale::radius::SM.into(),
                    ..Border::default()
                },
                ..button::Style::default()
            })
            .on_press(s.on_toggle.clone());
        let toggle = focus::ring(t, toggle)
            .radius(scale::radius::SM)
            .on_activate(s.on_toggle);
        let action = s.action.filter(|_| !s.collapsed);
        let trailing = if action.is_some() { end } else { 0.0 };
        let mut head = row![toggle].spacing(space::SM).align_y(Alignment::Center);
        if let Some(action) = action {
            head = head.push(action);
        }
        let head = container(head).padding(Padding {
            top: INSET,
            right: INSET + trailing,
            bottom: INSET,
            left: INSET,
        });

        let mut block = column![head];
        if let (Some(body), false) = (s.body, s.collapsed) {
            block = block.push(container(body).width(Length::Fill).padding(Padding {
                top: 0.0,
                right: s.padding,
                bottom: space::XL,
                left: s.padding,
            }));
        }
        if !s.last {
            block = block.push(rule::horizontal(BORDER_WIDTH).style(style::rule::hairline(t)));
        }
        block.into()
    }
}
