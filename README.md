# Claude Switch

Run your **work** and **personal** Claude side by side — **Claude Code** *and* **Claude Desktop** —
fully isolated, at the same time, with sign-in that always lands in the right account.

```
  ╭────────────────────────────────────────╮
  │ ◆ claude-switch v0.1.0                 │
  │   Work & personal Claude, side by side │
  ╰────────────────────────────────────────╯

  PROFILE        CLAUDE CODE                           DESKTOP     SIGN-IN BROWSER
  ● Work ★       ✔ samrand@company.com · Company     ● running   Edge · Work <samrand@company.com>
  ● Personal     ✔ samrand@gmail.com                 ● running   Chrome · private sandbox profile

? What would you like to do? ›
❯ ▶  Claude Code
  ⧉  Claude Code in a new tab / window
  ▶  Claude Desktop
  ▶  Claude Desktop — all profiles
  ⚿  Sign in
  ☰  Profiles
  ⚙  Settings & theme
  ✚  Doctor
```

One Rust binary (`cswitch`) for Windows, macOS and Linux, with arrow-key menus for everything —
no flags to memorize.

## Why

Running two Claude accounts on one machine breaks in three places:

| Problem | What actually happens | How Claude Switch fixes it |
| --- | --- | --- |
| Shared state | Claude Code keeps config, history *and credentials* in `~/.claude`; Claude Desktop keeps its session in one app-data folder. Only one account fits. | Every profile gets its own `CLAUDE_CONFIG_DIR` and its own Desktop `--user-data-dir`. Nothing is shared. |
| Can't run both | Claude Desktop is single-instance — a second launch just focuses the first window. | The single-instance lock is keyed by the data folder, so each profile is its own app. Both run at once. |
| **Sign-in opens the wrong Claude** | Claude Code opens the *default* browser (already signed in to the other account). Claude Desktop finishes Google/SSO/email sign-in through a `claude://` link, and Windows always hands it to the *default* Claude window — Claude even re-registers that link on every start. | Claude Code's `$BROWSER` points at Claude Switch, which opens the profile's **own browser profile** (or a private sandbox). For Desktop, Claude Switch becomes the `claude://` handler and **routes each sign-in link to the window that asked for it**, and re-claims the link whenever Claude takes it back. |

It also strips inherited `ANTHROPIC_API_KEY` / `CLAUDE_CODE_OAUTH_TOKEN` / `CLAUDE_CONFIG_DIR`
variables that would otherwise silently override a profile's sign-in, and warns you if two
profiles end up signed in to the same account.

## Install

