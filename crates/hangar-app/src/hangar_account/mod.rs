//! The optional Hangar account, seen from the window: the « Compte Hangar »
//! card of the settings, the statistics sync's schedule, and the merged
//! statistics of every machine that the Statistics page can show.
//!
//! Not `accounts`: that package is the Claude accounts a pane runs under.
//! This one is hangar-core's `cloud` module (docs/native/cloud.md) with a face.
//!
//! # What it does, and when
//!
//! - Nothing, for a person who never signed in. The only thing it does at
//!   start is ask the operating system's keychain whether a token is there
//!   (never the server, never the network); without one it stays quiet, and
//!   the card offers the two sign-in buttons.
//! - Signed in, it asks the server who this is (`me`) and lists the machines.
//!   Statistics leave only while the sync switch is on. The switch is off
//!   until the person turns it on, is kept in a small file of the app data
//!   folder (`hangar-account.json`), and is turned off again by signing out
//!   or deleting the account.
//! - With the switch on, a sync runs at once, then every 15 minutes
//!   (`Schedule`), whenever the Statistics page is opened, and on « Sync
//!   now ». A sync sends this machine's rows, then reads the whole account's
//!   back for the « Toutes les machines » view.
//!
//! Every call into the cloud runs on a blocking thread (`Task::perform`), so
//! the UI thread never waits on the keychain, the disk or the network, and a
//! server that is down or a PC that is offline shows a sentence, nothing else.

mod merge;
mod view;

pub use merge::{Merged, Totals, merge};
pub use view::card;

use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hangar_core::Host;
use hangar_core::cloud::{
    Account, Cancel, Cloud, Device, Error, Provider, Schedule, SignIn, SignedIn, StatsResponse,
    Synced,
};
use iced::{Subscription, Task};

use crate::message::{Intent, Message};
use crate::services::{CoreEvent, Services};
use crate::shell::View;
use crate::state::Model;
use crate::stats::time;

/// Where the privacy page lives, for « Read the privacy page ».
pub const PRIVACY_URL: &str = "https://hangar.simnjs.fr/privacy.html";

/// How often the schedule is looked at while the sync is on. The sync itself
/// is due every 15 minutes (`SYNC_EVERY`); this is only how often to ask.
const LOOK_EVERY: Duration = Duration::from_secs(60);

/// The small file of the app data folder that remembers the sync switch.
const PREFS_FILE: &str = "hangar-account.json";

/// The cloud handle, which has no `Debug` of its own.
#[derive(Clone)]
struct Link(Cloud);

impl fmt::Debug for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Cloud")
    }
}

/// A sign-in under way, passed through a message: whoever takes it first waits
/// for the browser.
#[derive(Clone)]
pub struct Pending(Arc<Mutex<Option<SignIn>>>);

impl fmt::Debug for Pending {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The address carries the sign-in's state: never printed.
        f.write_str("SignIn")
    }
}

#[derive(Clone)]
struct Canceller(Cancel);

impl fmt::Debug for Canceller {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Cancel")
    }
}

/// Where the account stands on this machine.
#[derive(Debug, Clone, Default)]
pub enum Phase {
    /// The keychain has not been asked yet.
    #[default]
    Unknown,
    SignedOut,
    /// The browser is open on the provider's page.
    Waiting,
    /// A token is held. `account` is `None` while the server has not said who
    /// it is (not asked yet, or unreachable): the card then says so.
    SignedIn {
        account: Option<Account>,
        /// The server's id of this PC's device.
        device: Option<String>,
    },
}

/// The rows of every machine, as last read.
#[derive(Debug, Clone)]
pub struct Pulled {
    pub answer: StatsResponse,
    /// When they were read, ms since the epoch.
    pub at: i64,
    /// The server's id of this PC's device, to mark it in the breakdown.
    pub this: Option<String>,
}

