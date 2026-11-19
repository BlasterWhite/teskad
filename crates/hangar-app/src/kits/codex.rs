//! « Add to Codex » in the app (docs/native/agents.md §6.4, D18).
//!
//! Codex takes a kit's skills and hooks only once they are installed into it:
//! hangar-core builds a Codex plugin from the kit and has Codex install it
//! (`kit_codex_add`), or uninstall it (`kit_codex_remove`). The app keeps,
//! per kit, where it stands (`kit_codex_status`, read with the kits and when
//! a kit's activation opens), and the two steps of the person's word: the
//! button opens what will be added and the commands that will run, and only
//! the confirmation runs them. Nothing here happens on its own.

use std::collections::HashMap;

use iced::Task;

use hangar_core::kits::{CodexPlugin, KitInfo};
use hangar_i18n::{Key, Translator, args};

use super::Msg;
use crate::message::Message;

/// What the person asks of Codex for a kit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexOp {
    Add,
    Remove,
}

/// One kit with Codex, as the app knows it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodexEntry {
    /// `None` until `kit_codex_status` has answered.
    pub status: Option<CodexPlugin>,
    /// What waits for the person's confirmation.
    pub asking: Option<CodexOp>,
    /// What is running.
    pub busy: Option<CodexOp>,
    /// Why the last add or removal failed.
    pub error: Option<String>,
}

/// The kits by id.
pub type CodexEntries = HashMap<String, CodexEntry>;

fn kits(msg: Msg) -> Message {
    Message::Kits(msg)
}

/// `kit_codex_status` of one kit.
pub fn read(kit: &str) -> Task<Message> {
    let id = kit.to_owned();
    Task::perform(hangar_core::kits::kit_codex_status(id.clone()), move |r| {
        kits(Msg::CodexRead(id.clone(), r.map(Box::new)))
    })
}

/// Where every usable kit stands with Codex, and the kits gone forgotten. A
/// read is a look at a few files, no `codex` started.
pub fn read_all(entries: &mut CodexEntries, list: &[KitInfo]) -> Task<Message> {
    entries.retain(|id, _| list.iter().any(|kit| &kit.id == id));
    Task::batch(
        list.iter()
            .filter(|kit| kit.error.is_none() && kit.manifest.is_some())
            .map(|kit| read(&kit.id)),
    )
}

/// The Codex messages of the package.
pub fn update(entries: &mut CodexEntries, t: Translator, msg: Msg) -> Task<Message> {
    match msg {
        Msg::CodexRead(kit, result) => {
            match result {
                Ok(found) => entries.entry(kit).or_default().status = Some(*found),
                // A kit that cannot be read says so in its own row: nothing
                // is offered for Codex meanwhile.
                Err(_) => {
                    if let Some(entry) = entries.get_mut(&kit) {
                        entry.status = None;
                    }
                }
            }
            Task::none()
        }
        Msg::CodexAsk { kit, op } => {
            let entry = entries.entry(kit).or_default();
            if entry.busy.is_none() {
                entry.asking = op;
                entry.error = None;
            }
            Task::none()
        }
        Msg::CodexRun { kit, op } => {
            let entry = entries.entry(kit.clone()).or_default();
            if entry.busy.is_some() {
                return Task::none();
            }
            entry.asking = None;
            entry.error = None;
            entry.busy = Some(op);
            let id = kit.clone();
            let done = move |result: Result<CodexPlugin, String>| {
                kits(Msg::CodexDone {
                    kit: id.clone(),
                    result: result.map(Box::new),
                })
            };
            match op {
                CodexOp::Add => Task::perform(hangar_core::kits::kit_codex_add(kit), done),
                CodexOp::Remove => Task::perform(hangar_core::kits::kit_codex_remove(kit), done),
            }
        }
        Msg::CodexDone { kit, result } => {
            let entry = entries.entry(kit.clone()).or_default();
            entry.busy = None;
            match result {
                Ok(found) => {
                    entry.status = Some(*found);
                    Task::none()
                }
                Err(detail) => {
                    entry.error = Some(
                        t.t_with(
                            Key::KitsManageErrorPrefix,
                            &args!("detail" => detail.as_str()),
                        )
                        .into_owned(),
                    );
                    // Half of it may have happened: what Codex holds now.
                    read(&kit)
                }
            }
        }
        _ => Task::none(),
    }
}

