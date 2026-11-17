//! The setup sheet, drawn: one dialog whose body and buttons follow the step,
//! and on NixOS the way to Claude Code from nixpkgs.

use hangar_i18n::{Key, args};
use hangar_theme::{style, tokens};
use hangar_ui::dialog::{self, Dialog};
use hangar_ui::{Icon, button as ui_button, text as typography};
use iced::widget::{Column, Space, column, container, row, scrollable};
use iced::{Alignment, Element, Length, Padding};

use super::{Failure, Msg, State, Step, install, msg, nixos, signed_in};
use crate::ctx::Ctx;
use crate::message::Message;

/// The dialog's width: room for the installer's lines.
const WIDTH: f32 = 600.0;
/// The tallest the installer's output grows before it scrolls.
const OUTPUT_HEIGHT: f32 = 220.0;

pub fn view<'a>(st: &'a State, ctx: Ctx<'a>) -> Element<'a, Message> {
    let t = ctx.theme;
    let installer = install::chosen();
    let mut body = Column::new().spacing(tokens::space::LG);
    let mut actions: Vec<Element<'a, Message>> = Vec::new();
    // The way out, everywhere but while the installer runs.
    let mut way_out = true;

    match &st.step {
        Step::Offer if st.offers_nixpkgs() => {
            body = nixpkgs(ctx, st, body, false);
            actions.push(
                ui_button::primary(t, ctx.t(Key::SetupNixCheck))
                    .on_press(msg(Msg::Recheck))
                    .into(),
            );
        }
        Step::Checking => {
            way_out = false;
            body = nixpkgs(ctx, st, body, true);
            actions.push(ui_button::primary(t, ctx.t(Key::SetupNixChecking)).into());
        }
        Step::Offer => {
            body = body
                .push(typography::body(t, ctx.t(Key::SetupLead)))
                .push(typography::muted(t, ctx.t(Key::SetupHow)))
                .push(command(ctx, installer.shown));
            actions.push(
                ui_button::primary(t, ctx.t(Key::SetupInstall))
                    .on_press(msg(Msg::Install))
                    .into(),
            );
            if st.nixos {
                body = body.push(back_to_nixpkgs(ctx));
            }
        }
        Step::Installing => {
            way_out = false;
            body = body
                .push(typography::heading(t, ctx.t(Key::SetupInstalling)))
                .push(typography::muted(t, ctx.t(Key::SetupInstallingHint)))
                .push(command(ctx, installer.shown));
            if !st.output.is_empty() {
                body = body.push(output(ctx, &st.output));
            }
            actions.push(ui_button::primary(t, ctx.t(Key::SetupInstalling)).into());
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
                Failure::NotFound => ctx.t(Key::SetupNotFound),
            };
            body = body
                .push(typography::danger(t, said))
                .push(typography::muted(t, ctx.t(Key::SetupFailedHint)))
                .push(command(ctx, installer.shown));
            body = if st.output.is_empty() {
                body.push(typography::hint(t, ctx.t(Key::SetupNoOutput)))
            } else {
                body.push(output(ctx, &st.output))
            };
            body =
                body.push(typography::mono(t, install::DOCS_URL).size(tokens::font_size::CAPTION));
            if st.nixos {
                body = body.push(back_to_nixpkgs(ctx));
            }
            actions.push(
                ui_button::secondary(t, ctx.t(Key::SetupDocs))
                    .on_press(msg(Msg::OpenDocs))
                    .into(),
            );
            actions.push(
                ui_button::primary(t, ctx.t(Key::SetupRetry))
                    .on_press(msg(Msg::Install))
                    .into(),
            );
        }
        Step::SignIn => {
            way_out = false;
            body = body.push(typography::heading(t, ctx.t(Key::SetupInstalled)));
            if signed_in(ctx.model) {
                body = body.push(typography::body(t, ctx.t(Key::SetupSignedIn)));
                actions.push(
                    ui_button::primary(t, ctx.t(Key::SetupDone))
                        .on_press(msg(Msg::Done))
                        .into(),
                );
            } else {
                body = body.push(typography::body(t, ctx.t(Key::SetupSignInLead)));
                actions.push(
                    ui_button::ghost(t, ctx.t(Key::SetupDone))
                        .on_press(msg(Msg::Done))
                        .into(),
                );
                actions.push(
                    ui_button::primary(t, ctx.t(Key::SetupSignIn))
                        .on_press(msg(Msg::SignIn))
                        .into(),
                );
            }
        }
    }

    if way_out {
        // Small, at the foot of the text: there for whoever wants it, out of
        // the way of the button that installs.
        body = body.push(
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
    let mut panel: Dialog<'a, Message> = dialog::dialog(t, ctx.t(Key::SetupTitle), body)
        .width(WIDTH)
        .body_max_height(520.0)
        .body_padding(Padding::from([18.0, 22.0]))
        .close_tip(ctx.t(Key::ModalClose));
    for action in actions {
        panel = panel.action(action);
    }
    // No `on_close`: no close button, and Escape and the veil leave it.
    let page = Space::new().width(Length::Fill).height(Length::Fill);
    dialog::over(t, page, Some(panel)).into()
}

/// On NixOS: where Claude Code comes from there, the lines that install it to
/// copy, and the installer behind a link (not while `checking`).
fn nixpkgs<'a>(
    ctx: Ctx<'a>,
    st: &State,
    body: Column<'a, Message>,
    checking: bool,
) -> Column<'a, Message> {
    let t = ctx.theme;
    let mut body = body
        .push(typography::body(t, ctx.t(Key::SetupLead)))
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
    if st.still_missing && !checking {
        body = body.push(typography::danger(t, ctx.t(Key::SetupNixMissing)));
    }
    if !checking {
        body = body.push(
            row![
                ui_button::link(t, ctx.t(Key::SetupNixAnyway), msg(Msg::InstallerAnyway)),
                typography::hint(t, ctx.t(Key::SetupNixAnywayHint)),
            ]
            .spacing(tokens::space::SM)
            .align_y(Alignment::Center),
        );
    }
    body
}

/// On NixOS, from the installer's sheet: back to nixpkgs'.
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
    .style(style::container::well(t))
    .into()
}

/// The command the installer runs, as the person would type it.
fn command<'a>(ctx: Ctx<'a>, line: String) -> Element<'a, Message> {
    let t = ctx.theme;
    container(typography::mono(t, line).size(tokens::font_size::LABEL))
        .padding(Padding::from([tokens::space::SM, tokens::space::MD]))
        .width(Length::Fill)
        .style(style::container::well(t))
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
    .style(style::container::well(t))
    .into()
}
