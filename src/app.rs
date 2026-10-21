//! Process entry: figures out *how* we were started (menu, command, `claude-<profile>` shim,
//! `$BROWSER` opener, `claude://` handler) and dispatches.

use std::ffi::OsString;
use std::path::Path;

use anyhow::Result;
use clap::{CommandFactory, Parser};

use crate::cli::{self, Cli, Commands};
use crate::config::Config;
use crate::core::{browser, router, sys};
use crate::ui;

pub fn main() {
    let args: Vec<String> = std::env::args_os()
        .map(|a: OsString| a.to_string_lossy().to_string())
        .collect();
    let stem = args
        .first()
        .and_then(|a| {
            Path::new(a)
                .file_stem()
                .map(|s| s.to_string_lossy().to_lowercase())
        })
        .unwrap_or_default();
    let windowed = stem == "cswitchw";
    ui::theme::init_color_support();
    let cfg_theme = Config::load().ok();
    if let Some(cfg) = &cfg_theme {
        ui::theme::set_theme(&cfg.theme);
        ui::set_fancy_glyphs(cfg.settings.fancy_glyphs);
    }

    let outcome = dispatch(&args, &stem, windowed);
    let code = match outcome {
        Ok(code) => code,
        Err(err) => {
            report_error(&err, windowed);
            1
        }
    };
    #[cfg(windows)]
    if !windowed && args.len() <= 1 && sys::win::owns_console() {
        // Started by double-click: keep the window open long enough to read the last message.
        ui::prompt::pause("Press Enter to close");
    }
    std::process::exit(code);
}

fn dispatch(args: &[String], stem: &str, windowed: bool) -> Result<i32> {
    // `claude-<profile> …` — hard link / symlink to this binary.
    if let Some(profile) = stem.strip_prefix("claude-") {
        return cli::code_cmd::run_shim(profile, args[1..].to_vec());
    }
    // `$BROWSER <url>` — Claude Code opening a sign-in page.
    if args.len() == 2 && browser::is_web_url(&args[1]) {
        cli::code_cmd::open_url(None, &args[1])?;
        return Ok(0);
    }
    // `cswitch "claude://…"` — some launchers pass the link without the subcommand.
    if args.len() == 2
        && args[1]
            .trim_matches('"')
            .to_lowercase()
            .starts_with("claude:")
    {
        router::handle(&args[1], windowed)?;
        return Ok(0);
    }
    if windowed && args.len() <= 1 {
        #[cfg(windows)]
        sys::win::message_box(
            "Claude Switch",
            "cswitchw.exe works in the background (links and sign-in pages).\nOpen \"Claude Switch\" from the Start menu or run cswitch.exe instead.",
            false,
        );
        return Ok(0);
    }

    let parsed = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(err) => {
            if windowed {
                sys::log(format!("windowed invocation with bad arguments: {args:?}"));
                return Ok(2);
            }
            err.exit();
        }
    };
    let mut cfg = Config::load()?;
    match parsed.command {
        None => cli::menu::run(&mut cfg),
        Some(command) => run_command(&mut cfg, command, windowed),
    }
}

fn run_command(cfg: &mut Config, command: Commands, windowed: bool) -> Result<i32> {
    match command {
        Commands::Code(args) => return cli::code_cmd::run(cfg, args),
        Commands::Desktop(args) => cli::desktop_cmd::run(cfg, args)?,
        Commands::Login(args) => cli::login::login(cfg, args)?,
        Commands::Logout(args) => cli::login::logout(cfg, args)?,
        Commands::Status(args) => cli::status::run(cfg, args)?,
        Commands::Profile(args) => cli::profiles::run(cfg, args.action)?,
        Commands::Shell(args) => return cli::code_cmd::shell(cfg, args.profile),
        Commands::Env(args) => cli::code_cmd::env_exports(cfg, args)?,
        Commands::Copy(args) => cli::profiles::copy(cfg, args)?,
        Commands::Doctor(args) => {
            let problems = cli::doctor::run(cfg, args)?;
            return Ok(i32::from(problems > 0));
        }
        Commands::Setup => cli::wizard::run(cfg)?,
        Commands::Install(args) => cli::install_cmd::install(cfg, args)?,
        Commands::Uninstall(args) => {
            cli::install_cmd::uninstall(cfg, args)?;
        }
        Commands::Router(args) => return cli::install_cmd::router_cmd(cfg, args.action),
        Commands::Config(args) => cli::settings::run(cfg, args.action)?,
        Commands::Completions { shell } => {
            let mut command = Cli::command();
            clap_complete::generate(shell, &mut command, "cswitch", &mut std::io::stdout());
        }
        Commands::OpenUrl { profile, url } => cli::code_cmd::open_url(profile, &url)?,
        Commands::Route { url } => router::handle(&url, windowed)?,
        Commands::Reassert => router::reassert_loop(),
        Commands::Procs { filter } => {
            if let Some(install) = crate::core::desktop::detect(cfg, false) {
                println!(
                    "install: {:?} root={:?} exe={:?}",
                    install.kind, install.root, install.exe
                );
                println!(
                    "normalized root={:?}",
                    crate::config::normalize_path(&install.root)
                );
            }
            for line in crate::core::desktop::debug_processes(&filter) {
                println!("{line}");
            }
        }
    }
    Ok(0)
}

fn report_error(err: &anyhow::Error, windowed: bool) {
    sys::log(format!("error: {err:#}"));
    if windowed {
        #[cfg(windows)]
        sys::win::message_box("Claude Switch", &format!("{err:#}"), true);
        return;
    }
    eprintln!();
    eprintln!(
        "  {} {}",
        ui::error(ui::g("✖", "[x]")),
        ui::error(err.to_string())
    );
    for cause in err.chain().skip(1) {
        eprintln!("    {}", ui::dim(format!("caused by: {cause}")));
    }
}