/// The package's UI state; lives in `App.pkg.hangar_account`.
#[derive(Debug, Default)]
pub struct State {
    cloud: Option<Link>,
    started: bool,
    phase: Phase,
    /// The statistics sync switch. Off until the person turns it on.
    sync_on: bool,
    machines: Option<Vec<Device>>,
    /// The machine being renamed, and the name as typed so far.
    renaming: Option<(String, String)>,
    /// The machine whose removal is being asked.
    removing: Option<String>,
    /// The deletion of the account is being asked: what was typed so far.
    deleting: Option<String>,
    /// A command of the card runs: its buttons wait.
    busy: bool,
    error: Option<Error>,
    syncing: bool,
    last_sync: Option<i64>,
    pulled: Option<Arc<Pulled>>,
    schedule: Schedule,
    canceller: Option<Canceller>,
}

impl State {
    /// Whether a token is held on this machine.
    pub fn signed_in(&self) -> bool {
        matches!(self.phase, Phase::SignedIn { .. })
    }

    /// Whether the statistics sync is on.
    #[cfg(test)]
    pub fn sync_on(&self) -> bool {
        self.sync_on
    }

    /// The account's rows from every machine, once they have been read, while
    /// signed in with the sync on: what the « Toutes les machines » view of
    /// the Statistics page draws. `None` hides that view's switch.
    pub fn all_machines(&self) -> Option<&Pulled> {
        if self.signed_in() && self.sync_on {
            self.pulled.as_deref()
        } else {
            None
        }
    }

    /// Signed in, the sync on, and `answer` read from the account: what the
    /// Statistics page needs to offer « Toutes les machines ».
    #[cfg(test)]
    pub(crate) fn synced_for_test(
        account: Account,
        device: Device,
        answer: StatsResponse,
    ) -> State {
        let mut st = State::signed_in_for_test(account, device.clone());
        st.sync_on = true;
        st.pulled = Some(Arc::new(Pulled {
            answer,
            at: 1_794_000_000_000,
            this: Some(device.id),
        }));
        st
    }

    #[cfg(test)]
    pub(crate) fn signed_in_for_test(account: Account, device: Device) -> State {
        State {
            started: true,
            phase: Phase::SignedIn {
                account: Some(account),
                device: Some(device.id.clone()),
            },
            machines: Some(vec![device]),
            ..State::default()
        }
    }
}

/// The package's messages, routed by the shell as `Message::HangarAccount`.
#[derive(Debug, Clone)]
pub enum Msg {
    /// The window started: ask the keychain.
    Boot,
    /// The keychain's answer, and what the server said of the token if there
    /// was one: whether the sync was left on, and who is signed in.
    Booted(Result<Option<SignedIn>, Error>, bool),

    SignIn(Provider),
    SignInStarted(Result<Pending, Error>),
    SignInDone(Result<SignedIn, Error>),
    SignInCancel,

    Machines(Result<Vec<Device>, Error>),
    RenameStart(String),
    RenameInput(String),
    RenameCancel,
    RenameSubmit,
    Renamed(Result<Device, Error>),
    RemoveAsk(String),
    RemoveCancel,
    RemoveConfirm(String),
    Removed(Result<(), Error>),

    SetSync(bool),
    SyncNow,
    /// Time to look at the schedule.
    Tick,
    Synced(Result<(Synced, StatsResponse), Error>),

    SignOut,
    SignedOut(Result<(), Error>),
    DeleteAsk,
    DeleteInput(String),
    DeleteCancel,
    DeleteConfirm,
    Deleted(Result<(), Error>),

    OpenPrivacy,
    /// The link was opened (or not): nothing to do about it.
    Opened,
}

fn say(msg: Msg) -> Message {
    Message::HangarAccount(msg)
}

/// Runs `work` on a blocking thread, for `Task::perform`.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, Error> + Send + 'static,
) -> Result<T, Error> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|err| Error::Local(format!("the task stopped: {err}")))?
}

// ---- the preference ----------------------------------------------------------------------------------

fn prefs_path(host: &Host) -> Option<PathBuf> {
    host.paths()
        .app_data_dir()
        .ok()
        .map(|dir| dir.join(PREFS_FILE))
}

