//! The Discord connector (docs/native/discord-connector.md): a Discord
//! server as one more surface of a Hangar, for whoever would rather not
//! open the app. A category per folder, a channel per workspace (its
//! orchestrator's conversation), a thread per worker opened when it
//! starts; cards for what waits on the person; the files agents share with
//! the phone, attached; only the paired account is heard.
//!
//! It serves whichever Hangar runs it (`Hangar`): the desktop app, where it
//! reaches the same workspaces as the phone, local and on nodes; or a node
//! with no desktop (`teskad-node serve`), its own workspaces.
//!
//! - `rest`, `gateway`: the thin client of Discord's own it speaks with;
//! - `cards`: the messages, as Components V2; `ids`: what a click acts on;
//! - `bot`: the connector, the server's structure and the dashboards;
//!   `follow`: the panes followed; `images`: what a Codex agent generates,
//!   on its turn's card; `shares`: the files agents share;
//!   `act`: what the person does; `lanes`: what waits on Discord kept in
//!   order per agent and channel, side by side between them.

mod act;
pub mod bot;
pub mod cards;
pub mod config;
mod follow;
pub mod gateway;
pub mod hangar;
pub mod ids;
pub mod images;
mod lanes;
pub mod rest;
pub mod setup;
pub mod shares;
pub mod words;

pub use bot::{Options, run};
pub use config::Settings;
pub use hangar::Hangar;
pub use setup::keep_running;
