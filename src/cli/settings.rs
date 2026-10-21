//! `cswitch config` and the interactive settings menu.

use anyhow::Result;

use super::ConfigAction;
use super::common::interactive;
use crate::config::{CodeLaunch, Config, DeepLinkPolicy, EnvIsolation, LaunchMethod, paths};
use crate::core::sys;
use crate::ui::{self, prompt, theme};

pub fn run(cfg: &mut Config, action: Option<ConfigAction>) -> Result<()> {
    match action {
        Some(ConfigAction::Path) => {
            println!("{}", paths::config_file()?.display());
            Ok(())
        }
        Some(ConfigAction::Show) => {
            println!("{}", toml::to_string_pretty(cfg)?);
            Ok(())
        }
        Some(ConfigAction::Edit) => {
            if !Config::exists() {
                cfg.save()?;
            }
            sys::edit_file(&paths::config_file()?)?;
            match Config::load() {
                Ok(_) => ui::ok("Configuration is valid"),
                Err(err) => ui::fail(format!("{err:#}")),
            }
            Ok(())
        }
        Some(ConfigAction::Theme { name: Some(name) }) => {
            if !theme::THEMES
                .iter()
                .any(|t| t.name.eq_ignore_ascii_case(&name))
            {
                let names: Vec<&str> = theme::THEMES.iter().map(|t| t.name).collect();
                anyhow::bail!("unknown theme '{name}' (available: {})", names.join(", "));
            }
            cfg.theme = name.to_lowercase();
            cfg.save()?;
            theme::set_theme(&cfg.theme);
            ui::ok(format!("Theme set to {}", ui::accent(&cfg.theme)));
            Ok(())
        }
        Some(ConfigAction::Theme { name: None }) => pick_theme(cfg),
        None if interactive() => menu(cfg),
        None => {
            println!("{}", paths::config_file()?.display());
            Ok(())
        }
    }
}

pub fn pick_theme(cfg: &mut Config) -> Result<()> {
    let items: Vec<String> = theme::THEMES
        .iter()
        .map(|t| {
            let p = t.palette;
            let swatch: String = [p.accent, p.accent2, p.success, p.warning, p.error, p.info]
                .iter()
                .map(|c| c.style().apply_to(ui::g("■", "#")).to_string())
                .collect();
            format!("{swatch}  {:<8} {}", t.name, ui::dim(t.description))
        })
        .collect();
    let current = theme::THEMES
        .iter()
        .position(|t| t.name == cfg.theme)
        .unwrap_or(0);
    if let Some(i) = prompt::select("Theme", &items, current)? {
        cfg.theme = theme::THEMES[i].name.to_string();
        theme::set_theme(&cfg.theme);
        cfg.save()?;
        ui::ok(format!("Theme set to {}", ui::accent(&cfg.theme)));
    }
    Ok(())
}

fn on_off(value: bool) -> String {
    if value {
        ui::success("on")
    } else {
        ui::dim("off")
    }
}

