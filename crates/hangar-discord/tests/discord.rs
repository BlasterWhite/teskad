//! The connector against a Discord of the test's own: its REST routes
//! record every call and answer as Discord would, its gateway says hello,
//! hears the bot identify, and hands it the events the test makes up. The
//! engine runs with no window over shells that only record what they are
//! typed.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde_json::{Value, json};

use hangar_core::agent::AgentKind;
use hangar_engine::control::port::ModelCmd;
use hangar_engine::host::{Config, EngineHost, Spawn, TermEvent, TermSink, Terminals};
use hangar_engine::orchestration::events::{DeliveryOutcome, OrchestratorEvent};
use hangar_engine::state::types::{AgentId, AppState, Pane, PaneRole, Settings, Workspace};
use hangar_i18n::Locale;

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/* ── shells that record ─────────────────────────────────────────── */

#[derive(Default)]
struct Shells {
    alive: Mutex<HashSet<String>>,
    typed: Mutex<Vec<(String, String)>>,
}

impl Shells {
    fn typed_into(&self, pane: &str) -> String {
        locked(&self.typed)
            .iter()
            .filter(|(id, _)| id == pane)
            .map(|(_, text)| text.as_str())
            .collect()
    }
}

impl Terminals for Shells {
    fn spawn(&self, spawn: Spawn, sink: TermSink) -> Result<(), String> {
        locked(&self.alive).insert(spawn.id.clone());
        sink(TermEvent::Output(spawn.id));
        Ok(())
    }
    fn write(&self, pane: &str, data: &str) -> Result<(), String> {
        locked(&self.typed).push((pane.to_string(), data.to_string()));
        Ok(())
    }
    fn alive(&self, pane: &str) -> bool {
        locked(&self.alive).contains(pane)
    }
    fn kill(&self, pane: &str) -> Result<(), String> {
        locked(&self.alive).remove(pane);
        Ok(())
    }
    fn lines(&self, pane: &str, _n: usize) -> Option<Vec<String>> {
        self.alive(pane).then(Vec::new)
    }
    fn screen(&self, pane: &str) -> Option<Vec<String>> {
        self.alive(pane).then(Vec::new)
    }
    fn kill_all(&self) {
        locked(&self.alive).clear();
    }
}

/* ── a Discord of the test's own ────────────────────────────────── */

#[path = "support/fake.rs"]
mod fake;
use fake::Discord;

