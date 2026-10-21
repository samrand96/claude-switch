//! Guided sign-in and sign-out for a profile — the part that makes "it opened the other
//! account" impossible.
//!
//! * Claude Code: `claude auth login` runs with the profile's `CLAUDE_CONFIG_DIR` and with
//!   `$BROWSER` pointing at Claude Switch, so the sign-in page opens in the profile's browser.
//!   The resulting account is checked against the profile's expected email.
//! * Claude Desktop (Windows / Linux): the `claude://` router is armed for this profile, so the
//!   Google / SSO / magic-link callback lands in this profile's window, not the default one.
//! * Claude Desktop (macOS): links go to the running Claude, so other instances are paused while
//!   this profile signs in.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use super::common::{interactive, require_desktop, resolve_profile};
use super::{LoginArgs, LogoutArgs};
use crate::config::{Config, Storage, normalize_path, paths};
use crate::core::desktop::{self, DesktopInstall};
use crate::core::router::{self, RouterState};
use crate::core::{browser, code, integrate, sys};
use crate::ui::{self, prompt};

pub fn login(cfg: &Config, args: LoginArgs) -> Result<()> {
    let name = resolve_profile(cfg, args.profile.as_deref(), "Sign in which profile?")?;
    let (do_code, do_desktop) = match (args.code, args.desktop) {
        (false, false) => {
            if interactive() {
                pick_products()?
            } else {
                (true, desktop::detect(cfg, false).is_some())
            }
        }
        (c, d) => (c, d),
    };
    if do_code {
        login_code(cfg, &name)?;
    }
    if do_desktop {
        login_desktop(cfg, &name, !args.no_wait && interactive())?;
    }
    Ok(())
}

fn pick_products() -> Result<(bool, bool)> {
    let items = [
        "Claude Code and Claude Desktop",
        "Claude Code only",
        "Claude Desktop only",
    ];
    Ok(
        match prompt::select("What do you want to sign in?", &items, 0)? {
            Some(0) => (true, true),
            Some(1) => (true, false),
            Some(2) => (false, true),
            _ => (false, false),
        },
    )
}

pub fn login_code(cfg: &Config, name: &str) -> Result<()> {
    let profile = cfg.profile(name)?;
    ui::section(format!("Claude Code sign-in · {}", cfg.display_name(name)));
    ui::note(format!(
        "Sign-in page opens in: {}",
        ui::accent2(browser::describe(&profile.browser))
    ));
    if let Some(email) = profile.email.as_deref().filter(|e| !e.is_empty()) {
        ui::note(format!("Expected account: {}", ui::accent2(email)));
    }
    ui::hint("If no browser appears, copy the link Claude prints into that browser yourself.");
    println!();
    let args = code::login_args(cfg, name)?;
    let ok = code::run_wait(cfg, name, &args)?;
    println!();
    if !ok {
        bail!("Claude Code sign-in did not finish");
    }
    verify_code_account(cfg, name);
    Ok(())
}

fn verify_code_account(cfg: &Config, name: &str) {
    let Ok(profile) = cfg.profile(name) else {
        return;
    };
    match code::account(cfg, name) {
        Some(account) => {
            match code::email_matches(profile.email.as_deref(), account.email.as_deref()) {
                Some(false) => {
                    ui::fail(format!(
                        "Signed in as {} — but this profile expects {}",
                        ui::bold(account.summary()),
                        ui::bold(profile.email.clone().unwrap_or_default())
                    ));
                    ui::hint(format!(
                        "Run `cswitch logout {name} --code`, then sign in again choosing the right account."
                    ));
                }
                _ => ui::ok(format!(
                    "Claude Code signed in as {}",
                    ui::bold(account.summary())
                )),
            }
            for other in cfg.profile_names().into_iter().filter(|n| n != name) {
                if let Some(theirs) = code::account(cfg, &other)
                    && theirs.email.is_some()
                    && theirs.email == account.email
                {
                    ui::warn(format!(
                        "Profile '{}' is signed in to the same account — they are not separated.",
                        cfg.display_name(&other)
                    ));
                }
            }
        }
        None => ui::ok("Claude Code sign-in finished"),
    }
}

