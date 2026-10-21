//! `cswitch code`, `cswitch shell`, `cswitch env`, `claude-<profile>` and the $BROWSER opener.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};

use super::common::{interactive, resolve_profile, split_profile_and_args};
use super::{CodeArgs, EnvArgs, EnvShell};
use crate::config::{CodeLaunch, Config};
use crate::core::env::{self, Target};
use crate::core::{browser, code, sys, terminal};
use crate::ui;

pub fn run(cfg: &Config, args: CodeArgs) -> Result<i32> {
    let (profile, extra) = split_profile_and_args(cfg, args.profile, args.args);
    let name = resolve_profile(
        cfg,
        profile.as_deref(),
        "Open Claude Code as which profile?",
    )?;
    let cwd = match args.dir {
        Some(dir) => dir,
        None => std::env::current_dir().context("could not read the current directory")?,
    };
    let new_window = !args.here
        && (args.new_window || cfg.settings.code_launch == CodeLaunch::NewWindow && interactive());
    if new_window {
        let how = terminal::open_code_in_new_terminal(cfg, &name, &cwd, &extra)?;
        ui::ok(format!(
            "Claude Code for {} opened in a {how}",
            ui::badge(&name, cfg.profiles.get(&name))
        ));
        return Ok(0);
    }
    launch_here(cfg, &name, &extra, Some(&cwd))
}

/// Replaces this terminal with Claude Code for the profile (also used by `claude-<profile>`).
pub fn launch_here(
    cfg: &Config,
    name: &str,
    extra: &[String],
    cwd: Option<&std::path::Path>,
) -> Result<i32> {
    cfg.profile(name)?;
    let quiet = extra.iter().any(|a| {
        matches!(
            a.as_str(),
            "-p" | "--print" | "--version" | "-v" | "--output-format"
        )
    });
    if cfg.settings.show_badge && !quiet && console::Term::stderr().is_term() {
        let account = code::account(cfg, name)
            .map(|a| a.summary())
            .unwrap_or_else(|| "not signed in yet".into());
        eprintln!(
            "{} {} {}",
            ui::badge(name, cfg.profiles.get(name)),
            ui::dim(ui::g("·", "-")),
            ui::dim(format!("Claude Code · {account}"))
        );
        ui::set_terminal_title(&format!("Claude · {}", cfg.display_name(name)));
    }
    code::run_foreground(cfg, name, extra, cwd)
}

/// Busybox-style entry: the binary was invoked as `claude-<profile>`.
pub fn run_shim(profile: &str, args: Vec<String>) -> Result<i32> {
    let cfg = Config::load()?;
    if !cfg.profiles.contains_key(profile) {
        anyhow::bail!(
            "this command belongs to profile '{profile}', which no longer exists — run `cswitch install` to refresh commands"
        );
    }
    launch_here(&cfg, profile, &args, None)
}

/// `$BROWSER <url>` entry: Claude Code wants to open a page for the profile in CSWITCH_PROFILE.
pub fn open_url(profile: Option<String>, raw_url: &str) -> Result<()> {
    let url = browser::clean_url_arg(raw_url);
    let cfg = Config::load()?;
    let profile = profile
        .or_else(|| std::env::var("CSWITCH_PROFILE").ok())
        .filter(|p| cfg.profiles.contains_key(p))
        .or_else(|| cfg.fallback_profile());
    match profile {
        Some(name) => browser::open(&cfg, &name, &url),
        None => {
            sys::log("open-url: no profile context, using the system browser");
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
            Ok(())
        }
    }
}

pub fn shell(cfg: &Config, profile: Option<String>) -> Result<i32> {
    let name = resolve_profile(cfg, profile.as_deref(), "Open a shell for which profile?")?;
    if let Some(dir) = cfg.code_dir(&name)? {
        std::fs::create_dir_all(&dir)?;
    }
    let plan = env::plan(cfg, &name, Target::Code)?;
    let (program, args): (String, Vec<String>) = if cfg!(windows) {
        if std::env::var_os("PSModulePath").is_some() && sys::has_program("pwsh") {
            ("pwsh".into(), vec!["-NoLogo".into()])
        } else if std::env::var_os("PSModulePath").is_some() {
            ("powershell".into(), vec!["-NoLogo".into()])
        } else {
            (
                std::env::var("COMSPEC").unwrap_or_else(|_| "cmd".into()),
                vec![],
            )
        }
    } else {
        (
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()),
            vec![],
        )
    };
    ui::ok(format!(
        "Entering a {} shell — `claude` here runs as this profile. Type {} to leave.",
        ui::badge(&name, cfg.profiles.get(&name)),
        ui::accent("exit")
    ));
    let mut cmd = Command::new(&program);
    cmd.args(&args);
    plan.apply(&mut cmd);
    cmd.env("CSWITCH_SHELL", &name);
    #[cfg(windows)]
    sys::win::ignore_ctrl_c();
    let status = cmd
        .status()
        .with_context(|| format!("could not start {program}"))?;
    ui::note(format!("Left the {} shell.", cfg.display_name(&name)));
    Ok(status.code().unwrap_or(0))
}

pub fn env_exports(cfg: &Config, args: EnvArgs) -> Result<()> {
    let name = resolve_profile(
        cfg,
        args.profile.as_deref(),
        "Print the environment of which profile?",
    )?;
    let plan = env::plan(cfg, &name, Target::Code)?;
    let shell = args.shell.unwrap_or_else(detect_shell);
    for (key, value) in plan.exports() {
        println!("{}", render_export(shell, &key, value.as_deref()));
    }
    Ok(())
}

fn detect_shell() -> EnvShell {
    if cfg!(windows) {
        if std::env::var_os("PSModulePath").is_some() {
            EnvShell::Powershell
        } else {
            EnvShell::Cmd
        }
    } else {
        let shell = std::env::var("SHELL").unwrap_or_default();
        if shell.ends_with("fish") {
            EnvShell::Fish
        } else if shell.ends_with("zsh") {
            EnvShell::Zsh
        } else {
            EnvShell::Bash
        }
    }
}

pub fn render_export(shell: EnvShell, key: &str, value: Option<&str>) -> String {
    let single = |v: &str| format!("'{}'", v.replace('\'', "'\\''"));
    match (shell, value) {
        (EnvShell::Bash | EnvShell::Zsh, None) => format!("unset {key}"),
        (EnvShell::Bash | EnvShell::Zsh, Some(v)) => format!("export {key}={}", single(v)),
        (EnvShell::Fish, None) => format!("set -e {key}"),
        (EnvShell::Fish, Some(v)) => format!("set -gx {key} {}", single(v)),
        (EnvShell::Powershell, None) => {
            format!("Remove-Item Env:{key} -ErrorAction SilentlyContinue")
        }
        (EnvShell::Powershell, Some(v)) => format!("$env:{key} = '{}'", v.replace('\'', "''")),
        (EnvShell::Cmd, None) => format!("set {key}="),
        (EnvShell::Cmd, Some(v)) => format!("set \"{key}={v}\""),
    }
}

pub fn default_cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| dirs::home_dir().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_exports_per_shell() {
        assert_eq!(
            render_export(EnvShell::Bash, "A", Some("it's")),
            "export A='it'\\''s'"
        );
        assert_eq!(render_export(EnvShell::Fish, "A", None), "set -e A");
        assert_eq!(
            render_export(EnvShell::Powershell, "A", Some("x'y")),
            "$env:A = 'x''y'"
        );
        assert_eq!(render_export(EnvShell::Cmd, "A", Some("v")), "set \"A=v\"");
    }
}
