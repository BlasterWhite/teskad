//! The setup sheet, drawn: one dialog with a card per agent, each following
//! its own step, and on NixOS the way to Claude Code from nixpkgs.

use hangar_core::agent::AgentKind;
use hangar_i18n::{Key, args};
use hangar_theme::{fonts, style, tokens};
use hangar_ui::badge::{self, Tone};
use hangar_ui::dialog::{self, Dialog};
use hangar_ui::{Icon, button as ui_button, text as typography};
use iced::widget::{Column, Row, Space, column, container, row, scrollable};
use iced::{Alignment, Element, Length, Padding};

use super::{
    Failure, Msg, State, Step, any_installed, closable, install, installed, msg, nixos, signed_in,
};
use crate::ctx::Ctx;
use crate::message::Message;

/// The dialog's width: room for the installer's lines.
const WIDTH: f32 = 640.0;
/// The tallest the installer's output grows before it scrolls.
const OUTPUT_HEIGHT: f32 = 200.0;

pub fn view<'a>(st: &'a State, ctx: Ctx<'a>) -> Element<'a, Message> {
    let t = ctx.theme;
    let m = ctx.model;
    let busy = super::busy(st);

    let mut body = Column::new()
        .spacing(tokens::space::LG)
        .push(typography::body(t, ctx.t(Key::SetupLead)));
    if AgentKind::ALL.into_iter().any(|kind| !installed(m, kind)) {
        body = body.push(typography::muted(t, ctx.t(Key::SetupHow)));
    }
    for kind in AgentKind::ALL {
        body = body.push(card(ctx, st, kind, busy));
    }

    let mut panel: Dialog<'a, Message> = dialog::dialog(t, ctx.t(Key::SetupTitle), body)
        .width(WIDTH)
        .body_max_height(600.0)
        .body_padding(Padding::from([18.0, 22.0]))
        .close_tip(ctx.t(Key::ModalClose));
    if !any_installed(m) && !crate::settings::shells_only(m.settings()) && !busy {
        // In the way while neither agent is installed: the way out, at the
        // left of the footer where every dialog keeps it, small, out of the
        // way of the buttons that install, and in view however long the
        // cards are.
        panel = panel.lead(
            row![
                ui_button::ghost(t, ctx.t(Key::SetupShellsOnly))
                    .small()
                    .on_press(msg(Msg::ShellsOnly)),
                typography::hint(t, ctx.t(Key::SetupShellsOnlyHint)),
            ]
            .spacing(tokens::space::SM)
            .align_y(Alignment::Center),
        );
    }
    if closable(st, m) {
        panel = panel
            .on_close(msg(Msg::Done))
            .action(ui_button::primary(t, ctx.t(Key::SetupDone)).on_press(msg(Msg::Done)));
    }
    // Until then, no `on_close`: no close button, and the veil leaves it.
    let page = Space::new().width(Length::Fill).height(Length::Fill);
    dialog::over(t, page, Some(panel)).into()
}

/// Codex support is labelled beta until real users confirm it
/// (docs/native/agents.md §0).
fn is_beta(kind: AgentKind) -> bool {
    matches!(kind, AgentKind::Codex)
}

/// What signing in to `kind` asks for, and where.
fn sign_in_hint(kind: AgentKind) -> Key {
    match kind {
        AgentKind::Claude => Key::SetupSignInClaude,
        AgentKind::Codex => Key::SetupSignInCodex,
    }
}

