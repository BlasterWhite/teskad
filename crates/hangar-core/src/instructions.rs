//! Writes the agent playbook into the file each agent reads on startup
//! (docs/native/agents.md §6.5, D19).
//!
//! A doc parked in `.hangar/` would be loaded by nobody. Each agent the
//! workspace runs has its project instructions file
//! (`AgentInfo::instructions_file`: `CLAUDE.md` for Claude Code, `AGENTS.md`
//! for Codex), and the block goes into each, between the same markers, never
//! touching the rest of the file.
//!
//! Written once: when one of those files is a link to the other (a
//! `CLAUDE.md` that links to `AGENTS.md` is a common setup, and `store.rs`
//! keeps it a link), both names reach one file, and a second write would only
//! say the same thing again. Hangar never makes Codex read `CLAUDE.md`
//! (`project_doc_fallback_filenames` is the person's Codex configuration).
//!
//! A workspace that names no agent runs Claude Code, and gets `CLAUDE.md`
//! alone, as before Codex: a block an earlier version left in `AGENTS.md` is
//! then neither touched nor counted as installed.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::agent::AgentKind;
use crate::store::{write_atomic, Access};

/// The product's name, as every text written for a person or an agent says it.
/// The same value as `hangar_i18n::APP_NAME` (hangar-core does not depend on
/// hangar-i18n; a test in hangar-app holds the two equal): change both together.
pub const APP_NAME: &str = "Teskad";

const START: &str = "<!-- HANGAR:START - generated, do not edit by hand -->";
const END: &str = "<!-- HANGAR:END -->";

/// Marker pairs written by earlier versions, newest first.
///
/// These are already sitting in users' `CLAUDE.md`. A rename
/// that only knew the new markers would not match the old block, so instead of
/// replacing the playbook it would append a second copy on every run.
const LEGACY_MARKERS: &[(&str, &str)] = &[(
    "<!-- IABENCH:START - generated, do not edit by hand -->",
    "<!-- IABENCH:END -->",
)];

/// The instructions file of each agent in `agents`, each once, in the order
/// given.
///
/// No agent named means Claude Code, as everywhere in Hangar (`AgentKind`'s
/// default): what the callers that predate Codex ask for.
fn instruction_files(agents: &[AgentKind]) -> Vec<&'static str> {
    let unnamed: [AgentKind; 1] = [Default::default()];
    let agents: &[AgentKind] = if agents.is_empty() { &unnamed } else { agents };
    let mut files: Vec<&'static str> = Vec::with_capacity(agents.len());
    for agent in agents {
        let file = agent.info().instructions_file;
        if !files.contains(&file) {
            files.push(file);
        }
    }
    files
}

#[derive(Debug, Serialize)]
pub struct InstructionReport {
    pub path: String,
    pub ok: bool,
    pub message: String,
}

