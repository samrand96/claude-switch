//! `cswitch profile …`, `cswitch copy` and the interactive profiles menu.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};

use super::common::{interactive, pick_profile, profile_item, resolve_profile};
use super::editor;
use super::{CopyArgs, ProfileAction};
use crate::config::{Config, Storage, paths, validate_profile_name};
use crate::core::{browser, code, desktop, integrate, sys};
use crate::ui::{self, prompt};

pub fn run(cfg: &mut Config, action: Option<ProfileAction>) -> Result<()> {
    match action {
        None if interactive() => menu(cfg),
        None | Some(ProfileAction::List) => {
            list(cfg);
            Ok(())
        }
        Some(ProfileAction::Add { name }) => add(cfg, name).map(|_| ()),
        Some(ProfileAction::Edit { name }) => {
            let name = resolve_profile(cfg, name.as_deref(), "Edit which profile?")?;
            editor::edit_profile(cfg, &name)?;
            refresh_integrations(cfg);
            Ok(())
        }
        Some(ProfileAction::Show { name }) => {
            let name = resolve_profile(cfg, name.as_deref(), "Show which profile?")?;
            show(cfg, &name)
        }
        Some(ProfileAction::Rename { from, to }) => rename(cfg, &from, &to),
        Some(ProfileAction::Default { name }) => set_default(cfg, &name),
        Some(ProfileAction::Remove { name, purge, yes }) => {
            // Never fall back to the default profile for a deletion.
            if name.is_none() && !interactive() {
                bail!("name the profile to remove: cswitch profile remove <name>");
            }
            let name = resolve_profile(cfg, name.as_deref(), "Remove which profile?")?;
            remove(cfg, &name, purge, yes)
        }
        Some(ProfileAction::Open { name }) => {
            let name = resolve_profile(
                cfg,
                name.as_deref(),
                "Open the data folder of which profile?",
            )?;
            let root = paths::profile_root(&name)?;
            fs::create_dir_all(&root)?;
            sys::reveal(&root)
        }
    }
}

pub fn list(cfg: &Config) {
    if cfg.profiles.is_empty() {
        ui::note("No profiles yet — run `cswitch setup`.");
        return;
    }
    for name in cfg.profile_names() {
        println!(
            "  {}  {}",
            profile_item(cfg, &name),
            ui::dim(format!("({name})"))
        );
    }
}

pub fn add(cfg: &mut Config, name: Option<String>) -> Result<Option<String>> {
    let name = match name {
        Some(n) => {
            validate_profile_name(&n)?;
            if cfg.profiles.contains_key(&n) {
                bail!("profile '{n}' already exists");
            }
            n
        }
        None => match editor::ask_new_name(cfg, "")? {
            Some(n) => n,
            None => return Ok(None),
        },
    };
    let profile = editor::create_profile(cfg, &name, cfg.profiles.len())?;
    cfg.profiles.insert(name.clone(), profile);
    if cfg.default_profile.is_none() {
        cfg.default_profile = Some(name.clone());
    }
    cfg.save()?;
    ui::ok(format!(
        "Profile {} created",
        ui::badge(&name, cfg.profiles.get(&name))
    ));
    refresh_integrations(cfg);
    Ok(Some(name))
}

pub fn show(cfg: &Config, name: &str) -> Result<()> {
    let profile = cfg.profile(name)?;
    let install = desktop::detect(cfg, false);
    ui::section(format!("Profile {}", ui::badge(name, Some(profile))));
    let row = |k: &str, v: String| println!("  {}  {}", ui::dim(format!("{k:<22}")), v);
    row("name", name.to_string());
    row(
        "account email",
        profile.email.clone().unwrap_or_else(|| "—".into()),
    );
    row("sign-in method", profile.login.describe().to_string());
    row(
        "Claude Code data",
        cfg.effective_code_dir(name)?.display().to_string(),
    );
    row(
        "Claude Code account",
        code::account(cfg, name)
            .map(|a| a.summary())
            .unwrap_or_else(|| "not signed in".into()),
    );
    row(
        "Claude Desktop data",
        desktop::effective_data_dir(cfg, install.as_ref(), name)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "—".into()),
    );
    row("sign-in browser", browser::describe(&profile.browser));
    row(
        "command",
        format!("claude-{name}  (or: cswitch code {name})"),
    );
    if !profile.env.is_empty() {
        row(
            "environment",
            profile.env.keys().cloned().collect::<Vec<_>>().join(", "),
        );
    }
    Ok(())
}

