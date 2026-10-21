//! Interactive profile editor and browser picker, shared by the wizard and the profiles menu.

use std::collections::BTreeMap;

use anyhow::Result;

use crate::config::{
    BrowserChoice, BrowserKind, BrowserMode, Config, LoginKind, Profile, Storage, expand_home,
    validate_profile_name,
};
use crate::core::{browser, sys};
use crate::ui::{self, prompt, theme::Rgb};

pub const COLORS: [(&str, &str); 9] = [
    ("Blue", "#3B82F6"),
    ("Claude orange", "#D97757"),
    ("Green", "#22C55E"),
    ("Purple", "#A855F7"),
    ("Pink", "#EC4899"),
    ("Teal", "#14B8A6"),
    ("Yellow", "#EAB308"),
    ("Red", "#EF4444"),
    ("Slate", "#94A3B8"),
];

pub fn suggested_color(name: &str, index: usize) -> String {
    match name {
        "work" | "job" | "office" | "company" => "#3B82F6".to_string(),
        "personal" | "home" | "me" | "private" => "#D97757".to_string(),
        _ => COLORS[(index + 2) % COLORS.len()].1.to_string(),
    }
}

pub fn title_case(name: &str) -> String {
    name.split(['-', '_'])
        .filter(|s| !s.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn ask_new_name(cfg: &Config, default: &str) -> Result<Option<String>> {
    let taken: Vec<String> = cfg.profiles.keys().cloned().collect();
    let name = prompt::text_validated(
        "Profile name (short, lowercase — e.g. work, personal)",
        default,
        move |v| {
            let v = v.trim();
            validate_profile_name(v).map_err(|e| e.to_string())?;
            if taken.iter().any(|t| t == v) {
                return Err(format!("'{v}' already exists"));
            }
            Ok(())
        },
    )?;
    Ok((!name.is_empty()).then_some(name))
}

/// Quick questions for a brand-new profile; returns the profile without saving it.
pub fn create_profile(cfg: &Config, name: &str, index: usize) -> Result<Profile> {
    let mut profile = Profile {
        label: title_case(name),
        color: suggested_color(name, index),
        ..Profile::default()
    };
    ui::section(format!("New profile · {}", ui::badge(name, Some(&profile))));
    profile.label = prompt::text("Display name", &profile.label)?;
    let email = prompt::text(
        "Account email for this profile (optional, pre-fills sign-in)",
        "",
    )?;
    profile.email = (!email.is_empty()).then_some(email);
    profile.color = pick_color(&profile.color)?;
    profile.login = pick_login_kind(profile.login)?;
    profile.code_storage = pick_storage(cfg, name, false, &Storage::Isolated)?;
    profile.desktop_storage = pick_storage(cfg, name, true, &Storage::Isolated)?;
    profile.browser = pick_browser(name, &profile.browser)?;
    Ok(profile)
}

pub fn pick_color(current: &str) -> Result<String> {
    let mut items: Vec<String> = COLORS
        .iter()
        .map(|(label, hex)| {
            let rgb = Rgb::parse(hex).unwrap_or(Rgb(255, 255, 255));
            format!("{} {label}", rgb.style().apply_to(ui::g("●", "*")))
        })
        .collect();
    items.push("Custom hex color…".to_string());
    let default = COLORS
        .iter()
        .position(|(_, h)| h.eq_ignore_ascii_case(current))
        .unwrap_or(0);
    match prompt::select("Profile color", &items, default)? {
        Some(i) if i < COLORS.len() => Ok(COLORS[i].1.to_string()),
        Some(_) => {
            let value = prompt::text_validated("Hex color (#RRGGBB)", current, |v| {
                Rgb::parse(v)
                    .map(|_| ())
                    .ok_or_else(|| "use the form #3B82F6".to_string())
            })?;
            Ok(Rgb::parse(&value)
                .map(Rgb::hex)
                .unwrap_or_else(|| current.to_string()))
        }
        None => Ok(current.to_string()),
    }
}

pub fn pick_login_kind(current: LoginKind) -> Result<LoginKind> {
    let items: Vec<&str> = LoginKind::ALL.iter().map(|k| k.describe()).collect();
    let default = LoginKind::ALL
        .iter()
        .position(|k| *k == current)
        .unwrap_or(0);
    Ok(
        prompt::select("How does this account sign in?", &items, default)?
            .map(|i| LoginKind::ALL[i])
            .unwrap_or(current),
    )
}

pub fn pick_storage(cfg: &Config, name: &str, desktop: bool, current: &Storage) -> Result<Storage> {
    let product = if desktop {
        "Claude Desktop"
    } else {
        "Claude Code"
    };
    let system_label = if desktop {
        "Keep using Claude Desktop's original data (keeps its current sign-in)"
    } else {
        "Keep using my existing ~/.claude (keeps its current sign-in, settings and history)"
    };
    let owner = cfg.system_owner(desktop, Some(name));
    let mut items = vec![format!(
        "Isolated — a fresh, private {product} just for this profile (recommended)"
    )];
    let system_allowed = owner.is_none();
    if system_allowed {
        items.push(system_label.to_string());
    }
    items.push("Custom folder…".to_string());
    let default = match current {
        Storage::Isolated => 0,
        Storage::System if system_allowed => 1,
        _ => items.len() - 1,
    };
    let picked = prompt::select(
        &format!("Where should {product} keep this profile's data?"),
        &items,
        default,
    )?;
    Ok(match picked {
        Some(0) => Storage::Isolated,
        Some(1) if system_allowed => Storage::System,
        Some(_) => {
            let start = match current {
                Storage::Custom(p) => p.clone(),
                _ => String::new(),
            };
            let path = prompt::text("Folder path (full path, ~ allowed)", &start)?;
            if path.is_empty() {
                current.clone()
            } else if !expand_home(&path).is_ok_and(|p| p.is_absolute()) {
                // A relative folder would resolve against wherever cswitch happens to run.
                ui::fail(format!(
                    "'{path}' is not a full path — keeping the previous setting"
                ));
                current.clone()
            } else {
                Storage::Custom(path)
            }
        }
        None => current.clone(),
    })
}

/// Two steps: which browser, then which of its profiles (or sandbox / private window).
pub fn pick_browser(profile_name: &str, current: &BrowserChoice) -> Result<BrowserChoice> {
    let detected = browser::detect_all();
    let mut items: Vec<String> = detected
        .iter()
        .map(|b| format!("{}  {}", b.kind.display(), ui::dim(b.exe.to_string_lossy())))
        .collect();
    items.push(format!(
        "System default browser  {}",
        ui::dim("(whatever opens links)")
    ));
    items.push(format!(
        "Custom command…  {}",
        ui::dim("(any browser, {url} placeholder)")
    ));
    let default = detected
        .iter()
        .position(|b| b.kind == current.kind)
        .unwrap_or(0);
    let Some(choice) = prompt::select(
        &format!("Which browser should open sign-in pages for '{profile_name}'?"),
        &items,
        default,
    )?
    else {
        return Ok(current.clone());
    };
    if choice == detected.len() {
        return Ok(BrowserChoice {
            kind: BrowserKind::System,
            ..BrowserChoice::default()
        });
    }
    if choice == detected.len() + 1 {
        let command = prompt::text(
            "Command ({url} is replaced by the address)",
            current
                .command
                .as_deref()
                .unwrap_or("\"C:\\Path\\browser.exe\" --new-window {url}"),
        )?;
        return Ok(BrowserChoice {
            kind: BrowserKind::Custom,
            command: Some(command),
            ..BrowserChoice::default()
        });
    }
    let browser = &detected[choice];
    let profiles = browser::profiles(browser);
    let private_label = match browser.kind {
        BrowserKind::Edge => "InPrivate window — nothing remembered, sign in every time",
        BrowserKind::Firefox => "Private window — nothing remembered, sign in every time",
        _ => "Incognito window — nothing remembered, sign in every time",
    };
    let mut options: Vec<String> = profiles
        .iter()
        .map(|p| format!("Profile: {}", p.describe()))
        .collect();
    let sandbox_index = options.len();
    options.push(format!(
        "{} {}",
        ui::accent("Sandbox"),
        "— a separate browser profile used only for this Claude profile (recommended)"
    ));
    options.push(private_label.to_string());
    options.push("Whatever profile the browser used last".to_string());
    let default = match current.mode {
        BrowserMode::Profile => current
            .profile
            .as_ref()
            .and_then(|id| profiles.iter().position(|p| &p.id == id))
            .unwrap_or(sandbox_index),
        BrowserMode::Sandbox => sandbox_index,
        BrowserMode::Private => sandbox_index + 1,
    };
    let picked = if options.len() > 10 {
        prompt::fuzzy(
            &format!("{} — how should it open?", browser.kind.display()),
            &options,
            default,
        )?
    } else {
        prompt::select(
            &format!("{} — how should it open?", browser.kind.display()),
            &options,
            default,
        )?
    };
    let Some(picked) = picked else {
        return Ok(current.clone());
    };
    let base = BrowserChoice {
        kind: browser.kind,
        ..BrowserChoice::default()
    };
    Ok(if picked < profiles.len() {
        BrowserChoice {
            mode: BrowserMode::Profile,
            profile: Some(profiles[picked].id.clone()),
            ..base
        }
    } else if picked == sandbox_index {
        BrowserChoice {
            mode: BrowserMode::Sandbox,
            ..base
        }
    } else if picked == sandbox_index + 1 {
        BrowserChoice {
            mode: BrowserMode::Private,
            ..base
        }
    } else {
        BrowserChoice {
            mode: BrowserMode::Profile,
            profile: None,
            ..base
        }
    })
}

fn storage_text(storage: &Storage, desktop: bool) -> String {
    match storage {
        Storage::Isolated => "isolated".to_string(),
        Storage::System if desktop => "Claude Desktop's original data".to_string(),
        Storage::System => "~/.claude (original)".to_string(),
        Storage::Custom(p) => p.clone(),
    }
}

/// Full field-by-field editor. Returns `true` if the profile was changed and saved.
pub fn edit_profile(cfg: &mut Config, name: &str) -> Result<bool> {
    let mut draft = cfg.profile(name)?.clone();
    loop {
        ui::section(format!("Edit profile · {}", ui::badge(name, Some(&draft))));
        let env_count = draft.env.len();
        let fields = vec![
            format!("Display name          {}", ui::accent2(&draft.label)),
            format!(
                "Color                 {} {}",
                Rgb::parse(&draft.color)
                    .map(|c| c.style().apply_to(ui::g("●", "*")).to_string())
                    .unwrap_or_default(),
                ui::accent2(&draft.color)
            ),
            format!(
                "Account email         {}",
                ui::accent2(draft.email.clone().unwrap_or_else(|| "—".into()))
            ),
            format!(
                "Sign-in method        {}",
                ui::accent2(draft.login.describe())
            ),
            format!(
                "Claude Code data      {}",
                ui::accent2(storage_text(&draft.code_storage, false))
            ),
            format!(
                "Claude Desktop data   {}",
                ui::accent2(storage_text(&draft.desktop_storage, true))
            ),
            format!(
                "Desktop's Code tab    {}",
                ui::accent2(if draft.desktop_shares_code {
                    "uses this profile's Claude Code"
                } else {
                    "uses default ~/.claude"
                })
            ),
            format!(
                "Sign-in browser       {}",
                ui::accent2(browser::describe(&draft.browser))
            ),
            format!(
                "Environment vars      {}",
                ui::accent2(format!("{env_count} set"))
            ),
            format!(
                "Extra claude args     {}",
                ui::accent2(if draft.claude_args.is_empty() {
                    "—".to_string()
                } else {
                    draft.claude_args.join(" ")
                })
            ),
            format!(
                "Icon / emoji          {}",
                ui::accent2(if draft.icon.is_empty() {
                    "—"
                } else {
                    &draft.icon
                })
            ),
            ui::success(format!("{} Save", ui::g("✔", "+"))),
            ui::dim(format!("{} Discard changes", ui::g("✖", "x"))),
        ];
        let Some(choice) = prompt::select("Choose a field", &fields, 0)? else {
            return Ok(false);
        };
        match choice {
            0 => draft.label = prompt::text("Display name", &draft.label)?,
            1 => draft.color = pick_color(&draft.color)?,
            2 => {
                let email = prompt::text(
                    "Account email (empty to clear)",
                    draft.email.as_deref().unwrap_or(""),
                )?;
                draft.email = (!email.is_empty()).then_some(email);
            }
            3 => draft.login = pick_login_kind(draft.login)?,
            4 => draft.code_storage = pick_storage(cfg, name, false, &draft.code_storage)?,
            5 => draft.desktop_storage = pick_storage(cfg, name, true, &draft.desktop_storage)?,
            6 => {
                draft.desktop_shares_code = prompt::confirm(
                    "Should Claude Desktop's built-in Claude Code use this profile's Claude Code data?",
                    draft.desktop_shares_code,
                )?
            }
            7 => draft.browser = pick_browser(name, &draft.browser)?,
            8 => edit_env(&mut draft.env)?,
            9 => {
                let current = sys::join_command(&draft.claude_args);
                let text = prompt::text(
                    "Arguments always passed to claude (e.g. --model opus)",
                    &current,
                )?;
                match sys::split_command(&text) {
                    Ok(args) => draft.claude_args = args,
                    Err(err) => ui::fail(format!("{err:#} — arguments left unchanged")),
                }
            }
            10 => draft.icon = prompt::text("Icon or emoji (optional)", &draft.icon)?,
            11 => {
                cfg.profiles.insert(name.to_string(), draft);
                cfg.save()?;
                ui::ok("Profile saved");
                return Ok(true);
            }
            _ => return Ok(false),
        }
    }
}

pub fn edit_env(env: &mut BTreeMap<String, String>) -> Result<()> {
    loop {
        let mut items: Vec<String> = env
            .iter()
            .map(|(k, v)| format!("{k} = {}", mask(k, v)))
            .collect();
        items.push(ui::success(format!(
            "{} Add / change a variable",
            ui::g("✚", "+")
        )));
        items.push(ui::dim(format!("{} Back", ui::g("←", "<"))));
        let Some(choice) = prompt::select(
            "Environment variables (select one to remove it)",
            &items,
            env.len(),
        )?
        else {
            return Ok(());
        };
        if choice < env.len() {
            let key = env.keys().nth(choice).cloned().unwrap_or_default();
            if prompt::confirm(&format!("Remove {key}?"), false)? {
                env.remove(&key);
            }
        } else if choice == env.len() {
            let key = prompt::text("Name (e.g. ANTHROPIC_MODEL, CLAUDE_CODE_USE_BEDROCK)", "")?;
            if key.is_empty() {
                continue;
            }
            let value = prompt::text("Value", env.get(&key).map(String::as_str).unwrap_or(""))?;
            env.insert(key, value);
        } else {
            return Ok(());
        }
    }
}

fn mask(key: &str, value: &str) -> String {
    let upper = key.to_uppercase();
    if upper.contains("KEY") || upper.contains("TOKEN") || upper.contains("SECRET") {
        let tail: String = value
            .chars()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("••••{tail}")
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_cases_names() {
        assert_eq!(title_case("work"), "Work");
        assert_eq!(title_case("client-a_b"), "Client A B");
    }

    #[test]
    fn masks_secrets() {
        assert_eq!(mask("ANTHROPIC_API_KEY", "sk-ant-123456"), "••••3456");
        assert_eq!(mask("ANTHROPIC_MODEL", "opus"), "opus");
    }
}
