//! `cswitch status`: one line per profile — who is signed in, what is running, which browser.

use anyhow::Result;
use serde_json::json;

use super::StatusArgs;
use crate::config::Config;
use crate::core::code::{self, Credentials};
use crate::core::desktop;
use crate::core::{browser, router};
use crate::ui::{self, table::Table};

pub fn run(cfg: &Config, args: StatusArgs) -> Result<()> {
    if args.json {
        return print_json(cfg);
    }
    if cfg.profiles.is_empty() {
        ui::note("No profiles yet — run `cswitch setup`.");
        return Ok(());
    }
    print_table(cfg);
    Ok(())
}

pub fn code_cell(cfg: &Config, name: &str) -> String {
    let account = code::account(cfg, name);
    match (code::credentials(cfg, name), account) {
        (Credentials::Missing, _) => ui::dim(format!("{} not signed in", ui::g("○", "-"))),
        (_, Some(account)) => {
            let profile = cfg.profiles.get(name);
            let mismatch = code::email_matches(
                profile.and_then(|p| p.email.as_deref()),
                account.email.as_deref(),
            ) == Some(false);
            if mismatch {
                ui::error(format!(
                    "{} {} (wrong account)",
                    ui::g("✖", "x"),
                    account.summary()
                ))
            } else {
                ui::success(format!("{} {}", ui::g("✔", "+"), account.summary()))
            }
        }
        (Credentials::Present, None) => ui::success(format!("{} signed in", ui::g("✔", "+"))),
        (Credentials::Unknown, None) => ui::dim(format!("{} unknown", ui::g("?", "?"))),
    }
}

pub fn print_table(cfg: &Config) {
    let install = desktop::detect(cfg, false);
    let running = install
        .as_ref()
        .map(|i| desktop::running_profiles(cfg, i))
        .unwrap_or_default();
    let mut table = Table::new(["PROFILE", "CLAUDE CODE", "DESKTOP", "SIGN-IN BROWSER"]);
    for name in cfg.profile_names() {
        let profile = cfg.profiles.get(&name);
        let mut label = ui::badge(&name, profile);
        if cfg.default_profile.as_deref() == Some(name.as_str()) {
            label.push_str(&ui::dim(format!(" {}", ui::g("★", "*"))));
        }
        let desktop_cell = match &install {
            None => ui::dim("not installed"),
            Some(_) if running.contains(&name) => {
                ui::success(format!("{} running", ui::g("●", "*")))
            }
            Some(_) => ui::dim(format!("{} stopped", ui::g("○", "-"))),
        };
        let browser_cell = profile
            .map(|p| browser::describe(&p.browser))
            .unwrap_or_default();
        table.row([
            label,
            code_cell(cfg, &name),
            desktop_cell,
            ui::dim(browser_cell),
        ]);
    }
    table.print();
    if let Some(arm) = router::active_arm() {
        let left = arm.until.saturating_sub(crate::core::sys::now_unix()) / 60 + 1;
        ui::hint(format!(
            "Sign-in routing → {} for ~{left} more min",
            cfg.display_name(&arm.profile)
        ));
    }
}

fn print_json(cfg: &Config) -> Result<()> {
    let install = desktop::detect(cfg, false);
    let running = install
        .as_ref()
        .map(|i| desktop::running_profiles(cfg, i))
        .unwrap_or_default();
    let profiles: Vec<serde_json::Value> = cfg
        .profile_names()
        .into_iter()
        .map(|name| {
            let account = code::account(cfg, &name);
            let creds = code::credentials(cfg, &name);
            let profile = cfg.profiles.get(&name);
            json!({
                "name": name,
                "label": cfg.display_name(&name),
                "default": cfg.default_profile.as_deref() == Some(name.as_str()),
                "expected_email": profile.and_then(|p| p.email.clone()),
                "code": {
                    "config_dir": cfg.effective_code_dir(&name).ok(),
                    "signed_in": creds == Credentials::Present,
                    "email": account.as_ref().and_then(|a| a.email.clone()),
                    "organization": account.as_ref().and_then(|a| a.organization.clone()),
                },
                "desktop": {
                    "data_dir": desktop::effective_data_dir(cfg, install.as_ref(), &name),
                    "running": running.contains(&name),
                },
                "browser": profile.map(|p| browser::describe(&p.browser)),
            })
        })
        .collect();
    let out = json!({
        "profiles": profiles,
        "desktop_install": install.as_ref().map(|i| json!({
            "kind": i.kind.describe(),
            "exe": i.exe,
            "version": i.version,
        })),
        "router": format!("{:?}", router::state()),
    });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}
