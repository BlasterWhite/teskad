//! The « Compte Hangar » card of the settings: signed out, waiting for the
//! browser, and signed in (identity, the sync switch, what leaves this PC, the
//! machines, sign out and delete).

use hangar_core::cloud::{Account, Device, Error};
use hangar_i18n::{Key, Translator, args};
use hangar_theme::tokens::{font_size, radius, space};
use hangar_theme::{fonts, style};
use hangar_ui::{badge, button as ui_button, form, input, text as typography};
use iced::widget::{Space, column, container, row, text};
use iced::{Alignment, Element, Length, Padding};

use super::{Msg, Phase, State, delete_word_typed, say};
use crate::ctx::Ctx;
use crate::message::Message;
use crate::stats::format::prose_time;

/// The card, as the settings page draws it.
pub fn card<'a>(ctx: Ctx<'a>) -> Element<'a, Message> {
    let st = &ctx.pkg.hangar_account;
    let t = ctx.theme;
    let mut body = column![].spacing(space::XL).width(Length::Fill);
    if let Some(error) = st
        .error
        .as_ref()
        .and_then(|error| describe(ctx.i18n, error))
    {
        body = body.push(typography::danger(t, error));
    }
    match &st.phase {
        Phase::Unknown => {}
        Phase::SignedOut => body = body.push(signed_out(ctx)),
        Phase::Waiting => body = body.push(waiting(ctx)),
        Phase::SignedIn { account, .. } => {
            body = body.push(identity(ctx, account.as_ref()));
            body = body.push(sync(st, ctx));
            body = body.push(leaves(ctx));
            body = body.push(machines(st, ctx));
            body = body.push(leaving(st, ctx));
        }
    }
    body.into()
}

/// An error in plain words, in the person's language. `None` for what needs
/// no sentence (a sign-in the person cancelled).
pub(super) fn describe(t: Translator, error: &Error) -> Option<String> {
    let said = |key: Key| Some(t.t(key).into_owned());
    let with =
        |key: Key, detail: String| Some(t.t_with(key, &args!("detail" => detail)).into_owned());
    match error {
        Error::Cancelled => None,
        Error::Unreachable(_) => said(Key::HangarAccountErrUnreachable),
        Error::SignedOut | Error::NotSignedIn => said(Key::HangarAccountErrSignedOut),
        Error::Denied => said(Key::HangarAccountErrDenied),
        Error::Provider => said(Key::HangarAccountErrProvider),
        Error::TimedOut => said(Key::HangarAccountErrTimedOut),
        Error::Refused { status: 429, .. } => said(Key::HangarAccountErrBusy),
        Error::Keychain(why) => with(Key::HangarAccountErrKeychain, why.clone()),
        other => with(Key::HangarAccountErrOther, other.to_string()),
    }
}

fn padded<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .padding(Padding::from([space::XL, space::XL]))
        .width(Length::Fill)
        .into()
}

fn signed_out<'a>(ctx: Ctx<'a>) -> Element<'a, Message> {
    let t = ctx.theme;
    form::group(
        t,
        vec![padded(
            ui_button::primary(t, ctx.t(Key::HangarAccountSignInGithub).into_owned())
                .on_press(say(Msg::SignIn(hangar_core::cloud::Provider::Github))),
        )],
    )
}

fn waiting<'a>(ctx: Ctx<'a>) -> Element<'a, Message> {
    let t = ctx.theme;
    form::group(
        t,
        vec![padded(
            row![
                column![
                    typography::body(t, ctx.t(Key::HangarAccountWaiting).into_owned())
                        .font(fonts::ui_semibold()),
                    typography::hint(t, ctx.t(Key::HangarAccountWaitingHint).into_owned()),
                ]
                .spacing(2)
                .width(Length::Fill),
                ui_button::ghost(t, ctx.t(Key::HangarAccountCancel).into_owned())
                    .on_press(say(Msg::SignInCancel)),
            ]
            .spacing(space::MD)
            .align_y(Alignment::Center),
        )],
    )
}