pub fn set_default(cfg: &mut Config, name: &str) -> Result<()> {
    cfg.profile(name)?;
    cfg.default_profile = Some(name.to_string());
    cfg.save()?;
    ui::ok(format!(
        "{} is now the default profile",
        ui::badge(name, cfg.profiles.get(name))
    ));
    Ok(())
}

pub fn rename(cfg: &mut Config, from: &str, to: &str) -> Result<()> {
    cfg.profile(from)?;
    validate_profile_name(to)?;
    if cfg.profiles.contains_key(to) {
        bail!("profile '{to}' already exists");
    }
    // Only isolated Desktop data lives in (and moves with) the profile folder.
    let moves_desktop_data = cfg.profile(from)?.desktop_storage == Storage::Isolated;
    if moves_desktop_data
        && let Some(install) = desktop::detect(cfg, false)
        && desktop::running_profiles(cfg, &install)
            .iter()
            .any(|p| p == from)
    {
        bail!("close the '{from}' Claude Desktop window first (`cswitch desktop {from} --stop`)");
    }
    let old_root = paths::profile_root(from)?;
    let new_root = paths::profile_root(to)?;
    if old_root.exists() {
        fs::rename(&old_root, &new_root)
            .with_context(|| format!("could not move {}", old_root.display()))?;
    }
    let profile = cfg.profiles.remove(from).context("profile vanished")?;
    cfg.profiles.insert(to.to_string(), profile);
    if cfg.default_profile.as_deref() == Some(from) {
        cfg.default_profile = Some(to.to_string());
    }
    cfg.save()?;
    ui::ok(format!(
        "Renamed '{from}' → {}",
        ui::badge(to, cfg.profiles.get(to))
    ));
    if cfg!(target_os = "macos") {
        ui::hint(
            "macOS keys Claude Code's keychain entry to the folder path — sign Claude Code in again: `cswitch login --code`.",
        );
    }
    refresh_integrations(cfg);
    Ok(())
}

pub fn remove(cfg: &mut Config, name: &str, purge: bool, yes: bool) -> Result<()> {
    cfg.profile(name)?;
    let root = paths::profile_root(name)?;
    let mut purge = purge;
    if !yes && !interactive() {
        bail!("add --yes to remove profile '{name}' without a prompt");
    }
    if !yes {
        if !prompt::confirm(
            &format!("Remove profile '{}'?", cfg.display_name(name)),
            false,
        )? {
            return Ok(());
        }
        if !purge && root.exists() {
            purge = prompt::confirm(
                &format!(
                    "Also delete its data ({})?",
                    sys::human_size(sys::dir_size(&root))
                ),
                false,
            )?;
        }
    }
    // Stop the instance only when its data is about to be deleted. A "system" or custom
    // Desktop folder is never deleted — and stopping a "system" profile would close the
    // user's normal Claude Desktop.
    if purge
        && cfg.profile(name)?.desktop_storage == Storage::Isolated
        && let Some(install) = desktop::detect(cfg, false)
    {
        let _ = desktop::stop(cfg, &install, name);
    }
    cfg.profiles.remove(name);
    if cfg.default_profile.as_deref() == Some(name) {
        cfg.default_profile = cfg.profiles.keys().next().cloned();
    }
    cfg.save()?;
    if purge && root.exists() {
        fs::remove_dir_all(&root)
            .with_context(|| format!("could not delete {}", root.display()))?;
        ui::ok(format!("Deleted {}", root.display()));
    }
    ui::ok(format!("Profile '{name}' removed"));
    refresh_integrations(cfg);
    Ok(())
}

