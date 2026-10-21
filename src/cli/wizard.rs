//! First-run wizard: detect, create profiles, install, wire up sign-in routing, shortcuts.

use anyhow::Result;

use super::common::profile_item;
use super::{InstallArgs, editor, install_cmd, login};
use crate::config::{Config, Storage, paths};
use crate::core::{browser, code, desktop, integrate};
use crate::ui::{self, prompt};

pub fn run(cfg: &mut Config) -> Result<()> {
    ui::banner("Setup · work & personal Claude, side by side");
    println!();
    ui::note(
        "Every profile gets its own Claude Code sign-in/settings/history and its own Claude Desktop",
    );
    ui::note("window. They never share anything, and they can all run at the same time.");

    ui::section("1 · Looking around");
    match code::resolve_claude(cfg) {
        Ok(path) => ui::ok(format!(
            "Claude Code {}",
            ui::dim(code::version(&path).unwrap_or_else(|| path.display().to_string()))
        )),
        Err(_) => {
            ui::warn("Claude Code not found — install it later; profiles will be ready for it")
        }
    }
    let install = desktop::detect(cfg, true);
    match &install {
        Some(i) => ui::ok(format!(
            "Claude Desktop · {} {}",
            i.kind.describe(),
            ui::dim(i.version.clone().unwrap_or_default())
        )),
        None => {
            ui::warn("Claude Desktop not found — Desktop features stay off until it is installed")
        }
    }
    let browsers = browser::detect_all();
    if browsers.is_empty() {
        ui::note("No supported browsers detected — sign-in pages will use the system browser");
    } else {
        let list: Vec<String> = browsers
            .iter()
            .map(|b| {
                format!(
                    "{} ({} profiles)",
                    b.kind.display(),
                    browser::profiles(b).len()
                )
            })
            .collect();
        ui::ok(format!("Browsers · {}", list.join(", ")));
    }
    let existing_code = paths::system_code_state_file()
        .map(|p| p.exists())
        .unwrap_or(false);
    let existing_account = paths::system_code_state_file()
        .ok()
        .and_then(|p| code::account_from_file(&p));
    if let Some(account) = &existing_account {
        ui::note(format!(
            "Your current Claude Code is signed in as {}",
            ui::bold(account.summary())
        ));
    }

    ui::section("2 · Profiles");
    let mut first_created: Option<String> = None;
    if cfg.profiles.is_empty() {
        let presets = ["work", "personal"];
        let picked = prompt::multi(
            "Which profiles do you want? (space toggles — you can add custom ones next)",
            &["work".to_string(), "personal".to_string()],
            &[true, true],
        )?
        .unwrap_or_default();
        let mut names: Vec<String> = picked.into_iter().map(|i| presets[i].to_string()).collect();
        while prompt::confirm("Add another profile (e.g. a client)?", names.is_empty())? {
            if let Some(name) = editor::ask_new_name(cfg, "")? {
                // ask_new_name only knows saved profiles; these are not saved yet.
                if names.contains(&name) {
                    ui::warn(format!("'{name}' is already on the list"));
                } else {
                    names.push(name);
                }
            }
        }
        if existing_code || existing_account.is_some() {
            ui::hint(
                "Tip: one profile may keep using your existing ~/.claude so its current sign-in survives.",
            );
        }
        first_created = names.first().cloned();
        for (index, name) in names.iter().enumerate() {
            let profile = editor::create_profile(cfg, name, index)?;
            cfg.profiles.insert(name.clone(), profile);
            cfg.save()?;
        }
    } else {
        super::profiles::list(cfg);
        while prompt::confirm("Add another profile?", false)? {
            super::profiles::add(cfg, None)?;
        }
    }
    if cfg.profiles.is_empty() {
        ui::warn("No profiles created — run `cswitch setup` again whenever you're ready.");
        return Ok(());
    }
    if cfg.profiles.len() > 1 {
        let names = cfg.profile_names();
        let items: Vec<String> = names.iter().map(|n| profile_item(cfg, n)).collect();
        let preferred = cfg
            .default_profile
            .clone()
            .or_else(|| first_created.clone());
        let current = preferred
            .and_then(|d| names.iter().position(|n| *n == d))
            .unwrap_or(0);
        if let Some(i) = prompt::select(
            "Default profile (used when you don't name one)",
            &items,
            current,
        )? {
            cfg.default_profile = Some(names[i].clone());
        }
    } else {
        cfg.default_profile = cfg.profiles.keys().next().cloned();
    }
    cfg.save()?;
    for name in cfg.profile_names() {
        if let Some(dir) = cfg.code_dir(&name)? {
            std::fs::create_dir_all(dir)?;
        }
    }

    ui::section("3 · Install");
    let shims: Vec<String> = cfg
        .profile_names()
        .iter()
        .map(|n| format!("claude-{n}"))
        .collect();
    ui::note(format!(
        "Copies Claude Switch to {} and adds the commands {}",
        ui::accent2(paths::bin_dir()?.display().to_string()),
        ui::accent2(shims.join(", "))
    ));
    if install.is_some() && !cfg!(target_os = "macos") {
        ui::note(
            "…and registers the claude:// link router so Desktop sign-in returns to the right window.",
        );
    }
    let shortcuts = prompt::confirm(
        if cfg!(windows) {
            "Also add Start menu shortcuts (Claude Desktop (Work), Claude Code (Work), …)?"
        } else {
            "Also add app launchers for each profile?"
        },
        true,
    )?;
    if prompt::confirm("Install now?", true)? {
        install_cmd::install(
            cfg,
            InstallArgs {
                no_path: false,
                no_shortcuts: !shortcuts,
                no_router: false,
            },
        )?;
        if install.is_some() && !cfg!(target_os = "macos") && cfg.desktop.router_enabled {
            ui::section("4 · Sign-in routing for Claude Desktop");
            login::ensure_router(true)?;
            if prompt::confirm("Send a test link to make sure it works?", true)? {
                install_cmd::test_router()?;
            }
        }
    } else {
        ui::hint("Skipped — run `cswitch install` any time.");
    }

    ui::section("Ready");
    for name in cfg.profile_names() {
        let p = cfg.profile(&name)?;
        let code_note = match p.code_storage {
            Storage::System => " (your existing ~/.claude)",
            _ => "",
        };
        println!("  {}", profile_item(cfg, &name));
        ui::hint(format!(
            "Claude Code:    claude-{name}   or   cswitch code {name}{code_note}"
        ));
        if install.is_some() {
            ui::hint(format!("Claude Desktop: cswitch desktop {name}"));
        }
        ui::hint(format!("Sign in:        cswitch login {name}"));
    }
    if !integrate::bin_on_path() && paths::installed_exe().map(|p| p.exists()).unwrap_or(false) {
        ui::hint("Open a new terminal so the new commands are on your PATH.");
    }
    println!();
    if prompt::confirm("Sign your profiles in now?", true)? {
        for name in cfg.profile_names() {
            let label = cfg.display_name(&name);
            if !prompt::confirm(&format!("Sign in '{label}' now?"), true)? {
                continue;
            }
            let desktop_too = install.is_some()
                && prompt::confirm(&format!("Also sign in Claude Desktop for '{label}'?"), true)?;
            // One profile's failed sign-in should not abort the others.
            if code::resolve_claude(cfg).is_ok()
                && let Err(err) = login::login_code(cfg, &name)
            {
                ui::fail(format!("{err:#}"));
            }
            if desktop_too && let Err(err) = login::login_desktop(cfg, &name, true) {
                ui::fail(format!("{err:#}"));
            }
        }
    }
    Ok(())
}