/// The avatar: the account's initial on a disc. The picture itself is not
/// fetched: loading an address the server was given would make the window
/// call a third party the person never chose.
fn avatar<'a>(ctx: Ctx<'a>, name: &str) -> Element<'a, Message> {
    let c = &ctx.theme.colors;
    let accent = c.accent;
    let ink = c.accent_fg;
    let initial = name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "H".to_string());
    container(
        text(initial)
            .size(font_size::BODY)
            .font(fonts::ui_semibold())
            .color(ink),
    )
    .width(34)
    .height(34)
    .center(Length::Shrink)
    .style(move |_: &iced::Theme| container::Style {
        background: Some(iced::Background::Color(accent)),
        border: iced::Border {
            radius: radius::FULL.into(),
            ..iced::Border::default()
        },
        ..container::Style::default()
    })
    .into()
}

fn identity<'a>(ctx: Ctx<'a>, account: Option<&'a Account>) -> Element<'a, Message> {
    let t = ctx.theme;
    let (name, email) = match account {
        Some(account) => (account.display_name.clone(), account.email.clone()),
        None => (ctx.t(Key::HangarAccountTitle).into_owned(), None),
    };
    let shown = match account {
        Some(_) => ctx
            .i18n
            .t_with(
                Key::HangarAccountSignedInAs,
                &args!("name" => name.as_str()),
            )
            .into_owned(),
        None => name.clone(),
    };
    let mut lines = column![typography::body(t, shown).font(fonts::ui_semibold())].spacing(2);
    if let Some(email) = email {
        lines = lines.push(typography::hint(t, email));
    }
    form::group(
        t,
        vec![padded(
            row![avatar(ctx, &name), lines]
                .spacing(space::LG)
                .align_y(Alignment::Center),
        )],
    )
}

fn sync<'a>(st: &'a State, ctx: Ctx<'a>) -> Element<'a, Message> {
    let t = ctx.theme;
    let switch = input::switch(t, st.sync_on).on_toggle(|on| say(Msg::SetSync(on)));
    let toggle = form::row(t, ctx.t(Key::HangarAccountSyncTitle).into_owned(), switch)
        .hint(ctx.t(Key::HangarAccountSyncHint).into_owned());

    let status = if st.syncing {
        ctx.t(Key::HangarAccountSyncing).into_owned()
    } else {
        match st.last_sync {
            Some(at) => ctx
                .i18n
                .t_with(
                    Key::HangarAccountLastSync,
                    &args!("time" => prose_time(at, ctx.i18n.locale())),
                )
                .into_owned(),
            None => ctx.t(Key::HangarAccountNeverSynced).into_owned(),
        }
    };
    let now = ui_button::secondary(t, ctx.t(Key::HangarAccountSyncNow).into_owned())
        .small()
        .on_press_maybe((st.sync_on && !st.syncing).then(|| say(Msg::SyncNow)));
    let mut rows = vec![toggle.into()];
    if st.sync_on {
        rows.push(padded(
            row![typography::hint(t, status).width(Length::Fill), now]
                .spacing(space::MD)
                .align_y(Alignment::Center),
        ));
    }
    form::group(t, rows)
}

fn leaves<'a>(ctx: Ctx<'a>) -> Element<'a, Message> {
    let t = ctx.theme;
    form::group(
        t,
        vec![padded(
            column![
                typography::body(t, ctx.t(Key::HangarAccountLeavesTitle).into_owned())
                    .font(fonts::ui_semibold()),
                typography::hint(t, ctx.t(Key::HangarAccountLeavesBody).into_owned()),
                ui_button::ghost(t, ctx.t(Key::HangarAccountPrivacy).into_owned())
                    .small()
                    .on_press(say(Msg::OpenPrivacy)),
            ]
            .spacing(space::MD)
            .align_x(Alignment::Start),
        )],
    )
}

