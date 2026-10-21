//! `claude://` link routing — the fix for "signing in opens the wrong Claude".
//!
//! Claude Desktop finishes Google / SSO / magic-link sign-in through a `claude://` deep link
//! that the operating system hands to *a* Claude process. Out of the box that is always the
//! default instance: on every start Claude re-registers the protocol pointing at its plain
//! executable, without any `--user-data-dir`. A second profile therefore can never complete
//! sign-in — the link lands in the other account's window.
//!
//! Claude Switch registers itself as the `claude://` handler and forwards each link to the
//! right instance by re-launching Claude with that profile's `--user-data-dir` and the link as
//! the last argument (Electron passes it to the running instance). Which profile is "right":
//!
//! 1. a profile that just asked to sign in (`cswitch login`) — valid for a few minutes;
//! 2. otherwise the only profile whose Desktop is running;
//! 3. otherwise the configured policy: ask, most recently launched, or default profile.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::config::{Config, DeepLinkPolicy, paths};
use crate::core::desktop::{self, DesktopInstall};
use crate::core::sys;

pub const PROGID: &str = "cswitch.claude";
pub const APP_NAME: &str = "Claude Switch";
const ARM_FILE: &str = "login-arm.json";
const REG_FILE: &str = "router.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Arm {
    pub profile: String,
    pub until: u64,
}

/// Routes every `claude://` link to `profile` for the next `minutes`.
pub fn arm(profile: &str, minutes: u64) -> Result<()> {
    let until = sys::now_unix() + minutes.max(1) * 60;
    sys::write_state(
        ARM_FILE,
        &Arm {
            profile: profile.to_string(),
            until,
        },
    )?;
    sys::log(format!(
        "router: armed for profile={profile} ({minutes} min)"
    ));
    Ok(())
}

pub fn disarm() {
    sys::remove_state(ARM_FILE);
}

pub fn active_arm() -> Option<Arm> {
    let arm: Arm = sys::read_state(ARM_FILE)?;
    (arm.until > sys::now_unix()).then_some(arm)
}

pub fn is_login_link(url: &str) -> bool {
    let lower = url.to_lowercase();
    lower.starts_with("claude://login/")
        || lower.contains("/magic-link")
        || lower.contains("/sso-callback")
        || lower.contains("google-auth")
}

/// Entry point for `cswitch route <url>` (what the OS runs for a `claude://` link).
pub fn handle(url: &str, windowed: bool) -> Result<()> {
    let url = url.trim().trim_matches('"').to_string();
    if !url.to_lowercase().starts_with("claude:") {
        bail!("not a claude:// link: {}", desktop::redact_link(&url));
    }
    if let Some(nonce) = ping_nonce(&url) {
        sys::log("router: self-test link received");
        return sys::write_state(&format!("ping-{nonce}"), &sys::now_unix());
    }
    let cfg = Config::load()?;
    let install = desktop::detect(&cfg, false)
        .context("Claude Desktop is not installed (or could not be found)")?;
    let target = choose_target(&cfg, &install, &url, windowed)?;
    let Some(target) = target else {
        // No profiles: behave exactly like the stock handler.
        sys::log("router: no profiles configured, passing link to stock Claude");
        let mut cmd = Command::new(&install.exe);
        cmd.arg(&url);
        sys::spawn_detached(&mut cmd)?;
        return Ok(());
    };
    sys::log(format!(
        "router: {} -> profile={target}",
        desktop::redact_link(&url)
    ));
    desktop::launch(&cfg, &install, &target, Some(&url))?;
    Ok(())
}

fn ping_nonce(url: &str) -> Option<String> {
    let rest = url.strip_prefix("claude://cswitch/ping?n=")?;
    let nonce: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    (!nonce.is_empty()).then_some(nonce)
}

