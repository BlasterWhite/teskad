# Changelog

What changed in each Teskad release, newest first. The format loosely follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versions follow
[Semantic Versioning](https://semver.org/); before 1.0.0, a minor version could
change behaviour as well as add to it. The 0.x history is grouped by minor version.

Every release, with its installers, is on the
[releases page](https://github.com/BlasterWhite/teskad/releases).

## [Unreleased]

## [2.6.0] - 2026-11-27

This fork is independent and not affiliated with the original HangarAI project: the app no longer points at that project's services.

### Added
- Two themes from the Teskad brand pack, **Teskad Dark** and **Teskad Light** (Settings › Appearance). Terminals follow: their background is the pack's sunken surface, ANSI red is the failure coral, green the « done » green, and the brand magenta is ANSI magenta only.
- Space Grotesk and JetBrains Mono are bundled, with their SIL Open Font License.
- Teskad Mobile wears the brand: the pack's colour tokens (dark by default, light when the phone asks for it), Inter, Space Grotesk and JetBrains Mono served from the app itself (no CDN, it works offline over Tailscale), and one status icon, with a shape as well as a colour, for agents and tasks.

### Changed (tab frame and Board, from the Teskad brand pack)
- Board, Memory and Statistics share a 48 px tab bar (count, filters or search, MCP note, main action) and a 32 px status bar: the folder, the agents' news (a click goes to the first pane in that state), the branch and the panes. Under 1280 px the MCP note is its icon (the words in a tip), the Board's filters fold into one « Filters ▾ » button with the number that are on, and the status bar drops the folder first, then the dictation keys, the branch and the panes.
- Board: columns on `deep` with a state mark, cards with the priority, titles on three lines at most, labels, why a task is blocked (a person's block in its own inset with « Unblock »), the agent as a link to its pane, and the clock, links and comments in mono; a card under the pointer shows « Block » or « Unblock » in place of its priority; an empty column offers a dashed « Add… »; under 1180 px the columns scroll sideways. Only colours change in Teskad Light.
- A task opens as a 520 px sheet down the right of the window (the whole content under 960 px): its state badge moves it, « … » deletes it, and the same fields, block form, dependencies, files and thread are in it as before.

### Changed (Memory and Statistics, from the Teskad brand pack)
- Memory: the tab bar has the count, a 300 px search, a Recent / Oldest control and « New entry » (the MCP note is information, so it is not a button). Entries are rows of a chevron, the title, a two-line excerpt, mono tags and, on the right, who wrote it, when it was updated and created, and the folder it came from (under 1100 px that column goes under the text, on one line). Opening an entry draws its content as Markdown in a sunken inset; Edit and Delete show under the pointer and on open entries. With the keyboard on a row, → opens, ← folds, Enter edits and Delete asks to delete it.
- Memory: writing or editing an entry is a 640 px window (« New entry » / « Edit entry ») with Title, Content (mono, 180 px) and Tags fields; a title that already exists shows « An entry already has this title: saving updates it instead of creating a new one. » at once, and Save still asks once more before it replaces. « Written » became « Created » in the dates.
- Statistics: the bar is the tab bar (the title, the period, the project, then « Up to date » with a check, the measure and the refresh), the cards are `surface` with a 12 px radius, the figures are in Space Grotesk 28, the lower cards are three columns (By project; Week rhythm; By model over Heaviest sessions) that become one under 960 px, the four figures go two by two under 1100 px and the legend goes under the chart's title under 900 px. Only colours change with the theme.

### Changed (shared controls, from the Teskad brand pack)
- Buttons (primary, secondary, text, icon-only in 26, 28 and 32 px), the segmented control, the switch (36 × 20), the slider, fields (text and mono, 32 px tall), the drop-down, settings blocks and rows, role, account and BETA badges, gauges and shortcut keys are drawn from the theme's semantic palette and the pack's scales (`hangar_theme::tokens::scale`, checked against `tokens.json` by a test). Only colours change with the theme; hover is the palette's `hoverOverlay`, pressed is twice that.
- Buttons, switches and segments take the keyboard: Tab reaches them (inside dialogs too, where a dialog still opens on its first field), Space and Enter press them, and a 2 px ring in the theme's focus colour shows where the focus is. The ring only shows for the keyboard, never after a click.
- A new component sheet in the UI gallery (`--snapshot --page controls --theme teskad-dark`) shows each control in every state, in Teskad Dark and Teskad Light.

### Changed (shared floating surfaces, from the Teskad brand pack)
- Menus, popovers and dialogs are drawn from the theme's palette: menus and popovers sit on `surfaceRaised` with a `border` hairline, corners of 8 and the popover shadow; dialogs on `surface` with `borderStrong`, corners of 12, the modal shadow, and a footer on the sunken surface, over the palette's `scrim`. A menu line is 30 px tall; the line under the pointer or the keys is lit by `hoverOverlay` and keeps its text colour, group titles are 11 px `textMuted`, separators are `border`, the current choice has a tick in `accentText` left of its label (at the right end only on a line that already has an icon), a destructive line is in the failure colour. A line reached with the keyboard also shows the 2 px focus ring. Same entries, actions and shortcuts as before.
- The task sheet, the settings and the diff window share that modal frame and veil (the task sheet's buttons now sit on the same sunken footer), and the « Needs you » list sits in the shared popover. Their layout, contents and shortcuts are unchanged.
- New shared popover (`hangar_ui::popover`): the raised surface for the session's « Details » and other small panels; Escape and a press elsewhere close it, Tab stays inside.
- The component sheet of the UI gallery now shows a context menu, a popover and a dialog, in Teskad Dark and Teskad Light.

### Changed (navigator and top bar, from the Teskad brand pack)
- The navigator is 248 px wide, on the window's own background. Its top holds the Fork symbol, « teskad » in Space Grotesk and the version; the project manager's button carries an **HQ** badge and shows as selected while the HQ is open. A workspace's row is 32 px: an empty ring at rest or a turning arc while an agent works (instead of the workspace icon), its name ending in an ellipsis when it is too long, its account's badge (with the icon and colour you chose for the account), the agents' counts in their states' colours, and the Fork symbol of an orchestrated workspace, in colour for the one in front. The plan usage sits in a card, one « Plan · account » block per account, with the shared gauges. The foot is 48 px.
- The top bar shows the workspace's name in Space Grotesk, the four views as a segmented control, the account as an outlined button, and 28 px icon buttons. **The button that shows the board beside the terminals is now in the « … » menu**, with a tick while it is on; its shortcut is unchanged.
- The view switcher, the « Chat / Panes » switch, the account and layout buttons, the project manager's button and the « only what waits » filter now take the keyboard, with the focus ring.
- Settings › About shows the stacked Teskad logo, in the version for the theme's light or dark appearance.

### Changed (Settings window, from the Teskad brand pack)
- The Settings window takes up to 1040 × 840. Its 240 px list **groups the categories**: Application (General, Appearance, Shortcuts), Terminal (Terminal, Shell), Agents (Accounts, Kits), Connections (Notifications, Mobile, Voice, Discord, and Nodes while its experiment is on), with Experimental and About set apart at the foot. Up and Down follow that order. Every category and every setting is still there, and the search works as before. Mobile and Experimental have icons of their own.
- Every category is laid out the same way: a 56 px header with the category's name in Space Grotesk, then sections with a small title over a block of settings, hairlines between its rows. Appearance keeps its content as it was.
- The settings of Codex show a **BETA** badge after their name instead of « (beta) ». The projects folder's row now says « Where repositories cloned from Git go » under its section's title.
- The keyboard reaches more of it: Tab goes to the categories, the theme and font cards, the drop-downs and the sliders; Enter or Space picks, and Left and Right move a slider or change a drop-down's choice.

### Changed (agent panes and broadcast bar, from the Teskad brand pack)
- Each pane is a card of its own: corners of 12, a `border` hairline, 8 px between cards. The focused one has an accent border and a halo; a pane that waits for you keeps its state's colour on its edge. Dragging, zooming, resizing and pinning work as before.
- **A pane's header is one 40 px line**: the state's glyph, the name, the role's pill, the account when the pane has its own, a worker's task, the branch, the context bar and its percentage, **Details ▾**, and the actions, now on every pane (saved commands, changes, zoom, « … », close). A narrow pane folds them in order, and the « … » menu takes in what folds, as before.
- **The « Details » popover** holds the rest: the agent CLI (with its beta badge and whether Teskad is attached to it), the shell, the session (a press opens the sessions to resume; a button copies its ID) and the state in words; in a narrow pane also the account (with a pending or blocked move to another one) and the branch, when the line has no room for them. It closes when the focus goes to another pane. Tab walks its controls, Enter and Space press them, Escape closes it.
- The broadcast bar is 52 px tall under the terminals: the field with its icon and its shortcut as keys, **When free**, **Broadcast**, then the branch and the number of panes. Same shortcuts as before.

### Changed (workspace menu and Orchestration window, from the Teskad brand pack)
- The workspace's menu is 248 px wide (wider for a long machine or folder name). Each agent under « Go to » says its role at the end of its line, « Open with VS Code » and « Kits… » have icons of their own, and « Orchestration… » carries the Fork symbol in colour. In every menu, labels are 13 px in the secondary text colour, group titles are no longer bold, and agent state icons are 12 px. Same entries, in the same order.
- The Orchestration window takes up to 780 px, with the Fork symbol beside its title. In **Models**, each role has its own symbol, its default, an **opus / sonnet / haiku** segmented control that fills the field (a full model id typed there leaves it unchosen) and the field itself. Codex's three models moved into a **Codex models** (BETA) section that folds; it stays open while one of its names is refused. The footer's buttons are 32 px, and « Cancel » is a secondary button.
- In a short window, a dialog's body now gives up its room and scrolls, so its head and the buttons of its footer stay in sight.
- Its three drop-downs now take the keyboard: Tab reaches them, Left and Right change the choice. Same settings, saved the same way.

### Changed (chat view, from the Teskad brand pack)
- An agent's chat view has a 44 px header and a 760 px column. Tool calls sit in framed blocks that fold, each with its state's glyph; Teskad's own messages sit in a marked inset; questions and permission requests are cards framed in the accent; the queue is a dashed list over the composer; a conversation that loads shows skeleton lines; the tasks panel folds away in a narrow window. Every line and chip takes the keyboard. Same messages, same actions.

### Changed
- **A project's folder is `.teskad/`, no longer `.hangar/`** (the task board `board.json`, the phone's `uploads/`, annotations). The first time Teskad opens a project that has a `.hangar/` and no `.teskad/`, it copies the content over and leaves `.hangar/` untouched, so the official app can still use it; a marker, `.teskad/.adopted-from-hangar`, says it was done, and nothing is copied when `.teskad/` exists. The local git exclusion and the `.gitignore` line the playbook adds now cover `.teskad/`; an existing `.hangar/` line is kept.
- **Teskad updates itself again, from this fork's releases only.** The feed is `https://github.com/BlasterWhite/teskad/releases/latest/download/latest.json` and the public key compiled into the app is the fork's own (key id `70C57C3F4975A19B`): an update signed by any other key, the official HangarAI's included, is refused. The updater's paths follow the packaging (`teskad`, `Teskad.app`, the `Teskad_…` installers). The release and publish workflows and `packaging/sign.sh` read the secrets `UPDATER_SIGNING_KEY` and `UPDATER_SIGNING_KEY_PASSWORD` (made by `cargo packager signer generate`, with a password; docs/native/PACKAGING.md), and the fallback on the Tauri CLI in `sign.sh` is gone. The Codex schema watch runs on `BlasterWhite/teskad`.
- One state indicator, as the Teskad brand pack draws it, wherever a state shows (agents, panes, tasks and board columns, the activity log, the status bar, setup steps): a turning arc for work under way (1.4 s a turn, still when the system asks for less motion), a solid dot for your turn, an empty ring for idle, a ticked disc for done, a crossed disc for failed, and the diamond stays for « waiting for an answer ». Colours come from the current theme.
- **The theme engine follows the brand pack.** The 35 themes are JSON files in the pack's format (`crates/hangar-theme/themes/`, `theme.schema.json`), embedded in the program and read strictly. A theme sets its terminal colours and, for the interface, only what it needs to (an accent); the rest is derived from the terminal colours by the pack's rules (`docs/themes.md`), then held to the pack's contrast: text 4.5:1 on the window and the surface, the text on the accent 4.5:1, states 3:1, borders 1.3:1. **Teskad Dark and Teskad Light are their pack files, exactly, with no calculation**: the surfaces are the pack's, and the text on a violet button is white. One set of semantic colours (`Palette`) feeds the whole window; the finer tokens views use are computed from it, and no colour is written outside the theme crate any more. Because the interface colours now follow the pack's rules, the other 33 themes' surfaces, borders and states shift a little from before (an agent at work is the accent's arc, your turn the active dot). Theme ids, the chosen theme, « Specific to this workspace » and the window opacity are unchanged.
- **Teskad Dark is the theme of a new install.** A theme you chose stays yours; the 33 other themes are all still there.
- JetBrains Mono is the terminals' font until you choose one (first in the default stack; a stack you already saved keeps its order), and the font of branches, hashes, paths, versions and shortcuts. Space Grotesk 600 sets view and dialog titles, panel headers and the name in the title bar.
- Corner radii follow the brand pack: 6 px for controls, 8 px for cards and fields, 12 px for dialogs and the command palette.
- **Teskad has an identity and folders of its own**, so it installs beside the official app instead of over it and never shares its data: identifier `io.github.blasterwhite.teskad`, publisher BlasterWhite, the program and installers are named `teskad` / « Teskad », the home folder is `~/.teskad` (endpoint, mods, accounts, kits, kit data) and the voice key has a keychain entry of its own. The legacy `~/.iabench/endpoint.json` is no longer written. `scripts/fork/install.ps1` and `restore-official.ps1`, which installed over the official app, are gone.
- **First launch copies your data from the official app**: when Teskad's folders do not exist yet, it copies (never moves or deletes) the official app's data folder (`com.simnjs.iabench`: workspaces, memory, activity, MCP configs, statistics) and `~/.hangar` (accounts, mods, kits), once, then rewrites the copied MCP configurations (and an older `iabench` entry of your projects') to run Teskad. The official app's folders stay as they were; FORK.md says where everything is and how to go back.
- The app is called **Teskad**, and the phone app **Teskad Mobile**: the interface, the agents' briefs and playbook (`CLAUDE.md`, `AGENTS.md`, the line added to `.gitignore`), the MCP tools' descriptions, the error messages, the docs and the issue forms say so. The name is one constant (`hangar_i18n::APP_NAME`, which the strings write as `{app}`), so a future change is a one-line edit. What agents see is renamed too (next bullet); the `.hangar/` folder of a project and the crates keep their name for now.
- New logo and icons (the Teskad brand pack, `docs/teskad-brand/`): the window and packaging icons, the mark in the sidebar and the voice bar (the variant for the theme, the small one under 24 px), `assets/logo.*`, the README's lockup, and Teskad Mobile's icons. `scripts/logo.ps1`, which drew the old mark, is gone.
- The title bar shows the name and the version, with no domain and no link. With the system's title bar, the window's title is « Teskad <version> ».
- Discord Rich Presence ships with no Discord application: it publishes nothing until you enter the ID of your own in Settings › Discord. The original project's application ID, if you had entered it, is ignored. The card's button, « View on GitHub », opens this fork's repository.
- Phone notifications are signed with this fork's repository address as their contact.
- **The programs Teskad ships no longer say « hangar ».** The node daemon is `teskad-node` (release files `teskad-node-<version>-linux-x86_64` and `-aarch64`; on a server, `~/.local/share/teskad-node/`, `~/.local/state/teskad-node/` and the `teskad-node.service` unit) and the helper that shows HTML tabs and PDFs is `teskad-webview`, in every installer, beside `teskad`. A server that already runs the official app's `hangar-node` keeps it untouched: Teskad installs its own beside it, with its own folders, unit and lock, on a port the system picks.

- **The texts written into the code say Teskad too.** Settings › About shows `teskad` as the program's name; the Discord bot (its messages in English and French, its webhooks and its identity toward Discord), the phone's notifications without a title, the errors the app and the agents' tools return, the name the app gives Codex's server, the messages sent to agents, the Discord status, the git author of the commits Teskad lands, the systemd scopes of the panes, and the temporary folders and sockets it creates all say Teskad instead of Hangar. **The log file is `teskad.log`** (it was `hangar.log`), in the same log folder: an old `hangar.log` stays there, no longer written. **The « verified » badge of the kit gallery now belongs to the repository owner `BlasterWhite`**, since this fork has not read the kits of the original project: the kits of the current catalog, published by `hangar-ai`, show without the badge until they are republished under this owner. Kept on purpose, because renaming them would break something that exists: the `X-Hangar` header and the `hangar.mobile.` storage of the phone app (pairing would be lost), the `window.hangar` global and the `hangar` import of kits, the `hangar-kit-` and `hangar-model` URL schemes, `[Hangar]` and `hangar-bridge` read in old conversations, `HANGAR_*` variables, `hangar-kit.json`, and the build-cache and node repository folders that already hold data.

- **What agents see says Teskad, not « hangar ».** Claude Code and Codex now find Teskad's MCP server under the name `teskad`, so its tools are `mcp__teskad__*` (they were `mcp__hangar__*`); the Claude Code mod is `teskad-bridge`; the messages the app sends agents start with `[Teskad]` (`[Teskad] Event`, `[Teskad] From agent "Ada"`, `[Teskad] Briefing`), and the variables it sets for its panes and servers are `TESKAD_*` (`TESKAD_PANE_ID`, `TESKAD_API_URL`, `TESKAD_MCP_CONFIG`, `TESKAD_PORT_BASE`…). The playbook block in `CLAUDE.md` and `AGENTS.md` is delimited by `TESKAD:START` / `TESKAD:END`.
  - **Restart your open agent sessions** (Claude Code, Codex) to see the `mcp__teskad__*` tools: a session started before the update keeps the old names until then.
  - Your projects carry over by themselves. At launch, a `.mcp.json` holding Teskad's server under the name `hangar` gets it renamed `teskad` (the file is first saved as `.mcp.json.hangar-bak`), the playbook block already written in `CLAUDE.md` / `AGENTS.md` is replaced in place, not duplicated, and the old mod Teskad had installed in `~/.teskad/mods/hangar-bridge` is replaced by `teskad-bridge`. The official app's own `hangar` entry, and its `~/.hangar/mods`, are never touched.
  - The variables you set yourself (`TESKAD_DATA_ROOT`, `TESKAD_WEBVIEW`, `TESKAD_NODE_BINARY`, the `TESKAD_DEV_*` and test ones, `TESKAD_PORT_BASE`…) are read under their new name first and, when it is not set, under the old `HANGAR_*` one: a script or a profile of yours keeps working. The ones the app sets for its own children have no fallback.
  - Conversations written before the update still read right: the old `[Hangar]` prefix, the `hangar-bridge` mod and `mcp__hangar__*` calls are recognised in their transcripts.
- **The last visible « hangar » names are gone.** The repository is `BlasterWhite/teskad` (GitHub redirects the old addresses; every link, the update feed and the Rich Presence image now use the new one; the catalogue stays `BlasterWhite/catalog`). The branch of a worker is `teskad/<task>-<id>` (the cleanup of finished work still recognises the `hangar/…` branches of before). The comments Teskad writes on the board and the messages it queues are signed `teskad`; boards and conversations signed `hangar` still read as the app's.
  - **A kit's manifest is `teskad-kit.json`.** Teskad reads it first and falls back to `hangar-kit.json`, so every kit that exists, the catalogue's included, keeps working; the kit example and the docs (`docs/kits.md`, `docs/kit-ui.md`) write the new name and mention the fallback.
  - The folder of the delivery checks is `<local data>/teskad/c` (the old `hangar/c` is emptied at launch). The texts of Settings › Kits (the gallery hint, the folder picker) and the node's « binary not found » help name Teskad.

### Removed
- The Hangar account (« Compte Hangar »), whole: Settings › Hangar account, signing in with GitHub, the statistics sync between machines and the « All machines » view of Statistics, the account server (the `hangar-cloud` and `hangar-cloud-api` crates, its Docker image and compose file) and its documentation. HangarAI no longer calls any account server. Statistics of this PC are as they were. An upgrade deletes nothing the account left: if you signed in, the keychain entry `hangar-account` and the files `hangar-account.json`, `cloud-sync.json` and `cloud-machine-id` of the app data folder stay, unread, and what the server holds stays there (delete the account from an earlier version first if you want it gone).

## [2.5.1] - 2026-11-19

### Fixed
- Codex: the reviewer set for a workspace (Settings › Agents, or the workspace's own choice) now reviews its deliveries, with an account of its own agent; it was always Claude Code.
- Codex: accounts follow the agent everywhere. A Codex pane, reviewer or new workspace picks among Codex accounts, and the new-workspace dialog's account picker follows the orchestrator you choose.
- Codex panes: the context gauge, the restart after an MCP change and Hangar Mobile's chat of a node's pane work for Codex too; a cloned repository gets your default orchestrator agent; the Discord presence no longer names Claude Code alone.

## [2.5.0] - 2026-11-19

### Added
- Codex (beta). HangarAI runs OpenAI's Codex CLI beside Claude Code, or on its own: Hangar works with either one installed, or both.
  - Setup: both agents on the first-run sheet, installed by their official installers and signed in from a pane; either one is enough.
  - Workspaces: choose the orchestrator's agent and the workers' agent when you create one; mixed teams work (a Claude Code orchestrator with Codex workers, or the reverse). Settings › Agents sets the defaults, Codex's launch command and models.
  - Panes: a Codex pane shows whether it works, waits or needs you, like a Claude Code pane. Hangar attaches to the pane's Codex server to follow its turns and answer its approvals (Settings › Attach Hangar to Codex, on by default); without it, the pane still runs, with less to show.
  - Board, memory, kits and Hangar's tools reach Codex too, through `-c` options: your `~/.codex/config.toml` is never edited. The playbook goes to `AGENTS.md` as well as `CLAUDE.md`.
  - Accounts: Codex accounts beside Claude ones, each in a home of its own, chosen per pane, workspace or folder.
  - Images: what Codex generates shows in the chat view, on Hangar Mobile and in Discord.
  - Delegate: an agent hands a sub-task to a Codex or Claude Code child that runs with no terminal, in its folder or a worktree of its own, and gets back its answer, files, images and branch (`delegate`, `delegation_status`, `delegation_cancel`, `delegation_list`).
  - Also: sessions resume and sleep for either agent, Hangar Mobile answers Codex's questions, Discord follows Codex panes, nodes run Codex, statistics per agent with OpenAI's prices, the reviewer can be Codex.
  - Codex is in beta: report what goes wrong with the « Codex bug » issue form.

### Changed
- README, guide and site speak of both agents.

## [2.4.6] - 2026-11-19

### Fixed

- hangarai.dev also shows in the title bar with the navigator hidden and in workspace windows.

## [2.4.5] - 2026-11-19

### Added

- Pin a pane: a pinned pane keeps its edge and its size while workers and other panes open and close, and presets never close it. Pin and unpin it from the pane's menu or the command palette; a pin marks it in its header.
- Window opacity, for every theme (Linux): Settings › Appearance › Window opacity lets the desktop show through the window, the panes and the terminals, while each theme keeps its own colours. Blur is the desktop's: KDE does it, GNOME with an extension such as Blur my Shell.

### Changed

- HangarAI: the new name, with hangarai.dev always shown in the title bar (a click opens the site). The program, its folders and its settings keep their names, so nothing moves on an upgrade.
- Contact: contact@hangarai.dev, for questions about your data, conduct reports and security reports by email.

### Fixed

- Discord: a message written in a channel while its agent was busy could lose its place in the queue, and its ⚡ then did nothing.

## [2.4.4] - 2026-11-19

### Changed

- Hangar's address is now hangarai.dev: the website at https://hangarai.dev and the optional Hangar account's server at https://account.hangarai.dev. The old addresses (hangar.simnjs.fr, hangar-account.simnjs.fr) keep working and lead to the new ones.

### Fixed

- Typing in a text field while a pane was selected went to the pane: the "Send the same instruction to all panes" bar, the terminal's search, renaming in the navigator and adding a card on the board lost the keyboard to the terminal, often after the first letter. A field that takes the keyboard now has it until you click a pane again, everywhere in the app.

## [2.4.3] - 2026-11-18

### Added

- Coloured badges for Claude accounts (Settings › Accounts, off by default): a colour and an icon per account, shown on the toolbar's account button and its menu, and wherever an account other than the default is named. Contributed by SimonB44470.
- A Claude account per navigator folder: workspaces without an account of their own run on their folder's, and follow it when moved; an explicit choice, "Default" included, still wins. A "+" on a folder's heading creates a workspace in it. Contributed by SimonB44470.
- With two Claude accounts or more, the navigator's plan block shows each account's own limits and next reset. Contributed by SimonB44470.

### Changed

- Choosing another Claude account no longer asks to restart running agents: each one moves by itself, on the same conversation, as soon as Hangar knows it has nothing to lose (its turn over, no draft in its prompt, no command of its own still running); meanwhile its header shows where it is going and why it waits. "Ask before moving running agents" in Settings › Accounts brings back the question. Contributed by SimonB44470.

## [2.4.2] - 2026-11-18

### Changed

- A quieter orchestrator on big projects. A worker waiting on a job of its own (a build, a benchmark, a CI run it watches) is no longer reported idle every ten minutes. A worker that is really idle is reported once, then again only after it did something or half an hour later. News that can wait (plain comments, idle workers) comes as one digest every five minutes at most, while deliveries, questions, a worker waiting for you or a crash still arrive at once.

### Fixed

- Enabling a kit works again. With its activation sheet open over a workspace's Kits dialog or over Settings, the click on "Accept and enable" went to the dialog underneath: the sheet closed and nothing was enabled, and in Settings the click could flip another switch. Only the dialog in front now takes the mouse and the keyboard.

## [2.4.1] - 2026-11-17

### Fixed

- The Hangar account reaches its server at `hangar-account.simnjs.fr`: 2.4.0 pointed at an address that does not exist, so signing in failed.

## [2.4.0] - 2026-11-17

### Added

- **An optional Hangar account.** Settings › Hangar account (« Compte Hangar ») lets you sign in with GitHub, in your system browser, to link your machines. It is off by default: Hangar works as before without one, never calls the account server until you sign in, and still sends no telemetry. Signed in, the page shows who you are, your machines (name, system, last seen, « This PC »; rename one, remove another with a confirmation), Sign out, and Delete my account (type the word to confirm; it deletes everything on the server, then signs this PC out). A « Sync statistics » switch, off until you turn it on, sends totals per machine, day, model and project name (tokens, active time and cost; never a path, a prompt or any code) at once, every 15 minutes, when you open Statistics, and on « Sync now », without ever waiting on the network in the window. A day's totals already sent are never replaced by lower ones, so Claude Code cleaning old transcripts does not shrink your history. Errors read as plain sentences (server unreachable, signed out from another machine). The token stays in your system's keychain. The details are on the [privacy page](https://hangar.simnjs.fr/privacy.html) and in docs/native/cloud.md.
- **Statistics of all your machines.** Once synced, the Statistics page has a « This PC / All machines » switch. This PC is the page as it was; All machines shows the merged totals of every machine of the account by machine, day, project and model, for the period chosen, with when the account was last synced.
- **The account server, ready to run with Docker.** `ghcr.io/hangar-ai/hangar-cloud`, built, checked and published by CI, and `crates/hangar-cloud/compose.yaml` to run it behind a reverse proxy or through a Cloudflare Tunnel (`docker compose up -d`), with its health check, a read-only root and no capability. For whoever hosts it; docs/native/cloud.md, « Deploy », walks through it.
- **Hangar on NixOS.** A Nix flake at the root of the repository: `nix run github:hangar-ai/hangar` builds and starts Hangar with its webview helper and Hangar Mobile, its package goes in a NixOS or Home Manager config, and `nix develop` opens a shell with everything a build needs. On Nix, the MCP configs and the login item name the `hangar-ai` on your `PATH`, not the store path, so they survive upgrades and garbage collections. Nodes are not in the package yet. Contributed by Kisakay.
  - On NixOS, the **Set up Claude Code** sheet offers Claude Code from nixpkgs instead of Anthropic's installer, whose `claude` does not start there without nix-ld: the line for the configuration (`environment.systemPackages = [ pkgs.claude-code ];`) and a `nix profile install` command, to copy, with what claude-code being unfree asks for, then **Check again**. The installer stays behind **Use Anthropic's installer anyway**.

### Changed

- **Discord: a thread per worker, shared files, and ⚡ to send now.** A worker's thread opens by itself when it starts (its « Ouvrir le fil » button stays only if Discord refuses it), a file an agent shares (`phone_share`) is posted in its thread, or the workspace's channel for the orchestrator (one too big for Discord becomes a line saying it is on the phone), and a message waiting for the agent's turn to end gets a ⚡ that you click to stop the turn and send it at once, as « Send now » does in the chat view.
- **The viewers open any file on the computer.** Images, PDFs, Markdown, code, video, web pages and 3D models shown by an agent (`preview_show`, `model_show`) or by you no longer have to be inside the workspace's folders, and a workspace whose folder is not a real path shows files too. What the viewer pages may load stays limited to what was shown (and, for a 3D model, the textures and buffers beside it): a page cannot read the rest of the disk.

## [2.3.0] - 2026-11-13

### Added

- **A Hangar with no window, on a server.** A node runs workspaces of its own now, with no desktop connected: their state is kept on the node, their panes start there with Claude Code, and their agents' requests (workers, deliveries, the board) are answered there, so they keep working with your computer off. Make one with `hangar-node workspace add <folder>` on the server (or from Discord); `hangar-node workspaces` lists them, and a connected desktop lists them in its navigator among the others, opens them with their terminals live, and changes them there (a rename, a pane opened, closed or restarted, a preset, a deletion: each is sent to the node, which stays their owner; while the node is offline they can be looked at, not changed). Merge on one of their deliveries and Stop workers are carried out by the node. A workspace made from the desktop with « Runs on » set to a node is the node's own from the start, and one made before, which still stops with your computer, has « Move to {machine} » in its menu in the navigator: its agents are restarted on the machine on their sessions. If the machine's answer is lost on the way, the workspace waits as it is and the machine's own list settles it at the next read; and a workspace a machine came back without after a restart is kept in the list, marked, with « Run on {machine} again » in its menu. `hangar-node service install` makes the node a systemd service that starts on its own.
- **The same repository on several machines (nodes, experimental).** Hangar knows a repository by its `origin`, however the address is spelled. When one is open on this computer and on a node, each of its rows says so in the navigator (a faint mark on a local row, « also on… » in the tooltip of a machine's mark), and its menu starts with « Also on », which goes to the other checkouts. « Open on another machine… » in that menu opens the new-workspace dialog ready to clone it where it is not yet. In that dialog « Runs on » is there for a clone too, and lists every machine (one that is offline or switched off is greyed, with its state; from four machines on, they are a menu): the node clones with its own keys, into a folder you pick there (`projects` in its home by default), shows its progress, and runs the workspace itself. If it cannot clone, git's own words are shown.
- **Hangar on Discord (experimental).** Control your Hangar from a Discord server of your own, as from the phone: a channel per workspace (those on your nodes included), its orchestrator's turns as cards and its answers under its name, each agent with its own face; what you write in the channel goes to the orchestrator, waits in its queue while it works (⏳, then ✅ when it is taken), and an edit or a delete changes what waits; permissions, questions and plans as cards answered with buttons; workers announced with « Ouvrir le fil », which opens a thread with their work since they started, then live, where you can talk to them; deliveries with Land and Send back; a pinned dashboard; `/stop`, `/model`, `/effort`, `/plan`, `/status` and `/workspace`. Only the account you pair with a code is heard. Turn it on in Settings › Experimental, then give the bot's token and your server's ID in Settings › Discord; it runs while Hangar runs, window closed or not. The same bot serves the workspaces your nodes run by themselves: the desktop hands it to them, and they keep answering in their own channels with your computer off. A node with no desktop can have a bot of its own (`hangar-node discord setup --guild <id>`).
- **Answers reach Claude Code as answers.** A permission allowed or denied, or a question answered, in the chat view or on the phone now goes to Claude Code as the decision itself, through the hangar-bridge mod, as an Agent SDK host's would: no keys typed into the dialog, no screen read back. The dialog stays up in the terminal meanwhile and either place can answer; a pane on a node, or a Claude Code without the mod, is still answered with keys.
- **Stop in the chat view.** While the orchestrator works, a button next to Send stops its turn, as Escape does in the terminal.
- **Model, effort and plan mode from the chat view's input.** Next to the input, the orchestrator's model, its effort level and Plan: a click shows the choices, and the choice reaches its Claude Code as `/model`, `/effort` or `/plan`, at once when it is idle, at the end of its turn otherwise. The hangar-bridge mod reports what the session runs with as it changes, so the input shows it. As in the terminal, `/model` and `/effort` also become the defaults of new sessions.
- **Files Claude names are links, in the chat view.** A file of the workspace that Claude names in a message (`src/search/index.ts:42` in code, or a link to a relative path) opens in the workspace's preview, beside the conversation, with a click. Only names that are files there become links; code blocks are left alone.
- **A queue you can see and change, in the chat view.** What you send while the orchestrator works now waits in its queue instead of being typed into the turn, and shows at the top of the input: each message can be edited (back into the input) or removed while it waits, and Send now stops the turn so the first one goes at once. Claude takes them when its turn ends. Through the hangar-bridge mod; without it, messages are typed as before.
- **What a delivery brought, in the chat view.** A delivery that landed or waits to land shows how many files it changed and their +/− on its line; a click lists the files, and a file opens the Changes window on exactly what that delivery brought (its commits, not the working tree). The orchestrator is told the same range (`git diff <base>..<merge>`), so it can look at the changes too.
- **Plans in the chat view.** When the orchestrator presents a plan (plan mode), the chat view shows it whole, formatted, in a card with Approve the plan and Keep planning; a field says what to change in it, and Claude plans again with those words, the turn going on. Through the hangar-bridge mod.

### Changed

- **Several machines, at a glance (nodes, experimental).** A machine goes by one icon, its kind's (server, cloud, laptop, desktop, mini PC, tower), chosen when it is added and changed from its row. In the navigator a workspace on a node wears its machine's icon, the filter finds it by the machine's name or narrows the list to one machine, and a line at the foot counts the machines and, in the warning colour, those that are offline (those switched off are in its tooltip); the palette and the new-workspace dialog name machines the same way. A node now connects as Hangar starts and reconnects for as long as it is down (2 s doubling to a minute, at once when you come back to the window or press Retry now; an attempt that hangs is given up after a deadline). A first connection that takes a while, an install for instance, reads as connecting, not offline. A link that drops says nothing for 5 seconds; then its rows step back with an unplugged mark, and its panes say why with Retry now and, after 20 seconds, Switch off, which hides the machine's workspaces, in Needs-you too, until it is switched on again in Settings › Nodes. That page lists the machines as rows, this computer first, each with a switch and a menu (Retry now, Rename, Remove from this computer). A failed connection reads as one short sentence (« Offline · retrying in 8 s: unreachable over SSH (Connection timed out) »), with what ssh said under it.
- **The chat view reads like a chat.** Claude's words sit open on the page, your messages in rounded bubbles on the right; its calls are quiet lines with an icon for what they do (a command, an edit, a read, a search, a task), folded in runs as before. While it works, a line at the bottom says so and for how long, and the header gives the model and how full its context is. The input is one rounded composer; while a question or a permission waits, its card takes the composer's place, with only the question being asked (no recap of the answers given before: Back goes to them).
- **The orchestrator's exchanges with its workers, short and in your language.** What Hangar tells the orchestrator about the workspace shows as one line per thing that happened, with an icon and a colour for what it means (Ada delivered “Search index”: merged, task done; Bo started on “Filters”; Cy has been waiting for you for 4 min) instead of the block written for the model. A worker's message to the orchestrator and the orchestrator's message to a worker each show as one line with their first words (Bo: “…”, To Bo: “…”), the whole message a click away, and a message is never folded into a run of calls. The state of the work handed to a conversation that starts is one line too.

### Fixed

- **A long question could not be answered from the chat view or the phone.** Claude Code now draws a bar down the left of a question's lines; when a question wrapped over several lines, Hangar no longer recognised it on the screen and refused to type the answer ("The question is not on the PC's screen…"), so the first, short questions went through and the longer ones that followed did not. The answer now goes through the mod with no screen involved, the screen check ignores the bar where keys are still typed, and a terminal the chat view hides starts at 160×50 instead of 80×24, so a long question is drawn whole.

## [2.2.1] - 2026-11-08

### Added

- **A delivery's progress is on its worker's pane.** A chip says where it stands (queued, merging, checking with the time it has taken, in a batch of N, reviewing, landing) and then how it ended; the chat view's tasks panel shows the same on the task.
- **Deliveries are checked together.** When several wait, Hangar merges them on the branch and runs the check once for all of them; if it fails, it splits them to find the one that breaks, and only that one is handed back. The branch stays exactly as safe: nothing lands on a tree the check did not pass. With five deliveries waiting and a one-minute check: 73 s instead of 315 s when all pass. The batch size is in the orchestration settings (4 by default).
- **Claude Code is set up with Hangar.** When it is missing, a first screen installs it in one click with Anthropic's official installer (its output shown as it runs), then opens Claude Code to sign in; a workspace cannot be created without it, unless you choose to use Hangar with shells only (Settings › General).

### Changed

- Releases ship Windows and Linux. macOS is no longer built for download: 2.2.0 is the last macOS release, and the README says how to build Hangar on a Mac.

## [2.2.0] - 2026-11-07

### Added

- **A new workspace starts with its orchestrator.** Creating a workspace now gives one Claude Code pane that is the orchestrator, ready to plan and hand work to workers; the layouts of several panes and shells are under More options. In a folder without git it still orchestrates, its workers sharing the folder (git init gives them worktrees).
- **Chat view (experimental).** Turn it on in Settings › Experiments, then switch any workspace between its panes and a chat with its orchestrator. Claude Code keeps running in the panes behind it; a side panel lists the board's tasks with the progress their agents report, and kits, previews and the 3D view open beside the chat as they do in the grid.
- Agents can report how far along a task is (`board_report_progress`, over MCP and HTTP): a percentage and a short note, shown with the task. Workers are asked to report at each real step.
- **Nodes (experimental).** A headless Hangar, `hangar-node`, runs on a Linux machine you reach over SSH: add it in Settings › Nodes and Hangar installs it, starts it and keeps a tunnel to it. A workspace can then live on the node: its terminals, Claude Code and board run there, and the window shows them like a local workspace's, catching up on what happened while it was away. Its memory, transcripts, chat view and orchestration work too: the orchestrator and its workers run on the node, in worktrees there, while the connected window drives them. A local workspace can also send a worker to a node (`worker_spawn` with `node`), for a heavy build or a GPU box: the worker runs there and its branch comes back to be checked, reviewed and landed here. Design in `docs/native/nodes.md`.
- **Pane scopes (experimental, Linux).** Each pane can run in a systemd scope of its own, so when memory runs out the system stops the pane that uses it (a build, say) rather than Hangar and every agent with it.

### Changed

- Released builds use link-time optimization across the whole program, rarely used code is built for size and unused image codecs are left out: Hangar's executable is 44 % smaller (119 → 67 MB, hangar-node 17 → 10 MB) and parses terminal output a few percent faster. TIFF and ICO images no longer show in kit tabs.
- Previews are lighter: a large image is shrunk when it is decoded and read in full only when zoomed (an 8000×6000 PNG: +554 → +143 MB), a Markdown page no longer colours its whole source unseen (38 % → 4 % CPU) and parses off the interface thread, memory freed by a closed preview goes back to the system, and the web view helper stops a minute after its last window.
- At rest, the panes no longer wake the window several times a second: about once every 20 s (43 wake-ups a minute → 3 with a shell pane), and Hangar's CPU at rest falls by about 40 %.
- Drawing costs half as much or less: a busy terminal 0.97 → 0.48 ms a frame, 16 printing panes 2.7 → 1.3 ms, scrolling back through history 4 → 1 ms (a scroll plans only the rows that came into sight), selection 3.9 → 0.9 ms, the statistics view 1.8 → 0.95 ms. The terminals' lock no longer makes the window wait behind panes that print.
- Output from panes that are not on screen no longer makes Hangar rebuild and redraw its window 60 times a second: with eight agents printing, Hangar's CPU falls from 8.1 % to 2.9 % in the chat view and from 6.4 % to 5.8 % in the panes view.
- On relaunch, the next wave of resumed sessions starts as soon as the previous one is under way (its agent printed and went quiet) instead of a fixed 2 s later: eight sessions in the chat view are all back in 3.2 s instead of 9.1 s. Agents that keep printing still hold the next wave as long as before.
- The login shell's PATH is read from the first instant of the launch, beside the window's start: with a slow shell profile (0.35 s), the first prompt comes about 90 ms sooner.
- In the chat view, the hidden terminals start their shells at once instead of waiting for a size they never get: a workspace in the chat view is ready in 0.38 s instead of 0.66 s.
- On Linux with a Vulkan GPU, Hangar no longer also sets up an OpenGL context it never draws with: about 27 MB less memory at rest and a faster start (0.40 → 0.36 s with 4 shells). A `WGPU_BACKEND` you set yourself is kept, and the one Hangar sets does not reach your panes' shells.
- Terminal output is parsed faster (+6 % on a listing, +8 % on an agent's interface): the scan for title and link sequences skips plain text in one search.

### Fixed

- Chat view: a question with several parts is asked one part at a time, as Claude Code asks it (1/3, Back, Next, then your answers and Send), its card scrolls instead of overflowing, and a missing answer or a refusal is said beside the buttons instead of Send doing nothing. Runs of tool calls fold into one quiet line ("3 commands, 2 tasks created · 1 failed") that opens on a click.
- Windows: a pane whose folder is longer than 258 characters starts its shell (in the folder's short 8.3 name) instead of failing with "The directory name is invalid".
- Windows: Hangar starts on a Windows without the Visual C++ Redistributable. The installer now puts the runtime's three DLLs beside the executables; before, they exited at once (STATUS_DLL_NOT_FOUND) on a fresh install.
- A Hangar started in a folder inside a worktree no longer removes other workers' worktrees: the sweep of orphan worktrees at start only removes the worktree of a task that is done, and never one a live process works in; landing a delivery and stopping a worker keep a folder another pane or process uses.
- Kits, Windows: removing a kit's clone folder retries for a moment when a git process that was just stopped still holds it, instead of failing.
- Tests remove the temporary folders they create, even when they fail; a full test run used to leave thousands behind in `/tmp`.

## [2.1.0] - 2026-11-06

### Changed

- On relaunch, the Claude panes on screen resume their conversations first; the others follow two at a time, or at once when they come into view, so a workspace full of big sessions no longer starts every Claude Code at the same moment.
- A change to the app's state no longer copies and compares the whole state on every action, the state snapshot is rebuilt only when it differs instead of on each keystroke, and saves are serialised off the UI thread.
- Less work while agents run: the session hunt lists transcript file names and dates only instead of reading the head of every transcript every few seconds, skips panes the mod already tracks, and settles to a 5 s cadence for resumed panes; the session picker reads at most 256 KB of each transcript and caches its labels.
- The context gauge and the phone's chat read transcripts without a lock shared by every pane, stream a transcript instead of loading it whole, and find it under `CLAUDE_CONFIG_DIR` and account profiles too.
- Kits ask again for your consent when a script, hook or MCP server file they run has changed since you allowed them, not only when their command lines change. An update is checked in full (manifest, permissions) before it replaces the installed kit, and a kit installed from a tag notices when that tag later points elsewhere.
- Each pane gets an API token of its own, kept across a restart or a hibernation and revoked when the pane is closed; a pane's token fixes who the caller is, so an agent can no longer act as another pane or as you through the local API. `~/.hangar/endpoint.json` now advertises a token for outside callers instead of Hangar's own.
- The terminal's scrollback is capped so a pane cannot hold tens of MB of history; box-drawing characters are cached instead of being redrawn from scratch every frame, and an open search no longer rescans the whole history on every chunk of output.
- The command palette filters once per keystroke instead of on every redraw; a native kit tab compiles off the UI thread; pictures, highlighted code blocks and the stats page are computed in the background.

### Security

- `~/.hangar/endpoint.json`, which holds the API token, is readable by its owner only (0600, folder 0700); trusting a folder keeps `~/.claude.json`'s permissions.
- The updater keeps its download in a private folder and checks the signature again right before installing, so the file cannot be swapped in between.
- Phone actions (send, allow, answer, land) carry a key and run once even when the phone retries; allowing a permission from the phone applies only to the prompt the phone showed.
- Voice models are pinned and checked against their SHA-256 before use.
- A kit tab cannot make Hangar hold unbounded logs, images or messages, or block the window with a long chain of updates.

### Fixed

- Hangar no longer replaces a `CLAUDE.md`, `.gitignore`, `board.json`, `.mcp.json` or `~/.claude/settings.json` it cannot read or parse (not UTF-8, unreadable, invalid JSON): it leaves the file alone and says so; a corrupt board is kept aside as `board.json.corrupt.<time>`. Every state, board and config file is written atomically and flushed to disk.
- One pane whose program stops reading its input no longer freezes typing in every other pane; panes get a short grace to exit on quit, so Claude Code can finish writing its transcript; on Windows, what a pane started no longer outlives Hangar.
- A `postCreate` hook that leaves a process running no longer blocks a worker's start; Hangar's git commands never wait for a password and stop after 2 minutes on the network; the delivery reads a task's `owns` exactly as the board does; check folders of deleted workspaces are removed.
- Copied history in resumed or forked sessions is counted once in the stats.
- A 3D model loader that crashes, or a snapshot of a model in a sleeping workspace, no longer leaves the 3D view waiting forever; a zoomed SVG is no longer rasterised at full zoom size.
- The terminal redraws the rows it scrolls to after a search, and clearing it during a synchronized update works.
- A field focused under a dialog loses the keyboard; menus taller than the window scroll to their last entry; the palette no longer jumps to the row under a resting pointer.
- The update banner no longer stays on "restarting" after a failed install; "Download again" repairs a damaged voice model; a voice transcription that cannot start no longer leaves the button stuck; Discord presence no longer lags one update behind.
- Deleting a workspace removes its delivery check worktree, and a last save that fails at quit is written to `hangar.log`.
- A worker is never given a block of ports one of which another process already listens on.
- Clicking a notification or a Needs-you item closes the settings cleanly, and a voice model download keeps showing its progress when you switch settings categories.
- A dialog opened over another one, such as a kit's activation dialog over the settings, is drawn opaque and on top: the settings' scrolled content no longer shows through it.
- A relaunch no longer leaves an agent pane on a bare prompt: its command is typed on an emptied line (keys that reached the focused pane first are no longer glued to its front) once the shell has printed its prompt and gone quiet (never before the launch delay, at most 3 s after it), is typed again if the shell runs nothing after it, and a launch lost twice is logged. A saved conversation whose transcript is not found once is started fresh but kept, and forgotten only after a second miss in a row.
- A pane started at launch before Hangar had finished looking for the machine's shells no longer runs the default shell instead of the one it was given: its shell starts once detection has answered.

## [2.0.0] - 2026-11-05

Hangar, rewritten as a native app. It keeps your data, your workspaces and the features
around the terminals, and needs a fraction of the memory and processor it did.

- **Native, in Rust.** The whole app is Rust now, drawn with iced, and runs no web view: 1.x
  was a web page in a Tauri window. The terminals are Hangar's own, on alacritty_terminal.
  - What only a browser engine can show, a kit's HTML tab, a PDF, a web page, a video,
    opens in a separate helper program, `hangar-webview`, in a window of its own.
  - Measured on the same Linux machine, 1.12.1 against 2.0.0:
    - starting cold: 0.96 s, now 0.57 s;
    - memory at rest: 238 MiB, now 166 MiB;
    - 16 terminals printing non-stop for 30 s: 8.1 GiB of memory, now 249 MiB, and 4.4
      processor cores, now 2.4;
    - the echo of text sent to a pane while 15 others print: none within a minute, now
      2.5 ms.
- **Claude Code only.** Hangar runs Claude Code and plain shells. Codex, Gemini CLI and
  OpenCode are gone, with their launch, session resume, statistics, prices and settings.
  - A pane saved with one of them opens as a shell in the same folder, says so once in its
    bar, and is saved as a shell.
  - The MCP panel writes Claude Code's `.mcp.json` only, and the agents' playbook goes to
    `CLAUDE.md` only. What earlier versions wrote for Codex, Gemini CLI, Cursor, VS Code or
    in `AGENTS.md` is left as it is.
  - The statistics read Claude Code's transcripts; the "By agent" card is gone.
- **Native kit tabs.** A kit's tab can be Rust compiled to WebAssembly. It describes its
  page as widgets that Hangar draws, in your theme and your language, with no browser
  engine, and runs in a sandbox that reaches only what its manifest declares.
  - The **kit SDK** (`hangar-kit-sdk`) has the `Kit` trait, view builders, the calls a tab
    makes, and a harness that runs and tests a tab without Hangar. The contract is in
    [docs/native/kits-native.md](docs/native/kits-native.md).
  - A native tab needs `"schema": 2`. A kit that must still open on 1.x keeps its HTML page
    and adds a `native` module beside it: 1.x opens the page, 2.0 the module.
  - HTML tabs keep working, with the same `window.hangar` API and the same permissions.
  - A compiled tab is kept on disk, so it opens at once after a restart.
- **Same data, same updates.**
  - 2.0 reads the same folders and the same `state.json` as 1.x: workspaces, panes, boards,
    memory, settings, theme, language, shortcuts, kits and Claude accounts are all there. It
    writes the file back in the shape 1.x reads, fields it does not know included.
  - A 1.x install updates to 2.0 through the same update feed, signed with the same key. The
    executable is still `hangar-ai`, so the MCP configurations agents already have keep
    working.
  - Two things 1.x kept in its web view's storage are not carried over: the kit tabs left
    open, and each Claude account's cached usage. Open the tabs again once; the usage comes
    back at its next refresh.
  - A `state.json` that cannot be read is moved aside as it is, never replaced by an empty
    one.
- **Hangar Mobile is unchanged**: the same phone app, the same pairing, the same
  notifications.
- **Hangar draws its own title bar, on Linux and Windows.** Its windows open without the
  system's title bar: the toolbar and the top of the navigator are the bar, in your theme.
  Press on it to move the window, press twice to maximise or restore it, right-press for the
  window menu; the edges resize the window.
  - On Linux, the buttons are the ones your desktop shows, on its side (GNOME's button
    layout), and the corners are rounded as GNOME's own windows are.
  - On Windows, minimize, maximize/restore and close sit on the right as in Windows 11, the
    close button red when you point at it. A window still snaps to the screen's edges, and
    Windows 11 rounds the corners and draws the shadow.
  - **Appearance › Use the system's title bar** brings the system's back, on Linux and
    Windows. macOS keeps its own.
- **A Linux icon shaped like the others.** The dock and the app grid now show Hangar on a
  rounded tile with room around it, the shape GNOME and KDE give their own app icons,
  instead of a square from edge to edge; the packages carry it from 16 to 512 px and as an
  SVG.
- **Images paste into Claude Code.** With an image on the clipboard (a screenshot), Ctrl+V
  in a pane now reaches Claude Code, which attaches the image as it does in any other
  terminal; before, Hangar took the key to paste text and, finding none, did nothing.
- **The arrows and Escape stay in the pane.** The sidebar's filter answered Down and Escape
  wherever they were typed, so the first Down arrow in Claude Code's choices took the
  keyboard to the workspace list; the status bar's line took Alt+Enter the same way, the
  board's new-card field Escape. Those fields now hear keys only while they have the
  keyboard. A click in a pane also always takes the keyboard back from the sidebar.
- **Agents' panes are laid out, not stacked.** A pane an agent opens or closes (through
  Hangar's MCP server, or the orchestrator's workers) no longer halves the pane beside it,
  which ended in a row of slivers: the workspace is laid out again, with the orchestrator
  on the left in the biggest pane and the others in an even grid beside it. `pane_create`
  no longer takes a `direction`; `near` says which pane the new one goes after.
- **Previews and kit tabs are panes of the grid.** What the side preview panel showed, a
  native kit tab, an image, code or text, Markdown, a 3D model, opens as a pane beside the
  terminals, with a terminal pane's header: drag it onto another pane, resize it with the
  dividers, zoom it, close it, reach it with `Ctrl`+`1`…`9`, which count every pane on
  screen. Build your own layout of terminals and kit tabs.
  - A new one opens beside the focused pane, split along its longer side as the + button
    splits; one already open takes the focus where it is. What an agent opens or closes
    lays the workspace out again, as its panes do.
  - A PDF, a web page, a video, a sound or an HTML kit tab still shows in a window of the
    helper: its pane holds a card with the button that brings the window forward, and
    closing the pane closes the window.
  - A workspace holds 16 panes, terminals and previews together. A preset sets the number
    of terminals and keeps every preview.
  - Kit tabs come back after a restart as the panes they were, where you left them. Files
    do not come back, as before.
  - The panel is gone with its tab strip, its drawer, its expanded mode, its toolbar button
    and its shortcut (`Mod+Alt+P`). 1.x still reads the file: it keeps the panes as
    `viewPanes`, a field it does not know, and lays out its terminals without them.
- **A shorter toolbar.** The four layouts are one button with a menu, and the activity
  capsule in the middle is gone: the pane's own header and the inspector say what it is
  doing.
- **No limit on the workers.** The orchestrator decides how many workers run at once and
  how many times a task is tried: **Workers at once** and **Attempts per task** are gone
  from the orchestration settings, and Hangar no longer hands a task back to you after its
  second worker or its third failed delivery. 1.x, reading the same file, goes back to its
  defaults, 3 workers and 2 attempts.
- **Faster orchestration.** The delivery check was the slowest step, and most of it was
  rebuilding the same code.
  - **The check has a build folder of its own.** Every check of a workspace runs in one
    folder with a short path (no more paths too long to build in on Windows), put back on
    each merge's files and cleaned of what the last check left, and builds into a target
    folder only checks use. A fresh checkout for every check made the build start over;
    in the same folder, only what changed is rebuilt.
  - **A shared build cache for the workers, if you want it.** It is off by default:
    workers building different branches in one shared folder can mix their builds, so turn
    it on in the orchestration settings only if they rarely build at once. The settings
    show its size and can empty it.
  - **Workers build what they touch.** They iterate with targeted builds and tests, and
    the full check runs once per delivery.
  - **A check that could not fail is skipped.** A delivery that changes only
    documentation (Markdown, text, `docs/`, licence and changelog files, issue templates)
    runs no check, and neither does one whose merged files are exactly a tree that already
    passed the same check. The review still runs, and the delivery says why.
  - **The orchestrator sees what the check costs.** Its briefing gives the last check's
    time and verdict ("last check 12 min, passed").
  - **Leftovers are removed.** At start, Hangar removes the delivery folders earlier
    versions left in the temporary folder and has git forget the worktrees whose folder is
    gone.
- **The orchestrator has a free hand.** Your instructions come first: its role says so at
  the top, and everything else in it is a default, not a wall.
  - It does small things itself (the changelog, docs, configuration, a fix to the check),
    commits, and can merge a finished branch into the target branch without waiting for the
    delivery queue. It decides when a delivery check and a review are worth their minutes.
    It pushes, tags or releases when you ask, never on its own.
  - A worktree per task is its choice now: `worker_spawn` with `worktree: false` starts a
    worker in the workspace's own folder, beside the others, with no branch and no
    delivery.
- **No worktree left behind.** After a delivery lands, what Windows kept of its folder (a
  `node_modules`, read-only files, paths too long) goes too, and the worktrees nobody uses
  any more, merged, clean and with no pane, are removed with their branch when Hangar
  starts or a workspace opens.
- **Not there yet.**
  - Windows and macOS are built and tested by CI, but nobody has used 2.0 on them yet. Linux
    is where it was used.
  - No screen-reader support (iced has no accessibility tree yet), and buttons do not take
    the keyboard focus: `F6` moves between regions, and the command palette reaches every
    command.
  - Text cannot be selected with the pointer in the diff, in native kit tabs and in most
    panels; a Copy button stands in. Terminals, code and Markdown previews select as before.
  - PDFs, web pages, video, audio and HTML kit tabs open in a window of their own rather
    than in their pane. On GNOME under Wayland, one that an agent opens waits for your click
    on **Open in a window**.
  - On Linux, video and audio previews may not play.
  - Dropping a file on a pane does not work under Wayland.
  - The 3D preview does not read Draco, meshopt or KTX2 compressed glTF files, nor USDZ, and
    plays no FBX animation.
  - A window last left on a second screen reopens centred on the first.

## [1.12.1] - 2026-11-03

- **Terminals no longer lag on Linux.** On Linux the webview is WebKitGTK, where drawing a
  terminal with WebGL was slow and, with some NVIDIA drivers, broken: typing, spaces and
  deletions came late, and a dead key followed by a letter could leave the accent stuck
  (`'` then `e` on a US International keyboard). Terminals are now drawn without WebGL
  there. Windows and macOS are unchanged.
  - To get WebGL back on a Linux machine where it is fast, run
    `localStorage.setItem("hangar.webgl", "1")` in the developer tools; `"0"` turns it off
    anywhere.
- **An Experimental page in the settings.** A place for features that are not finished: off
  until you turn them on, and free to change or disappear. The page stays hidden until a
  version ships one, so nothing shows up yet.

## [1.12.0] - 2026-11-02

- **Tell it where.** Ask any Claude to create a workspace from a repository and say where
  it goes, on any drive: "put it on my second SSD". The clone lands in that folder, and
  missing folders are made. An empty folder already there is used, and put back empty if
  the clone fails. Your **Projects folder** setting is not changed.
  - A repository that is not yours still asks for your click first, and the card now
    shows where it would go. For your own, the agent tells you the folder.
  - Still refused, with a message saying what to do next: a relative path, a path with
    `..`, the operating system's folders and Hangar's own data. A folder that already
    holds something else is refused untouched.
- **Blocked tasks.** A task that waits on something outside the board (a package release,
  a decision, a date) can be blocked with a reason, from the Board or by an agent with
  `board_block_task`. It goes back to To do holding no worker and no files, and agents are
  no longer handed it. The tasks that depend on it keep waiting.
  - Give it a day to look again: the day after, it unblocks itself.
  - Or choose **Only a person can unblock it**: no agent can lift that block, and the
    day only reminds you.
  - A worker that blocks its own task is told to stop.
  - Unblock from the Board, or with `board_unblock_task`. The Board can filter by blocked
    state, and the activity log records each block and unblock.
- **`board_next_task` skips tasks that need you.** Agents are no longer handed them.

## [1.11.1] - 2026-11-02

- Hangar's repository moved to hangar-ai/hangar, with the kits. Old links redirect, and updates keep coming.

## [1.11.0] - 2026-11-02

- **A workspace from a Git repository.** Ask any Claude "create a workspace for
  `owner/repo`", or pick **From a Git repository** when you create a workspace. Hangar
  clones the repository and starts an orchestrator in it. The orchestrator reads the
  project, installs its dependencies, sets the check command, runs the tests once, sums
  up, and waits for your objective. It changes no code.
  - If the repository is not yours or one of your organizations', an agent's request
    first asks you for a click: **Clone** or **Refuse**. Claude Code's own question about
    trusting the folder stays, and you answer it yourself.
  - If it is yours, Hangar records that trust for that folder alone, in Claude Code's
    config.
  - A new **Projects folder** setting chooses where the clones go.
  - Each answer to the click is written in the activity log.
- **Open with VS Code** when you right-click a workspace.
- **Kit tabs use less memory.** Tabs restored at startup load only when you first show
  them.

## [1.10.0] - 2026-11-01

- **The git branch in each pane's header.** You see at a glance which branch a pane's
  folder is on.
- **Worktrees, easier to keep tidy.**
  - Remove the worktrees whose branch is already merged in one go, from the worktree
    panel.
  - **Bring changes back** moves a worktree's uncommitted work into the main checkout.
    If a file was modified on both sides, nothing is moved.
  - A workspace can run a command after a worktree is created and another before it is
    removed, workers' worktrees included. Only you set them. They run on this machine,
    as you, and a command that fails or takes more than five minutes is reported.
- **Fixed: Board, shortcuts and Hangar Mobile.**
  - Moving Board cards by drag and drop now works on Windows.
  - The fullscreen shortcut works again.
  - Sending a file from the phone in Hangar Mobile finds its workspace's folder.
- **Fixed: sessions and panes.**
  - Compressed Codex sessions are now read by the session picker and the context gauge.
  - A pane pinned as orchestrator gets Hangar's tools: it restarts on the same
    conversation.
  - A pane moved to another window while it launches still starts its agent.
  - Launch notes stay visible when Claude Code starts, in a bar under the pane's header
    that you can dismiss.
  - After a delivery on Windows, worktree folders are no longer left on disk.
- **Fixed: the global memory.**
  - It warns you when it nears its limit, and never deletes anything by itself.
  - `memory_list` takes a limit.
  - Creating an entry with a title already taken asks before replacing it.
- **Fixed: agents' tools.** `board_list_tasks` and the other tools that filter ignore an
  empty filter instead of matching nothing.

## [1.9.1] - 2026-11-01

### Fixed

- The review of a delivery now runs on the Claude account of its workspace, as its panes do, not on the machine's default login. When that account hits its usage limit, the delivery says so plainly, and it no longer counts as a failed delivery.

## [1.9.0] - 2026-11-01

- **A kit Library.** **Settings → Kits → Library** lists the kits of the
  [hangar-ai](https://github.com/hangar-ai) organization, read from a static file: no
  server, nothing of yours sent.
  - Each kit is pinned to a tag the catalog names. An update reaches you only when the
    catalog names a new tag.
  - The organization's kits carry a **Verified** badge. Any other kit says that Hangar
    did not review it. Installing and accepting work the same for both.
- **Public kits**, ready to install from the Library.
  - **Guardrails** refuses dangerous commands and the reading of secrets.
  - **Editor**: an agent that needs a secret opens the `.env` for you to fill in, and
    never sees the value.
  - **Checklist**: you work through the manual checks an agent lists, and the result
    goes back to it.
  - **Annotate**: circle a problem on a capture, and the agent gets the image and your
    notes.
  - **CI status** follows a GitHub Actions run after a push.
  - **Tests** shows the last test run per project and agent.
  - **GitHub** shows pull requests and issues, and hands any of them to an agent.
  - **Dependencies** lists vulnerable and outdated packages, and hands them to an agent.
  - **Services** shows listening ports and Docker containers, tied to the worker they
    belong to. It stops nothing: an agent is asked to.
  - **TODO** groups the `TODO`, `FIXME`, `HACK` and `XXX` comments of the project.
  - **Database** is a read-only look at a SQLite file of the workspace.
- **Kits that do more.**
  - An agent, or a hook of a kit, can open one of the kit's tabs.
  - A kit can declare commands its tabs run. You accept them when you enable the kit.
  - A kit can be turned on for **all workspaces** from Settings → Kits, accepted once.
- **You see everything a kit starts.** Before enabling it, you see all that its plugin
  runs by itself: hooks, commands, MCP servers.
  - A kit from GitHub that runs more than hooks is asked again, once, after this update.
    Until you accept it, it stays out of new panes of the workspace.
- **Hangar UI, for kit makers.** A shared look for kits' tabs, following your theme and
  language, described in [docs/kit-ui.md](docs/kit-ui.md). The rest is in
  [docs/kits.md](docs/kits.md).
  - A kit's SVG icon now takes the theme's colour, so it stays visible on a dark theme.

## [1.8.0] - 2026-10-31

- **Kits can do more, and show more before you accept them.**
  - A kit's tabs can keep their own data, in a folder of the kit outside your projects.
    A kit can never reach another kit's folder.
  - A kit's tab can talk to a local server on a port it declares, such as a game editor's
    bridge. You accept each port when you enable the kit. Hangar makes the call: the
    tab's page still has no network of its own.
  - Before you enable a kit, you now see the hooks of its Claude Code plugin, exactly as
    they will run, since Claude Code runs them by itself. A kit from GitHub whose hooks
    change in an update stays out of your agents until you accept it again. A kit linked
    from a folder, which you are writing yourself, is not asked again.
  - The details for kit makers are in [docs/kits.md](docs/kits.md).

## [1.7.0] - 2026-10-31

- **Claude accounts.** Use several Claude subscriptions from one Hangar — say a work Max
  and a personal one — and give each workspace its own.
  - **Settings → Claude accounts** adds an account: name it, and a pane opens for you to
    log in through your browser. Rename, remove, and pick the default account there.
  - Each workspace can have its own account, from its right-click menu in the sidebar
    or when you create it. A workspace that is not on the default shows a small badge
    with the account's name.
  - The toolbar's account menu shows the account of the workspace on screen and each
    account's 5-hour and weekly usage. One click moves this workspace to another
    account, or every workspace that follows the default. Hangar then offers to
    restart the workspace's agents now, with their conversations kept, or at their
    next launch.
  - A single pane can run on another account too, from its menu.
  - Every account has the same setup: conversations, settings, skills, plugins and MCP
    servers come from your main Claude Code configuration. Only the login differs.
    Change the setup from a pane on the main account: the other accounts copy it again
    at each launch.
  - Hangar never switches account by itself, not even when a limit is reached.
  - On macOS, each account keeps its own login in the Keychain.

## [1.6.0] - 2026-10-31

- **Kits.** A kit brings a domain — a game engine, a framework, a studio's own tools —
  into the workspaces you link it to, without Hangar knowing anything about that
  domain. A kit can bring a Claude Code plugin (skills, commands, agents and MCP
  servers), tabs of its own, and setup commands.
  - **Settings → Kits** adds a kit from a folder, linked and read live while you make
    it, or from a GitHub link (a whole repository or one of its folders), checks for
    updates, updates and removes kits.
  - Enable a kit on a workspace from its right-click menu in the sidebar (**Kits…**) or
    when you create it. Before anything is enabled you see what the kit brings, the
    permissions its tabs ask for in plain words, and its setup commands exactly as
    they will run.
  - Setup commands run only when you click **Run**, in a visible pane, once per
    machine. A command that changes in an update is asked again.
  - Every Claude Code pane of the workspace, workers included, gets the kit's skills
    and MCP servers, also when the workspace picks its own MCP servers. Nothing is
    written into your project, and other workspaces are untouched.
  - A kit's tabs open in the side panel, from the toolbar's **Kits** menu or the command
    palette. They run isolated, with no network, and talk to Hangar only through a
    small API, with the permissions you accepted: see the workspace and its panes,
    write to its Claude Code agents, read its text files, open a file in the preview,
    raise a notification.
  - How to make a kit: [docs/kits.md](docs/kits.md).

## [1.5.0] - 2026-10-29

- **See the app an agent is building, on the phone.** In Hangar Mobile, a project whose
  worker runs a dev server shows **See the app**: it opens full screen, live, with hot
  reload, and a button to open it in Safari. Workers' servers are found by themselves
  on their own ports. Under **Settings → Mobile** you can add ports of your own for
  each workspace, such as your own `pnpm dev`.
  - Each app gets an HTTPS port of its own on your Tailscale address (8443 to 8448).
    Only your paired phones get through, and the dev server never sees the phone's
    session.
  - The first time an app is opened, Hangar adds that port to Tailscale Serve, which
    keeps it after a restart.
- **Merge from the phone.** A delivery ready to merge opens on the phone with the files
  it changes, their diff, the check's result and the review's verdict. **Merge** lands
  it, or says what stands in the way. **Send back** gives the worker an instruction to
  fix before it delivers again.

## [1.4.0] - 2026-10-29

- **Stats on the phone.** Hangar Mobile has a **Stats** tab with your agents' usage. Your
  plan's limits come first, when Claude Code reports them: how much of the 5-hour and
  weekly windows is used, and when each resets. Below them are the figures of the
  desktop's statistics page, computed the same way: a period (7, 30 or 90 days, or all),
  tokens or the API-equivalent cost, a chart per day, and rankings by project, agent and
  model.

## [1.3.0] - 2026-10-29

- **Preview any file.** The 3D panel becomes **Preview**: one tab per open file, whatever
  its kind. Agents show you a render, a screenshot, a recording, a report or a page with
  the new `preview_show` and `preview_close` MCP tools.
  - Images zoom and pan, and video and audio play with loop and speed.
  - Code and text get syntax highlighting, Markdown is rendered with a source view, and
    PDFs open in the panel or in your default app.
  - HTML pages run in an isolated sandbox that reaches neither Hangar nor your other files.
    Their scripts and styles must be inline.
  - 3D models work as before.
- **Shares open in place on the phone.** In Hangar Mobile, a file an agent shares opens
  right in **Shared**, whatever its kind: video and audio play, text and code are
  highlighted, Markdown renders, a PDF opens in the phone's viewer, and an HTML page
  renders isolated. Images open in a zoomable view.
- **Wake a sleeping project from the phone.** An asleep project has a **Wake up** button.
  A message, an answer or a permission sent to one of its agents now waits until that
  agent's session has resumed, instead of being lost while it started.
- **The phone layout holds on iPhone.** The app no longer shifts down, under the Dynamic
  Island, and pushes its bottom bar and composer off screen after the keyboard or a
  return to the app.
- Agents' instructions now mention `preview_show` and `phone_share`.

## [1.2.0] - 2026-10-29

- **A real chat on the phone.** In Hangar Mobile, a Claude Code pane (and the project
  manager) now opens as a conversation instead of a copy of its terminal: your messages,
  Claude's answers with their formatting, and between them one line per action ("Edited
  src/a.ts", "Ran pnpm test") that opens its detail. Older messages load as you scroll up.
  Codex, Gemini CLI and shells keep the terminal view, and a **Terminal** button still
  shows any pane's raw screen.
- **Claude's questions as buttons.** When Claude asks you something, the phone shows each
  question with its options as large buttons, several choices when it allows them,
  "Other" with a text field, and Next / Send across several questions. Your answer
  reaches the session on the PC as if you had typed it there.
- **Permissions say what they are for.** A permission shows the tool and what it will do
  (the command, the file) with Allow and Deny.

## [1.1.0] - 2026-10-28

- **Hangar Mobile.** Follow and steer your projects from your phone while Hangar keeps
  running on your PC. It is a web app that Hangar itself serves and that you add to the
  phone's Home Screen: no app store, no account, no server of ours in between. Set it up
  in about a minute under **Settings → Mobile** (see the README).
  - **For you** lists what waits for you in every project: an agent asking for a
    permission, a question for you, a delivery ready to land.
  - Read any pane's screen as it updates, answer an agent, allow or deny a permission,
    press `Esc` or `Enter`, attach a photo or a file, and talk to the project manager.
  - Agents can share images and files with your phone through the new `phone_share`
    MCP tool; images show in **Shares**.
  - Push notifications reach the phone even with the app closed, sent by the PC itself.
  - **Since your last visit** shows what happened while you were away, kept across
    restarts for 30 days. For a few days away, two options: keep the PC from sleeping
    while Hangar Mobile is on, and start Hangar when you log in.
  - Security: it listens on `127.0.0.1` only and is reached through Tailscale Serve over
    HTTPS; each phone pairs with a one-time code and can be revoked. Off by default.

## [1.0.3] - 2026-10-28

- **Project manager.** One agent to talk to instead of every project's agents. Open it
  from the top of the sidebar or the command palette: Hangar creates an HQ workspace
  with its own folder and one Claude Code agent (Opus by default) that follows every
  workspace, with or without an orchestrator.
  - Ask it where your projects stand: agents, boards, what waits for you, deliveries
    ready to land, each plan's status.
  - Ask it to pass an instruction on: it is queued for that project's orchestrator,
    and a sleeping workspace wakes up.
  - It speaks up by itself only for what waits for you: an agent waiting for an answer
    or a permission, a task marked needs-you, a delivery ready to land, a board whose
    tasks are all done.
  - It keeps a tracker of what you want from each project, handed back after every
    compaction. It never writes code, never acts on its own, and the HQ never sleeps.
- **New MCP tools for it:** `project_list`, `project_status` and `project_send`, which
  only the project manager can call.

## [1.0.2] - 2026-10-28

- **Orchestrator mode without git.** A workspace that is not a git repository, such as
  a UEFN island, can now run workers. They start in the workspace's own folder, side by
  side, with no worktree and no branch. When a worker delivers, nothing is merged,
  checked or reviewed: its task is done and its pane closes. Since nothing keeps two
  workers' changes apart there, a task that does not list the files it owns never runs
  beside another one. The orchestrator no longer asks you to make the folder a
  repository.
- **A pane that just started is no longer shown as working.** A Claude Code pane stayed
  "working" until its first turn, in the navigator, in `pane_list` and for sleep, so a
  workspace of fresh sessions never went to sleep. It now shows idle once its session
  is ready.

## [1.0.1] - 2026-10-27

- **Put a workspace to sleep by hand.** Right-click a workspace in the navigator, or run
  **Put to sleep** from the command palette. It is the same sleep Hangar applies on its
  own, without the idle delay, and with the same guards: the entry is greyed out, with
  the reason, while an agent works or waits for you, a shell runs a process, an agent
  could not resume its session, or a worker or a delivery is running. The workspace on
  screen hands over to the one you used last.
- **A delivery waiting for you can be redone.** When a worker fixes its branch after its
  delivery passed and waits to be merged, `delivery_submit` withdraws the waiting one and
  runs the whole chain again. **Merge** never lands a version the worker changed since it
  was tested, and the reason a merge was refused stays on screen.
- **No personal path on the launch line.** Orchestrator and worker panes read their MCP
  configuration from `HANGAR_MCP_CONFIG` instead of typing its full path, so a screen
  share or a recording no longer shows your user folder.

## [1.0.0] - 2026-10-27

The first stable release. It adds orchestrator mode on top of what the 0.12 to 0.15
releases brought the day before: the Pro interface, the 3D preview, usage
statistics, workspace sleep and per-workspace MCP servers.

- **Orchestrator mode.** Pin one Claude Code pane per workspace as its orchestrator
  (Opus by default). It plans on the board, with owned files, labels and dependencies
  per task, and delegates the work instead of writing code.
- **Workers.** `worker_spawn` claims a task, checks out a git worktree on a branch named
  after it, and opens a Claude Code worker there (Sonnet by default) with a brief, its
  own port range (`HANGAR_PORT_BASE`, `HANGAR_PORT_COUNT`) and its model.
  `worker_list` and `worker_stop` complete the set. Two tasks that own the same files
  cannot be in progress at once.
- **Delivery queue.** `delivery_submit` merges a worker's branch onto the target's
  current head in a temporary worktree, runs the workspace's check command if one is
  set, has a fresh `claude -p` session with read-only tools review it, then lands it
  locally. Hangar never pushes. A failure goes back to the worker with the conflicting files
  or the failing line; repeated failures are handed to you.
- **Event-driven orchestrator.** Board moves, comments, workers that wait for you, stop
  or exit, and delivery outcomes reach the orchestrator as batched messages, so it
  never polls.
  Orchestrators and workers get their role and the workspace's state when a session
  starts, resumes or is compacted.
- **"Needs you" inbox.** Agents waiting for an answer or a permission, tasks handed back
  with a question, and deliveries waiting to land, across every workspace.
- Per-workspace orchestration settings: maximum workers, attempts, automatic landing (on
  by default),
  check command, models, target branch and permission mode.
- A prompt queued by an agent or by Hangar is labelled with its sender, and the
  inspector shows who queued it.
- A workspace stays awake while its workers run or one of its deliveries lands.
- PowerShell panes start without the PowerShell banner.
- The workspace board (`.hangar/`) is kept out of `git status` through the repository's
  local `.git/info/exclude`.

## [0.15] - 2026-10-26

- Each workspace has its own 3D preview; a model shown in a workspace you are not
  looking at waits there, and the agent is told whether you can see it.
- Sidebar folders read as headings, and each workspace row sums up its agents.
- Changes agents make to the layout show as news rather than as a warning.
- The workspace path left the toolbar.
- A Dockerfile to self-host the website.

## [0.14] - 2026-10-26

- **Usage statistics**: a view built from Claude Code and Codex transcripts on disk,
  with tokens, API-equivalent cost, rankings by project, agent and model, and an
  activity heatmap. Prices are API list prices, not a plan's price.
- **Workspace sleep**: an idle workspace has its processes ended after a delay you set,
  and its agents resume their sessions when it is opened again. Agents acting on a
  sleeping workspace over MCP wake it.
- **MCP servers per workspace**: a panel lists the servers Claude Code would load in the
  workspace and lets you pick which ones its Claude Code panes start with.
- hangar-bridge no longer holds an agent on a file another pane just edited: the edit
  goes through and the model is told who edited it before.
- Closing a pane can end every process its shell started.

## [0.13] - 2026-10-26

- **3D preview over MCP**: `model_show`, `model_snapshot` and `model_close` let an agent
  open a model in a panel beside the terminals and render it to a PNG to check its work.
- Reads glTF/GLB, OBJ with MTL, STL, PLY, FBX and USDZ, only from the workspace's
  folders and its git worktrees, and reloads when the file changes.
- Captures of the preview can be saved where you choose.

## [0.12] - 2026-10-26

- **The Pro interface**: a three-column window with a navigator, the work area and an
  inspector, on a new default theme, Graphite (33 themes in all).
- The navigator shows workspaces as a tree with their agents, plan meters and a filter
  for the ones waiting on you.
- The inspector shows the focused pane's session, context and cost, board task, prompt
  queue, changed files and recent activity.
- A command palette (`Mod+Shift+P`) finds panes, commands and workspaces, in English or
  French.
- Keyboard access throughout: F6 walks the regions, pane boundaries resize from the
  keyboard, and focus returns to where a dialog was opened from.
- Contrast held to 4.5:1 for secondary text, light themes included; one icon set
  across the window.
- Side panels without room become drawers; a layout preset asks before closing busy
  panes.

## [0.11] - 2026-10-07

0.11.0 (2026-10-06), 0.11.1 (2026-10-07).

- Every pane has a first name unique across all workspaces, and Claude Code sessions
  run as "Ava (Shop)" so neighbouring agents know which project they work on.
- A pane resumes only a conversation that was actually written, and starts fresh
  otherwise, instead of failing with "No conversation found".
- A Tidy button lays the open panes out as an even grid without restarting anything.

## [0.10] - 2026-10-05

- **Pane tools over MCP**: `workspace_list`, `pane_list`, `pane_create`, `pane_close`,
  `pane_restart`, `pane_send`, `pane_read`, `pane_focus`, which a setting can turn off.
- A workspace can move to a window of its own, on another screen, with its agents
  still running; `workspace_window` does the same over MCP.
- Agents sign the board with their pane's name.
- `HANGAR_ENDPOINT_FILE` lets a development build run beside an installed Hangar.

## [0.9] - 2026-10-03

- **hangar-bridge**, a Claude Code mod loaded into Hangar's panes: it reports turns,
  permission dialogs, context, plan usage and edits, and submits prompts held until
  the agent is free.
- Agent state (working, waiting for you, done) in each pane header and per workspace
  in the sidebar; plan usage at the bottom of the sidebar.
- Reset every agent of a workspace to a new conversation.
- Zoom a pane, search a terminal, drop a file or image onto a pane to paste its path.
- WebGL rendering in the terminals.
- Board cards with assignee, blockers and filters; the board as a side panel; a diff
  view of what changed in a pane's folder or worktree.
- The Groq key moved to the system keychain.

## [0.8] - 2026-10-03

- A gauge of how full each agent's context window is.
- Saved commands, run from any pane.
- Folders to organise workspaces in the sidebar.

## [0.7] - 2026-08-11

- **Git worktrees**: create, open and remove linked worktrees from a panel.
- **Global memory over MCP**: `memory_*` tools give every agent a store of facts shared
  across workspaces, with a view to browse it.
- Clicking a desktop notification brings Hangar back on the right pane (Windows).
- The website and its browser demo joined the repository.

## [0.6] - 2026-08-10

0.6.0 and 0.6.1, both 2026-08-10.

- Dictation transcribes in seconds rather than minutes, on a smaller local Whisper
  checkpoint.
- whisper.cpp is built for AVX2 instead of the build machine's CPU, so releases run on
  CPUs without AVX-512.
- The custom Windows installer's buttons work.
- MCP registrations pointing at a missing executable are reported as stale and can be
  repaired; internal identifiers moved from `iabench` to `hangar` with fallbacks.

## [0.5] - 2026-08-09

0.5.0 (2026-08-08), 0.5.1 and 0.5.2 (2026-08-09).

- **Dictation**: hold a key, speak, and the text is typed into the focused pane. It
  runs locally on whisper.cpp, with Groq's hosted Whisper as an optional fallback.
- Dictation has its own section in the shortcut settings.
- One Windows installer, the per-user `.exe`, which needs no administrator rights.

## [0.4] - 2026-08-07

0.4.0 and 0.4.1, both 2026-08-07.

- macOS (universal `.dmg`) and Linux (`.AppImage`, `.deb`, `.rpm`) builds.
- Open a VS Code `.code-workspace` file; its extra folders are passed to Claude Code.
- A launch command of your own per agent, for wrappers and aliases.
- A new pane starts with the agent of the pane it was split from.
- Closing a pane no longer freezes the window.
- Shift+Enter inserts a newline in agent panes and still submits in a plain shell.

## [0.3] - 2026-08-05

- Renamed **Hangar.AI**, after shipping as the misspelt Hangar.IA in 0.2.0; updating
  replaces the old install and keeps its data.
- A logo, for the app icon, the installer and the sidebar.
- Twenty more themes ported from popular VS Code palettes, 32 in total.
- Settings rebuilt around a searchable list, with rebindable shortcuts.
- Discord Rich Presence, off by default.

## [0.2] - 2026-08-05

- Renamed from IaBench to Hangar.IA.
- The pane grid rewritten so a layout change no longer restarts every terminal: split,
  close, drag to move or resize, up to 16 panes, each with a first name.
- A desktop notification when a pane falls quiet after real work.
- Switching workspace no longer kills their shells.
- A custom Windows installer.

## [0.1] - 2026-08-05

0.1.0 and 0.1.1, released as IaBench, Windows only.

- Workspaces with saved pane layouts and a per-pane agent or shell.
- Real terminals in a resizable split, with 12 themes.
- Session resume for Claude Code and Codex.
- A shared task board per workspace, served to agents over MCP by the same binary run
  with `--mcp`, with atomic claims.
- One-click MCP registration for Claude Code, Codex, Gemini CLI, Cursor and VS Code.
- Automatic updates from signed GitHub releases.
- An interface in English and French (0.1.1).

[2.6.0]: https://github.com/BlasterWhite/teskad/releases/tag/v2.6.0
[2.5.1]: https://github.com/simnJS/hangar-ai/releases/tag/v2.5.1
[2.5.0]: https://github.com/simnJS/hangar-ai/releases/tag/v2.5.0
[2.4.6]: https://github.com/simnJS/hangar-ai/releases/tag/v2.4.6
[2.4.5]: https://github.com/simnJS/hangar-ai/releases/tag/v2.4.5
[2.4.4]: https://github.com/simnJS/hangar-ai/releases/tag/v2.4.4
[2.4.3]: https://github.com/simnJS/hangar-ai/releases/tag/v2.4.3
[2.4.2]: https://github.com/simnJS/hangar-ai/releases/tag/v2.4.2
[2.4.1]: https://github.com/simnJS/hangar-ai/releases/tag/v2.4.1
[2.4.0]: https://github.com/simnJS/hangar-ai/releases/tag/v2.4.0
[2.3.0]: https://github.com/simnJS/hangar-ai/releases/tag/v2.3.0
[2.2.1]: https://github.com/simnJS/hangar-ai/releases/tag/v2.2.1
[2.2.0]: https://github.com/simnJS/hangar-ai/releases/tag/v2.2.0
[2.1.0]: https://github.com/simnJS/hangar-ai/releases/tag/v2.1.0
[2.0.0]: https://github.com/simnJS/hangar-ai/releases/tag/v2.0.0
[1.0.1]: https://github.com/simnJS/hangar-ai/releases/tag/v1.0.1
[1.0.0]: https://github.com/simnJS/hangar-ai/releases/tag/v1.0.0
[0.15]: https://github.com/simnJS/hangar-ai/releases/tag/v0.15.0
[0.14]: https://github.com/simnJS/hangar-ai/releases/tag/v0.14.0
[0.13]: https://github.com/simnJS/hangar-ai/releases/tag/v0.13.0
[0.12]: https://github.com/simnJS/hangar-ai/releases/tag/v0.12.0
[0.11]: https://github.com/simnJS/hangar-ai/releases/tag/v0.11.1
[0.10]: https://github.com/simnJS/hangar-ai/releases/tag/v0.10.0
[0.9]: https://github.com/simnJS/hangar-ai/releases/tag/v0.9.0
[0.8]: https://github.com/simnJS/hangar-ai/releases/tag/v0.8.0
[0.7]: https://github.com/simnJS/hangar-ai/releases/tag/v0.7.0
[0.6]: https://github.com/simnJS/hangar-ai/releases/tag/v0.6.1
[0.5]: https://github.com/simnJS/hangar-ai/releases/tag/v0.5.2
[0.4]: https://github.com/simnJS/hangar-ai/releases/tag/v0.4.1
[0.3]: https://github.com/simnJS/hangar-ai/releases/tag/v0.3.0
[0.2]: https://github.com/simnJS/hangar-ai/releases/tag/v0.2.0
[0.1]: https://github.com/simnJS/hangar-ai/releases/tag/v0.1.1