fn playbook() -> String {
    let app = APP_NAME;
    format!(
        r#"{START}

## Shared task board ({app})

This project is driven from {app}. Several agents work **in parallel** in separate
terminals on this same directory. A shared kanban board is the source of truth for
who is doing what. Reach it through the MCP tools prefixed `board_`.

### Hard rule

**Never start work on a task without claiming it first.** Another agent may already
be on it. `board_claim_task` fails when the task is taken — when that happens, do not
retry it, pick a different one.

### Expected loop

1. **On startup**, call `board_list_tasks` to see the current state of the work.
2. **To pick work**, call `board_next_task`. It returns the highest-priority free task
   whose dependencies are all done. Do not pick arbitrarily from the todo column, or
   you may start work that is still blocked.
3. **Claim it** with `board_claim_task`. It moves to `doing` automatically.
   - If the call fails, go back to step 2.
4. **Do the work.**
5. **Report** with `board_comment_task`: decisions made, files touched, problems hit.
   This is the only channel other agents have to learn what you did.
6. **Finish** with `board_update_task`, setting `column` to `review` if the work needs
   a second pair of eyes, or `done` otherwise.
   - If you give up, call `board_update_task` with `release: true` so the task returns
     to the pool instead of sitting blocked under your name.

### Splitting work

When you find work beyond your current task, do not silently pile it into the same
change: create tasks with `board_create_task` so other agents can pick them up.

- Use `depends_on` with the ids of prerequisite tasks. `board_next_task` relies on it
  to never hand out work that cannot start yet.
- Use `priority` (integer, higher is more urgent) for work that unblocks others.

### Waiting on something outside the board

When a task cannot go on because of a package release, a person's decision or a date,
call `board_block_task` with a `reason` (and `recheck`, the day to look again), not a
comment: the task returns to `todo`, nobody picks it up, and the next agent does not
repeat your check. If it was your task, stop there and do not deliver. Once the wait
is over, `board_unblock_task`; a `human_only` block is lifted by the person alone.

### Columns

| Column   | Meaning |
|----------|---------|
| `todo`   | ready to be picked up |
| `doing`  | someone is working on it |
| `review` | finished, waiting for a review |
| `done`   | accepted - unblocks tasks that depend on it |

### Staying out of each other's way

Before editing a central file, check with `board_list_tasks` that no other agent holds
a `doing` task covering the same area. If one does, leave a comment on their task
rather than editing the same files at the same time.

### Talking to the agent next door

Claude Code sessions in {app} are named after their pane and its workspace, for
example `Ava (Shop)`. A pane's first name is unique across every workspace, so `Ava` is
one agent and the workspace in brackets tells you which project it works on. When another
agent needs to know something now (you changed a file it is working on, you answered its
question), message it directly with `SendMessage` to that name; `/list-agents` shows who
is reachable. The board stays the record: write the same thing there if it matters beyond
this moment.

### Showing a 3D model

To show the user a 3D model, open it with `model_show` rather than giving them a path, and
check your own work with `model_snapshot`, which returns a picture of it.

### Showing a file, or sending it to the phone

To show the user a render, screenshot, recording, report, Markdown, PDF or HTML page, open
it with `preview_show {{path, title?}}` rather than giving them a path: it opens as a pane
of the workspace's grid, beside the terminals. An HTML page runs isolated, so inline its scripts and styles
(its own `.js` and `.css` files do not load). Call `preview_close` once you are done, so it
does not keep a part of their screen.

To send a file to the user's phone ({app} Mobile), use `phone_share {{path, caption?}}`, when
they ask for it or a result is clearly worth it, never for routine output.

### Opening a kit's tab

A kit enabled on this workspace can bring small tabs that take params, such as one showing a
CI run for its id. `kit_tab_list` says which there are, `kit_tab_open {{kit, tab, params?, reveal?}}`
opens one as a pane of the grid too, without taking the keyboard from anyone, and
`kit_tab_close {{kit, tab, params?}}` closes it.

## Global memory

You also have a **persistent global memory**, reached through the MCP tools prefixed
`memory_`. It is shared by every workspace, every project and every agent, and it
survives across sessions: what you write there, the next agent reads. The origin
(workspace, agent) is recorded for you.

### Read before you dig

**At the start of a task, call `memory_search`** with the project name and the subject you
are about to touch. Another agent may already have paid the price of finding this out —
do not re-discover what is already written down. `memory_list` gives a compact index,
`memory_read` pulls one entry in full.

### Write the moment you learn

Call `memory_write` as soon as a durable, reusable fact appears, not at the end of the
task. Worth keeping:

- architecture decisions **and the reason behind them**
- environment or build traps: the flag that is required, the command that hangs, the
  version that breaks things
- project conventions that are not obvious from a quick read
- preferences the user stated explicitly

Do not write: task progress, which belongs in `board_comment_task`; anything readable
from the code or the git history; ephemeral details of the current conversation.

### How to write an entry

- **Title** short, stable, searchable. `memory_write` upserts on the title
  (case-insensitive), so reuse the same title to update a fact instead of stacking a
  near-duplicate next to it.
- **Content** brief and factual — actionable by someone arriving with no context.
- **Tags** must include the project name. The memory is global, so an entry that does not
  carry its own project context is noise everywhere else.

### Hygiene

When a fact turns out to be false, correct it with `memory_update` or drop it with
`memory_delete`. Never leave two entries contradicting each other: the next agent has no
way to tell which one is still true.

{END}"#
    )
}

