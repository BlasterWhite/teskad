//! The head of the column (`InspectorHeader`): the pane's monogram with its
//! state on a badge, its name and the word for its state, the key that
//! focuses it, the close cross, and under them what runs in it.

use hangar_i18n::{Key, args};
use hangar_theme::tokens::{radius, scale, scale::type_scale, space};
use hangar_theme::{AgentState, fonts, style};
use hangar_ui::Icon;
use hangar_ui::button::{Side, icon_button};
use hangar_ui::glyph::{self, GlyphState};
use hangar_ui::keycap::binding;
use hangar_ui::tail_text::tail_text;
use iced::widget::{column, container, row, stack, text};
use iced::{Alignment, Background, Border, Element, Length, Padding};

use super::Msg;
use super::sections::{Scene, ink, state_word};
use super::sources;
use crate::commands::CommandId;
use crate::message::Message;

/// What the head says besides the pane's name.
pub(super) struct Head<'a> {
    pub state: AgentState,
    pub agent_label: &'a str,
    pub is_shell: bool,
    /// The agent's own version, from the mod's hello.
    pub version: Option<&'a str>,
    /// The model, in the words people say ("Sonnet 4.5").
    pub model: Option<String>,
    /// A shell pane's shell, named in place of an agent's version and model.
    pub shell: Option<String>,
    /// 1-based place in reading order, for the "focus pane N" key.
    pub index: usize,
}

/// The first letter of the name, upper-cased, or a question mark.
fn monogram(name: &str) -> String {
    name.trim()
        .chars()
        .next()
        .map(|first| first.to_uppercase().collect())
        .unwrap_or_else(|| "?".into())
}

/// The width of the avatar and its badge's overhang.
const AVATAR: f32 = 32.0;
const OVERHANG: f32 = 3.0;
/// The size of the pane's name.
const NAME: f32 = 15.0;

fn avatar<'a>(s: &Scene<'a>, state: AgentState) -> Element<'a, Message> {
    let t = s.theme();
    let p = &t.palette;
    let (raised, edge, behind) = (p.surface_raised, p.border, p.bg);
    let face = container(
        text(monogram(&s.pane.name))
            .size(type_scale::BODY.size)
            .font(fonts::display())
            .style(style::text::primary(t)),
    )
    .center(AVATAR)
    .style(move |_: &iced::Theme| container::Style {
        background: Some(Background::Color(raised)),
        border: Border {
            color: edge,
            width: 1.0,
            radius: scale::radius::MD.into(),
        },
        ..container::Style::default()
    });
    // The state, pinned to the monogram's corner on a ring of the column's
    // own colour, so it reads as a badge on any theme.
    let badge = container(glyph::state(t, GlyphState::from(state), 8.0))
        .center(14.0)
        .style(move |_: &iced::Theme| container::Style {
            background: Some(Background::Color(behind)),
            border: Border {
                radius: radius::FULL.into(),
                ..Border::default()
            },
            ..container::Style::default()
        });
    stack![
        container(face).padding(Padding {
            right: OVERHANG,
            bottom: OVERHANG,
            ..Padding::ZERO
        }),
        container(badge)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(iced::alignment::Horizontal::Right)
            .align_y(iced::alignment::Vertical::Bottom),
    ]
    .width(AVATAR + OVERHANG)
    .height(AVATAR + OVERHANG)
    .into()
}

pub(super) fn view<'a>(s: &Scene<'a>, head: Head<'a>) -> Element<'a, Message> {
    let (t, tr) = (s.theme(), s.tr());
    let p = &t.palette;
    let small = type_scale::LABEL.size;

    let title = row![
        container(
            tail_text(s.pane.name.clone())
                .cut_end()
                .size(NAME)
                .font(fonts::display())
                .color(p.text)
                .width(Length::Shrink)
        )
        .max_width(150.0),
        tail_text(state_word(tr, head.state))
            .cut_end()
            .size(type_scale::CAPTION.size)
            .color(p.text_muted)
            .width(Length::Shrink),
    ]
    .spacing(space::SM)
    .align_y(Alignment::End);

    // The shortcut that focuses this pane, when it is one of the first nine.
    let focus_command: Option<CommandId> = (1..=9)
        .contains(&head.index)
        .then(|| format!("pane.focus{}", head.index).parse().ok())
        .flatten();
    let mut tools = row![].spacing(4).align_y(Alignment::Center);
    if let Some(keys) = focus_command.and_then(|command| sources::shortcut(s.ctx, command)) {
        let hint = tr
            .t_with(Key::CmdPaneFocusN, &args!("n" => head.index as i64))
            .into_owned();
        tools = tools.push(hangar_ui::tip::bottom(t, binding(t, keys), hint));
    }
    let mut close = icon_button(t, Icon::Close)
        .sized(Side::Xs)
        .icon_size(12.0)
        .tip(tr.t(Key::InspectorClose).into_owned())
        .on_press(Message::Inspector(Msg::Close(
            s.ws.id.clone(),
            Some(s.pane.id.clone()),
        )));
    if let Some(keys) = sources::shortcut(s.ctx, CommandId::ViewInspector) {
        close = close.keys(keys);
    }
    tools = tools.push(close);

    // What runs in it: the agent (and, when there is room, its version) and
    // the model, or the shell.
    let mut words = head.agent_label.to_string();
    if head.is_shell {
        if let Some(shell) = head.shell {
            words = format!("{words} · {shell}");
        }
    } else {
        // A narrow column keeps the essentials: the version goes first.
        if let Some(version) = head.version.filter(|_| s.width > 280.0) {
            words = format!("{words} {version}");
        }
        if let Some(model) = head.model {
            words = format!("{words} · {model}");
        }
    }
    let sub = container(ink(words, small, p.text_muted))
        .clip(true)
        .width(Length::Fill);

    container(
        row![
            avatar(s, head.state),
            column![title, sub].spacing(1).width(Length::Fill),
            tools,
        ]
        .spacing(space::LG)
        .align_y(Alignment::Center),
    )
    .padding(Padding {
        top: space::XL,
        right: space::MD,
        bottom: space::XL,
        left: Scene::PAD,
    })
    .width(Length::Fill)
    .into()
}

#[cfg(test)]
mod tests {
    use super::monogram;

    #[test]
    fn the_monogram_is_the_first_letter_upper_cased() {
        assert_eq!(monogram("ava"), "A");
        assert_eq!(monogram("  ben"), "B");
        assert_eq!(monogram("émile"), "É");
        assert_eq!(monogram(""), "?");
        assert_eq!(monogram("   "), "?");
    }
}
