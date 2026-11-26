# Install and getting started

[← Teskad](../../README.md) · [Guide](README.md)

Download the file for your system from the
**[latest release](https://github.com/BlasterWhite/hangar/releases/latest)**, under
**Assets**. You only download once: after that, the app offers each new version and
installs it in one click. A Teskad 1.x install offers 2.0 the same way.

| System | File | Notes |
|---|---|---|
| Windows (x64) | `Teskad_<version>_x64-setup.exe` | Installs for your user only, without administrator rights. HTML kit tabs and PDFs use WebView2, which ships with Windows 11; on Windows 10 the installer adds it if it is missing. |
| macOS (Apple Silicon and Intel) | `Teskad_<version>_universal.dmg` | One universal build for both, macOS 10.15 or later. |
| Linux (x86_64) | `.AppImage`, `.deb` or `.rpm` | The AppImage runs without installing (`chmod +x` it first) on distributions with glibc 2.35 or later (2022 on). HTML kit tabs and PDFs need WebKitGTK 4.1: apt and dnf install it with the `.deb` and `.rpm`; with the AppImage, install `libwebkit2gtk-4.1-0` (Debian, Ubuntu) or `webkit2gtk4.1` (Fedora) if your system lacks it. |

> [!NOTE]
> **Windows: SmartScreen.** The installer is not code-signed, so on first run SmartScreen
> shows "Windows protected your PC". Click **More info**, then **Run anyway**.
>
> **macOS: Gatekeeper.** The macOS build is signed and notarised, so it opens after
> macOS's usual first-launch confirmation. If a release's notes ever say the macOS build is
> unsigned (or you built it yourself), macOS blocks the first launch: right-click the app
> and choose **Open**. On macOS 15 and later, go to **System Settings → Privacy &
> Security** and click **Open Anyway**.

Teskad runs coding agents in its panes: [Claude Code](https://code.claude.com/docs/en/overview),
[Codex](https://learn.chatgpt.com/docs/codex/cli) (beta), or both; either one is enough. If
neither is installed, Teskad offers to install them the first time it opens (see below).

**Coming from HangarAI.** Teskad installs beside the official HangarAI and shares no folder
with it: its data is in `~/.teskad` and in the folder of its own identifier
(`io.github.blasterwhite.teskad`). The first time it starts it **copies** your workspaces, boards,
memory, settings, kits and Claude and Codex accounts from HangarAI (which is left as it was),
and points your projects' `.mcp.json` at Teskad. Sign in again if a copied account asks for it.
The kit tabs you had left open and each account's cached usage lived in the old web view, and
are not carried over. To go back, start HangarAI: see [FORK.md](../../FORK.md#going-back-to-the-official-app)
for what is where and how to remove Teskad. The [changelog](../../CHANGELOG.md) lists what is
not done yet: mainly, there is no screen reader support, and Windows and macOS have been
tested by CI but not yet used by a person.

## NixOS and Nix

The release files are made for distributions that keep libraries in `/usr/lib`; on NixOS,
use the flake at the root of the repository instead. It builds Teskad, its webview helper
(HTML kit tabs and PDFs, with WebKitGTK) and Teskad Mobile from source:

```bash
nix run github:BlasterWhite/hangar           # try it without installing
```

That builds `main`, the development version. The releases after 2.3.0 carry the flake too:
pin one with its tag, `github:BlasterWhite/hangar/v<version>`, here and in the input below.

To install it, add the flake to your system's `flake.nix` and its package to your packages:

```nix
# inputs
hangar.url = "github:BlasterWhite/hangar";
hangar.inputs.nixpkgs.follows = "nixpkgs";

# environment.systemPackages (or home.packages with Home Manager)
inputs.hangar.packages.${pkgs.stdenv.hostPlatform.system}.default
```

Teskad then appears in your launcher. It does not update itself on Nix: `nix flake update
hangar` and rebuild for a new version. The MCP configs and the login item it writes name the
`teskad` on your `PATH` (`/run/current-system/sw/bin`, your profile's `bin`), not the
store path, so they keep working across upgrades and garbage collections. Claude Code
itself comes from nixpkgs (`claude-code`) or its own installer; Codex's official installer
drops a binary that starts on NixOS as it is.

On NixOS, the setup sheet's Claude Code card points to nixpkgs instead of Anthropic's installer,
whose `claude` does not start there without nix-ld. It shows, to copy, the line for your
configuration (`environment.systemPackages = [ pkgs.claude-code ];`, or `home.packages` with
Home Manager) and a command that installs it in your profile right away
(`NIXPKGS_ALLOW_UNFREE=1 nix profile install --impure nixpkgs#claude-code`): claude-code is
unfree in nixpkgs, so a configuration needs `nixpkgs.config.allowUnfree = true`. **Check
again** then looks for `claude`. **Use Anthropic's installer anyway** is still there, for a
system with nix-ld.

Nodes (the experiment that runs workspaces on another Linux machine) are not available from
the flake yet: the package does not carry the `hangar-node` binaries a node is set up with.

## Getting started

0. **Your agents.** When Teskad finds neither Claude Code nor Codex on this machine, it
   opens a **Set up your coding agents** sheet at start, with a card per agent (Codex
   marked beta); it stands in the way only until one of them is installed.
   **Install Claude Code** runs Anthropic's official installer, exactly as
   [Claude Code's docs](https://code.claude.com/docs/en/setup) give it —
   `curl -fsSL https://claude.ai/install.sh | bash` on macOS and Linux,
   `irm https://claude.ai/install.ps1 | iex` in PowerShell on Windows. **Install Codex**
   runs OpenAI's — `curl -fsSL https://chatgpt.com/codex/install.sh | sh`, or
   `irm https://chatgpt.com/codex/install.ps1 | iex` ([Codex (beta)](codex.md)). Either
   installs for your user only, with no administrator password, and the sheet shows what
   it prints as it runs. If it fails, the sheet says why, shows the installer's output, and
   links to the install guide. Once an agent is found, **Sign in** opens it in a pane,
   where it asks for your account the first time (Claude, or ChatGPT or an OpenAI API key
   for Codex); the sheet notices when you are signed in. **Use Teskad with shells only**
   sets the agents aside: Teskad stops asking, and new workspaces start with a shell. Turn
   it back off in **Settings → General → Shells only, without Claude Code**.
1. **Create a workspace** with `Ctrl`/`Cmd`+`Shift`+`N` and pick a project folder. The
   workspace starts with one agent pane, its orchestrator: tell it what you want. *Starts
   with* offers Claude Code or Codex, Claude Code by default when both are installed (or the
   one in **Settings → General → Default agents**). Outside a git repository its workers
   share the folder (`git init` gives each one a worktree). **More options** has the other
   setups: the workers' agent, several panes, a layout, shells and agents of your choice.
   With no agent installed, the dialog says so and the workspace starts with a shell.
2. **Connect your agents to Teskad.** Codex panes get Teskad's MCP server at every launch,
   with nothing to set up. For Claude Code, open the MCP panel (`Ctrl`/`Cmd`+`Alt`+`M`;
   `Alt` is `Option` on a Mac) and click **Configure**. The panel can also write a short
   playbook into the project's `CLAUDE.md`, telling agents to claim tasks before they
   start. Restart the agents so they load the server.
3. **Give them work.** Add tasks on the board (`Ctrl`/`Cmd`+`Alt`+`B`), or ask an agent to
   split a goal into tasks. Other agents pick tasks up with `board_next_task`.
4. **Orchestration.** A new workspace has its orchestrator already; in another one, run
   **New orchestrator** from the command palette (`Ctrl`/`Cmd`+`Shift`+`P`), or pin an
   existing Claude Code or Codex pane from its **…** menu. Then describe the goal to it.
   **Orchestration…** in the same menu opens the workspace's settings: models,
   permissions, check command and automatic merging.

The command palette lists every command. Shortcuts can be changed in
**Settings → Shortcuts**, and the defaults use `Ctrl` on Windows and Linux and `Cmd` on
macOS.
