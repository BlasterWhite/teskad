//! Builders the tests of this package share: kits, manifests, tabs and the
//! workspace entries that go with them.

use std::collections::BTreeMap;

use hangar_core::kits::commands::CommandInfo;
use hangar_core::kits::{CommandArg, GitSource, Hook, Inspection, KitInfo, Manifest, Source, Tab};

use crate::state::types::WorkspaceKit;

pub const HOOKS: &str = "hooks:0123456789abcdef";
pub const OTHER_HOOKS: &str = "hooks:fedcba9876543210";
pub const COMMANDS: &str = "commands:0123456789abcdef";
pub const OTHER_COMMANDS: &str = "commands:fedcba9876543210";

/// A tab of an HTML page asking for `permissions`.
pub fn tab(permissions: &[&str]) -> Tab {
    Tab {
        id: "t".into(),
        title: "T".into(),
        entry: "tabs/t/index.html".into(),
        native: None,
        icon: None,
        permissions: permissions.iter().map(|p| (*p).to_owned()).collect(),
        params: BTreeMap::new(),
        singleton: false,
    }
}

/// A tab that is a WebAssembly module.
pub fn native_tab(permissions: &[&str]) -> Tab {
    Tab {
        entry: "tabs/t.wasm".into(),
        ..tab(permissions)
    }
}

pub fn manifest(id: &str, version: &str, tabs: Vec<Tab>) -> Manifest {
    Manifest {
        schema: 1,
        id: id.into(),
        name: "Hello kit".into(),
        version: version.into(),
        description: String::new(),
        author: None,
        homepage: None,
        icon: None,
        hangar: None,
        plugin: None,
        agents: vec![hangar_core::agent::AgentKind::Claude],
        tabs,
        setup: Vec::new(),
        commands: BTreeMap::new(),
    }
}

pub fn folder() -> Source {
    Source::Folder {
        path: "D:\\kits\\hello-kit".into(),
    }
}

pub fn git(url: &str, git_ref: Option<&str>) -> Source {
    Source::Git(GitSource {
        url: url.into(),
        git_ref: git_ref.map(str::to_owned),
        subdir: None,
    })
}

/// An installed kit with nothing set but its id and where it comes from: a
/// GitHub kit whose manifest could not be read.
pub fn installed(id: &str) -> KitInfo {
    KitInfo {
        id: id.into(),
        source: git("https://github.com/o/kits", None),
        dir: "C:/k".into(),
        installed_version: "1.0.0".into(),
        added_at: 0,
        manifest: None,
        error: None,
        update: None,
        hooks_permission: None,
        hooks_consent: false,
        commands_permission: None,
    }
}

/// `hello-kit` as a linked folder, usable, with these tabs.
pub fn kit(version: &str, tabs: Vec<Tab>) -> KitInfo {
    KitInfo {
        id: "hello-kit".into(),
        source: folder(),
        dir: "D:\\kits\\hello-kit".into(),
        installed_version: version.into(),
        manifest: Some(manifest("hello-kit", version, tabs)),
        ..installed("hello-kit")
    }
}

/// `hello-kit` from GitHub with one tab asking for `panes.read`, and this hash
/// of what its plugin runs by itself.
pub fn git_kit(hooks_permission: Option<&str>, hooks_consent: bool) -> KitInfo {
    KitInfo {
        source: git("https://github.com/o/kits", None),
        hooks_permission: hooks_permission.map(str::to_owned),
        hooks_consent,
        ..kit("1.0.0", vec![tab(&["panes.read"])])
    }
}

/// `hello-kit` with one tab asking for `command:ci-run`, and this hash of its
/// commands.
pub fn command_kit(commands_permission: Option<&str>, source: Source) -> KitInfo {
    KitInfo {
        source,
        commands_permission: commands_permission.map(str::to_owned),
        ..kit("1.0.0", vec![tab(&["command:ci-run"])])
    }
}

pub fn entry(id: &str, granted: &[&str]) -> WorkspaceKit {
    WorkspaceKit {
        id: id.into(),
        granted: granted.iter().map(|g| (*g).to_owned()).collect(),
    }
}