pub fn choose_target(
    cfg: &Config,
    install: &DesktopInstall,
    url: &str,
    windowed: bool,
) -> Result<Option<String>> {
    if cfg.profiles.is_empty() {
        return Ok(None);
    }
    if let Some(arm) = active_arm()
        && cfg.profiles.contains_key(&arm.profile)
    {
        return Ok(Some(arm.profile));
    }
    let running = desktop::running_profiles(cfg, install);
    if running.len() == 1 {
        return Ok(Some(running[0].clone()));
    }
    let candidates: Vec<String> = if running.len() > 1 {
        running
    } else {
        cfg.profile_names()
    };
    if candidates.len() == 1 {
        return Ok(Some(candidates[0].clone()));
    }
    let fallback = || {
        desktop::last_launched()
            .filter(|p| candidates.contains(p))
            .or_else(|| cfg.fallback_profile())
            .unwrap_or_else(|| candidates[0].clone())
    };
    Ok(Some(match cfg.settings.deep_link_policy {
        DeepLinkPolicy::LastLaunched => fallback(),
        DeepLinkPolicy::DefaultProfile => cfg.fallback_profile().unwrap_or_else(fallback),
        DeepLinkPolicy::Ask => ask(cfg, &candidates, url, windowed).unwrap_or_else(fallback),
    }))
}

/// Asks which profile should get the link: a console prompt on Windows, zenity/kdialog on Linux.
fn ask(cfg: &Config, candidates: &[String], url: &str, windowed: bool) -> Option<String> {
    let kind = if is_login_link(url) {
        "sign-in link"
    } else {
        "Claude link"
    };

    #[cfg(windows)]
    {
        if windowed && !sys::win::alloc_console("Claude Switch — choose a profile") {
            return None;
        }
        let pick = console_pick(cfg, candidates, kind);
        if windowed {
            sys::win::free_console();
        }
        pick
    }
    #[cfg(not(windows))]
    {
        let _ = windowed;
        if console::user_attended() {
            return console_pick(cfg, candidates, kind);
        }
        let labels: Vec<String> = candidates.iter().map(|n| cfg.display_name(n)).collect();
        if sys::has_program("zenity") {
            let mut cmd = Command::new("zenity");
            cmd.args([
                "--list",
                "--title=Claude Switch",
                &format!("--text=Open this {kind} in which profile?"),
            ])
            .args([
                "--column=Profile",
                "--column=Name",
                "--hide-column=1",
                "--print-column=1",
            ]);
            for (name, label) in candidates.iter().zip(&labels) {
                cmd.arg(name).arg(label);
            }
            let out = cmd.output().ok()?;
            let picked = String::from_utf8_lossy(&out.stdout).trim().to_string();
            return candidates.iter().find(|c| **c == picked).cloned();
        }
        if sys::has_program("kdialog") {
            let mut cmd = Command::new("kdialog");
            cmd.args([
                "--title",
                "Claude Switch",
                "--radiolist",
                &format!("Open this {kind} in which profile?"),
            ]);
            for (i, (name, label)) in candidates.iter().zip(&labels).enumerate() {
                cmd.arg(name)
                    .arg(label)
                    .arg(if i == 0 { "on" } else { "off" });
            }
            let out = cmd.output().ok()?;
            let picked = String::from_utf8_lossy(&out.stdout).trim().to_string();
            return candidates.iter().find(|c| **c == picked).cloned();
        }
        None
    }
}