fn os_name(os: &str) -> String {
    match os {
        "windows" => "Windows".into(),
        "macos" => "macOS".into(),
        "linux" => "Linux".into(),
        other => other.to_string(),
    }
}

fn machines<'a>(st: &'a State, ctx: Ctx<'a>) -> Element<'a, Message> {
    let t = ctx.theme;
    let this = match &st.phase {
        Phase::SignedIn { device, .. } => device.as_deref(),
        _ => None,
    };
    let title = typography::heading(t, ctx.t(Key::HangarAccountMachinesTitle).into_owned());
    let list: Element<'a, Message> = match &st.machines {
        None => typography::hint(t, ctx.t(Key::HangarAccountMachinesLoading).into_owned()).into(),
        Some(devices) => {
            let rows = devices
                .iter()
                .map(|device| machine_row(st, ctx, device, this == Some(device.id.as_str())))
                .collect();
            form::group(t, rows)
        }
    };
    column![title, list].spacing(space::MD).into()
}

fn machine_row<'a>(
    st: &'a State,
    ctx: Ctx<'a>,
    device: &'a Device,
    this: bool,
) -> Element<'a, Message> {
    let t = ctx.theme;
    let mut head = row![typography::body(t, device.name.as_str()).font(fonts::ui_semibold())]
        .spacing(space::MD)
        .align_y(Alignment::Center);
    if this {
        head = head.push(badge::tag(t, ctx.t(Key::HangarAccountThisPc).into_owned()));
    }
    if device.revoked_at.is_some() {
        head = head.push(badge::badge(
            t,
            ctx.t(Key::HangarAccountMachineSignedOut).into_owned(),
            badge::Tone::Neutral,
        ));
    }
    let seen = ctx
        .i18n
        .t_with(
            Key::HangarAccountLastSeen,
            &args!("time" => prose_time(device.last_seen_at, ctx.i18n.locale())),
        )
        .into_owned();
    let detail = typography::hint(t, format!("{} · {seen}", os_name(&device.os)));
    let mut lines = column![head, detail].spacing(space::SM);

    let renaming = st
        .renaming
        .as_ref()
        .filter(|(id, _)| *id == device.id)
        .map(|(_, name)| name.as_str());
    let controls: Element<'a, Message> = if let Some(name) = renaming {
        rename_form(st, ctx, name)
    } else if st.removing.as_deref() == Some(device.id.as_str()) {
        remove_confirm(st, ctx, device)
    } else {
        let idle = !st.busy;
        let mut actions = row![
            ui_button::ghost(t, ctx.t(Key::HangarAccountRename).into_owned())
                .small()
                .on_press_maybe(idle.then(|| say(Msg::RenameStart(device.id.clone()))))
        ]
        .spacing(space::MD);
        // This PC signs out (below); another machine can be removed.
        if !this {
            actions = actions.push(
                ui_button::ghost(t, ctx.t(Key::HangarAccountRemove).into_owned())
                    .small()
                    .on_press_maybe(idle.then(|| say(Msg::RemoveAsk(device.id.clone())))),
            );
        }
        actions.into()
    };
    lines = lines.push(controls);
    padded(lines)
}

fn rename_form<'a>(st: &'a State, ctx: Ctx<'a>, name: &'a str) -> Element<'a, Message> {
    let t = ctx.theme;
    let field = input::text_field(t, name)
        .on_input(|value| say(Msg::RenameInput(value)))
        .on_submit(say(Msg::RenameSubmit));
    column![
        field,
        row![
            ui_button::primary(t, ctx.t(Key::HangarAccountRenameSave).into_owned())
                .small()
                .on_press_maybe(
                    (!st.busy && !name.trim().is_empty()).then(|| say(Msg::RenameSubmit))
                ),
            ui_button::ghost(t, ctx.t(Key::HangarAccountCancel).into_owned())
                .small()
                .on_press(say(Msg::RenameCancel)),
        ]
        .spacing(space::MD),
    ]
    .spacing(space::MD)
    .into()
}