/// Waits up to a minute for `found` (a loaded machine is slow, a Windows
/// runner most of all). Timed by the clock, not by a count of sleeps: a
/// 10 ms sleep lasts about 15 ms on Windows.
async fn eventually<T>(what: &str, mut found: impl FnMut() -> Option<T>) -> T {
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(found) = found() {
            return found;
        }
        if std::time::Instant::now() >= deadline {
            panic!("never happened: {what}");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn pane(id: &str, name: &str, role: Option<PaneRole>) -> Pane {
    pane_of(id, name, role, AgentId::Claude)
}

fn pane_of(id: &str, name: &str, role: Option<PaneRole>, agent: AgentId) -> Pane {
    Pane {
        id: id.into(),
        name: name.into(),
        agent,
        role,
        ..Pane::default()
    }
}

struct Rig {
    host: EngineHost,
    shells: Arc<Shells>,
    discord: Discord,
    dir: PathBuf,
    bot: tokio::task::JoinHandle<()>,
    _root: tempfile::TempDir,
}

/// The connector of `host`, its files in `dir`, on `discord`.
fn start_bot(host: &EngineHost, discord: &Discord, dir: &Path) -> tokio::task::JoinHandle<()> {
    let options = hangar_discord::Options {
        api: discord.api(),
        gateway: Some(discord.gateway_url()),
        dir: dir.to_path_buf(),
    };
    let bot_host = host.clone();
    tokio::spawn(async move {
        if let Err(error) = hangar_discord::run(Arc::new(bot_host), options).await {
            panic!("the connector stopped: {error}");
        }
    })
}

impl Rig {
    /// The connector stopped, and started again over the same files.
    async fn restart(&mut self) {
        self.bot.abort();
        let _ = (&mut self.bot).await;
        self.bot = start_bot(&self.host, &self.discord, &self.dir);
    }

    /// A worker `name` added to the workspace on task `task`: its pane.
    fn add_worker(&self, name: &str, task: &str) -> String {
        let task = task.to_string();
        let added = self
            .host
            .apply(ModelCmd::AddPane {
                ws: "shop".into(),
                agent: Some(AgentId::Claude),
                near: None,
                dir: None,
                cwd: None,
                name: Some(name.into()),
                init: Some(hangar_engine::state::PanePatch::new(move |p: &mut Pane| {
                    p.role = Some(PaneRole::Worker);
                    p.task_id = Some(task.clone());
                })),
            })
            .unwrap();
        added.pane.unwrap()
    }

    /// The journal tells worker `name` started on task `task`.
    fn worker_started(&self, name: &str, task: &str) {
        self.host.orchestration().unwrap().post(
            "shop",
            OrchestratorEvent::WorkerStarted {
                worker: name.into(),
                task_id: Some(task.into()),
            },
        );
    }

    /// What the layout keeps of the worker of pane `pane`: its `line` or
    /// its `thread`.
    fn worker(&self, pane: &str, field: &str) -> Option<String> {
        layout(&self.dir)
            .and_then(|layout| layout["workers"][pane][field].as_str().map(str::to_string))
    }
}

/// The connector's layout, as saved.
fn layout(dir: &Path) -> Option<Value> {
    std::fs::read_to_string(dir.join("discord-layout.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
}

async fn rig() -> Rig {
    rig_with("leo", None).await
}

/// Workspace "shop" with its orchestrator Leo (pane `leo`: a pane's queue
/// is the process's own, and the tests run side by side), its connector
/// on a Discord of its own, paired with `owner` when given.
async fn rig_with(leo: &str, owner: Option<&str>) -> Rig {
    rig_of(leo, owner, AgentId::Claude, None).await
}

/// [`rig_with`], Leo running `agent`, the settings' default orchestrator
/// agent `orchestrator`.
async fn rig_of(
    leo: &str,
    owner: Option<&str>,
    agent: AgentId,
    orchestrator: Option<AgentKind>,
) -> Rig {
    let root = tempfile::tempdir().unwrap();
    let state = AppState {
        workspaces: vec![Workspace {
            id: "shop".into(),
            name: "My Shop".into(),
            cwd: root.path().to_string_lossy().into_owned(),
            panes: vec![pane_of(leo, "Leo", Some(PaneRole::Orchestrator), agent)],
            ..Workspace::default()
        }],
        settings: Settings {
            locale: Some(Locale::Fr),
            default_orchestrator_agent: orchestrator,
            ..Settings::default()
        },
        ..AppState::default()
    };
    let dir = root.path().join("node");
    let file = dir.join("state.json");
    hangar_engine::host::write_state_file(&file, &serde_json::to_value(&state).unwrap()).unwrap();
    let shells = Arc::new(Shells::default());
    let host = EngineHost::load(Config {
        core: hangar_core::Host::with_paths(hangar_core::paths::Paths::rooted(root.path())),
        terminals: shells.clone(),
        state_file: file,
    })
    .unwrap();
    host.start();
    let discord = Discord::start().await;
    hangar_discord::Settings {
        token: "secret".into(),
        guild: "g1".into(),
        owner: owner.map(str::to_string),
        ..hangar_discord::Settings::default()
    }
    .save(&dir)
    .unwrap();
    let bot = start_bot(&host, &discord, &dir);
    Rig {
        host,
        shells,
        discord,
        dir,
        bot,
        _root: root,
    }
}

fn interaction(
    id: &str,
    user: &str,
    channel: &str,
    kind: u8,
    data: Value,
    message: Option<&str>,
) -> Value {
    let mut i = json!({
        "id": id, "token": format!("{id}tok"), "type": kind, "channel_id": channel,
        "member": { "user": { "id": user } }, "data": data,
    });
    if let Some(message) = message {
        i["message"] = json!({ "id": message });
    }
    i
}

/// How many threads the bots started.
fn threads_made(d: &Discord) -> usize {
    d.calls()
        .iter()
        .filter(|c| c.method == "POST" && c.path.ends_with("/threads"))
        .count()
}

/// How many lines worker `name`'s starts posted in the channel. Not told
/// by its « Ouvrir le fil » button: the dashboard posted while the thread
/// is not open yet carries the same.
fn lines_of(d: &Discord, name: &str) -> usize {
    d.count("POST", &format!("🚀 **{name}**"))
}

/// Workspace "shop"'s channel, once made.
async fn shop_channel(rig: &Rig) -> String {
    eventually("the workspace's channel", || channel_of(&rig.dir, "shop")).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_workspace_on_discord_from_pairing_to_a_worker_s_thread() {
    let rig = rig().await;
    let d = &rig.discord;

    // The bot identifies with its token and the intents it needs.
    let identified = eventually("identify", || d.identified()).await;
    assert_eq!(identified["token"], "secret");
    assert_eq!(identified["intents"].as_u64().unwrap() & (1 << 15), 1 << 15);
    // Its commands, on the guild alone.
    eventually("the commands registered", || {
        d.calls_to("PUT", "/applications/app1/guilds/g1/commands")
            .pop()
    })
    .await;

    // A category, the workspace's channel, its webhook, its dashboard pinned.
    let made = eventually("the workspace's channel", || {
        d.calls_to("POST", "/guilds/g1/channels")
            .into_iter()
            .find(|c| c.body["type"] == 0)
    })
    .await;
    assert_eq!(made.body["name"], "my-shop");
    let category = d.calls_to("POST", "/guilds/g1/channels")[0].clone();
    assert_eq!(category.body["type"], 4);
    assert_eq!(category.body["permission_overwrites"][0]["id"], "bot1");
    let layout: Value = eventually("the layout saved", || {
        std::fs::read_to_string(rig.dir.join("discord-layout.json"))
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .filter(|layout| layout["workspaces"]["shop"]["dashboard"].is_string())
    })
    .await;
    let channel = layout["workspaces"]["shop"]["channel"]
        .as_str()
        .unwrap()
        .to_string();
    let hook = layout["workspaces"]["shop"]["webhook"][0]
        .as_str()
        .unwrap()
        .to_string();
    eventually("the dashboard pinned", || {
        d.calls_to("PUT", &format!("/channels/{channel}/messages/pins/"))
            .pop()
    })
    .await;

    // Not paired yet: the code in the settings pairs whoever sends it.
    let settings: Value =
        serde_json::from_str(&std::fs::read_to_string(rig.dir.join("discord.json")).unwrap())
            .unwrap();
    let code = settings["pairing"].as_str().unwrap().to_string();
    d.send(
        "MESSAGE_CREATE",
        json!({ "id": "m1", "channel_id": channel, "content": "hello",
        "author": { "id": "u1" } }),
    );
    eventually("told to pair", || d.find("POST", "pair <code>")).await;
    // The code is heard on the server alone: not in a direct message, not
    // on another server the bot was invited to.
    for (id, guild) in [("dm", Value::Null), ("elsewhere", json!("g2"))] {
        d.send(
            "MESSAGE_CREATE",
            json!({ "id": id, "channel_id": channel, "guild_id": guild,
            "content": format!("pair {code}"), "author": { "id": "intruder" } }),
        );
    }
    // A wrong code is told wrong, five times; after that its sender is
    // not heard any more, even with the right one.
    for n in 0..6 {
        d.send(
            "MESSAGE_CREATE",
            json!({ "id": format!("w{n}"), "channel_id": channel, "guild_id": "g1",
            "content": "pair 000000x", "author": { "id": "guesser" } }),
        );
    }
    eventually("five wrong codes told so", || {
        (d.count("POST", "Ce n'est pas le code") == 5).then_some(())
    })
    .await;
    d.send(
        "MESSAGE_CREATE",
        json!({ "id": "w6", "channel_id": channel, "guild_id": "g1",
        "content": format!("pair {code}"), "author": { "id": "guesser" } }),
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let unpaired = std::fs::read_to_string(rig.dir.join("discord.json")).unwrap();
    assert!(unpaired.contains("\"owner\": null"), "{unpaired}");
    assert_eq!(d.count("POST", "Ce n'est pas le code"), 5);
    d.send(
        "MESSAGE_CREATE",
        json!({ "id": "m2", "channel_id": channel, "guild_id": "g1",
        "content": format!("pair {code}"), "author": { "id": "u1" } }),
    );
    eventually("paired", || {
        std::fs::read_to_string(rig.dir.join("discord.json"))
            .ok()
            .filter(|raw| raw.contains("\"owner\": \"u1\""))
    })
    .await;

    // What the owner writes in the channel reaches the orchestrator.
    d.send(
        "MESSAGE_CREATE",
        json!({ "id": "m3", "channel_id": channel,
        "content": "Ajoute un panier", "author": { "id": "u1" } }),
    );
    // Typed when Leo is idle, queued (⏳) while it works.
    eventually("Leo given the message", || {
        let queued = !d
            .calls_to(
                "PUT",
                &format!("/channels/{channel}/messages/m3/reactions/%E2%8F%B3"),
            )
            .is_empty();
        (queued || rig.shells.typed_into("leo").contains("Ajoute un panier")).then_some(())
    })
    .await;
    eventually("the message marked", || {
        d.calls_to(
            "PUT",
            &format!("/channels/{channel}/messages/m3/reactions/"),
        )
        .pop()
    })
    .await;
    // Messages sent one after the other reach the agent in that order.
    for (n, word) in ["premier", "deuxième", "troisième", "quatrième"]
        .iter()
        .enumerate()
    {
        d.send(
            "MESSAGE_CREATE",
            json!({ "id": format!("o{n}"), "channel_id": channel, "content": word,
            "author": { "id": "u1" } }),
        );
    }
    eventually("the four typed", || {
        rig.shells
            .typed_into("leo")
            .contains("quatrième")
            .then_some(())
    })
    .await;
    let typed = rig.shells.typed_into("leo");
    let at = |word: &str| typed.find(word).expect(word);
    assert!(
        at("premier") < at("deuxième")
            && at("deuxième") < at("troisième")
            && at("troisième") < at("quatrième"),
        "{typed:?}"
    );
    // Anyone else is not heard.
    d.send(
        "MESSAGE_CREATE",
        json!({ "id": "m4", "channel_id": channel,
        "content": "rm -rf /", "author": { "id": "intruder" } }),
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!rig.shells.typed_into("leo").contains("rm -rf /"));

    // A turn of Leo's: its card under Leo's name, its calls, then folded.
    let core = rig.host.core().clone();
    let bridge = |event: Value| {
        core.emit(hangar_core::Event::BridgeEvent {
            pane_id: "leo".into(),
            event,
        })
        .unwrap();
    };
    bridge(json!({ "kind": "hello", "sessionId": "s1", "model": null, "version": "2.1" }));
    bridge(json!({ "kind": "turn", "phase": "start" }));
    let card = eventually("the turn's card", || d.find("POST", "travaille")).await;
    assert!(card.path.starts_with(&format!("/webhooks/{hook}/")));
    assert!(card.path.contains("with_components=true"));
    assert_eq!(card.body["username"], "Leo");
    assert!(
        card.body["avatar_url"]
            .as_str()
            .is_some_and(|url| url.starts_with("https://cdn.discordapp.com/embed/avatars/"))
    );

    // While Leo works, what the owner writes waits in its queue (⏳); an
    // edit changes what waits, a delete takes it back.
    eventually("Leo working", || {
        rig.host
            .snapshot()
            .activity
            .get("leo")
            .is_some_and(|a| a.activity == hangar_engine::state::AgentActivity::Working)
            .then_some(())
    })
    .await;
    let queued = |text: &str| {
        hangar_core::bridge::bridge_queue("leo".into())
            .iter()
            .any(|entry| entry.text == text)
    };
    d.send(
        "MESSAGE_CREATE",
        json!({ "id": "q1", "channel_id": channel, "content": "Et le paiement ?",
        "author": { "id": "u1" } }),
    );
    eventually("queued, marked ⏳", || {
        (queued("Et le paiement ?")
            && !d
                .calls_to(
                    "PUT",
                    &format!("/channels/{channel}/messages/q1/reactions/%E2%8F%B3"),
                )
                .is_empty())
        .then_some(())
    })
    .await;
    d.send(
        "MESSAGE_UPDATE",
        json!({ "id": "q1", "channel_id": channel, "content": "Et le paiement Stripe ?",
        "author": { "id": "u1" } }),
    );
    eventually("the edit waits instead", || {
        (queued("Et le paiement Stripe ?") && !queued("Et le paiement ?")).then_some(())
    })
    .await;
    d.send(
        "MESSAGE_DELETE",
        json!({ "id": "q1", "channel_id": channel }),
    );
    eventually("taken back", || {
        (!queued("Et le paiement Stripe ?")).then_some(())
    })
    .await;
    bridge(
        json!({ "kind": "tool", "phase": "start", "toolUseId": "t1", "tool": "Bash",
        "input": { "command": "npm test" } }),
    );
    eventually("the card edited with the call", || {
        d.find("PATCH", "Bash npm test")
    })
    .await;
    bridge(
        json!({ "kind": "turn", "phase": "end", "reason": "answer", "durationMs": 1000,
        "answer": "Le panier est prêt." }),
    );
    eventually("the turn folded", || d.find("PATCH", "1 action")).await;
    let answer = eventually("the answer", || d.find("POST", "Le panier est prêt.")).await;
    assert_eq!(answer.body["username"], "Leo");

    // A permission: a card that calls the owner, Allow answers it.
    bridge(
        json!({ "kind": "permission", "waiting": true, "tool": "Bash",
        "input": { "command": "rm -rf build" } }),
    );
    let asked = eventually("the permission's card", || d.find("POST", "p:leo:y")).await;
    assert!(asked.body.to_string().contains("<@u1>"));
    assert_eq!(asked.body["allowed_mentions"]["users"], json!(["u1"]));
    // A stranger's click is refused, to them alone.
    d.send(
        "INTERACTION_CREATE",
        interaction(
            "i0",
            "intruder",
            &channel,
            3,
            json!({ "custom_id": "p:leo:y" }),
            None,
        ),
    );
    let refused = eventually("the stranger refused", || {
        d.calls_to("POST", "/interactions/i0/i0tok/callback").pop()
    })
    .await;
    assert_eq!(refused.body["type"], 4);
    assert_eq!(refused.body["data"]["flags"], 64);
    let typed_before = rig.shells.typed_into("leo").len();
    // While Leo waits on that answer, what the owner writes is not typed:
    // it would land in the question, and its Enter would say yes.
    d.send(
        "MESSAGE_CREATE",
        json!({ "id": "m5", "channel_id": channel, "guild_id": "g1",
        "content": "attends, pourquoi ?", "author": { "id": "u1" } }),
    );
    eventually("told to answer the card first", || {
        d.find("POST", "attend une réponse")
    })
    .await;
    assert_eq!(rig.shells.typed_into("leo").len(), typed_before);
    // A card left from an earlier question (answered on the PC, or posted
    // before Hangar restarted) answers nothing: clicked, it says so, and
    // what Leo asks now is not allowed by it.
    d.send(
        "INTERACTION_CREATE",
        interaction(
            "iold",
            "u1",
            &channel,
            3,
            json!({ "custom_id": "p:leo:y" }),
            Some("999"),
        ),
    );
    let stale = eventually("the old card says it is answered", || {
        d.calls_to("PATCH", "/webhooks/app1/ioldtok/messages/@original")
            .pop()
    })
    .await;
    assert!(
        stale.body.to_string().contains("Répondu ailleurs"),
        "{stale:?}"
    );
    assert_eq!(
        d.calls_to("POST", "/interactions/iold/ioldtok/callback")[0].body["type"],
        6
    );
    assert_eq!(rig.shells.typed_into("leo").len(), typed_before);
    assert!(d.find("PATCH", "Autorisé").is_none());
    // The card of what Leo asks now does answer it.
    d.send(
        "INTERACTION_CREATE",
        interaction(
            "i1",
            "u1",
            &channel,
            3,
            json!({ "custom_id": "p:leo:y" }),
            Some(&asked.id),
        ),
    );
    let acked = eventually("the click acknowledged", || {
        d.calls_to("POST", "/interactions/i1/i1tok/callback").pop()
    })
    .await;
    assert_eq!(acked.body["type"], 6);
    eventually("the answer typed or handed to the mod", || {
        (rig.shells.typed_into("leo").len() > typed_before || d.find("PATCH", "Autorisé").is_some())
            .then_some(())
    })
    .await;

    // A worker started: its line, and its thread opened on that line by
    // itself (the line's button only until then).
    let ada = rig.add_worker("Ada", "t1");
    rig.worker_started("Ada", "t1");
    let line = eventually("the worker's line", || d.find("POST", &format!("t:{ada}"))).await;
    assert!(line.body.to_string().contains("Ouvrir le fil"));
    let line_id = eventually("its id kept", || rig.worker(&ada, "line")).await;
    let thread = eventually("the thread started on the line, unasked", || {
        d.calls_to(
            "POST",
            &format!("/channels/{channel}/messages/{line_id}/threads"),
        )
        .pop()
    })
    .await;
    assert_eq!(thread.body["auto_archive_duration"], 10080);
    assert!(thread.body["name"].as_str().unwrap().starts_with("Ada"));
    eventually("the line links to the thread", || {
        d.calls_to("PATCH", &format!("/channels/{channel}/messages/{line_id}"))
            .into_iter()
            .find(|c| c.body.to_string().contains("Voir le fil"))
    })
    .await;
    let thread_id = eventually("the thread kept", || rig.worker(&ada, "thread")).await;
    // Told again that Ada started: neither a second line nor a second
    // thread; its own is opened again, should it be archived.
    rig.worker_started("Ada", "t1");
    eventually("the thread unarchived", || {
        d.calls_to("PATCH", &format!("/channels/{thread_id}"))
            .into_iter()
            .find(|c| c.body["archived"] == false)
    })
    .await;
    assert_eq!(d.count("POST", &format!("t:{ada}")), 1);
    assert_eq!(threads_made(d), 1);
    // What the owner writes in the thread goes to the worker.
    d.send(
        "MESSAGE_CREATE",
        json!({ "id": "m5", "channel_id": thread_id,
        "content": "Pense aux tests", "author": { "id": "u1" } }),
    );
    eventually("Ada given the message", || {
        let queued = !d
            .calls_to(
                "PUT",
                &format!("/channels/{thread_id}/messages/m5/reactions/%E2%8F%B3"),
            )
            .is_empty();
        (queued || rig.shells.typed_into(&ada).contains("Pense aux tests")).then_some(())
    })
    .await;
    // The worker stops: a last line, the thread archived.
    rig.host.orchestration().unwrap().post(
        "shop",
        OrchestratorEvent::WorkerExited {
            worker: "Ada".into(),
            task_id: Some("t1".into()),
        },
    );
    eventually("the thread archived", || {
        d.calls_to("PATCH", &format!("/channels/{thread_id}"))
            .into_iter()
            .find(|c| c.body["archived"] == true)
    })
    .await;

    // Ada's delivery passed its check and review: the journal says so,
    // and the card that calls the owner has Land and Send back. Landed,
    // the card says so in place of its buttons; ready again later, it is
    // a card of its own.
    let delivery = |outcome| OrchestratorEvent::Delivery {
        task_id: "t1".into(),
        title: "Le panier".into(),
        worker: "Ada".into(),
        outcome,
        detail: String::new(),
    };
    let orchestration = rig.host.orchestration().unwrap();
    orchestration.post("shop", delivery(DeliveryOutcome::Ready));
    let ready = eventually("the delivery's card", || d.find("POST", "d:y:shop:t1")).await;
    assert!(
        ready.body.to_string().contains("Livraison prête"),
        "{ready:?}"
    );
    assert!(ready.body.to_string().contains("Le panier"));
    assert!(ready.body.to_string().contains("d:n:shop:t1"));
    assert_eq!(ready.body["allowed_mentions"]["users"], json!(["u1"]));
    orchestration.post("shop", delivery(DeliveryOutcome::Landed));
    let landed = eventually("the card says landed", || {
        d.calls_to(
            "PATCH",
            &format!("/channels/{channel}/messages/{}", ready.id),
        )
        .pop()
    })
    .await;
    assert!(landed.body.to_string().contains("Fusionné"), "{landed:?}");
    orchestration.post("shop", delivery(DeliveryOutcome::Ready));
    eventually("a second delivery, a second card", || {
        (d.count("POST", "d:y:shop:t1") == 2).then_some(())
    })
    .await;

    // A workspace made from Discord: its channel follows.
    let folder = rig._root.path().join("blog");
    std::fs::create_dir_all(&folder).unwrap();
    d.send("INTERACTION_CREATE", interaction("i3", "u1", &channel, 2, json!({
        "name": "workspace",
        "options": [{ "name": "folder", "value": folder.to_string_lossy() }, { "name": "name", "value": "Blog" }],
    }), None));
    eventually("the blog's channel", || {
        d.calls_to("POST", "/guilds/g1/channels")
            .into_iter()
            .find(|c| c.body["name"] == "blog")
    })
    .await;
    let said = eventually("the command answered", || d.find("PATCH", "Workspace créé")).await;
    assert!(
        said.path
            .starts_with("/webhooks/app1/i3tok/messages/@original")
    );
    assert!(
        rig.host
            .snapshot()
            .state
            .workspaces
            .iter()
            .any(|ws| ws.name == "Blog")
    );
}

/// A Hangar that shares its bot: whether it leads, and what the others
/// made.
struct Sharing {
    host: EngineHost,
    leads: bool,
    others: Arc<Mutex<HashSet<String>>>,
}

impl hangar_discord::Hangar for Sharing {
    fn port(&self) -> hangar_engine::control::port::Port {
        self.host.port()
    }
    fn desk(&self) -> Option<Arc<hangar_engine::mobile::engine::Mobile>> {
        self.host.desk()
    }
    fn journal(&self, ws: &str) -> Vec<hangar_engine::orchestration::events::JournalEntry> {
        hangar_discord::Hangar::journal(&self.host, ws)
    }
    fn tasks(&self, ws: &str) -> Vec<hangar_core::board::Task> {
        hangar_discord::Hangar::tasks(&self.host, ws)
    }
    fn events(&self) -> tokio::sync::broadcast::Receiver<hangar_core::Event> {
        self.host.core().subscribe()
    }
    fn leads(&self) -> bool {
        self.leads
    }
    fn others(&self) -> HashSet<String> {
        locked(&self.others).clone()
    }
}

struct Side {
    host: EngineHost,
    shells: Arc<Shells>,
    dir: PathBuf,
}

/// One Hangar with workspace `ws` (its orchestrator `pane_id`), its bot on
/// `d`.
fn side(
    root: &std::path::Path,
    name: &str,
    (ws, pane_id): (&str, &str),
    settings: hangar_discord::Settings,
    leads: bool,
    others: Arc<Mutex<HashSet<String>>>,
    d: &Discord,
) -> Side {
    let folder = root.join(name);
    std::fs::create_dir_all(&folder).unwrap();
    let state = AppState {
        workspaces: vec![Workspace {
            id: ws.into(),
            name: ws.into(),
            cwd: folder.to_string_lossy().into_owned(),
            panes: vec![pane(pane_id, "Orchestrator", Some(PaneRole::Orchestrator))],
            ..Workspace::default()
        }],
        ..AppState::default()
    };
    let dir = folder.join("hangar");
    let file = dir.join("state.json");
    hangar_engine::host::write_state_file(&file, &serde_json::to_value(&state).unwrap()).unwrap();
    let shells = Arc::new(Shells::default());
    let host = EngineHost::load(Config {
        core: hangar_core::Host::with_paths(hangar_core::paths::Paths::rooted(&folder)),
        terminals: shells.clone(),
        state_file: file,
    })
    .unwrap();
    host.start();
    settings.save(&dir).unwrap();
    let options = hangar_discord::Options {
        api: d.api(),
        gateway: Some(d.gateway_url()),
        dir: dir.clone(),
    };
    let hangar = Arc::new(Sharing {
        host: host.clone(),
        leads,
        others,
    });
    tokio::spawn(async move {
        if let Err(error) = hangar_discord::run(hangar, options).await {
            panic!("the connector stopped: {error}");
        }
    });
    Side { host, shells, dir }
}

fn channel_of(dir: &std::path::Path, ws: &str) -> Option<String> {
    std::fs::read_to_string(dir.join("discord-layout.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|layout| {
            layout["workspaces"][ws]["channel"]
                .as_str()
                .map(str::to_string)
        })
}

/// A desktop and the node it drives share one bot on one server: each
/// makes and answers in its own channels, the node's categories named
/// after it, and the desktop, which leads, answers where neither made a
/// channel.
#[tokio::test(flavor = "multi_thread")]
async fn a_desktop_and_its_node_share_one_bot() {
    let root = tempfile::tempdir().unwrap();
    let d = Discord::start().await;
    let paired = hangar_discord::Settings {
        token: "secret".into(),
        guild: "g1".into(),
        owner: Some("u1".into()),
        ..hangar_discord::Settings::default()
    };
    let node_channels = Arc::new(Mutex::new(HashSet::new()));
    let desktop = side(
        root.path(),
        "desktop",
        ("shop", "o-desk"),
        paired.clone(),
        true,
        node_channels.clone(),
        &d,
    );
    let node = side(
        root.path(),
        "node",
        ("lab", "o-node"),
        hangar_discord::Settings {
            shared: true,
            label: Some("vps".into()),
            ..paired
        },
        false,
        Arc::default(),
        &d,
    );
    // Both bots hear the gateway before anything is said on it.
    eventually("both bots connected", || {
        (d.identifies() >= 2).then_some(())
    })
    .await;
    let shop = eventually("the desktop's channel", || channel_of(&desktop.dir, "shop")).await;
    let lab = eventually("the node's channel", || channel_of(&node.dir, "lab")).await;
    // What the desktop learns from the node it drives.
    locked(&node_channels).insert(lab.clone());
    let categories: Vec<String> = d
        .calls_to("POST", "/guilds/g1/channels")
        .into_iter()
        .filter(|c| c.body["type"] == 4)
        .map(|c| c.body["name"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(categories.contains(&"Hangar".to_string()), "{categories:?}");
    assert!(
        categories.contains(&"Hangar · vps".to_string()),
        "{categories:?}"
    );

    // Written in the node's channel: the node's orchestrator alone hears it.
    d.send(
        "MESSAGE_CREATE",
        json!({ "id": "n1", "channel_id": lab,
        "content": "Lance les tests", "author": { "id": "u1" } }),
    );
    eventually("typed on the node", || {
        node.shells
            .typed_into("o-node")
            .contains("Lance les tests")
            .then_some(())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !desktop
            .shells
            .typed_into("o-desk")
            .contains("Lance les tests")
    );

    // A command is answered once: by the node in its channel, by the
    // desktop in its own, and by the desktop where neither made one.
    let status = json!({ "name": "status" });
    for (id, channel) in [
        ("c1", lab.as_str()),
        ("c2", shop.as_str()),
        ("c3", "general"),
    ] {
        d.send(
            "INTERACTION_CREATE",
            interaction(id, "u1", channel, 2, status.clone(), None),
        );
        eventually("answered", || {
            d.calls_to("POST", &format!("/interactions/{id}/")).pop()
        })
        .await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    for id in ["c1", "c2", "c3"] {
        assert_eq!(
            d.calls_to("POST", &format!("/interactions/{id}/")).len(),
            1,
            "{id} answered once"
        );
    }
    let said = |id: &str| {
        d.calls_to(
            "PATCH",
            &format!("/webhooks/app1/{id}tok/messages/@original"),
        )
        .pop()
        .map(|c| c.body["content"].as_str().unwrap_or_default().to_string())
    };
    let lab_status = eventually("the node's status", || said("c1")).await;
    assert!(lab_status.contains("lab"), "{lab_status}");
    let shop_status = eventually("the desktop's status", || said("c2")).await;
    assert!(shop_status.contains("shop"), "{shop_status}");
    let _ = (&desktop.host, &node.host);
}

/// A worker's thread is opened when it starts, once: the start told again
/// opens no other, and neither does the connector started again, which
/// finds the thread its layout kept.
#[tokio::test(flavor = "multi_thread")]
async fn a_worker_s_thread_is_opened_once_even_after_a_restart() {
    let mut rig = rig_with("leo-again", Some("u1")).await;
    let channel = shop_channel(&rig).await;
    let ada = rig.add_worker("Ada", "t1");
    rig.worker_started("Ada", "t1");
    let thread = eventually("Ada's thread kept", || rig.worker(&ada, "thread")).await;
    let line = rig.worker(&ada, "line").expect("Ada's line");
    let on_line = format!("/channels/{channel}/messages/{line}/threads");
    assert_eq!(rig.discord.calls_to("POST", &on_line).len(), 1);
    assert_eq!(threads_made(&rig.discord), 1);
    assert_eq!(lines_of(&rig.discord, "Ada"), 1);

    // Hangar's connector started again: once it is up, on the gateway again
    // (what is sent before is lost) and its loop running (the first look
    // draws the dashboard again; what the journal holds was marked as read
    // before it)…
    let identified = rig.discord.identifies();
    let seen = rig.discord.calls().len();
    rig.restart().await;
    eventually("the connector up again", || {
        let drawn = rig
            .discord
            .calls()
            .iter()
            .skip(seen)
            .any(|c| c.body.to_string().contains("Tableau de bord"));
        (rig.discord.identifies() > identified && drawn).then_some(())
    })
    .await;
    // …what the person writes in Ada's thread still reaches Ada,
    rig.discord.send(
        "MESSAGE_CREATE",
        json!({ "id": "m1", "channel_id": thread, "guild_id": "g1",
        "content": "Pense aux tests", "author": { "id": "u1" } }),
    );
    eventually("Ada given the message", || {
        let queued = !rig
            .discord
            .calls_to(
                "PUT",
                &format!("/channels/{thread}/messages/m1/reactions/%E2%8F%B3"),
            )
            .is_empty();
        (queued || rig.shells.typed_into(&ada).contains("Pense aux tests")).then_some(())
    })
    .await;
    // …and Ada's start told again finds its thread: unarchived, not made
    // again, no second line. That start is handled whole once its thread is
    // unarchived: a second line would have been posted before it, and a
    // second thread made instead of it.
    let unarchived = || {
        rig.discord
            .calls_to("PATCH", &format!("/channels/{thread}"))
            .iter()
            .filter(|c| c.body["archived"] == false)
            .count()
    };
    let before = unarchived();
    rig.worker_started("Ada", "t1");
    eventually("the thread unarchived", || {
        (unarchived() > before).then_some(())
    })
    .await;
    assert_eq!(threads_made(&rig.discord), 1);
    assert_eq!(lines_of(&rig.discord, "Ada"), 1);
    assert_eq!(rig.worker(&ada, "thread").as_deref(), Some(thread.as_str()));
}

/// Discord refusing the thread (a permission, its limits): the worker's
/// line keeps its « Ouvrir le fil » button, which opens it once Discord
/// takes it.
#[tokio::test(flavor = "multi_thread")]
async fn a_thread_discord_refuses_leaves_its_button_on_the_line() {
    let rig = rig_with("leo-refused", Some("u1")).await;
    let d = &rig.discord;
    let channel = shop_channel(&rig).await;
    d.refuse("POST", "/threads", 403);
    let ada = rig.add_worker("Ada", "t1");
    rig.worker_started("Ada", "t1");
    let line = eventually("Ada's line", || rig.worker(&ada, "line")).await;
    let on_line = format!("/channels/{channel}/messages/{line}/threads");
    eventually("the thread asked for", || {
        d.calls_to("POST", &on_line).pop()
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(rig.worker(&ada, "thread"), None);
    let posted = d.find("POST", &format!("t:{ada}")).unwrap();
    assert!(posted.body.to_string().contains("Ouvrir le fil"));
    let linked = |d: &Discord| {
        d.calls_to("PATCH", &format!("/channels/{channel}/messages/{line}"))
            .into_iter()
            .find(|c| c.body.to_string().contains("Voir le fil"))
    };
    assert!(linked(d).is_none());

    // The button, clicked once Discord takes the thread, opens it.
    d.refuse_nothing();
    d.send(
        "INTERACTION_CREATE",
        interaction(
            "i1",
            "u1",
            &channel,
            3,
            json!({ "custom_id": format!("t:{ada}") }),
            Some(&line),
        ),
    );
    eventually("the thread opened by the button", || {
        rig.worker(&ada, "thread")
    })
    .await;
    eventually("the line links to it", || linked(d)).await;
    assert_eq!(d.calls_to("POST", &on_line).len(), 2);
}

/// Workers started together have their threads opened one after the
/// other, apart: Discord limits how fast threads are made.
#[tokio::test(flavor = "multi_thread")]
async fn workers_started_together_have_their_threads_opened_in_turn() {
    let rig = rig_with("leo-many", Some("u1")).await;
    let d = &rig.discord;
    let channel = shop_channel(&rig).await;
    let names = ["Ada", "Bo", "Cy"];
    let panes: Vec<String> = names
        .iter()
        .map(|name| rig.add_worker(name, "t1"))
        .collect();
    for name in names {
        rig.worker_started(name, "t1");
    }
    let made = eventually("the three threads", || {
        let made: Vec<_> = d
            .calls_to("POST", &format!("/channels/{channel}/messages/"))
            .into_iter()
            .filter(|c| c.path.ends_with("/threads"))
            .collect();
        (made.len() == 3).then_some(made)
    })
    .await;
    for pair in made.windows(2) {
        let apart = pair[1].at.duration_since(pair[0].at);
        assert!(
            apart >= hangar_discord::bot::THREAD_GAP,
            "two threads {apart:?} apart"
        );
    }
    // Each on its own worker's line, named after it.
    for (pane, name) in panes.iter().zip(names) {
        let line = eventually("the line kept", || rig.worker(pane, "line")).await;
        let on_line = made
            .iter()
            .find(|c| c.path.contains(&format!("/messages/{line}/threads")))
            .unwrap_or_else(|| panic!("{name}'s thread on its line"));
        assert!(
            on_line.body["name"]
                .as_str()
                .is_some_and(|thread| thread.starts_with(name)),
            "{on_line:?}"
        );
        eventually("the thread kept", || rig.worker(pane, "thread")).await;
    }
    assert_eq!(threads_made(d), 3);
}

/// A share's record in the index core keeps beside the copies.
fn share_record(
    id: &str,
    at: u64,
    (workspace, pane): (&str, &str),
    (name, mime, size): (&str, &str, u64),
) -> hangar_core::mobile_data::Share {
    hangar_core::mobile_data::Share {
        id: id.into(),
        at,
        workspace: workspace.into(),
        pane: pane.into(),
        name: name.into(),
        mime: mime.into(),
        kind: Default::default(),
        size,
        caption: None,
    }
}

/// What agents share with the phone (`phone_share`) is posted under their
/// name: a worker's in its thread, the orchestrator's in the channel, one
/// too large for Discord as a line that says it is on the phone; nothing
/// from a workspace this Hangar has no channel for.
#[tokio::test(flavor = "multi_thread")]
async fn a_file_an_agent_shares_is_posted_where_it_speaks() {
    let rig = rig_with("leo-shares", Some("u1")).await;
    let d = &rig.discord;
    shop_channel(&rig).await;
    let ada = rig.add_worker("Ada", "t1");
    rig.worker_started("Ada", "t1");
    let thread = eventually("Ada's thread", || rig.worker(&ada, "thread")).await;

    // core's copies and their index (remote/files.rs), as phone_share
    // leaves them.
    let shares = rig._root.path().join("data").join("remote").join("shares");
    let folder = rig._root.path().to_string_lossy().into_owned();
    let picture = b"\x89PNG not quite a picture".to_vec();
    let notes = b"# Notes\n".to_vec();
    for (id, name, bytes) in [("s1", "ecran.png", &picture), ("s2", "notes.md", &notes)] {
        std::fs::create_dir_all(shares.join(id)).unwrap();
        std::fs::write(shares.join(id).join(name), bytes).unwrap();
    }
    let mut from_ada = share_record(
        "s1",
        1,
        (folder.as_str(), "Ada"),
        ("ecran.png", "image/png", picture.len() as u64),
    );
    from_ada.caption = Some("Le panier".into());
    let index = vec![
        from_ada,
        share_record(
            "s2",
            2,
            (folder.as_str(), "Leo"),
            ("notes.md", "text/markdown", notes.len() as u64),
        ),
        share_record(
            "s3",
            3,
            (folder.as_str(), "Leo"),
            (
                "demo.mp4",
                "video/mp4",
                hangar_discord::shares::UPLOAD_LIMIT + 1,
            ),
        ),
        share_record(
            "s4",
            4,
            ("/somewhere/else", "Zed"),
            ("secret.txt", "text/plain", 6),
        ),
    ];
    let written = shares.join("shares.json.tmp");
    std::fs::write(&written, serde_json::to_vec(&index).unwrap()).unwrap();
    std::fs::rename(&written, shares.join("shares.json")).unwrap();

    // Ada's picture, in her thread, shown in place, with its caption.
    let hers = eventually("Ada's picture", || d.find("POST", "ecran.png")).await;
    assert!(
        hers.path.contains(&format!("thread_id={thread}")),
        "{hers:?}"
    );
    assert_eq!(hers.body["username"], "Ada");
    let said = hers.body.to_string();
    assert!(said.contains("\"type\":12"), "{said}");
    assert!(said.contains("attachment://ecran.png"), "{said}");
    assert!(said.contains("Le panier"), "{said}");
    // Leo's notes, in the channel, as a file.
    let his = eventually("Leo's notes", || d.find("POST", "notes.md")).await;
    assert!(!his.path.contains("thread_id"), "{his:?}");
    assert_eq!(his.body["username"], "Leo");
    assert!(his.body.to_string().contains("\"type\":13"), "{his:?}");
    // Too large for Discord: a line, no file.
    let large = eventually("the video said", || d.find("POST", "demo.mp4")).await;
    let said = large.body.to_string();
    assert!(said.contains("téléphone"), "{said}");
    assert!(!said.contains("attachment://"), "{said}");
    assert!(!large.path.contains("thread_id"), "{large:?}");
    // From a workspace this Hangar does not follow: nothing.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(d.find("POST", "secret.txt").is_none());
    // Each once.
    assert_eq!(d.count("POST", "ecran.png"), 1);
    assert_eq!(d.count("POST", "notes.md"), 1);
}

/// ⚡ on what the person wrote that waits in the agent's queue sends it
/// now: it goes first, the others after it in their order, and the
/// agent's turn is stopped; anyone else's ⚡, or one on a message that
/// waits nowhere, does nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_lightning_bolt_sends_a_waiting_message_now() {
    const LEO: &str = "leo-zap";
    let rig = rig_with(LEO, Some("u1")).await;
    let d = &rig.discord;
    let channel = shop_channel(&rig).await;
    let core = rig.host.core().clone();
    let bridge = |event: Value| {
        core.emit(hangar_core::Event::BridgeEvent {
            pane_id: LEO.into(),
            event,
        })
        .unwrap();
    };
    bridge(json!({ "kind": "hello", "sessionId": "s1", "model": null, "version": "2.1" }));
    bridge(json!({ "kind": "turn", "phase": "start" }));
    eventually("Leo working", || {
        rig.host
            .snapshot()
            .activity
            .get(LEO)
            .is_some_and(|a| a.activity == hangar_engine::state::AgentActivity::Working)
            .then_some(())
    })
    .await;
    for (id, word) in [("q1", "un"), ("q2", "deux"), ("q3", "trois")] {
        d.send(
            "MESSAGE_CREATE",
            json!({ "id": id, "channel_id": channel, "guild_id": "g1",
            "content": word, "author": { "id": "u1" } }),
        );
    }
    // What the person wrote that waits for Leo (Hangar's own news for an
    // orchestrator wait there too, and are taken in the middle of a turn).
    let queue = || -> Vec<String> {
        hangar_core::bridge::bridge_queue(LEO.into())
            .into_iter()
            .filter(|entry| entry.from.is_none())
            .map(|entry| entry.text)
            .collect()
    };
    // What Leo's mod hands to Claude Code next of the person's.
    let take = || loop {
        let next =
            hangar_core::bridge::take_next(rig.host.core(), LEO, false).expect("a message waits");
        if next.from.is_none() {
            break next.text;
        }
    };
    eventually("the three waiting", || {
        (queue() == ["un", "deux", "trois"]).then_some(())
    })
    .await;
    // Each with ⏳, and ⚡ once the connector found its entry in the queue
    // core told it: from then on the person's ⚡ on it sends it now.
    eventually("each marked ⚡", || {
        (d.count("PUT", "/reactions/%E2%9A%A1/@me") == 3).then_some(())
    })
    .await;
    let zap = |user: &str, message: &str| {
        json!({ "user_id": user, "channel_id": channel, "message_id": message,
        "guild_id": "g1", "emoji": { "id": null, "name": "\u{26a1}" } })
    };
    // Someone else's ⚡ on the second and the owner's on a message that
    // waits nowhere, then the owner's on the third. The channel's reactions
    // are handled in turn: had either of the first two moved anything, the
    // owner's could not leave the queue as it alone leaves it.
    d.send("MESSAGE_REACTION_ADD", zap("intruder", "q2"));
    d.send("MESSAGE_REACTION_ADD", zap("u1", "nowhere"));
    d.send("MESSAGE_REACTION_ADD", zap("u1", "q3"));
    // The third first now, the others after it in their order, and Leo's
    // turn stopped.
    eventually("the third first, the others after it", || {
        (queue() == ["trois", "un", "deux"]).then_some(())
    })
    .await;
    eventually("Leo's turn stopped", || {
        rig.shells.typed_into(LEO).contains('\u{1b}').then_some(())
    })
    .await;

    // Its mod takes them in turn, the third first, and Leo starts a turn
    // with each: once its card is up, what core told of the take before it
    // is handled (one lane for Leo, in order). The one taken is marked ✅,
    // its ⚡ taken off; those still waiting are not marked.
    let check = |message: &str| {
        d.calls_to(
            "PUT",
            &format!("/channels/{channel}/messages/{message}/reactions/%E2%9C%85"),
        )
    };
    let turn_cards = || {
        d.calls()
            .iter()
            .filter(|c| c.method == "POST" && c.path.starts_with("/webhooks/"))
            .filter(|c| c.body.to_string().contains("travaille"))
            .count()
    };
    let mut waiting = vec!["q3", "q1", "q2"];
    for word in ["trois", "un", "deux"] {
        let message = waiting.remove(0);
        let turns = turn_cards();
        assert_eq!(take(), word);
        bridge(json!({ "kind": "turn", "phase": "start" }));
        eventually("Leo's next turn", || (turn_cards() > turns).then_some(())).await;
        assert_eq!(check(message).len(), 1, "{word} marked ✅ once taken");
        let unzapped = format!("/channels/{channel}/messages/{message}/reactions/%E2%9A%A1");
        assert!(!d.calls_to("DELETE", &unzapped).is_empty(), "{word}'s ⚡");
        for other in &waiting {
            assert!(check(other).is_empty(), "{other} marked ✅ while it waits");
        }
    }
}

/* ── Codex panes ───────────────────────────────────────────────── */

/// A Codex pane is followed as a Claude Code one is: the client Hangar
/// attaches to it reports the same events (hangar-core's
/// agent/codex_attach.rs; those below are written as it writes them), so
/// its turn has its card under its name, its calls (a command, a patch)
/// and its answer, and what the person writes while it works waits in its
/// queue, the ⚡ on it sending it first.
#[tokio::test(flavor = "multi_thread")]
async fn a_codex_orchestrator_is_followed_as_claude_code_is() {
    const LEO: &str = "leo-codex";
    let rig = rig_of(LEO, Some("u1"), AgentId::Codex, None).await;
    let d = &rig.discord;
    let channel = shop_channel(&rig).await;
    let core = rig.host.core().clone();
    let bridge = |event: Value| {
        core.emit(hangar_core::Event::BridgeEvent {
            pane_id: LEO.into(),
            event,
        })
        .unwrap();
    };
    // The thread joined: its id, Codex's model and version.
    bridge(
        json!({ "kind": "hello", "sessionId": "thr-1", "model": "gpt-5.5-codex",
        "version": "0.170.0" }),
    );
    bridge(json!({ "kind": "turn", "phase": "start" }));
    let card = eventually("the turn's card", || d.find("POST", "travaille")).await;
    assert!(card.path.starts_with("/webhooks/"), "{card:?}");
    assert_eq!(card.body["username"], "Leo");
    eventually("Leo working", || {
        rig.host
            .snapshot()
            .activity
            .get(LEO)
            .is_some_and(|a| a.activity == hangar_engine::state::AgentActivity::Working)
            .then_some(())
    })
    .await;

    // Its calls on its card: a command as a shell command, a patch as the
    // files it changes.
    bridge(
        json!({ "kind": "tool", "phase": "start", "toolUseId": "item-1", "tool": "Bash",
        "input": { "command": "cargo test", "cwd": "/work/shop" } }),
    );
    bridge(json!({ "kind": "tool", "phase": "end", "toolUseId": "item-1" }));
    bridge(
        json!({ "kind": "tool", "phase": "start", "toolUseId": "item-2",
        "tool": "apply_patch", "input": { "changes": [
            { "path": "src/cart.rs", "kind": "update", "diff": "@@ -1 +1 @@" },
            { "path": "src/pay.rs", "kind": "add", "diff": "+fn pay() {}" },
        ] } }),
    );
    eventually("the card edited with both calls", || {
        d.calls_to("PATCH", "/webhooks/").into_iter().find(|c| {
            let body = c.body.to_string();
            body.contains("Bash cargo test") && body.contains("apply_patch src/cart.rs +1")
        })
    })
    .await;

    // What the owner writes while Codex works waits in its queue (⏳, then
    // ⚡ once its entry is found); the ⚡ on the second sends it first, and
    // the turn is stopped.
    for (id, word) in [("c1", "un"), ("c2", "deux")] {
        d.send(
            "MESSAGE_CREATE",
            json!({ "id": id, "channel_id": channel, "guild_id": "g1",
            "content": word, "author": { "id": "u1" } }),
        );
    }
    let queue = || -> Vec<String> {
        hangar_core::bridge::bridge_queue(LEO.into())
            .into_iter()
            .filter(|entry| entry.from.is_none())
            .map(|entry| entry.text)
            .collect()
    };
    eventually("both waiting", || (queue() == ["un", "deux"]).then_some(())).await;
    eventually("each marked ⚡", || {
        (d.count("PUT", "/reactions/%E2%9A%A1/@me") == 2).then_some(())
    })
    .await;
    d.send(
        "MESSAGE_REACTION_ADD",
        json!({ "user_id": "u1", "channel_id": channel, "message_id": "c2",
        "guild_id": "g1", "emoji": { "id": null, "name": "\u{26a1}" } }),
    );
    eventually("the second first", || {
        (queue() == ["deux", "un"]).then_some(())
    })
    .await;
    eventually("Codex's turn stopped", || {
        rig.shells.typed_into(LEO).contains('\u{1b}').then_some(())
    })
    .await;

    // The turn ends, interrupted: its card folded with its two calls, its
    // answer posted under its name.
    bridge(json!({ "kind": "tool", "phase": "end", "toolUseId": "item-2" }));
    bridge(
        json!({ "kind": "turn", "phase": "end", "reason": "aborted", "durationMs": 4000,
        "answer": "Le panier est branché sur le paiement." }),
    );
    eventually("the turn folded", || d.find("PATCH", "2 actions")).await;
    let answer = eventually("the answer", || {
        d.find("POST", "Le panier est branché sur le paiement.")
    })
    .await;
    assert_eq!(answer.body["username"], "Leo");

    // Hangar's client takes the queue's first for Codex's next turn (as
    // the mod does for Claude Code): the one the ⚡ sent, marked ✅.
    let next = loop {
        let next =
            hangar_core::bridge::take_next(rig.host.core(), LEO, false).expect("a message waits");
        if next.from.is_none() {
            break next.text;
        }
    };
    assert_eq!(next, "deux");
    eventually("the one taken marked ✅", || {
        d.calls_to(
            "PUT",
            &format!("/channels/{channel}/messages/c2/reactions/%E2%9C%85"),
        )
        .pop()
    })
    .await;
}

/// Where Codex saves the images of thread `thread` (hangar-core's
/// agent/images.rs reads them there): a folder of the test's own.
fn codex_images(rig: &Rig, thread: &str) -> PathBuf {
    let saved = rig
        ._root
        .path()
        .join("codex")
        .join("generated_images")
        .join(thread);
    std::fs::create_dir_all(&saved).unwrap();
    saved
}

/// core's announcement of an image a Codex pane generated, as
/// agent/images.rs raises it.
fn image_generated(rig: &Rig, pane: &str, path: &Path, prompt: Option<&str>) {
    rig.host
        .core()
        .emit(hangar_core::Event::ImageGenerated {
            pane: pane.into(),
            session: "thr-1".into(),
            path: path.to_path_buf(),
            prompt: prompt.map(str::to_string),
        })
        .unwrap();
}

/// The calls that carried a file named `name`.
fn uploads_of(d: &Discord, name: &str) -> Vec<fake::Call> {
    d.calls()
        .into_iter()
        .filter(|c| c.files.iter().any(|(file, _)| file == name))
        .collect()
}

/// An image a Codex agent generates is attached to the card of its turn,
/// under its name, the prompt as its description; it stays on the card as
/// the card is edited with the turn's calls and folded at its end, and
/// Codex's file stays where it is. One too large for Discord is said by a
/// line, its file left alone.
#[tokio::test(flavor = "multi_thread")]
async fn an_image_codex_generates_is_attached_to_its_turn_card() {
    const LEO: &str = "leo-images";
    let rig = rig_of(LEO, Some("u1"), AgentId::Codex, None).await;
    let d = &rig.discord;
    shop_channel(&rig).await;
    let core = rig.host.core().clone();
    let bridge = |event: Value| {
        core.emit(hangar_core::Event::BridgeEvent {
            pane_id: LEO.into(),
            event,
        })
        .unwrap();
    };
    let saved = codex_images(&rig, "thr-1");
    let cat = saved.join("ig_cat.png");
    let picture = b"\x89PNG\r\n\x1a\n a red cat on a roof".to_vec();
    std::fs::write(&cat, &picture).unwrap();

    bridge(json!({ "kind": "turn", "phase": "start" }));
    let card = eventually("the turn's card", || {
        d.calls_to("POST", "/webhooks/")
            .into_iter()
            .find(|c| c.body.to_string().contains("travaille"))
    })
    .await;
    let on_card = format!("/messages/{}?", card.id);

    // The image, on the turn's card: the card edited with the file.
    image_generated(&rig, LEO, &cat, Some("Un chat roux sur un toit"));
    let attached = eventually("the image on the turn's card", || {
        uploads_of(d, "ig_cat.png").pop()
    })
    .await;
    assert_eq!(attached.method, "PATCH", "{attached:?}");
    assert!(attached.path.starts_with("/webhooks/"), "{attached:?}");
    assert!(attached.path.contains(&on_card), "{attached:?}");
    assert_eq!(
        attached.files,
        vec![("ig_cat.png".to_string(), picture.clone())]
    );
    let said = attached.body.to_string();
    assert!(said.contains("travaille"), "{said}");
    assert!(said.contains("\"type\":12"), "{said}");
    assert!(said.contains("attachment://ig_cat.png"), "{said}");
    assert!(said.contains("Un chat roux sur un toit"), "{said}");

    // Too large for Discord: a line under Leo's name, no file.
    let huge = saved.join("ig_huge.png");
    std::fs::File::create(&huge)
        .unwrap()
        .set_len(hangar_discord::images::IMAGE_LIMIT + 1)
        .unwrap();
    image_generated(&rig, LEO, &huge, Some("Un très grand chat"));
    let line = eventually("the large one said", || d.find("POST", "ig_huge.png")).await;
    let said = line.body.to_string();
    assert!(said.contains("trop lourde pour Discord"), "{said}");
    assert!(!said.contains("attachment://"), "{said}");
    assert!(line.files.is_empty(), "{line:?}");
    assert_eq!(line.body["username"], "Leo");

    // The card edited with a call keeps showing the image…
    bridge(
        json!({ "kind": "tool", "phase": "start", "toolUseId": "item-1", "tool": "Bash",
        "input": { "command": "cargo test" } }),
    );
    eventually("the card edited, its image kept", || {
        d.calls_to("PATCH", "/webhooks/").into_iter().find(|c| {
            let body = c.body.to_string();
            c.path.contains(&on_card)
                && body.contains("Bash cargo test")
                && body.contains("attachment://ig_cat.png")
        })
    })
    .await;
    // …and so does the card folded at the turn's end.
    bridge(json!({ "kind": "tool", "phase": "end", "toolUseId": "item-1" }));
    bridge(
        json!({ "kind": "turn", "phase": "end", "reason": "answer", "durationMs": 1000,
        "answer": "Voici le chat." }),
    );
    let folded = eventually("the turn folded", || d.find("PATCH", "1 action")).await;
    assert!(folded.path.contains(&on_card), "{folded:?}");
    assert!(
        folded.body.to_string().contains("attachment://ig_cat.png"),
        "{folded:?}"
    );

    // The file uploaded once, Codex's files where they were, as they were.
    assert_eq!(uploads_of(d, "ig_cat.png").len(), 1);
    assert!(uploads_of(d, "ig_huge.png").is_empty());
    assert_eq!(std::fs::read(&cat).unwrap(), picture);
    assert_eq!(
        std::fs::metadata(&huge).unwrap().len(),
        hangar_discord::images::IMAGE_LIMIT + 1
    );
}

/// An image of a pane the bot does not follow (a Codex pane that is
/// neither its channel's voice nor a worker with a thread, a headless
/// child) gives nothing on Discord; one of its channel's voice with no turn
/// under way comes in a message of its own.
#[tokio::test(flavor = "multi_thread")]
async fn an_image_of_a_pane_not_followed_gives_nothing() {
    const LEO: &str = "leo-images-elsewhere";
    let rig = rig_of(LEO, Some("u1"), AgentId::Codex, None).await;
    let d = &rig.discord;
    shop_channel(&rig).await;
    let cy = rig
        .host
        .apply(ModelCmd::AddPane {
            ws: "shop".into(),
            agent: Some(AgentId::Codex),
            near: None,
            dir: None,
            cwd: None,
            name: Some("Cy".into()),
            init: None,
        })
        .unwrap()
        .pane
        .unwrap();
    let saved = codex_images(&rig, "thr-1");
    let picture = b"\x89PNG\r\n\x1a\n a cat".to_vec();
    for name in ["ig_cy.png", "ig_child.png", "ig_leo.png"] {
        std::fs::write(saved.join(name), picture.as_slice()).unwrap();
    }

    // Cy's turn has no card, and its image goes nowhere; nor does that of
    // a headless child (a delegate's), whose id names no pane.
    rig.host
        .core()
        .emit(hangar_core::Event::BridgeEvent {
            pane_id: cy.clone(),
            event: json!({ "kind": "turn", "phase": "start" }),
        })
        .unwrap();
    image_generated(&rig, &cy, &saved.join("ig_cy.png"), Some("Le chat de Cy"));
    image_generated(&rig, "delegate-child-1", &saved.join("ig_child.png"), None);

    // Leo's, with no turn under way: a message of its own under his name,
    // its prompt too long to describe it left out.
    let long = "un chat ".repeat(200);
    image_generated(&rig, LEO, &saved.join("ig_leo.png"), Some(long.as_str()));
    let own = eventually("Leo's image", || uploads_of(d, "ig_leo.png").pop()).await;
    assert_eq!(own.method, "POST", "{own:?}");
    assert_eq!(own.body["username"], "Leo");
    let said = own.body.to_string();
    assert!(said.contains("a généré une image"), "{said}");
    assert!(said.contains("attachment://ig_leo.png"), "{said}");
    assert!(!said.contains("un chat un chat"), "{said}");

    tokio::time::sleep(Duration::from_millis(500)).await;
    for name in ["ig_cy.png", "ig_child.png"] {
        assert!(uploads_of(d, name).is_empty(), "{name}");
        assert!(d.find("POST", name).is_none(), "{name}");
        assert!(d.find("PATCH", name).is_none(), "{name}");
    }
    assert!(d.find("POST", "Le chat de Cy").is_none());
    assert!(d.calls().iter().all(|c| c.files.len() <= 1));
    assert_eq!(
        d.calls().iter().filter(|c| !c.files.is_empty()).count(),
        1,
        "Leo's image alone was uploaded"
    );
}

/// The dashboard of a workspace whose panes run both agents says which runs
/// which; that of a workspace of one agent reads as it always did.
#[tokio::test(flavor = "multi_thread")]
async fn a_dashboard_names_the_agents_of_a_mixed_workspace() {
    let rig = rig_of("leo-mixed", Some("u1"), AgentId::Codex, None).await;
    let d = &rig.discord;
    let channel = shop_channel(&rig).await;
    let dashboards = || {
        d.calls()
            .into_iter()
            .filter(|c| c.path.starts_with(&format!("/channels/{channel}/messages")))
            .filter(|c| c.body.to_string().contains("Tableau de bord"))
            .collect::<Vec<_>>()
    };
    let first = eventually("the dashboard", || dashboards().first().cloned()).await;
    assert!(
        !first.body.to_string().contains("Codex"),
        "a workspace of Codex alone: {first:?}"
    );
    // A Claude Code worker joins Codex's workspace.
    rig.add_worker("Ada", "t1");
    let mixed = eventually("the dashboard drawn again, naming both", || {
        dashboards().into_iter().find(|c| {
            let body = c.body.to_string();
            body.contains("**Leo** · Codex") && body.contains("**Ada** · Claude Code")
        })
    })
    .await;
    assert_eq!(mixed.method, "PATCH");
}

/// `/workspace` makes its orchestrator with the agent a new workspace's
/// orchestrator runs (D20): the settings' default, else Claude Code when it
/// is there or neither is, else Codex. Through the model writes (the
/// desktop's way, `add_workspace`), and through a host that makes it itself
/// (a node's).
#[tokio::test(flavor = "multi_thread")]
async fn a_workspace_made_from_discord_runs_the_default_orchestrator_agent() {
    use hangar_discord::hangar::add_workspace_with;
    let rig = rig().await;
    let port = rig.host.port();
    let orchestrator_of = |ws: &str| {
        let snapshot = rig.host.snapshot();
        let ws = snapshot
            .state
            .workspaces
            .iter()
            .find(|w| w.id == ws)
            .unwrap();
        assert_eq!(ws.panes.len(), 1);
        assert_eq!(ws.panes[0].role, Some(PaneRole::Orchestrator));
        ws.panes[0].agent
    };
    let folder = |name: &str| {
        let folder = rig._root.path().join(name);
        std::fs::create_dir_all(&folder).unwrap();
        folder.to_string_lossy().into_owned()
    };
    let cases: [(&str, &[AgentKind], AgentId); 4] = [
        (
            "both",
            &[AgentKind::Claude, AgentKind::Codex],
            AgentId::Claude,
        ),
        ("claude", &[AgentKind::Claude], AgentId::Claude),
        ("codex", &[AgentKind::Codex], AgentId::Codex),
        ("none", &[], AgentId::Claude),
    ];
    for (name, installed, agent) in cases {
        let ws = add_workspace_with(&port, name, &folder(name), |kind| installed.contains(&kind))
            .await
            .unwrap();
        assert_eq!(orchestrator_of(&ws), agent, "{name}");
    }

    // The settings say Codex: a workspace made from Discord runs Codex,
    // either way it is made, whatever is installed.
    let rig = rig_of(
        "leo-default",
        Some("u1"),
        AgentId::Claude,
        Some(AgentKind::Codex),
    )
    .await;
    let port = rig.host.port();
    let made = rig._root.path().join("made");
    std::fs::create_dir_all(&made).unwrap();
    let ws = add_workspace_with(&port, "Made", &made.to_string_lossy(), |_| true)
        .await
        .unwrap();
    let snapshot = rig.host.snapshot();
    let pane = &snapshot
        .state
        .workspaces
        .iter()
        .find(|w| w.id == ws)
        .unwrap()
        .panes[0];
    assert_eq!(pane.agent, AgentId::Codex);

    let d = &rig.discord;
    let channel = shop_channel(&rig).await;
    let blog = rig._root.path().join("blog");
    std::fs::create_dir_all(&blog).unwrap();
    d.send("INTERACTION_CREATE", interaction("iw", "u1", &channel, 2, json!({
        "name": "workspace",
        "options": [{ "name": "folder", "value": blog.to_string_lossy() }, { "name": "name", "value": "Blog" }],
    }), None));
    eventually("the command answered", || d.find("PATCH", "Workspace créé")).await;
    let snapshot = rig.host.snapshot();
    let blog = snapshot
        .state
        .workspaces
        .iter()
        .find(|w| w.name == "Blog")
        .unwrap();
    assert_eq!(blog.panes[0].agent, AgentId::Codex);
    assert_eq!(blog.panes[0].role, Some(PaneRole::Orchestrator));
}