pub fn login_desktop(cfg: &Config, name: &str, wait: bool) -> Result<()> {
    let install = require_desktop(cfg)?;
    ui::section(format!(
        "Claude Desktop sign-in · {}",
        cfg.display_name(name)
    ));
    if cfg!(target_os = "macos") {
        return login_desktop_macos(cfg, &install, name, wait);
    }
    if cfg.desktop.router_enabled {
        ensure_router(wait)?;
    } else {
        ui::warn(
            "The claude:// router is disabled in settings — the sign-in may land in another Claude window.",
        );
    }
    let minutes = cfg.settings.login_window_minutes;
    router::arm(name, minutes)?;
    super::desktop_cmd::start(cfg, &install, name)?;
    let badge = ui::badge(name, cfg.profiles.get(name));
    println!();
    ui::note(format!(
        "In the {badge} Claude window, sign in{}.",
        expected(cfg, name)
    ));
    ui::note(
        "When the browser asks to open Claude, allow it — Claude Switch delivers it to this profile.",
    );
    ui::hint("Google / SSO / email-link sign-ins are all covered.");
    if wait {
        println!();
        prompt::pause("Press Enter once you are signed in");
        router::disarm();
        ui::ok("Done — sign-in routing switched back to normal");
    } else {
        ui::hint(format!(
            "Sign-in links go to this profile for the next {minutes} minutes."
        ));
    }
    Ok(())
}

fn expected(cfg: &Config, name: &str) -> String {
    cfg.profiles
        .get(name)
        .and_then(|p| p.email.clone())
        .filter(|e| !e.is_empty())
        .map(|e| format!(" with {}", ui::bold(e)))
        .unwrap_or_default()
}

/// Makes sure `claude://` links reach Claude Switch, guiding the user where the OS requires it.
pub fn ensure_router(interactive_fix: bool) -> Result<()> {
    if !paths::installed_exe().map(|p| p.exists()).unwrap_or(false) {
        ui::note("Installing Claude Switch so the link router has a permanent home…");
        integrate::install_binary()?;
    }
    match router::state() {
        RouterState::Active | RouterState::Unsupported => Ok(()),
        RouterState::NotRegistered => {
            router::register()?;
            if router::state() == RouterState::Active {
                ui::ok("claude:// link router registered");
                Ok(())
            } else {
                guide_default_app(interactive_fix)
            }
        }
        RouterState::NotDefault { .. } => {
            // Re-registering is enough when Claude merely overwrote the classic handler; only an
            // explicit Windows default-app choice needs the Settings page.
            router::register()?;
            if router::state() == RouterState::Active {
                ui::ok("claude:// link router re-registered");
                return Ok(());
            }
            if cfg!(windows) {
                guide_default_app(interactive_fix)
            } else {
                Ok(())
            }
        }
    }
}

fn guide_default_app(interactive_fix: bool) -> Result<()> {
    let RouterState::NotDefault { current } = router::state() else {
        return Ok(());
    };
    ui::warn(format!(
        "claude:// links currently open in: {}",
        ui::bold(&current)
    ));
    ui::hint("Windows only lets you change this in Settings. One-time step:");
    ui::hint("  1. In the Settings page that opens, find the link type CLAUDE");
    ui::hint("  2. Click it and choose \"Claude Switch\"");
    if !interactive_fix {
        ui::hint("Run `cswitch router settings` to open that page.");
        return Ok(());
    }
    router::open_default_apps_settings()?;
    prompt::pause("Press Enter after choosing Claude Switch");
    match router::state() {
        RouterState::Active => ui::ok("claude:// links now go through Claude Switch"),
        _ => ui::warn(
            "Still not the default — the sign-in may land in the wrong window. `cswitch doctor` can re-check.",
        ),
    }
    Ok(())
}

fn login_desktop_macos(
    cfg: &Config,
    install: &DesktopInstall,
    name: &str,
    wait: bool,
) -> Result<()> {
    let others: Vec<desktop::Instance> = desktop::instances(cfg, install)
        .into_iter()
        .filter(|i| i.profile.as_deref() != Some(name))
        .collect();
    let mut paused: Vec<String> = Vec::new();
    if !others.is_empty() {
        ui::note("macOS delivers sign-in links to the running Claude app, so other Claude windows");
        ui::note("will close during sign-in and reopen afterwards.");
        if wait && !prompt::confirm("Close the other Claude windows now?", true)? {
            bail!("cancelled");
        }
        for instance in &others {
            match &instance.profile {
                Some(p) => {
                    desktop::stop(cfg, install, p)?;
                    paused.push(p.clone());
                }
                None => {
                    kill_pid(instance.pid);
                }
            }
        }
    }
    super::desktop_cmd::start(cfg, install, name)?;
    ui::note(format!(
        "Sign in inside the Claude window{}.",
        expected(cfg, name)
    ));
    if !wait {
        // Reopening now would let the other windows catch the sign-in link.
        for p in &paused {
            ui::hint(format!(
                "After signing in, reopen {}: cswitch desktop {p}",
                ui::badge(p, cfg.profiles.get(p))
            ));
        }
        return Ok(());
    }
    prompt::pause("Press Enter once you are signed in");
    for p in paused {
        let _ = desktop::launch(cfg, install, &p, None);
        ui::ok(format!("Reopened {}", ui::badge(&p, cfg.profiles.get(&p))));
    }
    Ok(())
}