/// Keeps `claude-<profile>` commands and shortcuts in sync after profile changes.
pub fn refresh_integrations(cfg: &Config) {
    if paths::installed_exe().map(|p| p.exists()).unwrap_or(false) {
        let _ = integrate::sync_shims(cfg);
        if integrate::shortcuts_dir()
            .map(|d| d.exists())
            .unwrap_or(false)
        {
            let _ = integrate::create_shortcuts(cfg, desktop::detect(cfg, false).as_ref());
        }
    }
}

fn menu(cfg: &mut Config) -> Result<()> {
    loop {
        ui::section("Profiles");
        list(cfg);
        println!();
        let items = vec![
            format!("{} Add a profile", ui::g("✚", "+")),
            format!("{} Edit a profile", ui::g("✎", "*")),
            format!("{} Copy settings between profiles", ui::g("⇄", "~")),
            format!("{} Set the default profile", ui::g("★", "*")),
            format!("{} Rename a profile", ui::g("✎", "*")),
            format!("{} Remove a profile", ui::g("✖", "x")),
            format!("{} Open a profile's data folder", ui::g("▤", "#")),
            ui::dim(format!("{} Back", ui::g("←", "<"))),
        ];
        let Some(choice) = prompt::select("Profiles", &items, 0)? else {
            return Ok(());
        };
        let result = match choice {
            0 => add(cfg, None).map(|_| ()),
            1 => match pick_profile(cfg, "Edit which profile?")? {
                Some(n) => editor::edit_profile(cfg, &n).map(|_| refresh_integrations(cfg)),
                None => Ok(()),
            },
            2 => copy(
                cfg,
                CopyArgs {
                    from: None,
                    to: None,
                },
            ),
            3 => match pick_profile(cfg, "Default profile")? {
                Some(n) => set_default(cfg, &n),
                None => Ok(()),
            },
            4 => match pick_profile(cfg, "Rename which profile?")? {
                Some(n) => match editor::ask_new_name(cfg, "")? {
                    Some(to) => rename(cfg, &n, &to),
                    None => Ok(()),
                },
                None => Ok(()),
            },
            5 => match pick_profile(cfg, "Remove which profile?")? {
                Some(n) => remove(cfg, &n, false, false),
                None => Ok(()),
            },
            6 => match pick_profile(cfg, "Open which profile's folder?")? {
                Some(n) => {
                    let root = paths::profile_root(&n)?;
                    fs::create_dir_all(&root)?;
                    sys::reveal(&root)
                }
                None => Ok(()),
            },
            _ => return Ok(()),
        };
        if let Err(err) = result {
            ui::fail(format!("{err:#}"));
        }
    }
}

/// Something that can be copied from one profile to another.
struct CopyItem {
    label: String,
    kind: CopyKind,
    default: bool,
}

enum CopyKind {
    Path(&'static str),
    McpServers,
    DesktopConfig,
}

const CODE_ITEMS: [(&str, &str, bool); 9] = [
    (
        "settings.json",
        "Claude Code settings (permissions, hooks, model, status line)",
        true,
    ),
    (
        "CLAUDE.md",
        "Personal memory / instructions (CLAUDE.md)",
        true,
    ),
    ("agents", "Custom sub-agents", true),
    ("commands", "Custom slash commands", true),
    ("skills", "Skills", true),
    ("output-styles", "Output styles", true),
    ("keybindings.json", "Key bindings", true),
    ("plugins", "Installed plugins & marketplaces", false),
    ("statusline.sh", "Status line script", true),
];

struct Side {
    label: String,
    code_dir: PathBuf,
    code_state: PathBuf,
    desktop_dir: Option<PathBuf>,
}

fn side(cfg: &Config, name: &str) -> Result<Side> {
    if name == "system" {
        return Ok(Side {
            label: "your original Claude setup".to_string(),
            code_dir: paths::system_code_dir()?,
            code_state: paths::system_code_state_file()?,
            desktop_dir: desktop::stock_data_dir(desktop::detect(cfg, false).as_ref()),
        });
    }
    Ok(Side {
        label: cfg.display_name(name),
        code_dir: cfg.effective_code_dir(name)?,
        code_state: cfg.code_state_file(name)?,
        desktop_dir: desktop::effective_data_dir(cfg, desktop::detect(cfg, false).as_ref(), name),
    })
}

fn mcp_servers(state_file: &Path) -> Map<String, Value> {
    fs::read_to_string(state_file)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|v| v.get("mcpServers").and_then(Value::as_object).cloned())
        .unwrap_or_default()
}

