//! Helpers shared by commands: choosing a profile, requiring Desktop, interactivity checks.

use anyhow::{Context, Result, bail};

use crate::config::Config;
use crate::core::desktop::{self, DesktopInstall};
use crate::ui::{self, prompt};

pub fn interactive() -> bool {
    console::user_attended() && console::Term::stdout().is_term()
}

/// Resolves an optional profile argument: validates a given name, otherwise asks (or falls back
/// to the default profile when there is no terminal).
pub fn resolve_profile(cfg: &Config, given: Option<&str>, question: &str) -> Result<String> {
    if cfg.profiles.is_empty() {
        bail!("no profiles yet — run `cswitch setup` (or just `cswitch`) to create them");
    }
    if let Some(name) = given {
        cfg.profile(name)?;
        return Ok(name.to_string());
    }
    if cfg.profiles.len() == 1 {
        return Ok(cfg.profiles.keys().next().cloned().unwrap_or_default());
    }
    if interactive() {
        return pick_profile(cfg, question)?.context("cancelled");
    }
    cfg.fallback_profile()
        .context("several profiles exist and no default is set — name the profile explicitly")
}

/// Arrow-key profile picker. `None` on Esc.
pub fn pick_profile(cfg: &Config, question: &str) -> Result<Option<String>> {
    let names = cfg.profile_names();
    let items: Vec<String> = names.iter().map(|n| profile_item(cfg, n)).collect();
    Ok(prompt::select(question, &items, 0)?.map(|i| names[i].clone()))
}

pub fn profile_item(cfg: &Config, name: &str) -> String {
    let profile = cfg.profiles.get(name);
    let mut text = ui::badge(name, profile);
    if cfg.default_profile.as_deref() == Some(name) {
        text.push_str(&ui::dim(format!(" {}", ui::g("★ default", "(default)"))));
    }
    if let Some(email) = profile
        .and_then(|p| p.email.as_deref())
        .filter(|e| !e.is_empty())
    {
        text.push_str(&ui::dim(format!("  {email}")));
    }
    text
}

pub fn require_desktop(cfg: &Config) -> Result<DesktopInstall> {
    desktop::detect(cfg, false).context(
        "Claude Desktop was not found. Install it (Microsoft Store or claude.com/download), or set its \
         path in `cswitch config` → Settings → Claude Desktop path",
    )
}

/// Splits a mistaken leading `--flag` that clap put into the profile slot back into the args.
pub fn split_profile_and_args(
    cfg: &Config,
    profile: Option<String>,
    mut args: Vec<String>,
) -> (Option<String>, Vec<String>) {
    match profile {
        Some(p) if p.starts_with('-') && !cfg.profiles.contains_key(&p) => {
            args.insert(0, p);
            (None, args)
        }
        other => (other, args),
    }
}