fn kill_pid(pid: u32) {
    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    if let Some(process) = system.process(sysinfo::Pid::from_u32(pid)) {
        process.kill();
    }
}

pub fn logout(cfg: &Config, args: LogoutArgs) -> Result<()> {
    let name = resolve_profile(cfg, args.profile.as_deref(), "Sign out which profile?")?;
    let (do_code, do_desktop) = match (args.code, args.desktop) {
        (false, false) => {
            if interactive() {
                let items = [
                    "Claude Code",
                    "Claude Desktop (wipe this profile's Desktop data)",
                    "Both",
                ];
                match prompt::select("Sign out of what?", &items, 0)? {
                    Some(0) => (true, false),
                    Some(1) => (false, true),
                    Some(2) => (true, true),
                    _ => return Ok(()),
                }
            } else {
                (true, false)
            }
        }
        (c, d) => (c, d),
    };
    let mut code_failed = false;
    if do_code {
        if code::run_wait(cfg, &name, &["auth".into(), "logout".into()])? {
            ui::ok(format!(
                "{} Claude Code signed out",
                ui::badge(&name, cfg.profiles.get(&name))
            ));
        } else {
            code_failed = true;
        }
    }
    if do_desktop {
        wipe_desktop(cfg, &name, args.yes)?;
    }
    if code_failed {
        bail!("`claude auth logout` failed for '{name}'");
    }
    Ok(())
}

/// A custom Desktop folder is deleted wholesale on sign-out, so refuse anything that is
/// clearly not a per-profile folder: relative paths, roots, home, Claude Desktop's own data.
fn unsafe_to_wipe(dir: &Path, install: Option<&DesktopInstall>) -> Option<String> {
    if !dir.is_absolute() {
        return Some(format!("{} is not a full path", dir.display()));
    }
    if dir.parent().is_none() {
        return Some(format!("{} is a drive / filesystem root", dir.display()));
    }
    let wanted = normalize_path(dir);
    let mut protected: Vec<PathBuf> = desktop::stock_data_dirs(install);
    protected.extend(paths::home_dir());
    if let Ok(home) = paths::cswitch_home() {
        protected.push(home.join("profiles"));
        protected.push(home);
    }
    protected
        .iter()
        .any(|p| normalize_path(p) == wanted)
        .then(|| {
            format!(
                "{} is a shared folder, not this profile's own Desktop data",
                dir.display()
            )
        })
}

pub fn wipe_desktop(cfg: &Config, name: &str, yes: bool) -> Result<()> {
    let profile = cfg.profile(name)?;
    if profile.desktop_storage == Storage::System {
        ui::warn(
            "This profile uses Claude Desktop's original data folder; sign out from inside the app instead (profile menu → Log out).",
        );
        return Ok(());
    }
    let Some(dir) = cfg.desktop_dir(name)? else {
        return Ok(());
    };
    if !dir.exists() {
        ui::note("Nothing to wipe — this profile's Desktop has no data yet.");
        return Ok(());
    }
    let install = desktop::detect(cfg, false);
    if let Some(reason) = unsafe_to_wipe(&dir, install.as_ref()) {
        bail!(
            "refusing to delete it: {reason}. Point the profile at its own folder (`cswitch profile edit {name}`) or sign out inside the app."
        );
    }
    if !yes && !interactive() {
        bail!("add --yes to wipe '{name}'s Claude Desktop data without a prompt");
    }
    let size = sys::human_size(sys::dir_size(&dir));
    if !yes
        && !prompt::confirm(
            &format!(
                "Delete {} ({size})? This signs the Desktop app out and removes its local data",
                dir.display()
            ),
            false,
        )?
    {
        return Ok(());
    }
    if let Some(install) = &install {
        desktop::stop(cfg, install, name)?;
    }
    fs::remove_dir_all(&dir)?;
    ui::ok(format!(
        "{} Claude Desktop data wiped",
        ui::badge(name, cfg.profiles.get(name))
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_to_wipe_shared_folders() {
        let home = paths::home_dir().unwrap();
        assert!(unsafe_to_wipe(&home, None).is_some());
        assert!(unsafe_to_wipe(Path::new("relative/dir"), None).is_some());
        let root = home.ancestors().last().unwrap().to_path_buf();
        assert!(unsafe_to_wipe(&root, None).is_some());
        let own = paths::profile_root("work").unwrap().join("desktop");
        assert!(unsafe_to_wipe(&own, None).is_none());
    }
}