fn console_pick(cfg: &Config, candidates: &[String], kind: &str) -> Option<String> {
    use crate::ui;
    ui::banner("claude:// link router");
    println!();
    let items: Vec<String> = candidates
        .iter()
        .map(|n| ui::badge(n, cfg.profiles.get(n)))
        .collect();
    let choice = ui::prompt::select(
        &format!("Open this {kind} in which Claude profile?"),
        &items,
        0,
    )
    .ok()??;
    Some(candidates[choice].clone())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouterState {
    /// Claude Switch receives `claude://` links.
    Active,
    /// Registered, but another handler (usually the Claude app) is still the default.
    NotDefault {
        current: String,
    },
    NotRegistered,
    /// macOS: links always go to the frontmost Claude; use one-at-a-time sign-in instead.
    Unsupported,
}

#[derive(Serialize, Deserialize, Default)]
struct Registration {
    registered: bool,
    /// Previous `claude://` command, restored on unregister.
    previous_command: Option<String>,
}

pub fn handler_command() -> Result<String> {
    let exe = paths::helper_exe()?;
    Ok(format!("\"{}\" route \"%1\"", exe.display()))
}

pub fn state() -> RouterState {
    platform::state()
}

pub fn register() -> Result<()> {
    platform::register()?;
    sys::log("router: registered as claude:// handler");
    Ok(())
}

pub fn unregister() -> Result<()> {
    platform::unregister()?;
    sys::log("router: unregistered");
    Ok(())
}

pub fn is_registered_by_us() -> bool {
    sys::read_state::<Registration>(REG_FILE)
        .map(|r| r.registered)
        .unwrap_or(false)
}

/// Claude Desktop re-registers `claude://` on every start; restore ours shortly afterwards.
pub fn schedule_reassert() {
    if !is_registered_by_us() {
        return;
    }
    if let Ok(helper) = paths::helper_exe() {
        let mut cmd = Command::new(helper);
        cmd.arg("__reassert");
        let _ = sys::spawn_detached(&mut cmd);
    }
}

/// Body of the hidden `__reassert` command.
pub fn reassert_loop() {
    for delay in [12u64, 30] {
        std::thread::sleep(Duration::from_secs(delay));
        if let Ok(cfg) = Config::load()
            && (!cfg.desktop.router_enabled || !is_registered_by_us())
        {
            return;
        }
        if let Err(err) = platform::reassert() {
            sys::log(format!("router: reassert failed: {err:#}"));
        }
    }
}

/// Fires `claude://cswitch/ping` through the OS and waits for our handler to receive it.
pub fn self_test(timeout: Duration) -> Result<bool> {
    let nonce = format!("{:x}{:x}", sys::now_unix(), std::process::id());
    let url = format!("claude://cswitch/ping?n={nonce}");
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", &url]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(&url);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(&url);
        c
    };
    sys::spawn_detached(&mut cmd)?;
    let marker: PathBuf = paths::state_dir()?.join(format!("ping-{nonce}"));
    let started = Instant::now();
    while started.elapsed() < timeout {
        if marker.exists() {
            let _ = std::fs::remove_file(&marker);
            return Ok(true);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Ok(false)
}

/// Opens the OS page where the user picks the default handler for `claude://` links.
pub fn open_default_apps_settings() -> Result<()> {
    if cfg!(windows) {
        let uri = format!(
            "ms-settings:defaultapps?registeredAppUser={}",
            APP_NAME.replace(' ', "%20")
        );
        let mut cmd = Command::new("explorer");
        cmd.arg(uri);
        sys::spawn_detached(&mut cmd)?;
    }
    Ok(())
}

mod platform {
    use super::*;

    #[cfg(windows)]
    mod keys {
        pub const CLASSES: &str = r"Software\Classes";
        pub const PROTOCOL: &str = r"Software\Classes\claude";
        pub const PROTOCOL_COMMAND: &str = r"Software\Classes\claude\shell\open\command";
        pub const CAPABILITIES: &str = r"Software\ClaudeSwitch\Capabilities";
        pub const REGISTERED_APPS: &str = r"Software\RegisteredApplications";
        pub const USER_CHOICE: &str =
            r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\claude\UserChoice";
        pub const USER_CHOICE_LATEST: &str = r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\claude\UserChoiceLatest";
    }

    #[cfg(windows)]
    pub fn register() -> Result<()> {
        use winreg::RegKey;
        use winreg::enums::HKEY_CURRENT_USER;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let command = handler_command()?;
        let helper = paths::helper_exe()?;
        let icon = format!("\"{}\",0", helper.display());

        // Our own ProgID — what the user selects in Settings → Default apps.
        let (progid, _) = hkcu.create_subkey(format!(r"{}\{PROGID}", keys::CLASSES))?;
        progid.set_value("", &"URL:Claude (via Claude Switch)")?;
        progid.set_value("URL Protocol", &"")?;
        progid.set_value("FriendlyTypeName", &"Claude link (Claude Switch)")?;
        let (icon_key, _) = progid.create_subkey("DefaultIcon")?;
        icon_key.set_value("", &icon)?;
        let (cmd_key, _) = progid.create_subkey(r"shell\open\command")?;
        cmd_key.set_value("", &command)?;

        // Capabilities make "Claude Switch" show up as a candidate for CLAUDE links.
        let (caps, _) = hkcu.create_subkey(keys::CAPABILITIES)?;
        caps.set_value("ApplicationName", &APP_NAME)?;
        caps.set_value(
            "ApplicationDescription",
            &"Opens claude:// links in the Claude profile they belong to",
        )?;
        caps.set_value("ApplicationIcon", &icon)?;
        let (assoc, _) = caps.create_subkey("URLAssociations")?;
        assoc.set_value("claude", &PROGID)?;
        let (apps, _) = hkcu.create_subkey(keys::REGISTERED_APPS)?;
        apps.set_value(APP_NAME, &keys::CAPABILITIES)?;

        // Classic registration — used when no explicit default has been chosen.
        let previous = hkcu
            .open_subkey(keys::PROTOCOL_COMMAND)
            .and_then(|k| k.get_value::<String, _>(""))
            .ok()
            .filter(|c| !c.contains("cswitch"));
        let mut reg: Registration = sys::read_state(REG_FILE).unwrap_or_default();
        if previous.is_some() {
            reg.previous_command = previous;
        }
        write_classic(&command)?;
        reg.registered = true;
        sys::write_state(REG_FILE, &reg)?;
        sys::win::notify_association_change();
        Ok(())
    }

    #[cfg(windows)]
    fn write_classic(command: &str) -> Result<()> {
        use winreg::RegKey;
        use winreg::enums::HKEY_CURRENT_USER;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (proto, _) = hkcu.create_subkey(keys::PROTOCOL)?;
        proto.set_value("", &"URL:claude")?;
        proto.set_value("URL Protocol", &"")?;
        let (cmd_key, _) = hkcu.create_subkey(keys::PROTOCOL_COMMAND)?;
        cmd_key.set_value("", &command)?;
        Ok(())
    }

    #[cfg(windows)]
    pub fn reassert() -> Result<()> {
        use winreg::RegKey;
        use winreg::enums::HKEY_CURRENT_USER;
        let command = handler_command()?;
        let current = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(keys::PROTOCOL_COMMAND)
            .and_then(|k| k.get_value::<String, _>(""))
            .unwrap_or_default();
        if current != command {
            write_classic(&command)?;
            sys::win::notify_association_change();
            sys::log("router: restored claude:// registration after Claude re-registered itself");
        }
        Ok(())
    }

    #[cfg(windows)]
    pub fn unregister() -> Result<()> {
        use winreg::RegKey;
        use winreg::enums::HKEY_CURRENT_USER;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let reg: Registration = sys::read_state(REG_FILE).unwrap_or_default();
        let _ = hkcu.delete_subkey_all(format!(r"{}\{PROGID}", keys::CLASSES));
        let _ = hkcu.delete_subkey_all(r"Software\ClaudeSwitch");
        if let Ok(apps) =
            hkcu.open_subkey_with_flags(keys::REGISTERED_APPS, winreg::enums::KEY_ALL_ACCESS)
        {
            let _ = apps.delete_value(APP_NAME);
        }
        match reg.previous_command {
            Some(previous) => write_classic(&previous)?,
            None => {
                let ours = hkcu
                    .open_subkey(keys::PROTOCOL_COMMAND)
                    .and_then(|k| k.get_value::<String, _>(""))
                    .map(|c| c.contains("cswitch"))
                    .unwrap_or(false);
                if ours {
                    let _ = hkcu.delete_subkey_all(keys::PROTOCOL);
                }
            }
        }
        sys::remove_state(REG_FILE);
        sys::win::notify_association_change();
        Ok(())
    }

    #[cfg(windows)]
    pub fn state() -> RouterState {
        use winreg::RegKey;
        use winreg::enums::HKEY_CURRENT_USER;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let ours_registered = hkcu
            .open_subkey(format!(r"{}\{PROGID}", keys::CLASSES))
            .is_ok();
        if !ours_registered {
            return RouterState::NotRegistered;
        }
        let choice = [keys::USER_CHOICE_LATEST, keys::USER_CHOICE]
            .iter()
            .find_map(|k| {
                hkcu.open_subkey(k)
                    .and_then(|key| key.get_value::<String, _>("ProgId"))
                    .ok()
            });
        match choice {
            Some(progid) if progid.eq_ignore_ascii_case(PROGID) => RouterState::Active,
            Some(progid) => RouterState::NotDefault {
                current: describe_progid(&progid),
            },
            None => {
                let classic = hkcu
                    .open_subkey(keys::PROTOCOL_COMMAND)
                    .and_then(|k| k.get_value::<String, _>(""))
                    .unwrap_or_default();
                if classic.contains("cswitch") {
                    RouterState::Active
                } else {
                    RouterState::NotDefault {
                        current: "Claude (re-registered itself)".to_string(),
                    }
                }
            }
        }
    }

    #[cfg(windows)]
    fn describe_progid(progid: &str) -> String {
        if progid.starts_with("AppX") {
            format!("Claude app ({progid})")
        } else {
            progid.to_string()
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    const DESKTOP_FILE: &str = "cswitch-router.desktop";

    #[cfg(all(unix, not(target_os = "macos")))]
    fn applications_dir() -> Result<PathBuf> {
        Ok(dirs::data_dir()
            .context("no XDG data dir")?
            .join("applications"))
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    pub fn register() -> Result<()> {
        let dir = applications_dir()?;
        std::fs::create_dir_all(&dir)?;
        let exe = paths::helper_exe()?;
        let entry = format!(
            "[Desktop Entry]\nType=Application\nName={APP_NAME}\nComment=Opens claude:// links in the right Claude profile\n\
             Exec=\"{}\" route %u\nTerminal=false\nNoDisplay=true\nMimeType=x-scheme-handler/claude;\n",
            exe.display()
        );
        std::fs::write(dir.join(DESKTOP_FILE), entry)?;
        let previous = query_default();
        let mut reg: Registration = sys::read_state(REG_FILE).unwrap_or_default();
        if previous.as_deref().is_some_and(|p| p != DESKTOP_FILE) {
            reg.previous_command = previous;
        }
        set_default(DESKTOP_FILE)?;
        let _ = Command::new("update-desktop-database").arg(&dir).output();
        reg.registered = true;
        sys::write_state(REG_FILE, &reg)?;
        Ok(())
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    fn query_default() -> Option<String> {
        let out = Command::new("xdg-mime")
            .args(["query", "default", "x-scheme-handler/claude"])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (!text.is_empty()).then_some(text)
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    fn set_default(desktop_file: &str) -> Result<()> {
        let status = Command::new("xdg-mime")
            .args(["default", desktop_file, "x-scheme-handler/claude"])
            .status()
            .context("xdg-mime is not available")?;
        if !status.success() {
            bail!("xdg-mime could not set the claude:// handler");
        }
        Ok(())
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    pub fn reassert() -> Result<()> {
        if query_default().as_deref() != Some(DESKTOP_FILE) {
            set_default(DESKTOP_FILE)?;
            sys::log("router: restored claude:// handler after Claude re-registered itself");
        }
        Ok(())
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    pub fn unregister() -> Result<()> {
        let reg: Registration = sys::read_state(REG_FILE).unwrap_or_default();
        if let Some(previous) = reg.previous_command {
            let _ = set_default(&previous);
        }
        let _ = std::fs::remove_file(applications_dir()?.join(DESKTOP_FILE));
        sys::remove_state(REG_FILE);
        Ok(())
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    pub fn state() -> RouterState {
        if !applications_dir()
            .map(|d| d.join(DESKTOP_FILE).exists())
            .unwrap_or(false)
        {
            return RouterState::NotRegistered;
        }
        match query_default() {
            Some(d) if d == DESKTOP_FILE => RouterState::Active,
            Some(other) => RouterState::NotDefault { current: other },
            None => RouterState::NotDefault {
                current: "nothing".to_string(),
            },
        }
    }

    #[cfg(target_os = "macos")]
    pub fn register() -> Result<()> {
        bail!(
            "on macOS sign-in links are delivered to the running Claude; use `cswitch login` (one-at-a-time sign-in) instead"
        )
    }

    #[cfg(target_os = "macos")]
    pub fn reassert() -> Result<()> {
        Ok(())
    }

    #[cfg(target_os = "macos")]
    pub fn unregister() -> Result<()> {
        Ok(())
    }

    #[cfg(target_os = "macos")]
    pub fn state() -> RouterState {
        RouterState::Unsupported
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_login_links() {
        assert!(is_login_link("claude://login/google-auth?code=x"));
        assert!(is_login_link("claude://claude.ai/magic-link#a:b"));
        assert!(is_login_link("claude://claude.ai/sso-callback?x=1"));
        assert!(!is_login_link("claude://claude.ai/new?q=hello"));
    }

    #[test]
    fn extracts_ping_nonce() {
        assert_eq!(
            ping_nonce("claude://cswitch/ping?n=abc123").as_deref(),
            Some("abc123")
        );
        assert_eq!(ping_nonce("claude://cswitch/ping?n=").as_deref(), None);
        assert_eq!(ping_nonce("claude://login/x").as_deref(), None);
    }
}