/// Whether the sync was left on. Off for a missing or damaged file.
fn read_sync_flag(host: &Host) -> bool {
    prefs_path(host).is_some_and(|path| read_flag_at(&path))
}

fn read_flag_at(path: &std::path::Path) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|json| json.get("sync").and_then(serde_json::Value::as_bool))
        .unwrap_or(false)
}

fn write_sync_flag(host: &Host, on: bool) -> Result<(), String> {
    let path = prefs_path(host).ok_or("no app data folder")?;
    write_flag_at(&path, on)
}

fn write_flag_at(path: &std::path::Path, on: bool) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|err| err.to_string())?;
    }
    std::fs::write(path, serde_json::json!({ "sync": on }).to_string())
        .map_err(|err| err.to_string())
}

fn save_sync_flag(sv: &Services, on: bool) {
    let host = sv.core().host().clone();
    tokio::task::spawn_blocking(move || {
        let _ = write_sync_flag(&host, on);
    });
}

// ---- the update --------------------------------------------------------------------------------------

fn cloud(st: &mut State, sv: &Services) -> Result<Cloud, Error> {
    if let Some(Link(cloud)) = &st.cloud {
        return Ok(cloud.clone());
    }
    let cloud = Cloud::new(sv.core().host())?;
    st.cloud = Some(Link(cloud.clone()));
    Ok(cloud)
}

/// The account's own sentence for what it was asked, when it failed: a token
/// that no longer works signs the card out.
fn fail(st: &mut State, error: Error) {
    if matches!(error, Error::SignedOut | Error::NotSignedIn) {
        st.phase = Phase::SignedOut;
        st.sync_on = false;
        st.pulled = None;
        st.machines = None;
    }
    st.error = Some(error);
}

fn load_machines(st: &mut State, sv: &Services) -> Task<Message> {
    let Ok(cloud) = cloud(st, sv) else {
        return Task::none();
    };
    Task::perform(blocking(move || cloud.devices()), |result| {
        say(Msg::Machines(result))
    })
}

/// Starts a sync, unless one runs or the preconditions are not there.
fn start_sync(st: &mut State, sv: &Services) -> Task<Message> {
    if st.syncing || !st.signed_in() || !st.sync_on {
        return Task::none();
    }
    let Ok(cloud) = cloud(st, sv) else {
        return Task::none();
    };
    st.syncing = true;
    st.schedule.ran(Instant::now());
    let host = sv.core().host().clone();
    Task::perform(
        async move {
            let synced = cloud.sync_stats(host, true).await?;
            let reader = cloud.clone();
            let answer = blocking(move || reader.pull_stats(true, None, None)).await?;
            Ok((synced, answer))
        },
        |result| say(Msg::Synced(result)),
    )
}

fn signed_in_as(st: &mut State, who: SignedIn) {
    st.phase = Phase::SignedIn {
        device: Some(who.device.id.clone()),
        account: Some(who.account),
    };
    st.error = None;
}