/// One agent's card: its name and state, then what its step offers, its
/// buttons on its last line at the right.
fn card<'a>(ctx: Ctx<'a>, st: &'a State, kind: AgentKind, busy: bool) -> Element<'a, Message> {
    let t = ctx.theme;
    let m = ctx.model;
    let info = kind.info();
    let card = st.card(kind);
    let here = installed(m, kind);
    let label = info.label;

    let mut head = Row::new()
        .spacing(tokens::space::SM)
        .align_y(Alignment::Center)
        .push(typography::body(t, label).font(fonts::ui_semibold()));
    if is_beta(kind) {
        head = head.push(badge::badge(t, ctx.t(Key::SetupBeta), Tone::Accent));
    }
    head = head.push(Space::new().width(Length::Fill));
    head = head.push(if here {
        let said = match card
            .found
            .as_ref()
            .and_then(|found| found.version.as_deref())
        {
            Some(version) => ctx
                .i18n
                .t_with(Key::SetupInstalledVersion, &args!("version" => version)),
            None => ctx.t(Key::SetupInstalled),
        };
        typography::ui(t, said).style(style::text::success(t))
    } else {
        typography::muted(t, ctx.t(Key::SetupNotInstalled))
    });

    let mut body = Column::new().spacing(tokens::space::MD).push(head);
    // The last line: what stands at its left (filling it), then the buttons.
    let mut left: Option<Element<'a, Message>> = None;
    let mut buttons: Vec<Element<'a, Message>> = Vec::new();
    let installer = install::chosen(kind);
    let named = args!("agent" => label);

    match &card.step {
        Step::Installing => {
            body = body.push(typography::muted(t, ctx.t(Key::SetupInstallingHint)));
            if !card.output.is_empty() {
                body = body.push(output(ctx, &card.output));
            }
            left = Some(command(ctx, installer.shown));
            buttons
                .push(ui_button::primary(t, ctx.i18n.t_with(Key::SetupInstalling, &named)).into());
        }
        Step::Checking => {
            body = nixpkgs(ctx, st, kind, body, true);
            buttons.push(ui_button::primary(t, ctx.t(Key::SetupNixChecking)).into());
        }
        Step::Failed(failure) => {
            let said = match failure {
                Failure::Exit(code) => ctx
                    .i18n
                    .t_with(Key::SetupFailedExit, &args!("code" => code.to_string())),
                Failure::Stopped => ctx.t(Key::SetupFailedSignal),
                Failure::Start(error) => ctx
                    .i18n
                    .t_with(Key::SetupFailedStart, &args!("error" => error.as_str())),
                Failure::NotFound => ctx
                    .i18n
                    .t_with(Key::SetupNotFound, &args!("command" => info.program)),
            };
            body = body
                .push(typography::danger(t, said))
                .push(typography::muted(t, ctx.t(Key::SetupFailedHint)))
                .push(command(ctx, installer.shown));
            body = if card.output.is_empty() {
                body.push(typography::hint(t, ctx.t(Key::SetupNoOutput)))
            } else {
                body.push(output(ctx, &card.output))
            };
            if st.nixos && nixos::from_nixpkgs(kind) {
                body = body.push(back_to_nixpkgs(ctx));
            }
            left = Some(
                typography::mono(t, install::docs_url(kind))
                    .size(tokens::font_size::CAPTION)
                    .width(Length::Fill)
                    .into(),
            );
            buttons.push(
                ui_button::secondary(t, ctx.t(Key::SetupDocs))
                    .on_press(msg(Msg::OpenDocs(kind)))
                    .into(),
            );
            buttons.push(
                ui_button::primary(t, ctx.t(Key::SetupRetry))
                    .on_press_maybe((!busy).then(|| msg(Msg::Install(kind))))
                    .into(),
            );
        }
        Step::Idle if here => {
            if let Some(found) = card.found.as_ref().filter(|found| !found.on_path) {
                let path = found.path.display().to_string();
                body = body.push(typography::hint(
                    t,
                    ctx.i18n
                        .t_with(Key::SetupNotOnPath, &args!("path" => path.as_str())),
                ));
            }
            match signed_in(st, m, kind) {
                Some(true) => {
                    body = body.push(
                        typography::body(t, ctx.t(Key::SetupSignedIn))
                            .style(style::text::success(t)),
                    );
                }
                Some(false) => {
                    left = Some(
                        typography::muted(t, ctx.t(sign_in_hint(kind)))
                            .width(Length::Fill)
                            .into(),
                    );
                    buttons.push(
                        ui_button::primary(t, ctx.t(Key::SetupSignIn))
                            .on_press_maybe((!busy).then(|| msg(Msg::SignIn(kind))))
                            .into(),
                    );
                }
                // Not read yet: nothing said rather than a guess.
                None => {}
            }
        }
        Step::Idle if st.offers_nixpkgs(kind) => {
            body = nixpkgs(ctx, st, kind, body, false);
            left = Some(
                row![
                    ui_button::link(t, ctx.t(Key::SetupNixAnyway), msg(Msg::InstallerAnyway)),
                    typography::hint(t, ctx.t(Key::SetupNixAnywayHint)),
                ]
                .spacing(tokens::space::SM)
                .align_y(Alignment::Center)
                .width(Length::Fill)
                .into(),
            );
            buttons.push(
                ui_button::primary(t, ctx.t(Key::SetupNixCheck))
                    .on_press_maybe((!busy).then(|| msg(Msg::Recheck(kind))))
                    .into(),
            );
        }
        Step::Idle => {
            if st.nixos && nixos::from_nixpkgs(kind) {
                body = body.push(back_to_nixpkgs(ctx));
            }
            left = Some(command(ctx, installer.shown));
            buttons.push(
                ui_button::primary(t, ctx.i18n.t_with(Key::SetupInstall, &named))
                    .on_press_maybe((!busy).then(|| msg(Msg::Install(kind))))
                    .into(),
            );
        }
    }

    if left.is_some() || !buttons.is_empty() {
        let mut line = Row::new()
            .spacing(tokens::space::MD)
            .align_y(Alignment::Center)
            .push(left.unwrap_or_else(|| Space::new().width(Length::Fill).into()));
        for button in buttons {
            line = line.push(button);
        }
        body = body.push(line);
    }
    container(body)
        .padding(Padding::from([tokens::space::MD, tokens::space::LG]))
        .width(Length::Fill)
        .style(style::container::well(t))
        .into()
}