#[cfg(test)]
mod tests {
    use hangar_core::kits::CodexState;
    use hangar_i18n::Locale;

    use super::*;
    use crate::kits::testing::{installed, kit};

    fn found(state: CodexState) -> CodexPlugin {
        CodexPlugin {
            state,
            plugin: "hello-kit@hangar-kits".into(),
            skills: vec!["hello".into()],
            hooks: Vec::new(),
            codex: true,
            add: vec!["codex plugin add hello-kit@hangar-kits".into()],
            remove: vec!["codex plugin remove hello-kit@hangar-kits".into()],
        }
    }

    fn t() -> Translator {
        Translator::new(Locale::En)
    }

    fn ask(op: Option<CodexOp>) -> Msg {
        Msg::CodexAsk {
            kit: "hello-kit".into(),
            op,
        }
    }

    #[test]
    fn nothing_runs_before_the_person_confirms() {
        let mut entries = CodexEntries::new();
        let _ = update(
            &mut entries,
            t(),
            Msg::CodexRead(
                "hello-kit".into(),
                Ok(Box::new(found(CodexState::NotAdded))),
            ),
        );
        let _ = update(&mut entries, t(), ask(Some(CodexOp::Add)));
        let entry = &entries["hello-kit"];
        assert_eq!(entry.asking, Some(CodexOp::Add));
        assert_eq!(entry.busy, None);

        // Taken back: nothing asked any more, nothing ran.
        let _ = update(&mut entries, t(), ask(None));
        assert_eq!(entries["hello-kit"].asking, None);
        assert_eq!(entries["hello-kit"].busy, None);
    }

    #[test]
    fn the_confirmation_runs_it_once_and_says_where_it_then_stands() {
        let mut entries = CodexEntries::new();
        let _ = update(&mut entries, t(), ask(Some(CodexOp::Add)));
        let run = || Msg::CodexRun {
            kit: "hello-kit".into(),
            op: CodexOp::Add,
        };
        let _ = update(&mut entries, t(), run());
        assert_eq!(entries["hello-kit"].busy, Some(CodexOp::Add));
        assert_eq!(entries["hello-kit"].asking, None);
        // A second click while it runs asks nothing of Codex.
        let _ = update(&mut entries, t(), ask(Some(CodexOp::Remove)));
        assert_eq!(entries["hello-kit"].asking, None);

        let _ = update(
            &mut entries,
            t(),
            Msg::CodexDone {
                kit: "hello-kit".into(),
                result: Ok(Box::new(found(CodexState::Added))),
            },
        );
        let entry = &entries["hello-kit"];
        assert_eq!(entry.busy, None);
        assert_eq!(entry.status.as_ref().unwrap().state, CodexState::Added);
    }

    #[test]
    fn a_refusal_is_said_and_the_status_read_again() {
        let mut entries = CodexEntries::new();
        let _ = update(
            &mut entries,
            t(),
            Msg::CodexRun {
                kit: "hello-kit".into(),
                op: CodexOp::Remove,
            },
        );
        let _ = update(
            &mut entries,
            t(),
            Msg::CodexDone {
                kit: "hello-kit".into(),
                result: Err("`codex plugin remove hello-kit@hangar-kits --json` failed: no".into()),
            },
        );
        let entry = &entries["hello-kit"];
        assert_eq!(entry.busy, None);
        assert_eq!(
            entry.error.as_deref(),
            Some("Error: `codex plugin remove hello-kit@hangar-kits --json` failed: no")
        );
    }

    #[test]
    fn the_kits_gone_are_forgotten() {
        let mut entries = CodexEntries::new();
        entries.insert("gone".into(), CodexEntry::default());
        entries.insert("hello-kit".into(), CodexEntry::default());
        let _ = read_all(&mut entries, &[kit("1.0.0", vec![]), installed("broken")]);
        assert!(entries.contains_key("hello-kit"));
        assert!(!entries.contains_key("gone"));
    }
}
