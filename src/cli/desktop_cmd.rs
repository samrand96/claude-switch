//! `cswitch desktop`: start, focus, stop and restart Claude Desktop per profile.

use anyhow::Result;

use super::DesktopArgs;
use super::common::{require_desktop, resolve_profile};
use crate::config::Config;
use crate::core::desktop::{self, DesktopInstall};
use crate::core::router::{self, RouterState};
use crate::ui;

pub fn run(cfg: &Config, args: DesktopArgs) -> Result<()> {
    let install = require_desktop(cfg)?;
    if args.all {
        for name in cfg.profile_names() {
            if args.stop || args.restart {
                stop(cfg, &install, &name)?;
            }
            if !args.stop {
                start(cfg, &install, &name)?;
            }
        }
        return Ok(());
    }
    let question = if args.stop {
        "Stop Claude Desktop for which profile?"
    } else {
        "Open Claude Desktop as which profile?"
    };
    let name = resolve_profile(cfg, args.profile.as_deref(), question)?;
    if args.stop || args.restart {
        stop(cfg, &install, &name)?;
        if args.stop {
            return Ok(());
        }
    }
    start(cfg, &install, &name)
}

pub fn start(cfg: &Config, install: &DesktopInstall, name: &str) -> Result<()> {
    let report = desktop::launch(cfg, install, name, None)?;
    let badge = ui::badge(name, cfg.profiles.get(name));
    if report.already_running {
        ui::ok(format!(
            "{badge} Claude Desktop is already running — brought it to the front"
        ));
    } else {
        let how = match report.method {
            "activate" => " (via Windows Store activation)",
            _ => "",
        };
        ui::ok(format!("{badge} Claude Desktop started{how}"));
    }
    if cfg.desktop.router_enabled
        && matches!(
            router::state(),
            RouterState::NotRegistered | RouterState::NotDefault { .. }
        )
    {
        ui::hint(
            "Tip: sign-in links are not routed per profile yet — run `cswitch doctor` to fix it.",
        );
    }
    Ok(())
}

pub fn stop(cfg: &Config, install: &DesktopInstall, name: &str) -> Result<()> {
    let killed = desktop::stop(cfg, install, name)?;
    let badge = ui::badge(name, cfg.profiles.get(name));
    if killed == 0 {
        ui::note(format!("{badge} Claude Desktop was not running"));
    } else {
        ui::ok(format!("{badge} Claude Desktop stopped"));
    }
    Ok(())
}