Download the installer for your system from the
[Releases page](https://github.com/samrand96/claude-switch/releases), install it, then run
`cswitch` — the setup wizard does the rest. Every build exists for **x86_64 and arm64**.

| System | Installer | Install with |
| --- | --- | --- |
| Windows 10 / 11 (x64) | `claude-switch-<version>-windows-x64-setup.exe` | double-click |
| Windows 11 on ARM | `claude-switch-<version>-windows-arm64-setup.exe` | double-click |
| macOS, Apple Silicon | `claude-switch-<version>-macos-arm64.pkg` | double-click |
| macOS, Intel | `claude-switch-<version>-macos-x86_64.pkg` | double-click |
| Debian, Ubuntu, Mint… | `claude-switch_<version>-1_amd64.deb` / `_arm64.deb` | `sudo apt install ./claude-switch_*.deb` |
| Fedora, RHEL, openSUSE… | `claude-switch-<version>-1.x86_64.rpm` / `.aarch64.rpm` | `sudo dnf install ./claude-switch-*.rpm` |
| Alpine | `claude-switch_<version>-r1_x86_64.apk` / `_aarch64.apk` | `sudo apk add --allow-untrusted ./claude-switch_*.apk` |
| Arch, Manjaro… | `claude-switch-<version>-1-x86_64.pkg.tar.zst` / `-aarch64` | `sudo pacman -U ./claude-switch-*.pkg.tar.zst` |
| Any Linux | `cswitch-x86_64-unknown-linux-musl.tar.gz` / `aarch64` | portable static binary — run `./cswitch install` |

Portable archives (`cswitch-<target>.zip` / `.tar.gz`) and `SHA256SUMS.txt` are attached too.

What the installers do:

- **Windows** — per-user, no administrator rights. Copies Claude Switch to
  `%LOCALAPPDATA%\Programs\Claude Switch` and runs `cswitch install`: `%USERPROFILE%\.cswitch\bin`
  on your PATH, `claude-<profile>` commands, Start-menu shortcuts and the `claude://` link
  router. Remove it from **Settings → Apps** (it asks whether to keep your profiles). Silent
  install for scripts / Intune: `claude-switch-…-setup.exe /VERYSILENT /SUPPRESSMSGBOXES`.
- **macOS** — installs into `/usr/local/claude-switch` and links `/usr/local/bin/cswitch` plus
  zsh / bash / fish completions (it never changes ownership of `/usr/local` folders, so
  Homebrew is unaffected).
- **Linux** — installs `/usr/bin/cswitch` and shell completions. The binary is static, so the
  same package works on any distribution of that family.

On macOS and Linux, `cswitch install` (run by the wizard) links `~/.cswitch/bin/cswitch` to the
packaged binary, so upgrading the package upgrades every `claude-<profile>` command too.

> **Unsigned downloads.** The installers are not code-signed yet. Windows SmartScreen may say
> "Windows protected your PC" → **More info → Run anyway**. macOS may refuse to open the `.pkg`
> → **System Settings → Privacy & Security → Open Anyway** (or use `install.sh` below, which is
> not affected).

**One-time Windows step.** Windows only lets *you* choose which app opens `claude://` links.
The wizard opens the right Settings page (`cswitch router settings` does too):
find the link type **CLAUDE** and pick **Claude Switch**. `cswitch router test` confirms it.

### Install from a terminal

```powershell
# Windows (PowerShell) — latest release, or -Version nightly / v1.2.3, or -Path .\cswitch.exe
irm https://raw.githubusercontent.com/samrand96/claude-switch/HEAD/install.ps1 | iex
```

```bash
# macOS / Linux — CSWITCH_VERSION=nightly or v1.2.3, CSWITCH_BIN=./cswitch, CSWITCH_NO_SETUP=1 are optional
curl -fsSL https://raw.githubusercontent.com/samrand96/claude-switch/HEAD/install.sh | sh
```

Both scripts pick the right build for your CPU, verify it against `SHA256SUMS.txt` and start the
setup wizard. Until there is a stable release they install the `nightly` pre-release.

### Build from source

```bash
cargo build --release            # target/release/cswitch(.exe)
target/release/cswitch install   # or: cswitch setup
# Static Linux binaries (any distro), x86_64 or arm64 — links with Rust's own rust-lld:
rustup target add aarch64-unknown-linux-musl
cargo build --release --target aarch64-unknown-linux-musl
```

## Quick start

```bash
cswitch                 # menu (first run: setup wizard)
cswitch login work      # sign 'work' in: Code + Desktop, right browser, right window
claude-work             # Claude Code as work   (same as: cswitch code work)
claude-personal         # …and personal, in another terminal, at the same time
cswitch desktop work    # Claude Desktop as work
cswitch desktop --all   # every profile's Desktop
cswitch status          # who is signed in where
```

## Commands

| Command | What it does |
| --- | --- |
| `cswitch` | Interactive menu (status table + everything below) |
| `cswitch setup` | Wizard: detect, create profiles, pick browsers, install, link router, shortcuts |
| `cswitch code [profile] [-n\|--here] [-C dir] [-- claude args]` | Claude Code in the profile (`-n` = new terminal tab/window; Windows Terminal tabs get the profile color; `--here` = this terminal even if Settings say new window) |
| `claude-<profile> [args]` | Same as `cswitch code <profile>` — installed commands |
| `cswitch desktop [profile] [--all\|--stop\|--restart]` | Start / focus / stop Claude Desktop per profile |
| `cswitch login [profile] [--code\|--desktop]` | Guided sign-in with account verification |
| `cswitch logout [profile] [--code\|--desktop]` | Sign out (Desktop: wipes that profile's Desktop data) |
| `cswitch status [--json]` | Accounts, running instances, sign-in browsers |
| `cswitch profile [list\|add\|edit\|show\|rename\|default\|remove\|open]` | Manage profiles (menu when no action) |
| `cswitch copy [--from p\|system] [--to p]` | Copy settings, CLAUDE.md, agents, commands, skills, MCP servers… (never credentials) |
| `cswitch shell [profile]` | Sub-shell where plain `claude` runs as the profile |
| `cswitch env [profile] [--shell powershell\|bash\|zsh\|fish\|cmd]` | Print the profile environment |
| `cswitch doctor [--fix] [--test-router]` | Check everything, explain fixes, repair (exits 1 when it finds problems) |
| `cswitch router [status\|register\|unregister\|test\|settings\|arm\|disarm]` | The `claude://` link router |
| `cswitch install` / `cswitch uninstall [--purge]` | Integrations on / off |
| `cswitch config [path\|show\|edit\|theme]` | Settings menu, themes (`claude`, `ocean`, `matrix`, `dusk`, `light`) |
| `cswitch completions <shell>` | Shell completions |

## How sign-in routing works

**Claude Code.** `cswitch login work` runs `claude auth login --sso --email you@company.com`
(method and email come from the profile) with `CLAUDE_CONFIG_DIR` set to the profile and
`BROWSER` set to Claude Switch. Claude Code hands the sign-in URL to `$BROWSER`; Claude Switch
opens it in the profile's browser:

- a **browser profile** you pick (Chrome/Edge/Brave/Vivaldi/Chromium profiles are listed with
  their signed-in email; Firefox profiles by name),
- a **sandbox** — a separate browser profile owned by Claude Switch, used for nothing else
  (recommended: it can never pick up the other account's session),
- a **private / InPrivate window**, the system browser, or any custom command (`{url}` placeholder).

The callback returns to the right `claude` process because each one listens on its own
localhost port. Afterwards Claude Switch reads the signed-in account and flags it if it does
not match the profile's email.

**Claude Desktop (Windows / Linux).** `cswitch login work` *arms* the router for ~10 minutes and
opens Work's Desktop. Sign in there; when the browser hands back
`claude://login/google-auth?…`, `claude://claude.ai/sso-callback?…` or a magic link, Windows runs
Claude Switch, which relaunches Claude with Work's `--user-data-dir` and the link as the last
argument — Electron passes it to the already-running Work window. Without an armed sign-in, a
link goes to the only running profile, or you are asked (configurable: ask / last launched /
default profile).

**Claude Desktop (macOS).** macOS delivers links to the running Claude, so `cswitch login`
briefly closes the other Claude windows while one profile signs in and reopens them afterwards.

## Profiles and storage

```
~/.cswitch/
  config.toml                   settings + profiles (edit by menu or by hand)
  bin/                          cswitch, cswitchw.exe (no-console helper), claude-<profile>
  profiles/<name>/code/         CLAUDE_CONFIG_DIR: settings, history, MCP, credentials
  profiles/<name>/desktop/      Claude Desktop data: session, cookies, claude_desktop_config.json
  profiles/<name>/browser/      sandbox browser profile for sign-in pages
  state/                        router state, caches, cswitch.log
```

Data lives under your home folder rather than `AppData` on purpose: Microsoft Store apps get
`AppData` writes redirected into a private package folder.

Each product in a profile can use **isolated** storage (default), the **system** location
(`~/.claude`, Claude Desktop's original folder — keeps an existing sign-in; one profile at a
time) or a **custom** folder.

Example `config.toml`:

```toml
default_profile = "work"
theme = "claude"

[settings]
deep_link_policy = "ask"          # ask | last-launched | default-profile
login_window_minutes = 10
env_isolation = "strict"          # strict | standard | off
env_passthrough = ["CLAUDE_CODE_GIT_BASH_PATH"]
code_launch = "here"              # here | new-window

[desktop]
launch_method = "auto"            # auto | direct | activate (Store apps)
router_enabled = true

[profiles.work]
label = "Work"
color = "#3B82F6"
email = "samrand@company.com"
login = "sso"                     # claude-ai | sso | console
code_storage = "isolated"         # isolated | system | C:\path
desktop_storage = "isolated"
desktop_shares_code = true        # Desktop's Code tab uses the profile's Claude Code data

[profiles.work.browser]
kind = "edge"                     # system | chrome | edge | brave | vivaldi | chromium | firefox | custom
mode = "profile"                  # profile | sandbox | private
profile = "Profile 1"

[profiles.work.env]
ANTHROPIC_MODEL = "claude-opus-5-5"

[profiles.personal]
label = "Personal"
color = "#D97757"
code_storage = "system"           # keep using the existing ~/.claude

[profiles.personal.browser]
kind = "chrome"
mode = "sandbox"
```

## Platform notes

- **Windows Store (MSIX) Claude Desktop** is detected through `Get-AppxPackage`. Isolated profiles
  start the app's executable directly (so per-profile environment variables apply); if Windows
  refuses, Claude Switch falls back to package activation automatically. A profile on the
  *system* storage always uses package activation so it sees the app's private data.
  Override in Settings → Desktop launch method.
- **Classic installer** (`%LOCALAPPDATA%\AnthropicClaude`) is detected too.
- `cswitchw.exe` is a GUI-subsystem copy of `cswitch.exe` (made at install time) used for links
  and sign-in pages, so no console window ever flashes.
- Two Desktop windows look identical — the Start-menu entries "Claude Desktop (Work)" /
  "(Personal)" get profile-colored icons, and `cswitch status` shows what is running.
- **Linux**: works with community Claude Desktop builds (`claude-desktop`); the router registers
  via `xdg-mime`; the "which profile?" prompt uses zenity/kdialog when available.
- Organization **managed settings** (`managed-settings.json`) apply to every profile by design;
  `cswitch doctor` points them out.

## Troubleshooting

Run `cswitch doctor` first — it checks the install, PATH, Claude Code, inherited variables,
Claude Desktop detection, the link router, each profile's sign-in and browser, and storage
conflicts, and prints the exact fix. `cswitch doctor --fix` repairs what it can.

- **Desktop sign-in opened the other window** → `cswitch router test`. On Windows, choose
  "Claude Switch" for CLAUDE links in Settings → Default apps (`cswitch router settings`).
- **Sign-in page opened in the wrong browser account** → `cswitch profile edit <name>` →
  Sign-in browser → *Sandbox*.
- **Signed in to the wrong account** → `cswitch logout <name> --code`, then `cswitch login <name>`.
- Logs: `~/.cswitch/state/cswitch.log` (sign-in codes are redacted).

## Uninstall

```bash
cswitch uninstall           # restores the claude:// handler, removes PATH entry, shortcuts, binaries
cswitch uninstall --purge   # …and deletes every profile's data
```

Without a terminal attached (scripts, CI) add `--yes`; destructive commands never run unattended
without it.

If you used an installer, also remove the package itself:

| Installed with | Remove with |
| --- | --- |
| Windows setup `.exe` | **Settings → Apps → Claude Switch → Uninstall** (runs `cswitch uninstall` for you) |
| macOS `.pkg` | `cswitch uninstall`, then `sudo /usr/local/claude-switch/uninstall.sh` |
| `.deb` / `.rpm` / `.apk` / Arch | `cswitch uninstall`, then `sudo apt remove claude-switch` (or `dnf remove`, `apk del`, `pacman -R`) |

## Development

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

See [ARCHITECTURE.md](ARCHITECTURE.md) for the module layout.

## License

MIT
