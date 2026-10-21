//! `cswitch install`, `cswitch uninstall`, `cswitch router …` and the integrations menu.

use std::fs;
use std::process::Command;
use std::time::Duration;

use anyhow::{Result, bail};

use super::common::interactive;
use super::{InstallArgs, RouterAction, UninstallArgs};
use crate::config::{Config, paths};
use crate::core::router::{self, RouterState};
use crate::core::{desktop, integrate, sys};
use crate::ui::{self, prompt};

pub fn install(cfg: &mut Config, args: InstallArgs) -> Result<()> {
    ui::section("Install Claude Switch");
    let exe = integrate::install_binary()?;
    ui::ok(format!(
        "Installed {}",
        ui::accent2(exe.display().to_string())
    ));
    if cfg!(windows) {
        ui::hint(
            "cswitchw.exe (no console window) handles links and sign-in pages in the background.",
        );
    }
    match integrate::sync_shims(cfg) {
        Ok(shims) if !shims.is_empty() => {
            let names: Vec<String> = shims
                .iter()
                .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
                .collect();
            ui::ok(format!("Commands: {}", ui::accent2(names.join(", "))));
        }
        Ok(_) => ui::hint("Create profiles to get claude-<profile> commands."),
        Err(err) => ui::warn(format!("Could not create profile commands: {err:#}")),
    }
    if !args.no_path {
        match integrate::add_bin_to_path() {
            Ok(msg) => ui::ok(format!("PATH: {msg}")),
            Err(err) => ui::warn(format!("Could not update PATH: {err:#}")),
        }
    }
    let install = desktop::detect(cfg, true);
    match &install {
        Some(i) => ui::ok(format!(
            "Claude Desktop found: {} {}",
            i.kind.describe(),
            ui::dim(i.version.clone().unwrap_or_default())
        )),
        None => ui::note("Claude Desktop not found — Claude Code profiles work anyway."),
    }
    if !args.no_shortcuts {
        match integrate::create_shortcuts(cfg, install.as_ref()) {
            Ok(made) if !made.is_empty() => ui::ok(format!("Shortcuts: {}", made.join(", "))),
            Ok(_) => {}
            Err(err) => ui::warn(format!("Could not create shortcuts: {err:#}")),
        }
    }
    if !args.no_router && install.is_some() && !cfg!(target_os = "macos") {
        router::register()?;
        cfg.desktop.router_enabled = true;
        cfg.save()?;
        report_router_state();
    }
    Ok(())
}

pub fn report_router_state() {
    match router::state() {
        RouterState::Active => ui::ok("claude:// sign-in links are routed per profile"),
        RouterState::NotDefault { current } => {
            ui::warn(format!("claude:// links still open in {current}"));
            if cfg!(windows) {
                ui::hint(
                    "One-time fix: `cswitch router settings` → link type CLAUDE → choose \"Claude Switch\".",
                );
            }
        }
        RouterState::NotRegistered => ui::warn("claude:// link router is not registered"),
        RouterState::Unsupported => ui::note(
            "macOS: Desktop sign-in uses one-at-a-time mode (`cswitch login`) instead of a link router.",
        ),
    }
}

/// Returns whether it uninstalled (false when the user declined).
pub fn uninstall(cfg: &Config, args: UninstallArgs) -> Result<bool> {
    if !args.yes {
        if !interactive() {
            bail!("add --yes to uninstall without a prompt");
        }
        let question = if args.purge {
            "Uninstall Claude Switch AND delete every profile's data?"
        } else {
            "Uninstall Claude Switch? (profile data is kept)"
        };
        if !prompt::confirm(question, false)? {
            return Ok(false);
        }
    }
    let _ = router::unregister();
    ui::ok("claude:// handler restored");
    integrate::remove_shortcuts();
    ui::ok("Shortcuts removed");
    integrate::remove_bin_from_path()?;
    ui::ok("PATH entry removed");
    if args.purge
        && let Some(install) = desktop::detect(cfg, false)
    {
        for name in cfg.profile_names() {
            let _ = desktop::stop(cfg, &install, &name);
        }
    }
    let home = paths::cswitch_home()?;
    let target = if args.purge {
        home.clone()
    } else {
        paths::bin_dir()?
    };
    remove_dir_later(&target)?;
    if args.purge {
        ui::ok(format!("Removed {} (all profiles)", home.display()));
    } else {
        ui::ok(format!("Removed {}", target.display()));
        ui::hint(format!(
            "Profiles and config are still in {} — delete that folder to remove them.",
            home.display()
        ));
    }
    Ok(true)
}

/// Deletes a folder that may contain the running executable.
fn remove_dir_later(dir: &std::path::Path) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    if cfg!(windows) {
        let line = format!(
            "/c ping -n 3 127.0.0.1 >nul & rmdir /s /q {}",
            sys::quote_windows_arg(&dir.to_string_lossy())
        );
        let mut cmd = Command::new("cmd");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.raw_arg(line);
        }
        #[cfg(not(windows))]
        let _ = line;
        sys::spawn_detached(&mut cmd)?;
        Ok(())
    } else {
        fs::remove_dir_all(dir)?;
        Ok(())
    }
}

/// Returns the process exit code (`router test` fails with 1).
pub fn router_cmd(cfg: &mut Config, action: Option<RouterAction>) -> Result<i32> {
    if let Some(RouterAction::Test) = action {
        return Ok(if test_router()? { 0 } else { 1 });
    }
    router_action(cfg, action).map(|_| 0)
}