fn remove_confirm<'a>(st: &'a State, ctx: Ctx<'a>, device: &'a Device) -> Element<'a, Message> {
    let t = ctx.theme;
    let ask = ctx
        .i18n
        .t_with(
            Key::HangarAccountRemoveAsk,
            &args!("name" => device.name.as_str()),
        )
        .into_owned();
    container(
        column![
            typography::body(t, ask).font(fonts::ui_semibold()),
            typography::hint(t, ctx.t(Key::HangarAccountRemoveBody).into_owned()),
            row![
                ui_button::danger(t, ctx.t(Key::HangarAccountRemoveConfirm).into_owned())
                    .small()
                    .on_press_maybe((!st.busy).then(|| say(Msg::RemoveConfirm(device.id.clone())))),
                ui_button::ghost(t, ctx.t(Key::HangarAccountCancel).into_owned())
                    .small()
                    .on_press(say(Msg::RemoveCancel)),
            ]
            .spacing(space::MD),
        ]
        .spacing(space::MD),
    )
    .padding(space::XL)
    .width(Length::Fill)
    .style(style::container::well(t))
    .into()
}

/// Sign out, and the deletion of the account behind its strong confirmation:
/// the word has to be typed.
fn leaving<'a>(st: &'a State, ctx: Ctx<'a>) -> Element<'a, Message> {
    let t = ctx.theme;
    let idle = !st.busy;
    let sign_out = form::row(
        t,
        ctx.t(Key::HangarAccountSignOut).into_owned(),
        ui_button::secondary(t, ctx.t(Key::HangarAccountSignOut).into_owned())
            .small()
            .on_press_maybe(idle.then(|| say(Msg::SignOut))),
    )
    .hint(ctx.t(Key::HangarAccountSignOutHint).into_owned());

    let delete: Element<'a, Message> = match &st.deleting {
        None => form::row(
            t,
            ctx.t(Key::HangarAccountDeleteTitle).into_owned(),
            ui_button::danger(t, ctx.t(Key::HangarAccountDeleteTitle).into_owned())
                .small()
                .on_press_maybe(idle.then(|| say(Msg::DeleteAsk))),
        )
        .hint(ctx.t(Key::HangarAccountDeleteBody).into_owned())
        .into(),
        Some(typed) => {
            let word = ctx.t(Key::HangarAccountDeleteWord).into_owned();
            let ready = delete_word_typed(typed, &word);
            let ask = ctx
                .i18n
                .t_with(
                    Key::HangarAccountDeleteType,
                    &args!("word" => word.as_str()),
                )
                .into_owned();
            padded(
                column![
                    typography::body(t, ctx.t(Key::HangarAccountDeleteTitle).into_owned())
                        .font(fonts::ui_semibold()),
                    typography::hint(t, ctx.t(Key::HangarAccountDeleteBody).into_owned()),
                    typography::hint(t, ask),
                    input::text_field(t, typed.as_str())
                        .on_input(|value| say(Msg::DeleteInput(value)))
                        .on_submit(say(Msg::DeleteConfirm)),
                    row![
                        ui_button::danger(t, ctx.t(Key::HangarAccountDeleteConfirm).into_owned())
                            .small()
                            .on_press_maybe((idle && ready).then(|| say(Msg::DeleteConfirm))),
                        ui_button::ghost(t, ctx.t(Key::HangarAccountCancel).into_owned())
                            .small()
                            .on_press(say(Msg::DeleteCancel)),
                    ]
                    .spacing(space::MD),
                ]
                .spacing(space::MD),
            )
        }
    };
    let _ = Space::new();
    form::group(t, vec![sign_out.into(), delete])
}
