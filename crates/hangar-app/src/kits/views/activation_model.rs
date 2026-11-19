//! What the activation view says, as data: `KitActivationView` of
//! `KitActivation.tsx` without the markup.
//!
//! The web app drew this view from props alone, so every state could be
//! rendered in a test. The same split here: [`ActivationModel::build`] turns
//! the kit, what `kit_inspect` found and what the workspace holds into the
//! sentences and tags the view shows, with every decision made (what is new,
//! what waits to be accepted, whether Run is enabled); `activation.rs` only
//! draws it. The tests read the model's text, in English and French.

use hangar_core::agent::{self, KitPart};
use hangar_core::kits::commands::CommandInfo;
use hangar_core::kits::{
    AgentParts, Inspection, KitInfo, Part, Source, StepStatus, Tab, TabKind, Why,
};
use hangar_i18n::{Key, Translator, args};

use crate::kits::activation::{
    RunGroup, RunItem, commands_to_accept, hooks_to_accept, kit_argv_line, kit_name,
    kit_plain_hooks, kit_requested, kit_rule_text, kit_running_groups, kit_running_line,
    permission_diff, permission_label, requested_permissions,
};
use crate::kits::codex::CodexEntry;
use crate::kits::model::{
    kit_argv_uses_kit_dir, kit_commands_grant, kit_hooks_grant, kit_reads_hidden_files,
};
use crate::kits::setup::KitOs;
use crate::state::types::WorkspaceKit;

use super::codex::{CodexControl, hook_title};

/// A setup step that was started in a pane, waiting for the person to say how
/// it went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepRun {
    pub step: String,
    pub pane_name: String,
}

/// What the view is built from (`KitActivationViewProps`).
pub struct ActivationInput<'a> {
    pub kit: &'a KitInfo,
    /// `None` until `kit_inspect` has answered.
    pub inspection: Option<&'a Inspection>,
    /// `None` until `kit_setup_status` has answered.
    pub steps: Option<&'a [StepStatus]>,
    /// What the workspace holds for this kit now; `None`: not enabled.
    pub entry: Option<&'a WorkspaceKit>,
    pub workspace_name: Option<&'a str>,
    /// Accepting holds for all workspaces, whichever one is open: none is
    /// named.
    pub everywhere: bool,
    /// The workspace exists, so a pane can be opened for a setup step.
    pub can_run: bool,
    pub os: KitOs,
    /// The step whose pane is being opened.
    pub running: Option<&'a str>,
    /// A step that ran and awaits "it worked".
    pub ran: Option<&'a StepRun>,
    pub error: Option<&'a str>,
    /// Where the kit stands with Codex, once read (« Add to Codex »).
    pub codex: Option<&'a CodexEntry>,
}

/// A section's body while it is read, when it has nothing, and once it has
/// something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loadable<T> {
    Loading(String),
    Empty(String),
    Ready(T),
}

/// A small tag: its words, and whether it flags something new.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    pub label: String,
    pub tone: TagTone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagTone {
    Plain,
    /// Done, accepted.
    Ok,
    /// New, to review.
    New,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    pub name: String,
    /// `v1.0.0`.
    pub version: String,
    pub description: Option<String>,
}

/// A name in a list of what a kit brings, with how its tab is made when it is
/// a tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    pub name: String,
    pub kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameGroup {
    pub title: String,
    pub count: usize,
    pub chips: Vec<Chip>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Brings {
    pub heading: String,
    pub body: Loadable<Vec<NameGroup>>,
    /// Said once when a tab is native: what that is.
    pub tab_kind_hint: Option<String>,
}

/// One part of the plugin an agent takes: its name, how it takes it, and
/// what of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartLine {
    pub title: String,
    /// « with each launch », « once added to Codex ».
    pub how: String,
    pub items: Vec<String>,
}

