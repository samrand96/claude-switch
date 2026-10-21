# Architecture

Claude Switch is organized by responsibility, like git-ai: platform logic never prints menus,
and menus never touch the registry directly.

## Top-level layers

- `src/main.rs` — calls `app::main`.
- `src/app.rs` — process entry. Decides *how* the binary was started before handing off to clap:
  - invoked as `claude-<profile>` (hard link / symlink) → run Claude Code in that profile;
  - invoked with a single `http(s)://` argument → `$BROWSER` opener for Claude Code sign-in;
  - invoked with a `claude://` argument → link router;
  - invoked as `cswitchw` (Windows GUI-subsystem copy) → never touches a console, errors become
    message boxes;
  - otherwise clap commands, or the interactive menu when there are none.
- `src/config/` — `schema.rs` (TOML types), `paths.rs` (filesystem layout), `store.rs`
  (load/save, per-profile directory resolution, validation).
- `src/core/` — platform logic, UI-free:
  - `env.rs` builds a profile's environment: strips inherited identity variables, sets
    `CLAUDE_CONFIG_DIR`, `BROWSER`, `CSWITCH_PROFILE` and the profile's own variables.
  - `code.rs` locates `claude`, runs it in a profile, reads the signed-in account from
    `.claude.json`, checks credentials (file on Windows/Linux, keychain service
    `Claude Code-credentials-<sha256(dir)[..8]>` on macOS).
  - `browser.rs` detects browsers and their profiles (Chromium `Local State`, Firefox
    `profiles.ini`) and builds the command that opens a URL in a profile's browser / sandbox /
    private window.
  - `desktop.rs` detects Claude Desktop (MSIX via `Get-AppxPackage`, classic installer, macOS
    app, Linux package), launches instances with `--user-data-dir` (direct or MSIX activation via
    `IApplicationActivationManager`), maps running processes to profiles, stops them.
  - `router.rs` the `claude://` handler: login arming, target selection, registration
    (Windows: `HKCU\Software\Classes` + RegisteredApplications capabilities + UserChoice
    inspection; Linux: `xdg-mime`), re-assertion after Claude re-registers itself, self-test.
  - `integrate.rs` installs the binary (a copy; on macOS / Linux a link when the running binary
    is root-owned, i.e. installed by a package, so package upgrades apply), generates
    `cswitchw.exe` by flipping the PE subsystem field, `claude-<profile>` shims, PATH,
    profile-colored ICO icons, shortcuts.
  - `terminal.rs` opens Claude Code in a new Windows Terminal tab / Terminal.app / Linux terminal.
  - `sys.rs` detached processes, state files, log, Windows API wrappers, encoded PowerShell.
- `src/cli/` — one module per workflow (`code_cmd`, `desktop_cmd`, `login`, `profiles`,
  `status`, `doctor`, `install_cmd`, `settings`, `wizard`, `menu`), `editor.rs` for the shared
  profile editor and browser picker, `common.rs` for profile selection.
- `src/ui/` — palettes (`theme.rs`), styled messages/badges/banner (`mod.rs`), dialoguer
  prompts with the palette (`prompt.rs`), aligned tables (`table.rs`).

## Key decisions

- **Isolation through Claude's own switches.** `CLAUDE_CONFIG_DIR` and Chromium's
  `--user-data-dir` are honored by the products themselves, so no files are swapped, nothing is
  symlinked, and two profiles can run concurrently.
- **Links are routed, not raced.** Desktop sign-in callbacks are delivered by relaunching the
  app with the profile's data dir and the link as the *last* argument — the position Claude
  Desktop's `second-instance` handler reads.
- **Strict environment by default.** Every inherited `ANTHROPIC_*` / `CLAUDE*` / `BROWSER`
  variable is removed unless allow-listed, because a single global API key would override any
  profile's sign-in.
- **No secrets stored.** Claude Switch never reads or copies credentials; it only checks whether
  they exist.

## Packaging

Installers only *deliver* the binary; `cswitch install` stays the single place that integrates
with the system, so installed and hand-run setups behave the same.

- `packaging/windows/cswitch.iss` — Inno Setup, per-user (no admin). Copies `cswitch.exe` to
  `%LOCALAPPDATA%\Programs\Claude Switch`, then runs `cswitch install`; the uninstaller runs
  `cswitch uninstall --yes [--purge]`. One AppId for x64 and arm64.
- `packaging/macos/build-pkg.sh` — `pkgbuild` + `productbuild`. Payload in
  `/usr/local/claude-switch` only; `postinstall` symlinks `/usr/local/bin/cswitch` and the
  completions, so shared `/usr/local` folders are never re-owned.
- `packaging/linux/nfpm.yaml` — `.deb`, `.rpm`, `.apk`, Arch package around the static musl binary.
- `.cargo/config.toml` — static CRT on Windows (no VC++ runtime), `rust-lld` for musl targets.
- `.github/workflows/release.yml` builds, packages, installs/uninstalls and publishes all of it:
  a `v*` tag makes a release, the `nightly` tag a pre-release that replaces the previous nightly.

## Errors

Commands return `anyhow::Result`; `app.rs` renders them (or shows a message box from
`cswitchw`). Background activity (router, opener, launches) is appended to
`~/.cswitch/state/cswitch.log` with sign-in codes redacted.