fn router_action(cfg: &mut Config, action: Option<RouterAction>) -> Result<()> {
    match action.unwrap_or(RouterAction::Status) {
        RouterAction::Status => {
            report_router_state();
            if let Some(arm) = router::active_arm() {
                ui::note(format!(
                    "Sign-in routing → {} (until {} UTC)",
                    cfg.display_name(&arm.profile),
                    sys::timestamp(arm.until)
                ));
            }
            Ok(())
        }
        RouterAction::Register => {
            if !paths::installed_exe().map(|p| p.exists()).unwrap_or(false) {
                integrate::install_binary()?;
            }
            router::register()?;
            cfg.desktop.router_enabled = true;
            cfg.save()?;
            report_router_state();
            Ok(())
        }
        RouterAction::Unregister => {
            router::unregister()?;
            cfg.desktop.router_enabled = false;
            cfg.save()?;
            ui::ok("claude:// handler restored to the previous app");
            Ok(())
        }
        RouterAction::Test => test_router().map(|_| ()),
        RouterAction::Settings => {
            if cfg!(windows) {
                router::open_default_apps_settings()?;
                ui::note("Find the link type CLAUDE and choose \"Claude Switch\".");
            } else {
                ui::note("Only needed on Windows.");
            }
            Ok(())
        }
        RouterAction::Arm { profile, minutes } => {
            cfg.profile(&profile)?;
            router::arm(&profile, minutes)?;
            ui::ok(format!(
                "claude:// links go to {} for {minutes} minutes",
                ui::badge(&profile, cfg.profiles.get(&profile))
            ));
            Ok(())
        }
        RouterAction::Disarm => {
            router::disarm();
            ui::ok("Sign-in routing back to normal");
            Ok(())
        }
    }
}

/// Returns whether the test link reached Claude Switch (true where no router is used).
pub fn test_router() -> Result<bool> {
    if cfg!(target_os = "macos") {
        ui::note("Not used on macOS.");
        return Ok(true);
    }
    ui::note("Sending a test claude:// link through the operating system…");
    if cfg!(windows) {
        ui::hint("If Windows asks which app to use, pick \"Claude Switch\" and tick \"Always\".");
    }
    let reached = router::self_test(Duration::from_secs(20))?;
    if reached {
        ui::ok("The link reached Claude Switch — Desktop sign-in will land in the right profile.");
    } else {
        ui::fail("The link did not reach Claude Switch (it probably opened Claude directly).");
        if cfg!(windows) {
            ui::hint(
                "Fix: `cswitch router settings` → link type CLAUDE → \"Claude Switch\", then test again.",
            );
        } else {
            ui::hint("Fix: `cswitch router register`, then test again.");
        }
    }
    Ok(reached)
}

pub fn menu(cfg: &mut Config) -> Result<()> {
    loop {
        ui::section("Install & integrations");
        let installed = paths::installed_exe().map(|p| p.exists()).unwrap_or(false);
        let on_path = integrate::bin_on_path();
        ui::note(format!(
            "Installed: {}   On PATH: {}",
            if installed {
                ui::success("yes")
            } else {
                ui::warning("no")
            },
            if on_path {
                ui::success("yes")
            } else {
                ui::warning("no")
            }
        ));
        report_router_state();
        println!();
        let items = vec![
            format!(
                "{} Install / update (binary, claude-<profile> commands, PATH, shortcuts, router)",
                ui::g("⬇", "v")
            ),
            format!("{} Recreate shortcuts", ui::g("⌘", "#")),
            format!("{} Register the claude:// link router", ui::g("⇢", ">")),
            format!("{} Test the link router", ui::g("✓", "?")),
            format!("{} Open Windows default-apps settings", ui::g("⚙", "*")),
            format!("{} Shell completions", ui::g("⇥", ">")),
            format!("{} Uninstall", ui::g("✖", "x")),
            ui::dim(format!("{} Back", ui::g("←", "<"))),
        ];
        let Some(choice) = prompt::select("Integrations", &items, 0)? else {
            return Ok(());
        };
        let result = match choice {
            0 => install(
                cfg,
                InstallArgs {
                    no_path: false,
                    no_shortcuts: false,
                    no_router: false,
                },
            ),
            1 => {
                integrate::create_shortcuts(cfg, desktop::detect(cfg, false).as_ref()).map(|made| {
                    ui::ok(format!("Shortcuts: {}", made.join(", ")));
                })
            }
            2 => router_action(cfg, Some(RouterAction::Register)),
            3 => test_router().map(|_| ()),
            4 => router_action(cfg, Some(RouterAction::Settings)),
            5 => {
                completions_help();
                Ok(())
            }
            6 => {
                let removed = uninstall(
                    cfg,
                    UninstallArgs {
                        purge: false,
                        yes: false,
                    },
                )?;
                if removed {
                    std::process::exit(0);
                }
                continue;
            }
            _ => return Ok(()),
        };
        if let Err(err) = result {
            ui::fail(format!("{err:#}"));
        }
        prompt::pause("Press Enter to continue");
    }
}

pub fn completions_help() {
    ui::section("Shell completions");
    ui::note("PowerShell:  cswitch completions powershell | Out-String | Invoke-Expression");
    ui::hint("(add that line to your $PROFILE to keep it)");
    ui::note("bash:        echo 'source <(cswitch completions bash)' >> ~/.bashrc");
    ui::note("zsh:         cswitch completions zsh > \"${fpath[1]}/_cswitch\"");
    ui::note("fish:        cswitch completions fish > ~/.config/fish/completions/cswitch.fish");
}