/// Replaces our block if present, otherwise appends it, leaving the rest of the
/// file untouched.
fn upsert_block(existing: &str, block: &str) -> String {
    let known = std::iter::once((START, END)).chain(LEGACY_MARKERS.iter().copied());
    for (start_marker, end_marker) in known {
        // The block is an END and the START closest before it. Pairing the
        // first START with the first END instead would, once the person had
        // deleted an END (and a block had been appended after it), take
        // everything between the two blocks, their own text included.
        let Some(end) = existing.find(end_marker) else {
            continue;
        };
        if let Some(start) = existing[..end].rfind(start_marker) {
            let mut out = String::with_capacity(existing.len());
            out.push_str(&existing[..start]);
            out.push_str(block);
            out.push_str(&existing[end + end_marker.len()..]);
            return out;
        }
    }

    if existing.trim().is_empty() {
        return format!("{block}\n");
    }
    format!("{}\n\n{}\n", existing.trim_end(), block)
}

/// What a file Hangar adds to holds now: `None` when there is none yet.
///
/// Only a missing file counts as empty. One that cannot be read (EACCES), or
/// that is not UTF-8 text (a `CLAUDE.md` saved as UTF-16 or Latin-1 by
/// Notepad), is not ours to rewrite: written back, it would hold Hangar's
/// lines alone and the person's would be gone.
fn read_existing(path: &Path) -> Result<Option<String>, String> {
    match fs::read(path) {
        Ok(bytes) => String::from_utf8(bytes).map(Some).map_err(|_| {
            "ce fichier n'est pas du texte UTF-8 ; il est laissé tel quel".to_string()
        }),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(format!(
            "lecture impossible ({err}) ; le fichier est laissé tel quel"
        )),
    }
}

fn refused(path: &Path, message: String) -> InstructionReport {
    InstructionReport {
        path: path.to_string_lossy().into_owned(),
        ok: false,
        message,
    }
}

fn write_playbook(root: &Path, file: &str) -> InstructionReport {
    let path = root.join(file);
    let existing = match read_existing(&path) {
        Ok(existing) => existing.unwrap_or_default(),
        Err(message) => return refused(&path, message),
    };
    let updated = upsert_block(&existing, &playbook());

    match write_atomic(&path, updated.as_bytes(), Access::AsBefore) {
        Ok(()) => InstructionReport {
            path: path.to_string_lossy().into_owned(),
            ok: true,
            message: if existing.is_empty() {
                "créé".into()
            } else {
                "bloc mis à jour".into()
            },
        },
        Err(err) => InstructionReport {
            path: path.to_string_lossy().into_owned(),
            ok: false,
            message: err,
        },
    }
}

/// Appends the ignore rule only when no existing line already covers it, so
/// running this repeatedly does not pile up duplicates.
fn update_gitignore(root: &Path) -> InstructionReport {
    let path = root.join(".gitignore");
    let existing = match read_existing(&path) {
        Ok(existing) => existing.unwrap_or_default(),
        Err(message) => return refused(&path, message),
    };

    // Deliberately only looks for the current directory: a repository that
    // already ignores `.iabench/` still needs a rule for `.hangar/`, or the
    // board lands in the next commit. The two coexist until the old board is
    // gone, and both deserve a line.
    let already = existing.lines().any(|line| {
        let trimmed = line.trim().trim_end_matches('/');
        trimmed == ".hangar" || trimmed == "/.hangar"
    });

    if already {
        return InstructionReport {
            path: path.to_string_lossy().into_owned(),
            ok: true,
            message: "déjà présent".into(),
        };
    }

    let mut updated = existing;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    if !updated.is_empty() {
        updated.push('\n');
    }
    updated.push_str(&format!(
        "# {APP_NAME} — tableau de tâches local\n.hangar/\n"
    ));

    match write_atomic(&path, updated.as_bytes(), Access::AsBefore) {
        Ok(()) => InstructionReport {
            path: path.to_string_lossy().into_owned(),
            ok: true,
            message: "règle ajoutée".into(),
        },
        Err(err) => InstructionReport {
            path: path.to_string_lossy().into_owned(),
            ok: false,
            message: err,
        },
    }
}

/// The playbook for a workspace that names no agent: Claude Code's
/// `CLAUDE.md` alone. See [`write_agent_instructions_for`].
pub fn write_agent_instructions(workspace_cwd: String) -> Vec<InstructionReport> {
    write_agent_instructions_for(workspace_cwd, &[])
}

