//! « Add to Codex », said and drawn: where a kit stands with Codex, its
//! buttons, and the confirmation that shows what will be added and the very
//! commands that will run before anything does (docs/native/agents.md §6.4).
//! Drawn in the kit's activation, under what reaches Codex, and in its row of
//! Settings › Kits.

use iced::widget::{column, row};
use iced::{Alignment, Element, Length};

use hangar_core::kits::{CodexState, Hook};
use hangar_i18n::{Key, Translator, args};
use hangar_theme::{fonts, tokens};
use hangar_ui::badge::Tone;
use hangar_ui::{button, text as typography};

use super::activation_model::{Tag, TagTone};
use super::{code_block, msg, notice, tag};
use crate::ctx::Ctx;
use crate::kits::Msg;
use crate::kits::codex::{CodexEntry, CodexOp};
use crate::message::Message;

/// What the buttons are, or what replaces them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexActions {
    /// « Add to Codex » (or « Update in Codex ») and « Remove from Codex »,
    /// each when it applies.
    Offer {
        add: Option<String>,
        remove: Option<String>,
        /// Codex is there to run them.
        enabled: bool,
    },
    /// What the confirmation says, before anything runs.
    Confirm {
        op: CodexOp,
        ask: String,
        /// What is added: the skills, the hooks.
        items: Vec<String>,
        /// The commands, each as it runs.
        lines: Vec<String>,
        /// Said of hooks: Codex asks to trust them.
        trust: Option<String>,
        cancel: String,
        confirm: String,
    },
    /// Running.
    Busy(String),
}

/// A kit with Codex, as the views show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexControl {
    pub kit: String,
    pub status: Tag,
    pub hint: String,
    /// Codex is not on this machine.
    pub missing: Option<String>,
    pub actions: CodexActions,
    pub error: Option<String>,
}

/// A hook, as a line names it: its event and what it matches.
pub fn hook_title(hook: &Hook) -> String {
    if hook.matcher.is_empty() {
        hook.event.clone()
    } else {
        format!("{} · {}", hook.event, hook.matcher)
    }
}

impl CodexControl {
    /// `None` while unread, and for a kit with nothing for Codex and
    /// nothing in it.
    pub fn build(t: Translator, kit: &str, name: &str, entry: &CodexEntry) -> Option<Self> {
        let found = entry.status.as_ref()?;
        let has_contents = !found.skills.is_empty() || !found.hooks.is_empty();
        let status = match found.state {
            CodexState::Nothing => return None,
            CodexState::NotAdded => Tag {
                label: t.t(Key::KitsAgentCodexNotAdded).into_owned(),
                tone: TagTone::Plain,
            },
            CodexState::Added => Tag {
                label: t.t(Key::KitsAgentCodexAdded).into_owned(),
                tone: TagTone::Ok,
            },
            CodexState::Changed => Tag {
                label: t.t(Key::KitsAgentCodexChanged).into_owned(),
                tone: TagTone::New,
            },
        };
        let actions = if let Some(op) = entry.busy {
            CodexActions::Busy(
                t.t(match op {
                    CodexOp::Add => Key::KitsAgentCodexAdding,
                    CodexOp::Remove => Key::KitsAgentCodexRemoving,
                })
                .into_owned(),
            )
        } else if let Some(op) = entry.asking.filter(|_| found.codex) {
            let (ask, lines, confirm) = match op {
                CodexOp::Add => (
                    Key::KitsAgentCodexAskAdd,
                    found.add.clone(),
                    if found.state == CodexState::Changed {
                        Key::KitsAgentCodexUpdate
                    } else {
                        Key::KitsAgentCodexAdd
                    },
                ),
                CodexOp::Remove => (
                    Key::KitsAgentCodexAskRemove,
                    found.remove.clone(),
                    Key::KitsAgentCodexRemove,
                ),
            };
            let mut items = Vec::new();
            if op == CodexOp::Add {
                if !found.skills.is_empty() {
                    items.push(
                        t.t_with(
                            Key::KitsAgentCodexSkills,
                            &args!("names" => found.skills.join(", ").as_str()),
                        )
                        .into_owned(),
                    );
                }
                if !found.hooks.is_empty() {
                    let names: Vec<String> = found.hooks.iter().map(hook_title).collect();
                    items.push(
                        t.t_with(
                            Key::KitsAgentCodexHooks,
                            &args!("names" => names.join(", ").as_str()),
                        )
                        .into_owned(),
                    );
                }
            }
            CodexActions::Confirm {
                op,
                ask: t.t_with(ask, &args!("name" => name)).into_owned(),
                items,
                lines,
                trust: (op == CodexOp::Add && !found.hooks.is_empty())
                    .then(|| t.t(Key::KitsAgentCodexTrust).into_owned()),
                cancel: t.t(Key::KitsManageCancel).into_owned(),
                confirm: t.t(confirm).into_owned(),
            }
        } else {
            CodexActions::Offer {
                add: match found.state {
                    CodexState::NotAdded => Some(Key::KitsAgentCodexAdd),
                    CodexState::Changed if has_contents => Some(Key::KitsAgentCodexUpdate),
                    _ => None,
                }
                .map(|key| t.t(key).into_owned()),
                remove: matches!(found.state, CodexState::Added | CodexState::Changed)
                    .then(|| t.t(Key::KitsAgentCodexRemove).into_owned()),
                enabled: found.codex,
            }
        };
        Some(CodexControl {
            kit: kit.to_owned(),
            status,
            hint: t.t(Key::KitsAgentCodexHint).into_owned(),
            missing: (!found.codex).then(|| t.t(Key::KitsAgentCodexMissing).into_owned()),
            actions,
            error: entry.error.clone(),
        })
    }