pub fn menu(cfg: &mut Config) -> Result<()> {
    loop {
        ui::section("Settings");
        let s = &cfg.settings;
        let d = &cfg.desktop;
        let items = vec![
            format!("Theme                          {}", ui::accent2(&cfg.theme)),
            format!(
                "Unclear claude:// links        {}",
                ui::accent2(s.deep_link_policy.describe())
            ),
            format!(
                "Sign-in routing window         {}",
                ui::accent2(format!("{} min", s.login_window_minutes))
            ),
            format!(
                "Environment isolation          {}",
                ui::accent2(s.env_isolation.describe())
            ),
            format!(
                "Always-kept variables          {}",
                ui::accent2(if s.env_passthrough.is_empty() {
                    "—".to_string()
                } else {
                    s.env_passthrough.join(", ")
                })
            ),
            format!(
                "Claude Code opens in           {}",
                ui::accent2(match s.code_launch {
                    CodeLaunch::Here => "this terminal",
                    CodeLaunch::NewWindow => "a new tab / window",
                })
            ),
            format!("Profile badge before Claude    {}", on_off(s.show_badge)),
            format!("Unicode glyphs                 {}", on_off(s.fancy_glyphs)),
            format!(
                "claude executable              {}",
                ui::accent2(
                    cfg.code
                        .claude_path
                        .clone()
                        .unwrap_or_else(|| "auto-detect".into())
                )
            ),
            format!(
                "Claude Desktop executable      {}",
                ui::accent2(d.exe_path.clone().unwrap_or_else(|| "auto-detect".into()))
            ),
            format!(
                "Desktop launch method          {}",
                ui::accent2(d.launch_method.to_string())
            ),
            format!(
                "Desktop extra switches         {}",
                ui::accent2(if d.extra_args.is_empty() {
                    "—".to_string()
                } else {
                    d.extra_args.join(" ")
                })
            ),
            format!(
                "claude:// link router          {}",
                on_off(d.router_enabled)
            ),
            format!("{} Open config file in editor", ui::g("✎", "*")),
            ui::dim(format!("{} Back", ui::g("←", "<"))),
        ];
        let Some(choice) = prompt::select("Settings", &items, 0)? else {
            return Ok(());
        };
        match choice {
            0 => pick_theme(cfg)?,
            1 => {
                let labels: Vec<&str> = DeepLinkPolicy::ALL.iter().map(|p| p.describe()).collect();
                let cur = DeepLinkPolicy::ALL
                    .iter()
                    .position(|p| *p == cfg.settings.deep_link_policy)
                    .unwrap_or(0);
                if let Some(i) = prompt::select(
                    "When it's unclear which profile a claude:// link is for",
                    &labels,
                    cur,
                )? {
                    cfg.settings.deep_link_policy = DeepLinkPolicy::ALL[i];
                }
            }
            2 => {
                let value = prompt::text_validated(
                    "Minutes a sign-in keeps routing links to its profile",
                    &cfg.settings.login_window_minutes.to_string(),
                    |v| {
                        v.trim()
                            .parse::<u64>()
                            .map(|_| ())
                            .map_err(|_| "enter a number".to_string())
                    },
                )?;
                cfg.settings.login_window_minutes = value.parse().unwrap_or(10).clamp(1, 120);
            }
            3 => {
                let labels: Vec<&str> = EnvIsolation::ALL.iter().map(|p| p.describe()).collect();
                let cur = EnvIsolation::ALL
                    .iter()
                    .position(|p| *p == cfg.settings.env_isolation)
                    .unwrap_or(0);
                if let Some(i) = prompt::select("Environment isolation", &labels, cur)? {
                    cfg.settings.env_isolation = EnvIsolation::ALL[i];
                }
            }
            4 => {
                let text = prompt::text(
                    "Variables kept despite isolation (comma-separated)",
                    &cfg.settings.env_passthrough.join(", "),
                )?;
                cfg.settings.env_passthrough = text
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            5 => {
                let labels = [
                    "This terminal (replaces the menu)",
                    "A new terminal tab / window",
                ];
                let cur = usize::from(cfg.settings.code_launch == CodeLaunch::NewWindow);
                if let Some(i) = prompt::select("Where should Claude Code open?", &labels, cur)? {
                    cfg.settings.code_launch = if i == 0 {
                        CodeLaunch::Here
                    } else {
                        CodeLaunch::NewWindow
                    };
                }
            }
            6 => cfg.settings.show_badge = !cfg.settings.show_badge,
            7 => {
                cfg.settings.fancy_glyphs = !cfg.settings.fancy_glyphs;
                ui::set_fancy_glyphs(cfg.settings.fancy_glyphs);
            }
            8 => {
                let text = prompt::text(
                    "Path to claude (empty = auto-detect)",
                    cfg.code.claude_path.as_deref().unwrap_or(""),
                )?;
                cfg.code.claude_path = (!text.is_empty()).then_some(text);
            }
            9 => {
                let text = prompt::text(
                    "Path to Claude Desktop (empty = auto-detect)",
                    cfg.desktop.exe_path.as_deref().unwrap_or(""),
                )?;
                cfg.desktop.exe_path = (!text.is_empty()).then_some(text);
                crate::core::sys::remove_state("desktop-install.json");
            }
            10 => {
                let labels = [
                    "auto — best method for the installation (recommended)",
                    "direct — start the executable (keeps per-profile environment)",
                    "activate — Windows Store activation (package identity)",
                ];
                let methods = [
                    LaunchMethod::Auto,
                    LaunchMethod::Direct,
                    LaunchMethod::Activate,
                ];
                let cur = methods
                    .iter()
                    .position(|m| *m == cfg.desktop.launch_method)
                    .unwrap_or(0);
                if let Some(i) =
                    prompt::select("How should Claude Desktop be started?", &labels, cur)?
                {
                    cfg.desktop.launch_method = methods[i];
                }
            }
            11 => {
                let text = prompt::text(
                    "Extra switches for every Desktop instance",
                    &sys::join_command(&cfg.desktop.extra_args),
                )?;
                match sys::split_command(&text) {
                    Ok(args) => cfg.desktop.extra_args = args,
                    Err(err) => ui::fail(format!("{err:#} — switches left unchanged")),
                }
            }
            12 => {
                cfg.desktop.router_enabled = !cfg.desktop.router_enabled;
                if cfg.desktop.router_enabled {
                    let _ = crate::core::router::register();
                } else {
                    let _ = crate::core::router::unregister();
                }
            }
            13 => {
                cfg.save()?;
                sys::edit_file(&paths::config_file()?)?;
                *cfg = Config::load()?;
                continue;
            }
            _ => return Ok(()),
        }
        cfg.save()?;
    }
}