/// Writes the playbook into the instructions file of each agent in `agents`,
/// the kinds the workspace runs, then the `.gitignore` rule. One report per
/// file: plain files first, links after, the `.gitignore` last.
///
/// A file that one written before already reached (a `CLAUDE.md` that links
/// to `AGENTS.md`, or the reverse) is not written a second time: its report
/// says which name holds the block.
pub fn write_agent_instructions_for(
    workspace_cwd: String,
    agents: &[AgentKind],
) -> Vec<InstructionReport> {
    let root = PathBuf::from(&workspace_cwd);
    if !root.is_dir() {
        return vec![InstructionReport {
            path: workspace_cwd,
            ok: false,
            message: "dossier introuvable".into(),
        }];
    }

    // Links last: one to an `AGENTS.md` that does not exist yet reaches it
    // once it is written, where written first it would have been replaced by
    // a file of its own.
    let mut files = instruction_files(agents);
    files.sort_by_key(|file| {
        fs::symlink_metadata(root.join(file)).is_ok_and(|meta| meta.file_type().is_symlink())
    });

    // The real file behind each name written so far, links followed.
    let mut written: Vec<(PathBuf, &str)> = Vec::new();
    let mut reports = Vec::new();
    for file in files {
        let path = root.join(file);
        let real = fs::canonicalize(&path).ok();
        if let Some((_, first)) = written.iter().find(|(done, _)| real.as_ref() == Some(done)) {
            reports.push(InstructionReport {
                path: path.to_string_lossy().into_owned(),
                ok: true,
                message: format!("même fichier : bloc déjà écrit dans {first}"),
            });
            continue;
        }
        let report = write_playbook(&root, file);
        // Looked up after the write: a file it created had no real path
        // before.
        if report.ok {
            if let Ok(real) = fs::canonicalize(&path) {
                written.push((real, file));
            }
        }
        reports.push(report);
    }

    reports.push(update_gitignore(&root));
    reports
}

/// Lets the UI show whether the playbook is already in place, for a
/// workspace that names no agent. See [`agent_instructions_status_for`].
pub fn agent_instructions_status(workspace_cwd: String) -> bool {
    agent_instructions_status_for(workspace_cwd, &[])
}