    /// Every sentence and command it shows, one per line, for the tests.
    #[cfg(test)]
    pub fn text(&self) -> String {
        let mut out = vec![self.status.label.clone(), self.hint.clone()];
        out.extend(self.missing.clone());
        match &self.actions {
            CodexActions::Offer { add, remove, .. } => {
                out.extend(add.clone());
                out.extend(remove.clone());
            }
            CodexActions::Confirm {
                ask,
                items,
                lines,
                trust,
                cancel,
                confirm,
                ..
            } => {
                out.push(ask.clone());
                out.extend(items.iter().cloned());
                out.extend(lines.iter().cloned());
                out.extend(trust.clone());
                out.extend([cancel.clone(), confirm.clone()]);
            }
            CodexActions::Busy(words) => out.push(words.clone()),
        }
        out.extend(self.error.clone());
        out.join("\n")
    }
}

/// The control, drawn: its state and buttons on one line, then the
/// confirmation or what runs, then the words on what it does.
pub fn view<'a>(ctx: Ctx<'a>, control: &CodexControl) -> Element<'a, Message> {
    let kit = control.kit.clone();
    let ask = |op: Option<CodexOp>| {
        msg(Msg::CodexAsk {
            kit: kit.clone(),
            op,
        })
    };
    let mut head = row![tag(ctx, &control.status)]
        .spacing(tokens::space::MD)
        .align_y(Alignment::Center);
    let mut parts: Vec<Element<'a, Message>> = Vec::new();
    match &control.actions {
        CodexActions::Offer {
            add,
            remove,
            enabled,
        } => {
            if let Some(label) = add {
                head = head.push(
                    button::secondary(ctx.theme, label.clone())
                        .small()
                        .on_press_maybe(enabled.then(|| ask(Some(CodexOp::Add)))),
                );
            }
            if let Some(label) = remove {
                head = head.push(
                    button::ghost(ctx.theme, label.clone())
                        .small()
                        .on_press_maybe(enabled.then(|| ask(Some(CodexOp::Remove)))),
                );
            }
        }
        CodexActions::Busy(words) => {
            head = head.push(typography::hint(ctx.theme, words.clone()));
        }
        CodexActions::Confirm {
            op,
            ask: question,
            items,
            lines,
            trust,
            cancel,
            confirm,
        } => {
            parts.push(
                typography::ui(ctx.theme, question.clone())
                    .font(fonts::ui_semibold())
                    .width(Length::Fill)
                    .into(),
            );
            parts.extend(items.iter().map(|item| {
                typography::hint(ctx.theme, item.clone())
                    .width(Length::Fill)
                    .into()
            }));
            parts.extend(lines.iter().map(|line| code_block(ctx, line)));
            if let Some(trust) = trust {
                parts.push(notice(ctx, Tone::Warning, trust.clone()));
            }
            let go = match op {
                CodexOp::Add => button::primary(ctx.theme, confirm.clone()),
                CodexOp::Remove => button::danger(ctx.theme, confirm.clone()),
            };
            parts.push(
                row![
                    button::ghost(ctx.theme, cancel.clone()).on_press(ask(None)),
                    go.on_press(msg(Msg::CodexRun {
                        kit: kit.clone(),
                        op: *op,
                    })),
                ]
                .spacing(tokens::space::MD)
                .into(),
            );
        }
    }
    parts.insert(0, head.wrap().into());
    parts.push(
        typography::hint(ctx.theme, control.hint.clone())
            .width(Length::Fill)
            .into(),
    );
    if let Some(missing) = &control.missing {
        parts.push(notice(ctx, Tone::Warning, missing.clone()));
    }
    if let Some(error) = &control.error {
        parts.push(notice(ctx, Tone::Danger, error.clone()));
    }
    column(parts)
        .spacing(tokens::space::SM)
        .width(Length::Fill)
        .into()
}