/// What reaches one agent, part by part, and what does not, and why
/// (`Inspection::by_agent`, docs/native/agents.md §6.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentBlock {
    pub title: String,
    /// The manifest leaves this agent out: said in place of its parts.
    pub not_meant: Option<String>,
    pub loads: Vec<PartLine>,
    /// Nothing of the plugin reaches it.
    pub nothing: Option<String>,
    /// « Not in Codex ».
    pub left_title: String,
    /// One line per part and reason.
    pub left_out: Vec<String>,
    /// « Add to Codex », for the agent that takes parts once installed.
    pub codex: Option<CodexControl>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agents {
    pub heading: String,
    /// Empty without a plugin: tabs and setup steps are all there is.
    pub body: Loadable<Vec<AgentBlock>>,
    /// Its tabs and setup commands work whatever the agent.
    pub own: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermItem {
    pub label: String,
    pub tag: Option<Tag>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabPerms {
    /// `{tab} may:`.
    pub title: String,
    pub kind: String,
    pub items: Vec<PermItem>,
    /// For a tab that reads hidden files.
    pub secrets: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Perms {
    pub heading: String,
    pub body: Loadable<Vec<TabPerms>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepAction {
    /// Nothing to do: it ran with this very command.
    None,
    /// The command was typed in a pane; was it done?
    Ask {
        opened: String,
        confirm: String,
        not_yet: String,
    },
    Run {
        label: String,
        enabled: bool,
        /// Said beside the button when there is no workspace to run in.
        needs_workspace: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepModel {
    pub id: String,
    pub title: String,
    pub tag: Tag,
    pub description: Option<String>,
    /// `Command for Windows`.
    pub os_label: String,
    pub command: String,
    pub action: StepAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setup {
    pub heading: String,
    pub body: Loadable<(String, Vec<StepModel>)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookItem {
    pub title: String,
    pub tag: Option<Tag>,
    pub command: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookList {
    pub title: String,
    pub count: usize,
    pub hint: String,
    pub items: Vec<HookItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunItemModel {
    pub title: String,
    pub tag: Option<Tag>,
    pub description: Option<String>,
    pub line: Option<String>,
    pub what: String,
    pub env: Option<String>,
    pub headers: Option<String>,
    pub unreadable: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunGroupModel {
    pub title: String,
    pub count: usize,
    pub when: String,
    pub items: Vec<RunItemModel>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HooksBody {
    pub changed: Option<String>,
    pub live: Option<String>,
    pub hooks: Option<HookList>,
    pub groups: Vec<RunGroupModel>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Runs {
    pub heading: String,
    pub body: Loadable<HooksBody>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleModel {
    /// `{id}`.
    pub placeholder: String,
    pub pattern: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandModel {
    pub name: String,
    pub tag: Option<Tag>,
    pub description: String,
    pub line: String,
    pub what: String,
    pub hint: String,
    pub kit_dir: Option<String>,
    pub rules: Vec<RuleModel>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandsBody {
    pub changed: Option<String>,
    pub live: Option<String>,
    pub items: Vec<CommandModel>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commands {
    pub heading: String,
    pub body: Loadable<CommandsBody>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Footer {
    /// Something to accept.
    Decide {
        refuse: String,
        accept: String,
        accept_enabled: bool,
    },
    /// Nothing to accept.
    Close(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationModel {
    pub title: String,
    pub target: Option<String>,
    pub head: Option<Head>,
    pub brings: Brings,
    pub agents: Agents,
    pub perms: Perms,
    pub setup: Setup,
    pub runs: Runs,
    pub commands: Commands,
    pub error: Option<String>,
    pub footer: Footer,
}

/// How a tab is made, in a word.
pub fn tab_kind_label(t: Translator, tab: &Tab) -> String {
    let key = match (tab.kind(), tab.page().is_some()) {
        (TabKind::Html, _) => Key::KitsManageTabKindHtml,
        (TabKind::Native, true) => Key::KitsManageTabKindNativeFallback,
        (TabKind::Native, false) => Key::KitsManageTabKindNative,
    };
    t.t(key).into_owned()
}

fn names(t: Translator, title: Key, names: &[String]) -> Option<NameGroup> {
    if names.is_empty() {
        return None;
    }
    Some(NameGroup {
        title: t.t(title).into_owned(),
        count: names.len(),
        chips: names
            .iter()
            .map(|name| Chip {
                name: name.clone(),
                kind: None,
            })
            .collect(),
    })
}

impl ActivationModel {
    pub fn build(t: Translator, input: &ActivationInput<'_>) -> ActivationModel {
        let groups = kit_running_groups(input.inspection);
        Self::build_with(t, input, groups)
    }

    /// [`build`](Self::build) with what else the plugin runs given as groups:
    /// core builds a `Running` that nothing outside it can, so a test that
    /// wants one hands the groups over.
    pub fn build_with(
        t: Translator,
        input: &ActivationInput<'_>,
        groups: Vec<RunGroup>,
    ) -> ActivationModel {
        let ActivationInput {
            kit,
            inspection,
            steps,
            entry,
            ..
        } = *input;
        let name = kit_name(kit);
        let requested = kit_requested(kit);
        let diff = permission_diff(
            requested.as_deref().unwrap_or_default(),
            entry.map_or(&[][..], |entry| &entry.granted[..]),
        );
        let needs_accept = entry.is_none() || !diff.added.is_empty();
        let hooks_new = hooks_to_accept(&diff);
        let commands_new = commands_to_accept(&diff);

        let target = if input.everywhere {
            Some(t.t(Key::KitsManageEverywhereNote).into_owned())
        } else {
            input.workspace_name.map(|workspace| {
                t.t_with(Key::KitsManageActOnWorkspace, &args!("name" => workspace))
                    .into_owned()
            })
        };

        let head = kit.manifest.as_ref().map(|manifest| Head {
            name: manifest.name.clone(),
            version: format!("v{}", kit.installed_version),
            description: Some(manifest.description.clone()).filter(|text| !text.is_empty()),
        });

        let title = t
            .t_with(
                if entry.is_some() {
                    Key::KitsManageActReviewTitle
                } else {
                    Key::KitsManageActTitle
                },
                &args!("name" => name),
            )
            .into_owned();

        ActivationModel {
            title,
            target,
            head,
            brings: Self::brings(t, inspection),
            agents: Self::agents(t, kit, inspection, input.codex),
            perms: Self::perms(t, inspection, &diff.added, entry.is_some()),
            setup: Self::setup(t, input, steps),
            runs: Self::runs(t, kit, inspection, &groups, entry.is_some(), hooks_new),
            commands: Self::commands(t, kit, inspection, entry.is_some(), commands_new),
            // A kit that cannot be used cannot be accepted: the button is off,
            // and the dialog says why rather than leaving it greyed out.
            error: input.error.map(str::to_owned).or_else(|| {
                requested.is_none().then(|| {
                    kit.error
                        .clone()
                        .unwrap_or_else(|| t.t(Key::KitsManageBroken).into_owned())
                })
            }),
            footer: if needs_accept {
                Footer::Decide {
                    refuse: t.t(Key::KitsManageActRefuse).into_owned(),
                    accept: t
                        .t(if entry.is_none() {
                            Key::KitsManageActAccept
                        } else if hooks_new || commands_new {
                            Key::KitsManageActAcceptChanged
                        } else {
                            Key::KitsManageActAcceptPerms
                        })
                        .into_owned(),
                    accept_enabled: requested.is_some() && inspection.is_some(),
                }
            } else {
                Footer::Close(t.t(Key::KitsManageActClose).into_owned())
            },
        }
    }

    fn brings(t: Translator, inspection: Option<&Inspection>) -> Brings {
        let loading = || t.t(Key::KitsManageLoading).into_owned();
        let body = match inspection {
            None => Loadable::Loading(loading()),
            Some(found) => {
                let total = found.skills.len()
                    + found.commands.len()
                    + found.agents.len()
                    + found.mcp_servers.len()
                    + found.tabs.len()
                    + found.declared_commands.len();
                if total == 0 {
                    Loadable::Empty(t.t(Key::KitsManageActNothing).into_owned())
                } else {
                    let tabs = if found.tabs.is_empty() {
                        None
                    } else {
                        Some(NameGroup {
                            title: t.t(Key::KitsManageActTabs).into_owned(),
                            count: found.tabs.len(),
                            chips: found
                                .tabs
                                .iter()
                                .map(|tab| Chip {
                                    name: tab.title.clone(),
                                    kind: Some(tab_kind_label(t, tab)),
                                })
                                .collect(),
                        })
                    };
                    Loadable::Ready(
                        [
                            names(t, Key::KitsManageActSkills, &found.skills),
                            names(t, Key::KitsManageActCommands, &found.commands),
                            names(t, Key::KitsManageActAgents, &found.agents),
                            names(t, Key::KitsManageActMcp, &found.mcp_servers),
                            tabs,
                        ]
                        .into_iter()
                        .flatten()
                        .collect(),
                    )
                }
            }
        };
        let any_native = inspection
            .is_some_and(|found| found.tabs.iter().any(|tab| tab.kind() == TabKind::Native));
        Brings {
            heading: t.t(Key::KitsManageActBrings).into_owned(),
            body,
            tab_kind_hint: any_native.then(|| t.t(Key::KitsManageTabKindHint).into_owned()),
        }
    }

    fn agents(
        t: Translator,
        kit: &KitInfo,
        inspection: Option<&Inspection>,
        codex: Option<&CodexEntry>,
    ) -> Agents {
        let body = match inspection {
            None => Loadable::Loading(t.t(Key::KitsManageLoading).into_owned()),
            // Without a plugin nothing differs from one agent to another.
            Some(found) if found.plugin.is_none() => Loadable::Empty(String::new()),
            Some(found) => Loadable::Ready(
                found
                    .by_agent
                    .iter()
                    .map(|parts| Self::agent_block(t, kit, parts, codex))
                    .collect(),
            ),
        };
        Agents {
            heading: t.t(Key::KitsAgentHeading).into_owned(),
            body,
            own: t.t(Key::KitsAgentOwn).into_owned(),
        }
    }

    fn agent_block(
        t: Translator,
        kit: &KitInfo,
        parts: &AgentParts,
        codex: Option<&CodexEntry>,
    ) -> AgentBlock {
        let label = agent::info(parts.agent).label;
        let mut loads = Vec::new();
        let mut left_out = Vec::new();
        if parts.meant_for {
            let lines = [
                Self::part(
                    t,
                    label,
                    Key::KitsManageActMcp,
                    &parts.mcp_servers,
                    String::clone,
                ),
                Self::part(
                    t,
                    label,
                    Key::KitsManageActSkills,
                    &parts.skills,
                    String::clone,
                ),
                Self::part(t, label, Key::KitsAgentHooks, &parts.hooks, hook_title),
                Self::part(
                    t,
                    label,
                    Key::KitsManageActCommands,
                    &parts.commands,
                    String::clone,
                ),
                Self::part(
                    t,
                    label,
                    Key::KitsManageActAgents,
                    &parts.agents,
                    String::clone,
                ),
                Self::part(
                    t,
                    label,
                    Key::KitsAgentOutputStyles,
                    &parts.output_styles,
                    String::clone,
                ),
            ];
            for (line, left) in lines {
                loads.extend(line);
                left_out.extend(left);
            }
        }
        // Read by capability: the agent that takes parts only once they are
        // installed into it is the one « Add to Codex » is for.
        let installs =
            parts.skills.how == KitPart::Installed || parts.hooks.how == KitPart::Installed;
        let name = kit_name(kit);
        AgentBlock {
            title: label.to_owned(),
            not_meant: (!parts.meant_for).then(|| {
                t.t_with(Key::KitsAgentNotMeant, &args!("agent" => label))
                    .into_owned()
            }),
            nothing: (parts.meant_for && loads.is_empty()).then(|| {
                t.t_with(Key::KitsAgentNothing, &args!("agent" => label))
                    .into_owned()
            }),
            loads,
            left_title: t
                .t_with(Key::KitsAgentLeftOut, &args!("agent" => label))
                .into_owned(),
            left_out,
            codex: codex
                .filter(|_| installs)
                .and_then(|entry| CodexControl::build(t, &kit.id, name, entry)),
        }
    }

    /// What of `part` reaches the agent `label`, and a line per reason for
    /// what does not.
    fn part<T>(
        t: Translator,
        label: &str,
        title: Key,
        part: &Part<T>,
        name: impl Fn(&T) -> String,
    ) -> (Option<PartLine>, Vec<String>) {
        let title = t.t(title).into_owned();
        let loads = (!part.loads.is_empty()).then(|| PartLine {
            title: title.clone(),
            how: match part.how {
                KitPart::PerLaunch => t.t(Key::KitsAgentHowLaunch).into_owned(),
                KitPart::Installed => t
                    .t_with(Key::KitsAgentHowInstalled, &args!("agent" => label))
                    .into_owned(),
                KitPart::None => String::new(),
            },
            items: part.loads.iter().map(&name).collect(),
        });
        let mut groups: Vec<(Why, Vec<String>)> = Vec::new();
        for left in &part.left_out {
            match groups.iter_mut().find(|(why, _)| *why == left.why) {
                Some((_, items)) => items.push(name(&left.item)),
                None => groups.push((left.why, vec![name(&left.item)])),
            }
        }
        let lines = groups
            .into_iter()
            .map(|(why, items)| {
                let why = t
                    .t_with(
                        match why {
                            // Not listed part by part: said once, for the agent.
                            Why::NotMeantFor | Why::NoSuchPart => Key::KitsAgentWhyNoSuchPart,
                            Why::Event => Key::KitsAgentWhyEvent,
                            Why::Handler => Key::KitsAgentWhyHandler,
                            Why::Tool => Key::KitsAgentWhyTool,
                            Why::Transport => Key::KitsAgentWhyTransport,
                            Why::Variables => Key::KitsAgentWhyVariables,
                        },
                        &args!("agent" => label),
                    )
                    .into_owned();
                t.t_with(
                    Key::KitsAgentLeftLine,
                    &args!("part" => title.as_str(), "why" => why.as_str(), "names" => items.join(", ").as_str()),
                )
                .into_owned()
            })
            .collect();
        (loads, lines)
    }

    fn perms(
        t: Translator,
        inspection: Option<&Inspection>,
        added: &[String],
        has_entry: bool,
    ) -> Perms {
        let body = match inspection {
            // The web app drew nothing while it read.
            None => Loadable::Loading(String::new()),
            Some(found) if requested_permissions(&found.tabs).is_empty() => {
                Loadable::Empty(t.t(Key::KitsManageActNoPerms).into_owned())
            }
            Some(found) => Loadable::Ready(
                found
                    .tabs
                    .iter()
                    .filter(|tab| !tab.permissions.is_empty())
                    .map(|tab| TabPerms {
                        title: t
                            .t_with(
                                Key::KitsManageActPermsFor,
                                &args!("tab" => tab.title.as_str()),
                            )
                            .into_owned(),
                        kind: tab_kind_label(t, tab),
                        items: tab
                            .permissions
                            .iter()
                            .map(|permission| PermItem {
                                label: permission_label(t, permission),
                                tag: has_entry.then(|| {
                                    if added.contains(permission) {
                                        Tag {
                                            label: t.t(Key::KitsManageActPermsNew).into_owned(),
                                            tone: TagTone::New,
                                        }
                                    } else {
                                        Tag {
                                            label: t.t(Key::KitsManageActPermsGranted).into_owned(),
                                            tone: TagTone::Plain,
                                        }
                                    }
                                }),
                            })
                            .collect(),
                        secrets: kit_reads_hidden_files(&tab.permissions)
                            .then(|| t.t(Key::KitsTabFilesSecrets).into_owned()),
                    })
                    .collect(),
            ),
        };
        Perms {
            heading: t.t(Key::KitsManageActPerms).into_owned(),
            body,
        }
    }

    fn setup(t: Translator, input: &ActivationInput<'_>, steps: Option<&[StepStatus]>) -> Setup {
        let heading = t.t(Key::KitsManageActSetup).into_owned();
        let os_key = match input.os {
            KitOs::Windows => Key::KitsManageOsWindows,
            KitOs::Macos => Key::KitsManageOsMacos,
            KitOs::Linux => Key::KitsManageOsLinux,
        };
        let os = t.t(os_key);
        let body = match steps {
            None => Loadable::Loading(t.t(Key::KitsManageLoading).into_owned()),
            Some([]) => Loadable::Empty(t.t(Key::KitsManageActNoSetup).into_owned()),
            Some(steps) => Loadable::Ready((
                t.t(Key::KitsManageActSetupHint).into_owned(),
                steps
                    .iter()
                    .map(|step| {
                        let asked = input.ran.filter(|ran| ran.step == step.id);
                        StepModel {
                            id: step.id.clone(),
                            title: step.title.clone(),
                            tag: Tag {
                                label: t
                                    .t(if step.done {
                                        Key::KitsManageStepDone
                                    } else {
                                        Key::KitsManageStepTodo
                                    })
                                    .into_owned(),
                                tone: if step.done {
                                    TagTone::Ok
                                } else {
                                    TagTone::Plain
                                },
                            },
                            description: step.description.clone(),
                            os_label: t
                                .t_with(Key::KitsManageActCommandOs, &args!("os" => &*os))
                                .into_owned(),
                            command: step.command.clone(),
                            action: if let Some(ran) = asked {
                                StepAction::Ask {
                                    opened: t
                                        .t_with(
                                            Key::KitsManageStepOpened,
                                            &args!("pane" => ran.pane_name.as_str()),
                                        )
                                        .into_owned(),
                                    confirm: t.t(Key::KitsManageStepConfirm).into_owned(),
                                    not_yet: t.t(Key::KitsManageStepNotYet).into_owned(),
                                }
                            } else if step.done {
                                StepAction::None
                            } else {
                                StepAction::Run {
                                    label: t
                                        .t(if input.running == Some(step.id.as_str()) {
                                            Key::KitsManageStepRunning
                                        } else {
                                            Key::KitsManageStepRun
                                        })
                                        .into_owned(),
                                    enabled: input.can_run
                                        && input.running.is_none()
                                        && input.ran.is_none(),
                                    needs_workspace: (!input.can_run)
                                        .then(|| t.t(Key::KitsManageStepRunNeedsWs).into_owned()),
                                }
                            },
                        }
                    })
                    .collect(),
            )),
        };
        Setup { heading, body }
    }

    fn runs(
        t: Translator,
        kit: &KitInfo,
        inspection: Option<&Inspection>,
        groups: &[RunGroup],
        has_entry: bool,
        hooks_new: bool,
    ) -> Runs {
        let heading = t.t(Key::KitsManageActHooks).into_owned();
        let Some(found) = inspection else {
            return Runs {
                heading,
                body: Loadable::Loading(t.t(Key::KitsManageLoading).into_owned()),
            };
        };
        let hooks = kit_plain_hooks(Some(found));
        if hooks.is_empty() && groups.is_empty() {
            return Runs {
                heading,
                body: Loadable::Empty(t.t(Key::KitsManageActNoHooks).into_owned()),
            };
        }
        // Said on each thing the plugin runs, once the workspace has an entry
        // and the kit is one whose hooks are asked about.
        let consent = (has_entry && kit_hooks_grant(kit).is_some()).then(|| {
            if hooks_new {
                Tag {
                    label: t.t(Key::KitsManageActPermsNew).into_owned(),
                    tone: TagTone::New,
                }
            } else {
                Tag {
                    label: t.t(Key::KitsManageActPermsGranted).into_owned(),
                    tone: TagTone::Plain,
                }
            }
        });
        let body = HooksBody {
            changed: (hooks_new && has_entry)
                .then(|| t.t(Key::KitsManageActHooksChanged).into_owned()),
            live: matches!(kit.source, Source::Folder { .. })
                .then(|| t.t(Key::KitsManageActHooksLive).into_owned()),
            hooks: (!hooks.is_empty()).then(|| HookList {
                title: t.t(Key::KitsManageRunHook).into_owned(),
                count: hooks.len(),
                hint: t.t(Key::KitsManageActHooksHint).into_owned(),
                items: hooks
                    .iter()
                    .map(|hook| HookItem {
                        title: if hook.matcher.is_empty() {
                            hook.event.clone()
                        } else {
                            format!("{} · {}", hook.event, hook.matcher)
                        },
                        tag: consent.clone(),
                        command: hook.command.clone(),
                    })
                    .collect(),
            }),
            groups: groups
                .iter()
                .map(|group| {
                    let (title, when) = group.kind.label();
                    RunGroupModel {
                        title: t.t(title).into_owned(),
                        count: group.items.len(),
                        when: t.t(when).into_owned(),
                        items: group
                            .items
                            .iter()
                            .map(|item| Self::run_item(t, item, consent.clone()))
                            .collect(),
                    }
                })
                .collect(),
        };
        Runs {
            heading,
            body: Loadable::Ready(body),
        }
    }

    fn run_item(t: Translator, item: &RunItem, tag: Option<Tag>) -> RunItemModel {
        let name = kit_rule_text(&item.name);
        RunItemModel {
            title: match &item.when {
                Some(when) => format!("{name} · {}", kit_rule_text(when)),
                None => name.clone(),
            },
            tag,
            description: item.description.as_deref().map(kit_rule_text),
            line: kit_running_line(item),
            what: t
                .t_with(Key::KitsManageRunWhat, &args!("name" => name.as_str()))
                .into_owned(),
            env: (!item.env.is_empty()).then(|| {
                t.t_with(
                    Key::KitsManageRunEnv,
                    &args!("names" => kit_rule_text(&item.env.join(", ")).as_str()),
                )
                .into_owned()
            }),
            headers: (!item.headers.is_empty()).then(|| {
                t.t_with(
                    Key::KitsManageRunHeaders,
                    &args!("names" => kit_rule_text(&item.headers.join(", ")).as_str()),
                )
                .into_owned()
            }),
            unreadable: item
                .unreadable
                .then(|| t.t(Key::KitsManageRunUnreadable).into_owned()),
        }
    }

    fn commands(
        t: Translator,
        kit: &KitInfo,
        inspection: Option<&Inspection>,
        has_entry: bool,
        commands_new: bool,
    ) -> Commands {
        let heading = t.t(Key::KitsTabCommands).into_owned();
        let Some(found) = inspection else {
            return Commands {
                heading,
                body: Loadable::Loading(t.t(Key::KitsManageLoading).into_owned()),
            };
        };
        if found.declared_commands.is_empty() {
            return Commands {
                heading,
                body: Loadable::Empty(t.t(Key::KitsTabCommandsNone).into_owned()),
            };
        }
        let tag = (has_entry && kit_commands_grant(kit).is_some()).then(|| {
            if commands_new {
                Tag {
                    label: t.t(Key::KitsManageActPermsNew).into_owned(),
                    tone: TagTone::New,
                }
            } else {
                Tag {
                    label: t.t(Key::KitsManageActPermsGranted).into_owned(),
                    tone: TagTone::Plain,
                }
            }
        });
        Commands {
            heading,
            body: Loadable::Ready(CommandsBody {
                changed: (commands_new && has_entry)
                    .then(|| t.t(Key::KitsTabCommandsChanged).into_owned()),
                live: matches!(kit.source, Source::Folder { .. })
                    .then(|| t.t(Key::KitsTabCommandsLive).into_owned()),
                items: found
                    .declared_commands
                    .iter()
                    .map(|command| Self::command(t, command, tag.clone()))
                    .collect(),
            }),
        }
    }

    fn command(t: Translator, command: &CommandInfo, tag: Option<Tag>) -> CommandModel {
        CommandModel {
            name: command.name.clone(),
            tag,
            description: command.description.clone(),
            line: kit_argv_line(&command.argv),
            what: t
                .t_with(
                    Key::KitsTabCommandLabel,
                    &args!("name" => command.name.as_str()),
                )
                .into_owned(),
            hint: t
                .t(if command.args.is_empty() {
                    Key::KitsTabCommandHintFixed
                } else {
                    Key::KitsTabCommandHint
                })
                .into_owned(),
            kit_dir: kit_argv_uses_kit_dir(&command.argv)
                .then(|| t.t(Key::KitsTabCommandKitDir).into_owned()),
            rules: command
                .args
                .iter()
                .map(|(name, rule)| RuleModel {
                    placeholder: format!("{{{name}}}"),
                    pattern: kit_rule_text(&rule.pattern),
                    description: rule.description.clone(),
                })
                .collect(),
        }
    }

    /// Every sentence and command the view shows, one per line, for the tests
    /// to look through the way the web app's looked through its markup.
    #[cfg(test)]
    pub fn text(&self) -> String {
        let mut out: Vec<String> = vec![self.title.clone()];
        out.extend(self.target.clone());
        if let Some(head) = &self.head {
            out.push(head.name.clone());
            out.push(head.version.clone());
            out.extend(head.description.clone());
        }
        out.push(self.brings.heading.clone());
        match &self.brings.body {
            Loadable::Loading(text) | Loadable::Empty(text) => out.push(text.clone()),
            Loadable::Ready(groups) => {
                for group in groups {
                    out.push(format!("{} {}", group.title, group.count));
                    for chip in &group.chips {
                        out.push(chip.name.clone());
                        out.extend(chip.kind.clone());
                    }
                }
            }
        }
        out.extend(self.brings.tab_kind_hint.clone());
        out.push(self.agents.heading.clone());
        match &self.agents.body {
            Loadable::Loading(text) | Loadable::Empty(text) => out.push(text.clone()),
            Loadable::Ready(blocks) => {
                for block in blocks {
                    out.push(block.title.clone());
                    out.extend(block.not_meant.clone());
                    for line in &block.loads {
                        out.push(format!("{} {}", line.title, line.how));
                        out.extend(line.items.iter().cloned());
                    }
                    out.extend(block.nothing.clone());
                    if !block.left_out.is_empty() {
                        out.push(block.left_title.clone());
                        out.extend(block.left_out.iter().cloned());
                    }
                    out.extend(block.codex.as_ref().map(CodexControl::text));
                }
            }
        }
        out.push(self.agents.own.clone());
        out.push(self.perms.heading.clone());
        match &self.perms.body {
            Loadable::Loading(text) | Loadable::Empty(text) => out.push(text.clone()),
            Loadable::Ready(tabs) => {
                for tab in tabs {
                    out.push(tab.title.clone());
                    out.push(tab.kind.clone());
                    for item in &tab.items {
                        out.push(item.label.clone());
                        out.extend(item.tag.iter().map(|tag| tag.label.clone()));
                    }
                    out.extend(tab.secrets.clone());
                }
            }
        }
        out.push(self.setup.heading.clone());
        match &self.setup.body {
            Loadable::Loading(text) | Loadable::Empty(text) => out.push(text.clone()),
            Loadable::Ready((hint, steps)) => {
                out.push(hint.clone());
                for step in steps {
                    out.push(step.title.clone());
                    out.push(step.tag.label.clone());
                    out.extend(step.description.clone());
                    out.push(step.os_label.clone());
                    out.push(step.command.clone());
                    match &step.action {
                        StepAction::None => {}
                        StepAction::Ask {
                            opened,
                            confirm,
                            not_yet,
                        } => out.extend([opened.clone(), confirm.clone(), not_yet.clone()]),
                        StepAction::Run {
                            label,
                            needs_workspace,
                            ..
                        } => {
                            out.push(label.clone());
                            out.extend(needs_workspace.clone());
                        }
                    }
                }
            }
        }
        out.push(self.runs.heading.clone());
        match &self.runs.body {
            Loadable::Loading(text) | Loadable::Empty(text) => out.push(text.clone()),
            Loadable::Ready(body) => {
                out.extend(body.changed.clone());
                out.extend(body.live.clone());
                if let Some(list) = &body.hooks {
                    out.push(format!("{} {}", list.title, list.count));
                    out.push(list.hint.clone());
                    for item in &list.items {
                        out.push(item.title.clone());
                        out.extend(item.tag.iter().map(|tag| tag.label.clone()));
                        out.push(item.command.clone());
                    }
                }
                for group in &body.groups {
                    out.push(format!("{} {}", group.title, group.count));
                    out.push(group.when.clone());
                    for item in &group.items {
                        out.push(item.title.clone());
                        out.extend(item.tag.iter().map(|tag| tag.label.clone()));
                        out.extend(item.description.clone());
                        out.extend(item.line.clone());
                        out.extend(item.env.clone());
                        out.extend(item.headers.clone());
                        out.extend(item.unreadable.clone());
                    }
                }
            }
        }
        out.push(self.commands.heading.clone());
        match &self.commands.body {
            Loadable::Loading(text) | Loadable::Empty(text) => out.push(text.clone()),
            Loadable::Ready(body) => {
                out.extend(body.changed.clone());
                out.extend(body.live.clone());
                for item in &body.items {
                    out.push(item.name.clone());
                    out.extend(item.tag.iter().map(|tag| tag.label.clone()));
                    out.push(item.description.clone());
                    out.push(item.line.clone());
                    out.push(item.hint.clone());
                    out.extend(item.kit_dir.clone());
                    for rule in &item.rules {
                        out.push(format!(
                            "{} {} {}",
                            rule.placeholder,
                            rule.pattern,
                            rule.description.clone().unwrap_or_default()
                        ));
                    }
                }
            }
        }
        out.extend(self.error.clone());
        match &self.footer {
            Footer::Decide { refuse, accept, .. } => out.extend([refuse.clone(), accept.clone()]),
            Footer::Close(close) => out.push(close.clone()),
        }
        out.join("\n")
    }

    /// How many of the tags that flag something new the view shows.
    #[cfg(test)]
    pub fn new_tags(&self) -> usize {
        let mut count = 0;
        let mut see = |tag: &Option<Tag>| {
            if tag.as_ref().is_some_and(|tag| tag.tone == TagTone::New) {
                count += 1;
            }
        };
        if let Loadable::Ready(tabs) = &self.perms.body {
            for item in tabs.iter().flat_map(|tab| &tab.items) {
                see(&item.tag);
            }
        }
        if let Loadable::Ready(body) = &self.runs.body {
            for item in body.hooks.iter().flat_map(|list| &list.items) {
                see(&item.tag);
            }
            for item in body.groups.iter().flat_map(|group| &group.items) {
                see(&item.tag);
            }
        }
        if let Loadable::Ready(body) = &self.commands.body {
            for item in &body.items {
                see(&item.tag);
            }
        }
        count
    }

    /// Every text a code block shows: what the view makes copyable.
    #[cfg(test)]
    pub fn commands_shown(&self) -> Vec<String> {
        let mut shown = Vec::new();
        if let Loadable::Ready((_, steps)) = &self.setup.body {
            shown.extend(steps.iter().map(|step| step.command.clone()));
        }
        if let Loadable::Ready(body) = &self.runs.body {
            shown.extend(
                body.hooks
                    .iter()
                    .flat_map(|list| list.items.iter().map(|i| i.command.clone())),
            );
            shown.extend(
                body.groups
                    .iter()
                    .flat_map(|group| group.items.iter().filter_map(|item| item.line.clone())),
            );
        }
        if let Loadable::Ready(body) = &self.commands.body {
            shown.extend(body.items.iter().map(|item| item.line.clone()));
        }
        shown
    }
}

#[cfg(test)]
mod tests;