/// Copies selected, non-identity settings. Credentials, account info and history never move.
pub fn copy(cfg: &Config, args: CopyArgs) -> Result<()> {
    let from = match args.from {
        Some(f) if f == "system" => f,
        Some(f) => {
            cfg.profile(&f)?;
            f
        }
        None => {
            let mut names = vec!["system".to_string()];
            names.extend(cfg.profile_names());
            let items: Vec<String> = names
                .iter()
                .map(|n| {
                    if n == "system" {
                        "Original Claude setup (~/.claude, Claude Desktop)".to_string()
                    } else {
                        profile_item(cfg, n)
                    }
                })
                .collect();
            match prompt::select("Copy settings FROM", &items, 0)? {
                Some(i) => names[i].clone(),
                None => return Ok(()),
            }
        }
    };
    let to = match args.to {
        Some(t) => {
            cfg.profile(&t)?;
            t
        }
        None => match pick_profile(cfg, "Copy settings TO")? {
            Some(t) => t,
            None => return Ok(()),
        },
    };
    if from == to {
        bail!("source and destination are the same profile");
    }
    let src = side(cfg, &from)?;
    let dst = side(cfg, &to)?;
    if src.code_dir == dst.code_dir {
        bail!("both profiles use the same Claude Code folder — nothing to copy");
    }

    let mut items: Vec<CopyItem> = CODE_ITEMS
        .iter()
        .filter(|(path, _, _)| src.code_dir.join(path).exists())
        .map(|(path, label, default)| CopyItem {
            label: label.to_string(),
            kind: CopyKind::Path(path),
            default: *default,
        })
        .collect();
    let servers = mcp_servers(&src.code_state);
    if !servers.is_empty() {
        items.push(CopyItem {
            label: format!(
                "Claude Code MCP servers ({})",
                servers.keys().cloned().collect::<Vec<_>>().join(", ")
            ),
            kind: CopyKind::McpServers,
            default: true,
        });
    }
    let desktop_cfg = src
        .desktop_dir
        .as_ref()
        .map(|d| d.join("claude_desktop_config.json"));
    if desktop_cfg.as_ref().is_some_and(|p| p.exists())
        && src.desktop_dir != dst.desktop_dir
        && dst.desktop_dir.is_some()
    {
        items.push(CopyItem {
            label: "Claude Desktop MCP servers & settings (claude_desktop_config.json)".to_string(),
            kind: CopyKind::DesktopConfig,
            default: true,
        });
    }
    if items.is_empty() {
        ui::note(format!("Nothing to copy from {}.", src.label));
        return Ok(());
    }
    ui::section(format!("Copy {} → {}", src.label, dst.label));
    ui::hint("Sign-ins, account info and history are never copied.");
    let labels: Vec<String> = items.iter().map(|i| i.label.clone()).collect();
    let defaults: Vec<bool> = items.iter().map(|i| i.default).collect();
    let selected = if interactive() {
        match prompt::multi(
            "Select what to copy (space toggles, enter confirms)",
            &labels,
            &defaults,
        )? {
            Some(s) => s,
            None => return Ok(()),
        }
    } else {
        (0..items.len()).filter(|i| defaults[*i]).collect()
    };
    fs::create_dir_all(&dst.code_dir)?;
    for index in selected {
        let item = &items[index];
        match item.kind {
            CopyKind::Path(path) => {
                let from_path = src.code_dir.join(path);
                let to_path = dst.code_dir.join(path);
                backup(&to_path)?;
                if from_path.is_dir() {
                    sys::copy_dir_recursive(&from_path, &to_path)?;
                } else {
                    fs::copy(&from_path, &to_path)?;
                }
            }
            CopyKind::McpServers => merge_mcp(&servers, &dst.code_state)?,
            CopyKind::DesktopConfig => {
                if let (Some(from_file), Some(to_dir)) = (&desktop_cfg, &dst.desktop_dir) {
                    fs::create_dir_all(to_dir)?;
                    let to_file = to_dir.join("claude_desktop_config.json");
                    backup(&to_file)?;
                    fs::copy(from_file, &to_file)?;
                }
            }
        }
        ui::ok(&item.label);
    }
    Ok(())
}