/// What `kit_inspect` would answer for a kit that brings these tabs and nothing
/// else.
pub fn inspection(tabs: Vec<Tab>) -> Inspection {
    Inspection {
        plugin: None,
        skills: Vec::new(),
        commands: Vec::new(),
        agents: Vec::new(),
        mcp_servers: Vec::new(),
        hooks: Vec::new(),
        running: Vec::new(),
        hooks_permission: None,
        tabs,
        setup: Vec::new(),
        declared_commands: Vec::new(),
        commands_permission: None,
        by_agent: Vec::new(),
    }
}

/// The one tab of `ci-status`: a native module that asks for the kit's three
/// commands and for notifications.
pub fn ci_tab() -> Tab {
    Tab {
        id: "ci".into(),
        title: "CI".into(),
        ..native_tab(&[
            "command:ci-run",
            "command:ci-list",
            "command:ci-log",
            "notify",
        ])
    }
}

/// `ci-status` as the library installs it (hangar-ai/kit-ci): from GitHub, a
/// Claude Code plugin whose hook runs by itself after a `git push` (its hash
/// is [`HOOKS`]), three declared commands (`gh run …`, hashed as
/// [`COMMANDS`]), and one tab that asks for them and for notifications.
/// Accepting it takes the tab's permissions, the hook and the commands at once.
pub fn ci_status() -> KitInfo {
    let mut found = manifest("ci-status", "0.1.1", vec![ci_tab()]);
    found.name = "CI status".into();
    found.description = "A tab that follows a GitHub Actions run.".into();
    found.plugin = Some(".".into());
    KitInfo {
        source: git("https://github.com/hangar-ai/kit-ci", None),
        dir: "/kits/ci-status".into(),
        installed_version: "0.1.1".into(),
        manifest: Some(found),
        hooks_permission: Some(HOOKS.into()),
        hooks_consent: true,
        commands_permission: Some(COMMANDS.into()),
        ..installed("ci-status")
    }
}

/// What `kit_inspect` answers for [`ci_status`]: what the person is shown, and
/// accepts.
pub fn ci_inspection() -> Inspection {
    Inspection {
        skills: vec!["ci-status".into()],
        hooks: vec![Hook {
            event: "PostToolUse".into(),
            matcher: "Bash".into(),
            command: "node \"${CLAUDE_PLUGIN_ROOT}/hooks/open-ci.js\"".into(),
        }],
        hooks_permission: Some(HOOKS.into()),
        declared_commands: vec![
            ci_command(
                "ci-list",
                "The 15 most recent GitHub Actions runs.",
                &["gh", "run", "list", "--limit", "15"],
            ),
            ci_command(
                "ci-log",
                "The log of the steps that failed in one run.",
                &["gh", "run", "view", "{id}", "--log-failed"],
            ),
            ci_command(
                "ci-run",
                "The state of one GitHub Actions run.",
                &[
                    "gh",
                    "run",
                    "view",
                    "{id}",
                    "--json",
                    "status,conclusion,jobs",
                ],
            ),
        ],
        commands_permission: Some(COMMANDS.into()),
        ..inspection(vec![ci_tab()])
    }
}

/// One of the commands `ci-status` declares, as `kit_inspect` lists it.
fn ci_command(name: &str, description: &str, argv: &[&str]) -> CommandInfo {
    let args = argv
        .contains(&"{id}")
        .then(|| {
            (
                "id".to_owned(),
                CommandArg {
                    pattern: "[0-9]+".into(),
                    description: Some("The run's number".into()),
                },
            )
        })
        .into_iter()
        .collect();
    CommandInfo {
        name: name.into(),
        description: description.into(),
        argv: argv.iter().map(|item| (*item).to_owned()).collect(),
        args,
        timeout_seconds: 30,
    }
}

/// What accepting [`ci_status`] stores: its tab's permissions in their order,
/// then the hashes of its hook and of its commands.
pub const CI_GRANTED: [&str; 6] = [
    "notify",
    "command:ci-list",
    "command:ci-log",
    "command:ci-run",
    HOOKS,
    COMMANDS,
];
