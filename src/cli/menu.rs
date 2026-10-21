//! The interactive home screen shown by `cswitch` with no arguments.

use anyhow::Result;

use super::common::{interactive, pick_profile};
use super::{
    DesktopArgs, DoctorArgs, LoginArgs, LogoutArgs, code_cmd, desktop_cmd, doctor, install_cmd,
    login, profiles, settings, status, wizard,
};
use crate::config::{Config, paths};
use crate::core::{desktop, terminal};
use crate::ui::{self, prompt};

enum Action {
    Code,
    CodeNewWindow,
    Desktop,
    DesktopAll,
    StopDesktop,
    Login,
    Logout,
    Profiles,
    Copy,
    Settings,
    Integrations,
    Doctor,
    Setup,
    Quit,
}

pub fn run(cfg: &mut Config) -> Result<i32> {
    if !interactive() {
        status::print_table(cfg);
        return Ok(0);
    }
    if cfg.profiles.is_empty() {
        wizard::run(cfg)?;
        if cfg.profiles.is_empty() {
            return Ok(0);
        }
    } else if !paths::installed_exe().map(|p| p.exists()).unwrap_or(false) {
        ui::banner("Work & personal Claude, side by side");
        ui::warn("Claude Switch isn't installed on this computer yet.");
        if prompt::confirm("Install it now (recommended)?", true)? {
            install_cmd::install(
                cfg,
                super::InstallArgs {
                    no_path: false,
                    no_shortcuts: false,
                    no_router: false,
                },
            )?;
            prompt::pause("Press Enter to continue");
        }
    }
    loop {
        let _ = console::Term::stdout().clear_screen();
        ui::banner("Work & personal Claude, side by side");
        println!();
        status::print_table(cfg);
        println!();
        let has_desktop = desktop::detect(cfg, false).is_some();
        let mut actions: Vec<(String, Action)> = vec![
            (
                format!("{}  Claude Code", ui::accent(ui::g("▶", ">"))),
                Action::Code,
            ),
            (
                format!(
                    "{}  Claude Code in a new tab / window",
                    ui::accent(ui::g("⧉", "+"))
                ),
                Action::CodeNewWindow,
            ),
        ];
        if has_desktop {
            actions.push((
                format!("{}  Claude Desktop", ui::accent(ui::g("▶", ">"))),
                Action::Desktop,
            ));
            if cfg.profiles.len() > 1 {
                actions.push((
                    format!(
                        "{}  Claude Desktop — all profiles",
                        ui::accent(ui::g("▶", ">"))
                    ),
                    Action::DesktopAll,
                ));
            }
            actions.push((
                format!("{}  Stop a Claude Desktop", ui::g("■", "x")),
                Action::StopDesktop,
            ));
        }
        actions.extend([
            (format!("{}  Sign in", ui::g("⚿", "*")), Action::Login),
            (format!("{}  Sign out", ui::g("⏏", "-")), Action::Logout),
            (format!("{}  Profiles", ui::g("☰", "=")), Action::Profiles),
            (
                format!("{}  Copy settings between profiles", ui::g("⇄", "~")),
                Action::Copy,
            ),
            (
                format!("{}  Settings & theme", ui::g("⚙", "*")),
                Action::Settings,
            ),
            (
                format!("{}  Install & integrations", ui::g("⬇", "v")),
                Action::Integrations,
            ),
            (format!("{}  Doctor", ui::g("✚", "+")), Action::Doctor),
            (
                format!("{}  Run the setup wizard again", ui::g("↺", "@")),
                Action::Setup,
            ),
            (ui::dim(format!("{}  Quit", ui::g("✖", "x"))), Action::Quit),
        ]);
        let labels: Vec<String> = actions.iter().map(|(l, _)| l.clone()).collect();
        let Some(choice) = prompt::select("What would you like to do?", &labels, 0)? else {
            return Ok(0);
        };
        let result: Result<Option<i32>> = match actions[choice].1 {
            Action::Quit => return Ok(0),
            Action::Code => match pick_profile(cfg, "Claude Code as which profile?")? {
                Some(name) => {
                    let _ = console::Term::stdout().clear_screen();
                    let code = code_cmd::launch_here(cfg, &name, &[], None)?;
                    return Ok(code);
                }
                None => Ok(None),
            },
            Action::CodeNewWindow => match pick_profile(cfg, "Claude Code as which profile?")? {
                Some(name) => {
                    let dir = prompt::text(
                        "Working directory",
                        &code_cmd::default_cwd().display().to_string(),
                    )?;
                    terminal::open_code_in_new_terminal(cfg, &name, std::path::Path::new(&dir), &[])
                        .map(|how| {
                            ui::ok(format!(
                                "Opened Claude Code for {} in a {how}",
                                ui::badge(&name, cfg.profiles.get(&name))
                            ));
                            None
                        })
                }
                None => Ok(None),
            },
            Action::Desktop => match pick_profile(cfg, "Claude Desktop as which profile?")? {
                Some(name) => desktop_cmd::run(
                    cfg,
                    DesktopArgs {
                        profile: Some(name),
                        all: false,
                        stop: false,
                        restart: false,
                    },
                )
                .map(|_| Some(0)),
                None => Ok(None),
            },
            Action::DesktopAll => desktop_cmd::run(
                cfg,
                DesktopArgs {
                    profile: None,
                    all: true,
                    stop: false,
                    restart: false,
                },
            )
            .map(|_| Some(0)),
            Action::StopDesktop => {
                match pick_profile(cfg, "Stop Claude Desktop for which profile?")? {
                    Some(name) => desktop_cmd::run(
                        cfg,
                        DesktopArgs {
                            profile: Some(name),
                            all: false,
                            stop: true,
                            restart: false,
                        },
                    )
                    .map(|_| Some(0)),
                    None => Ok(None),
                }
            }
            Action::Login => match pick_profile(cfg, "Sign in which profile?")? {
                Some(name) => login::login(
                    cfg,
                    LoginArgs {
                        profile: Some(name),
                        code: false,
                        desktop: false,
                        no_wait: false,
                    },
                )
                .map(|_| Some(0)),
                None => Ok(None),
            },
            Action::Logout => match pick_profile(cfg, "Sign out which profile?")? {
                Some(name) => login::logout(
                    cfg,
                    LogoutArgs {
                        profile: Some(name),
                        code: false,
                        desktop: false,
                        yes: false,
                    },
                )
                .map(|_| Some(0)),
                None => Ok(None),
            },
            Action::Profiles => profiles::run(cfg, None).map(|_| None),
            Action::Copy => profiles::copy(
                cfg,
                super::CopyArgs {
                    from: None,
                    to: None,
                },
            )
            .map(|_| Some(0)),
            Action::Settings => settings::menu(cfg).map(|_| None),
            Action::Integrations => install_cmd::menu(cfg).map(|_| None),
            Action::Doctor => doctor::run(
                cfg,
                DoctorArgs {
                    fix: false,
                    test_router: false,
                },
            )
            .map(|_| Some(0)),
            Action::Setup => wizard::run(cfg).map(|_| Some(0)),
        };
        match result {
            Ok(Some(_)) => {
                println!();
                prompt::pause("Press Enter to return to the menu");
            }
            Ok(None) => {}
            Err(err) => {
                ui::fail(format!("{err:#}"));
                prompt::pause("Press Enter to return to the menu");
            }
        }
        // Profiles may have changed in a sub-menu.
        if let Ok(fresh) = Config::load() {
            *cfg = fresh;
        }
    }
}