/// Whether every agent in `agents` finds the playbook in the file it reads.
pub fn agent_instructions_status_for(workspace_cwd: String, agents: &[AgentKind]) -> bool {
    let root = PathBuf::from(workspace_cwd);
    instruction_files(agents).iter().all(|file| {
        fs::read_to_string(root.join(file))
            .map(|content| {
                content.contains(START)
                    || LEGACY_MARKERS
                        .iter()
                        .any(|(start, _)| content.contains(start))
            })
            .unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reason `upsert_block` knows the old markers at all: without this the
    /// generated section is appended a second time instead of replaced, and it
    /// grows by one copy on every install.
    #[test]
    fn a_block_under_the_previous_markers_is_replaced_not_duplicated() {
        let (legacy_start, legacy_end) = LEGACY_MARKERS[0];
        let existing =
            format!("# Projet\n\n{legacy_start}\nancien contenu\n{legacy_end}\n\n## Suite\n");

        let updated = upsert_block(&existing, &playbook());

        assert_eq!(updated.matches(START).count(), 1);
        assert!(!updated.contains(legacy_start));
        assert!(!updated.contains("ancien contenu"));
        assert!(updated.contains("# Projet"));
        assert!(updated.contains("## Suite"));
    }

    #[test]
    fn a_block_under_the_current_markers_is_replaced_in_place() {
        let existing = format!("avant\n\n{START}\nx\n{END}\n\napres\n");

        let updated = upsert_block(&existing, &playbook());

        assert_eq!(updated.matches(START).count(), 1);
        assert!(updated.starts_with("avant"));
        assert!(updated.trim_end().ends_with("apres"));
    }

    /// The memory tools are useless if the playbook does not name them: an agent
    /// that never reads about `memory_search` will never call it. This pins the
    /// two halves of the loop — read on startup, write when you learn — and the
    /// place they must not end up in.
    #[test]
    fn the_generated_block_teaches_the_memory_tools() {
        let block = playbook();

        assert!(block.contains("## Global memory"));
        for tool in [
            "memory_search",
            "memory_list",
            "memory_read",
            "memory_write",
        ] {
            assert!(block.contains(tool), "playbook never mentions {tool}");
        }
        // Task progress belongs to the board, not to the memory.
        assert!(block.contains("board_comment_task"));
    }

    /// Same reason as the memory tools: an agent that has never read about
    /// `model_show` hands the user a file path instead.
    #[test]
    fn the_generated_block_teaches_blocking_a_task() {
        let block = playbook();
        for tool in ["board_block_task", "board_unblock_task"] {
            assert!(block.contains(tool), "playbook never mentions {tool}");
        }
    }

    #[test]
    fn the_generated_block_names_the_model_tools() {
        let block = playbook();

        assert!(block.contains("model_show"));
        assert!(block.contains("model_snapshot"));
    }

    /// Same again for the previews and the phone.
    #[test]
    fn the_generated_block_names_the_preview_and_phone_tools() {
        let block = playbook();

        for tool in ["preview_show", "preview_close", "phone_share"] {
            assert!(block.contains(tool), "playbook never mentions {tool}");
        }
    }

    /// And for a kit's tabs: an agent that never read about them cannot open one.
    #[test]
    fn the_generated_block_names_the_kit_tab_tools() {
        let block = playbook();

        for tool in ["kit_tab_list", "kit_tab_open", "kit_tab_close"] {
            assert!(block.contains(tool), "playbook never mentions {tool}");
        }
    }

    /// A START left alone (the person deleted its END) must not pair with the
    /// END of a later block and take the text between them.
    #[test]
    fn a_start_without_its_end_does_not_swallow_the_text_after_it() {
        let existing = format!("{START}\nreste\n\n# Mes notes\n\n{START}\nx\n{END}\n");

        let updated = upsert_block(&existing, &playbook());

        assert!(updated.contains("# Mes notes"));
        assert!(updated.contains("reste"));
        assert!(!updated.contains("\nx\n"));
        assert_eq!(updated.matches(END).count(), 1);
    }

    fn scratch() -> PathBuf {
        crate::testdir::scratch("instructions")
    }

    /// A `CLAUDE.md` and a `.gitignore` that are not UTF-8 (saved as UTF-16 or
    /// Latin-1) are reported, and left byte for byte as they were.
    #[test]
    fn files_that_are_not_utf8_are_left_untouched() {
        let root = scratch();
        let latin1 = b"# Caf\xe9\n\nMes r\xe8gles.\n".to_vec();
        let utf16 = b"\xff\xfe#\x00 \x00n\x00o\x00d\x00e\x00\n\x00".to_vec();
        fs::write(root.join("CLAUDE.md"), &latin1).unwrap();
        fs::write(root.join(".gitignore"), &utf16).unwrap();

        let reports = write_agent_instructions(root.to_string_lossy().into_owned());

        assert_eq!(reports.len(), 2);
        assert!(reports.iter().all(|report| !report.ok), "{reports:?}");
        assert_eq!(fs::read(root.join("CLAUDE.md")).unwrap(), latin1);
        assert_eq!(fs::read(root.join(".gitignore")).unwrap(), utf16);
        fs::remove_dir_all(&root).ok();
    }

    /// One Hangar may not read is not taken for an empty one.
    #[cfg(unix)]
    #[test]
    fn a_file_that_cannot_be_read_is_not_written_over() {
        use std::os::unix::fs::PermissionsExt;
        // Permissions do not stop root.
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let root = scratch();
        let path = root.join("CLAUDE.md");
        fs::write(&path, "# Mes règles\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o200)).unwrap();

        let report = write_playbook(&root, "CLAUDE.md");

        assert!(!report.ok, "{report:?}");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "# Mes règles\n");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_file_with_no_marker_keeps_what_it_had() {
        let updated = upsert_block("# Mon projet\n", &playbook());

        assert!(updated.contains("# Mon projet"));
        assert_eq!(updated.matches(START).count(), 1);
    }

    /// The file names the reports are about, in their order.
    fn names(reports: &[InstructionReport]) -> Vec<String> {
        reports
            .iter()
            .map(|report| {
                Path::new(&report.path)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }

    fn read(root: &Path, file: &str) -> String {
        fs::read_to_string(root.join(file)).unwrap()
    }

    /// A workspace that names no agent runs Claude Code, which reads
    /// `CLAUDE.md`: the playbook goes there only, as before Codex, and one an
    /// earlier version left in `AGENTS.md` is neither touched nor counted.
    #[test]
    fn a_workspace_that_names_no_agent_gets_claude_md_only() {
        let root = scratch();
        let old = format!("# Notes\n\n{}\n", playbook());
        fs::write(root.join("AGENTS.md"), &old).unwrap();
        let cwd = root.to_string_lossy().into_owned();
        assert!(!agent_instructions_status(cwd.clone()));

        let reports = write_agent_instructions(cwd.clone());

        assert_eq!(names(&reports), ["CLAUDE.md", ".gitignore"]);
        assert!(reports.iter().all(|report| report.ok));
        assert!(agent_instructions_status(cwd));
        assert_eq!(read(&root, "AGENTS.md"), old);
    }

    /// Each agent's own file, each once; no agent named is Claude Code.
    #[test]
    fn each_agent_brings_its_instructions_file_once() {
        assert_eq!(instruction_files(&[]), ["CLAUDE.md"]);
        assert_eq!(instruction_files(&[AgentKind::Claude]), ["CLAUDE.md"]);
        assert_eq!(instruction_files(&[AgentKind::Codex]), ["AGENTS.md"]);
        assert_eq!(
            instruction_files(&[AgentKind::Codex, AgentKind::Claude, AgentKind::Codex]),
            ["AGENTS.md", "CLAUDE.md"]
        );
    }

    /// Codex reads `AGENTS.md`: the block goes after the person's text there,
    /// and their `CLAUDE.md` is left as it was.
    #[test]
    fn a_codex_workspace_gets_the_playbook_in_agents_md() {
        let root = scratch();
        let mine = "# Mes règles\n\nNe touche pas à ça.\n";
        fs::write(root.join("AGENTS.md"), mine).unwrap();
        fs::write(root.join("CLAUDE.md"), "# Claude\n").unwrap();
        let cwd = root.to_string_lossy().into_owned();
        assert!(!agent_instructions_status_for(
            cwd.clone(),
            &[AgentKind::Codex]
        ));

        let reports = write_agent_instructions_for(cwd.clone(), &[AgentKind::Codex]);

        assert_eq!(names(&reports), ["AGENTS.md", ".gitignore"]);
        assert!(reports.iter().all(|report| report.ok), "{reports:?}");
        assert_eq!(
            read(&root, "AGENTS.md"),
            format!("{}\n\n{}\n", mine.trim_end(), playbook())
        );
        assert_eq!(read(&root, "CLAUDE.md"), "# Claude\n");
        assert!(agent_instructions_status_for(cwd, &[AgentKind::Codex]));
    }

    /// The block an earlier version left in `AGENTS.md` becomes the Codex
    /// workspace's own: replaced where it stands, the text around it kept to
    /// the byte.
    #[test]
    fn an_old_block_in_agents_md_is_replaced_in_place() {
        let root = scratch();
        let (legacy_start, legacy_end) = LEGACY_MARKERS[0];
        fs::write(
            root.join("AGENTS.md"),
            format!("# Notes\n\n{legacy_start}\nvieux\n{legacy_end}\n\n## Fin\n"),
        )
        .unwrap();

        let reports =
            write_agent_instructions_for(root.to_string_lossy().into_owned(), &[AgentKind::Codex]);

        assert!(reports.iter().all(|report| report.ok), "{reports:?}");
        assert_eq!(
            read(&root, "AGENTS.md"),
            format!("# Notes\n\n{}\n\n## Fin\n", playbook())
        );
    }

    /// Two plain files for two agents: each holds the block once, and a second
    /// run leaves every file as the first one did.
    #[test]
    fn both_agents_get_their_file_and_a_second_run_changes_nothing() {
        let root = scratch();
        fs::write(root.join("CLAUDE.md"), "# Pour Claude\n").unwrap();
        fs::write(root.join("AGENTS.md"), "# Pour Codex\n").unwrap();
        let cwd = root.to_string_lossy().into_owned();
        let both = [AgentKind::Claude, AgentKind::Codex];

        let reports = write_agent_instructions_for(cwd.clone(), &both);
        assert_eq!(names(&reports), ["CLAUDE.md", "AGENTS.md", ".gitignore"]);
        assert!(reports.iter().all(|report| report.ok), "{reports:?}");
        let first: Vec<String> = ["CLAUDE.md", "AGENTS.md", ".gitignore"]
            .iter()
            .map(|file| read(&root, file))
            .collect();
        assert!(first[0].starts_with("# Pour Claude\n"));
        assert!(first[1].starts_with("# Pour Codex\n"));
        for content in &first[..2] {
            assert_eq!(content.matches(START).count(), 1);
            assert_eq!(content.matches(END).count(), 1);
        }
        assert!(agent_instructions_status_for(cwd.clone(), &both));

        let again = write_agent_instructions_for(cwd, &both);

        assert!(again.iter().all(|report| report.ok), "{again:?}");
        let second: Vec<String> = ["CLAUDE.md", "AGENTS.md", ".gitignore"]
            .iter()
            .map(|file| read(&root, file))
            .collect();
        assert_eq!(second, first);
    }

    /// The same agent named twice (an orchestrator and a worker on Codex)
    /// is one file to write.
    #[test]
    fn an_agent_named_twice_writes_its_file_once() {
        let root = scratch();

        let reports = write_agent_instructions_for(
            root.to_string_lossy().into_owned(),
            &[AgentKind::Codex, AgentKind::Codex],
        );

        assert_eq!(names(&reports), ["AGENTS.md", ".gitignore"]);
        assert_eq!(read(&root, "AGENTS.md").matches(START).count(), 1);
    }

    /// A `CLAUDE.md` that links to `AGENTS.md`: both agents read one file,
    /// which holds the block once, and the link stays a link.
    #[cfg(unix)]
    #[test]
    fn a_claude_md_linking_to_agents_md_gets_the_block_once() {
        let root = scratch();
        fs::write(root.join("AGENTS.md"), "# Règles communes\n").unwrap();
        std::os::unix::fs::symlink("AGENTS.md", root.join("CLAUDE.md")).unwrap();
        let cwd = root.to_string_lossy().into_owned();
        let both = [AgentKind::Claude, AgentKind::Codex];

        for _ in 0..2 {
            let reports = write_agent_instructions_for(cwd.clone(), &both);

            assert!(reports.iter().all(|report| report.ok), "{reports:?}");
            assert_eq!(names(&reports), ["AGENTS.md", "CLAUDE.md", ".gitignore"]);
            assert!(reports[1].message.contains("AGENTS.md"), "{reports:?}");
            assert_eq!(
                read(&root, "AGENTS.md"),
                format!("# Règles communes\n\n{}\n", playbook())
            );
            assert!(fs::symlink_metadata(root.join("CLAUDE.md"))
                .unwrap()
                .file_type()
                .is_symlink());
        }
        assert!(agent_instructions_status_for(cwd, &both));
    }

    /// The reverse link, `AGENTS.md` to `CLAUDE.md`, is one file too.
    #[cfg(unix)]
    #[test]
    fn an_agents_md_linking_to_claude_md_gets_the_block_once() {
        let root = scratch();
        fs::write(root.join("CLAUDE.md"), "# Règles communes\n").unwrap();
        std::os::unix::fs::symlink("CLAUDE.md", root.join("AGENTS.md")).unwrap();
        let cwd = root.to_string_lossy().into_owned();

        let reports =
            write_agent_instructions_for(cwd.clone(), &[AgentKind::Codex, AgentKind::Claude]);

        assert!(reports.iter().all(|report| report.ok), "{reports:?}");
        assert_eq!(names(&reports), ["CLAUDE.md", "AGENTS.md", ".gitignore"]);
        assert_eq!(
            read(&root, "CLAUDE.md"),
            format!("# Règles communes\n\n{}\n", playbook())
        );
        assert!(fs::symlink_metadata(root.join("AGENTS.md"))
            .unwrap()
            .file_type()
            .is_symlink());
    }

    /// A `CLAUDE.md` linking to an `AGENTS.md` that does not exist yet: the
    /// file is created through its own name first, so the link then reaches
    /// it instead of being replaced by a second copy.
    #[cfg(unix)]
    #[test]
    fn a_link_to_an_agents_md_not_written_yet_stays_a_link() {
        let root = scratch();
        std::os::unix::fs::symlink("AGENTS.md", root.join("CLAUDE.md")).unwrap();

        let reports = write_agent_instructions_for(
            root.to_string_lossy().into_owned(),
            &[AgentKind::Claude, AgentKind::Codex],
        );

        assert!(reports.iter().all(|report| report.ok), "{reports:?}");
        assert_eq!(names(&reports), ["AGENTS.md", "CLAUDE.md", ".gitignore"]);
        assert_eq!(read(&root, "AGENTS.md"), format!("{}\n", playbook()));
        assert!(fs::symlink_metadata(root.join("CLAUDE.md"))
            .unwrap()
            .file_type()
            .is_symlink());
    }
}