/// On NixOS: where Claude Code comes from there, the lines that install it to
/// copy, and whether « Check again » found it (not while `checking`). The
/// installer, behind a link, is on the card's last line.
fn nixpkgs<'a>(
    ctx: Ctx<'a>,
    st: &State,
    kind: AgentKind,
    body: Column<'a, Message>,
    checking: bool,
) -> Column<'a, Message> {
    let t = ctx.theme;
    let mut body = body
        .push(typography::muted(t, ctx.t(Key::SetupNixLead)))
        .push(
            column![
                typography::muted(t, ctx.t(Key::SetupNixSystem)),
                copyable(ctx, nixos::SYSTEM_PACKAGES),
            ]
            .spacing(tokens::space::SM),
        )
        .push(
            column![
                typography::muted(t, ctx.t(Key::SetupNixProfile)),
                copyable(ctx, nixos::PROFILE_INSTALL),
            ]
            .spacing(tokens::space::SM),
        )
        .push(typography::hint(t, ctx.t(Key::SetupNixUnfree)));
    if st.card(kind).still_missing && !checking {
        body = body.push(typography::danger(t, ctx.t(Key::SetupNixMissing)));
    }
    body
}

/// On NixOS, from the installer's card: back to nixpkgs'.
fn back_to_nixpkgs<'a>(ctx: Ctx<'a>) -> Element<'a, Message> {
    ui_button::link(ctx.theme, ctx.t(Key::SetupNixBack), msg(Msg::Nixpkgs))
}

/// A line to copy, with the button that copies it: iced cannot select text.
fn copyable<'a>(ctx: Ctx<'a>, line: &'static str) -> Element<'a, Message> {
    let t = ctx.theme;
    container(
        row![
            typography::mono(t, line)
                .size(tokens::font_size::LABEL)
                .width(Length::Fill),
            ui_button::icon_button(t, Icon::Copy)
                .small()
                .tip(ctx.t(Key::PreviewCopy))
                .on_press(msg(Msg::Copy(line))),
        ]
        .spacing(tokens::space::MD)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([tokens::space::SM, tokens::space::MD]))
    .width(Length::Fill)
    .style(style::container::card(t))
    .into()
}

/// The command the installer runs, as the person would type it.
fn command<'a>(ctx: Ctx<'a>, line: String) -> Element<'a, Message> {
    let t = ctx.theme;
    container(typography::mono(t, line).size(tokens::font_size::LABEL))
        .padding(Padding::from([tokens::space::SM, tokens::space::MD]))
        .width(Length::Fill)
        .style(style::container::card(t))
        .into()
}

/// What the installer printed, the last line in view.
fn output<'a>(ctx: Ctx<'a>, lines: &'a [String]) -> Element<'a, Message> {
    let t = ctx.theme;
    let lines = column(lines.iter().map(|line| {
        typography::mono(t, line.as_str())
            .size(tokens::font_size::CAPTION)
            .into()
    }))
    .spacing(1)
    .width(Length::Fill);
    container(
        scrollable(container(lines).padding(Padding::from([tokens::space::SM, tokens::space::MD])))
            .anchor_bottom()
            .style(style::scrollable::default(t)),
    )
    .max_height(OUTPUT_HEIGHT)
    .width(Length::Fill)
    .style(style::container::card(t))
    .into()
}