fn backup(path: &Path) -> Result<()> {
    if path.is_file() {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let bak = path.with_file_name(format!("{name}.bak-{}", sys::now_unix()));
        fs::copy(path, &bak)?;
    }
    Ok(())
}

fn merge_mcp(servers: &Map<String, Value>, state_file: &Path) -> Result<()> {
    // `.claude.json` also holds the sign-in (`oauthAccount`) and history: if it exists but cannot
    // be parsed, stop rather than replace it with just the MCP servers.
    let mut root: Value = match fs::read_to_string(state_file) {
        Ok(raw) => serde_json::from_str(raw.trim_start_matches('\u{feff}')).with_context(|| {
            format!(
                "{} is not valid JSON — fix it (or remove it) and copy again",
                state_file.display()
            )
        })?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Value::Object(Map::new()),
        Err(err) => {
            return Err(err).with_context(|| format!("could not read {}", state_file.display()));
        }
    };
    let obj = root
        .as_object_mut()
        .context("unexpected .claude.json format")?;
    let target = obj
        .entry("mcpServers")
        .or_insert_with(|| Value::Object(Map::new()));
    if let Some(map) = target.as_object_mut() {
        for (k, v) in servers {
            map.insert(k.clone(), v.clone());
        }
    }
    backup(state_file)?;
    if let Some(parent) = state_file.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(state_file, serde_json::to_string_pretty(&root)?)?;
    Ok(())
}

/// Storage that already holds a sign-in should not be silently shared by two profiles.
pub fn storage_conflicts(cfg: &Config) -> Vec<String> {
    let mut problems = Vec::new();
    let names = cfg.profile_names();
    for (i, a) in names.iter().enumerate() {
        for b in names.iter().skip(i + 1) {
            if let (Ok(x), Ok(y)) = (cfg.effective_code_dir(a), cfg.effective_code_dir(b))
                && crate::config::normalize_path(&x) == crate::config::normalize_path(&y)
            {
                problems.push(format!("'{a}' and '{b}' share the same Claude Code folder"));
            }
            let da = cfg.desktop_dir(a).ok().flatten();
            let db = cfg.desktop_dir(b).ok().flatten();
            let both_system = cfg
                .profiles
                .get(a)
                .map(|p| p.desktop_storage == Storage::System)
                .unwrap_or(false)
                && cfg
                    .profiles
                    .get(b)
                    .map(|p| p.desktop_storage == Storage::System)
                    .unwrap_or(false);
            let same_custom = matches!((&da, &db), (Some(x), Some(y)) if crate::config::normalize_path(x) == crate::config::normalize_path(y));
            if both_system || same_custom {
                problems.push(format!(
                    "'{a}' and '{b}' share the same Claude Desktop folder"
                ));
            }
        }
    }
    problems
}