pub fn update(st: &mut State, m: &mut Model, sv: &Services, msg: Msg) -> Task<Message> {
    let _ = &m;
    match msg {
        Msg::Boot => {
            st.started = true;
            let host = sv.core().host().clone();
            let Ok(cloud) = cloud(st, sv) else {
                st.phase = Phase::SignedOut;
                return Task::none();
            };
            Task::perform(
                blocking(move || {
                    let on = read_sync_flag(&host);
                    // The keychain only: a PC nobody signed in on never
                    // reaches the server.
                    if !cloud.signed_in()? {
                        return Ok((None, false));
                    }
                    Ok((Some(cloud.me()?), on))
                }),
                |result: Result<(Option<hangar_core::cloud::MeResponse>, bool), Error>| {
                    say(match result {
                        Ok((me, on)) => Msg::Booted(
                            Ok(me.map(|me| SignedIn {
                                account: me.account,
                                device: me.device,
                            })),
                            on,
                        ),
                        Err(err) => Msg::Booted(Err(err), false),
                    })
                },
            )
        }
        Msg::Booted(result, on) => match result {
            Ok(Some(who)) => {
                signed_in_as(st, who);
                st.sync_on = on;
                let machines = load_machines(st, sv);
                Task::batch([machines, start_sync(st, sv)])
            }
            Ok(None) => {
                st.phase = Phase::SignedOut;
                Task::none()
            }
            // A token is held, the server did not answer: still signed in, and
            // the card says it cannot reach the server.
            Err(Error::Unreachable(_) | Error::Answer(_) | Error::Refused { .. }) => {
                let on = read_sync_flag(sv.core().host());
                st.phase = Phase::SignedIn {
                    account: None,
                    device: None,
                };
                st.sync_on = on;
                st.error = Some(Error::Unreachable(String::new()));
                Task::none()
            }
            Err(err) => {
                st.phase = Phase::SignedOut;
                fail(st, err);
                Task::none()
            }
        },

        Msg::SignIn(provider) => {
            if st.busy || matches!(st.phase, Phase::Waiting) {
                return Task::none();
            }
            let cloud = match cloud(st, sv) {
                Ok(cloud) => cloud,
                Err(err) => {
                    st.error = Some(err);
                    return Task::none();
                }
            };
            st.error = None;
            st.phase = Phase::Waiting;
            Task::perform(
                blocking(move || {
                    cloud
                        .sign_in(provider)
                        .map(|sign_in| Pending(Arc::new(Mutex::new(Some(sign_in)))))
                }),
                |result| say(Msg::SignInStarted(result)),
            )
        }
        Msg::SignInStarted(result) => {
            let pending = match result {
                Ok(pending) => pending,
                Err(err) => {
                    st.phase = Phase::SignedOut;
                    st.error = Some(err);
                    return Task::none();
                }
            };
            let Some(sign_in) = pending.0.lock().ok().and_then(|mut slot| slot.take()) else {
                return Task::none();
            };
            if !matches!(st.phase, Phase::Waiting) {
                // Cancelled while it started: dropping it calls it off.
                return Task::none();
            }
            st.canceller = Some(Canceller(sign_in.canceller()));
            let url = sign_in.url().to_string();
            Task::batch([
                // Opening the browser is the first thing the sign-in sends.
                Task::perform(
                    async move {
                        let _ = tokio::task::spawn_blocking(move || {
                            hangar_core::opener::open_url(&url)
                        })
                        .await;
                    },
                    |()| say(Msg::Opened),
                ),
                Task::perform(blocking(move || sign_in.wait()), |result| {
                    say(Msg::SignInDone(result))
                }),
            ])
        }
        Msg::SignInDone(result) => {
            st.canceller = None;
            // Cancelled from the card: the card already went back.
            if !matches!(st.phase, Phase::Waiting) {
                return Task::none();
            }
            match result {
                Ok(who) => {
                    signed_in_as(st, who);
                    // Signing in never turns the sync on.
                    st.sync_on = false;
                    save_sync_flag(sv, false);
                    load_machines(st, sv)
                }
                Err(Error::Cancelled) => {
                    st.phase = Phase::SignedOut;
                    Task::none()
                }
                Err(err) => {
                    st.phase = Phase::SignedOut;
                    st.error = Some(err);
                    Task::none()
                }
            }
        }
        Msg::SignInCancel => {
            if let Some(Canceller(cancel)) = st.canceller.take() {
                cancel.cancel();
            }
            if matches!(st.phase, Phase::Waiting) {
                st.phase = Phase::SignedOut;
            }
            Task::none()
        }

        Msg::Machines(result) => {
            match result {
                Ok(devices) => st.machines = Some(devices),
                Err(err) => fail(st, err),
            }
            Task::none()
        }
        Msg::RenameStart(id) => {
            let name = st
                .machines
                .iter()
                .flatten()
                .find(|device| device.id == id)
                .map(|device| device.name.clone())
                .unwrap_or_default();
            st.renaming = Some((id, name));
            st.removing = None;
            Task::none()
        }
        Msg::RenameInput(value) => {
            if let Some((_, name)) = &mut st.renaming {
                *name = value;
            }
            Task::none()
        }
        Msg::RenameCancel => {
            st.renaming = None;
            Task::none()
        }
        Msg::RenameSubmit => {
            let Some((id, name)) = st.renaming.clone() else {
                return Task::none();
            };
            if st.busy || name.trim().is_empty() {
                return Task::none();
            }
            let Ok(cloud) = cloud(st, sv) else {
                return Task::none();
            };
            st.busy = true;
            st.error = None;
            Task::perform(blocking(move || cloud.rename_device(&id, &name)), |r| {
                say(Msg::Renamed(r))
            })
        }
        Msg::Renamed(result) => {
            st.busy = false;
            match result {
                Ok(device) => {
                    st.renaming = None;
                    if let Some(known) = st
                        .machines
                        .iter_mut()
                        .flatten()
                        .find(|known| known.id == device.id)
                    {
                        *known = device;
                    }
                }
                Err(err) => fail(st, err),
            }
            Task::none()
        }
        Msg::RemoveAsk(id) => {
            st.removing = Some(id);
            st.renaming = None;
            Task::none()
        }
        Msg::RemoveCancel => {
            st.removing = None;
            Task::none()
        }
        Msg::RemoveConfirm(id) => {
            // This PC signs out; it is never "removed".
            let this = matches!(&st.phase, Phase::SignedIn { device: Some(own), .. } if *own == id);
            if st.busy || this {
                return Task::none();
            }
            let Ok(cloud) = cloud(st, sv) else {
                return Task::none();
            };
            st.busy = true;
            st.error = None;
            Task::perform(blocking(move || cloud.forget_device(&id)), |r| {
                say(Msg::Removed(r))
            })
        }
        Msg::Removed(result) => {
            st.busy = false;
            match result {
                Ok(()) => {
                    st.removing = None;
                    // What that machine sent leaves the merged view too.
                    let sync = start_sync_now(st, sv);
                    Task::batch([load_machines(st, sv), sync])
                }
                Err(err) => {
                    fail(st, err);
                    Task::none()
                }
            }
        }

        Msg::SetSync(on) => {
            if !st.signed_in() || st.sync_on == on {
                return Task::none();
            }
            st.sync_on = on;
            save_sync_flag(sv, on);
            if on {
                start_sync_now(st, sv)
            } else {
                st.pulled = None;
                Task::none()
            }
        }
        Msg::SyncNow => {
            if st.signed_in() && st.profile_unknown() {
                return refresh_profile(st, sv);
            }
            start_sync_now(st, sv)
        }
        Msg::Tick => {
            if st.schedule.due(Instant::now(), st.signed_in(), st.sync_on) {
                start_sync(st, sv)
            } else {
                Task::none()
            }
        }
        Msg::Synced(result) => {
            st.syncing = false;
            match result {
                Ok((_, answer)) => {
                    let now = time::now_ms();
                    st.error = None;
                    st.last_sync = Some(now);
                    let this = match &st.phase {
                        Phase::SignedIn { device, .. } => device.clone(),
                        _ => None,
                    };
                    // The sync may have been turned off while it ran.
                    if st.sync_on {
                        st.pulled = Some(Arc::new(Pulled {
                            answer,
                            at: now,
                            this,
                        }));
                    }
                    load_machines(st, sv)
                }
                Err(err) => {
                    if let Error::Refused {
                        retry_after: Some(wait),
                        ..
                    } = &err
                    {
                        st.schedule.hold(Instant::now(), *wait);
                    }
                    fail(st, err);
                    Task::none()
                }
            }
        }

        Msg::SignOut => {
            if st.busy {
                return Task::none();
            }
            let Ok(cloud) = cloud(st, sv) else {
                return Task::none();
            };
            st.busy = true;
            Task::perform(blocking(move || cloud.sign_out()), |r| {
                say(Msg::SignedOut(r))
            })
        }
        Msg::SignedOut(result) => {
            st.busy = false;
            forget_everything(st, sv);
            if let Err(err) = result {
                st.error = Some(err);
            }
            Task::none()
        }
        Msg::DeleteAsk => {
            st.deleting = Some(String::new());
            Task::none()
        }
        Msg::DeleteInput(text) => {
            if st.deleting.is_some() {
                st.deleting = Some(text);
            }
            Task::none()
        }
        Msg::DeleteCancel => {
            st.deleting = None;
            Task::none()
        }
        Msg::DeleteConfirm => {
            let word = hangar_i18n::Translator::new(m.locale())
                .t(hangar_i18n::Key::HangarAccountDeleteWord)
                .into_owned();
            let typed = st
                .deleting
                .as_deref()
                .is_some_and(|t| delete_word_typed(t, &word));
            if st.busy || !typed {
                return Task::none();
            }
            let Ok(cloud) = cloud(st, sv) else {
                return Task::none();
            };
            st.busy = true;
            Task::perform(blocking(move || cloud.delete_account()), |r| {
                say(Msg::Deleted(r))
            })
        }
        Msg::Deleted(result) => {
            st.busy = false;
            match result {
                Ok(()) => forget_everything(st, sv),
                Err(err) => fail(st, err),
            }
            Task::none()
        }

        Msg::OpenPrivacy => Task::perform(
            async {
                let _ = tokio::task::spawn_blocking(|| hangar_core::opener::open_url(PRIVACY_URL))
                    .await;
            },
            |()| say(Msg::Opened),
        ),
        Msg::Opened => Task::none(),
    }
}

/// Whether what was typed is the word that confirms the deletion, whatever the
/// case and the spaces around it.
pub fn delete_word_typed(typed: &str, word: &str) -> bool {
    typed.trim().to_lowercase() == word.trim().to_lowercase()
}

fn start_sync_now(st: &mut State, sv: &Services) -> Task<Message> {
    st.schedule.sync_now();
    start_sync(st, sv)
}

/// Asks the server who this is, again: the card was signed in but offline.
fn refresh_profile(st: &mut State, sv: &Services) -> Task<Message> {
    let Ok(cloud) = cloud(st, sv) else {
        return Task::none();
    };
    st.error = None;
    let on = st.sync_on;
    Task::perform(blocking(move || cloud.me()), move |result| {
        say(Msg::Booted(
            result.map(|me| {
                Some(SignedIn {
                    account: me.account,
                    device: me.device,
                })
            }),
            on,
        ))
    })
}

/// Signed out or deleted: nothing of the account is kept in the window, and
/// the sync is off again.
fn forget_everything(st: &mut State, sv: &Services) {
    st.phase = Phase::SignedOut;
    st.sync_on = false;
    st.machines = None;
    st.pulled = None;
    st.last_sync = None;
    st.renaming = None;
    st.removing = None;
    st.deleting = None;
    st.syncing = false;
    st.error = None;
    save_sync_flag(sv, false);
}

impl State {
    fn profile_unknown(&self) -> bool {
        matches!(&self.phase, Phase::SignedIn { account: None, .. })
    }
}

// ---- the package surface -----------------------------------------------------------------------------

pub fn subscription(st: &State, _m: &Model) -> Subscription<Message> {
    let mut all = Vec::new();
    if !st.started {
        all.push(Subscription::run(|| {
            iced::futures::stream::once(async { say(Msg::Boot) })
        }));
    }
    if st.signed_in() && st.sync_on {
        all.push(iced::time::every(LOOK_EVERY).map(|_| say(Msg::Tick)));
    }
    Subscription::batch(all)
}

/// Opening the Statistics page syncs, when the sync is on.
pub fn on_intent(
    st: &mut State,
    _m: &mut Model,
    sv: &Services,
    intent: &Intent,
) -> Option<Task<Message>> {
    match intent {
        Intent::SetView(View::Stats) if st.signed_in() && st.sync_on => {
            Some(start_sync_now(st, sv))
        }
        _ => None,
    }
}

pub fn on_event(
    _st: &mut State,
    _m: &mut Model,
    _sv: &Services,
    _event: &CoreEvent,
) -> Option<Task<Message>> {
    None
}

#[cfg(test)]
mod tests;
