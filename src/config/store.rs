//! Loading, saving and querying the configuration.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::paths;
use super::schema::{Config, Profile, Storage};

impl Config {
    pub fn load() -> Result<Self> {
        let path = paths::config_file()?;
        if !path.exists() {
            return Ok(Config::default());
        }
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("could not read {}", path.display()))?;
        // Some Windows editors save UTF-8 with a byte-order mark, which TOML rejects.
        let cfg: Config = toml::from_str(raw.trim_start_matches('\u{feff}'))
            .with_context(|| format!("{} is not valid TOML", path.display()))?;
        Ok(cfg)
    }

    pub fn exists() -> bool {
        paths::config_file().map(|p| p.exists()).unwrap_or(false)
    }

    pub fn save(&self) -> Result<()> {
        let path = paths::config_file()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("could not create {}", parent.display()))?;
        }
        let body = toml::to_string_pretty(self).context("could not serialize configuration")?;
        let header =
            "# Claude Switch configuration — edit with `cswitch` (interactive) or by hand.\n\n";
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, format!("{header}{body}"))
            .with_context(|| format!("could not write {}", tmp.display()))?;
        fs::rename(&tmp, &path).with_context(|| format!("could not replace {}", path.display()))?;
        Ok(())
    }

    pub fn profile(&self, name: &str) -> Result<&Profile> {
        match self.profiles.get(name) {
            Some(profile) => Ok(profile),
            None => {
                let known = self.profile_names().join(", ");
                if known.is_empty() {
                    bail!(
                        "profile '{name}' does not exist — run `cswitch setup` to create profiles"
                    )
                }
                bail!("profile '{name}' does not exist (known profiles: {known})")
            }
        }
    }

    pub fn profile_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.profiles.keys().cloned().collect();
        // Default profile first, then alphabetical.
        if let Some(default) = &self.default_profile
            && let Some(pos) = names.iter().position(|n| n == default)
        {
            let d = names.remove(pos);
            names.insert(0, d);
        }
        names
    }

    /// The profile a command should use when the user did not name one.
    pub fn fallback_profile(&self) -> Option<String> {
        if let Some(default) = &self.default_profile
            && self.profiles.contains_key(default)
        {
            return Some(default.clone());
        }
        if self.profiles.len() == 1 {
            return self.profiles.keys().next().cloned();
        }
        None
    }

    /// Directory Claude Code uses for this profile. `None` means "Claude Code's own default".
    pub fn code_dir(&self, name: &str) -> Result<Option<PathBuf>> {
        let profile = self.profile(name)?;
        storage_dir(&profile.code_storage, name, "code")
    }

    /// Directory Claude Code actually reads for this profile (resolving "system").
    pub fn effective_code_dir(&self, name: &str) -> Result<PathBuf> {
        match self.code_dir(name)? {
            Some(dir) => Ok(dir),
            None => paths::system_code_dir(),
        }
    }

    /// `.claude.json` used by this profile.
    pub fn code_state_file(&self, name: &str) -> Result<PathBuf> {
        match self.code_dir(name)? {
            Some(dir) => Ok(dir.join(".claude.json")),
            None => paths::system_code_state_file(),
        }
    }

    /// Directory Claude Desktop uses for this profile. `None` means "Desktop's own default".
    pub fn desktop_dir(&self, name: &str) -> Result<Option<PathBuf>> {
        let profile = self.profile(name)?;
        storage_dir(&profile.desktop_storage, name, "desktop")
    }

    pub fn sandbox_browser_dir(&self, name: &str) -> Result<PathBuf> {
        Ok(paths::profile_root(name)?.join("browser"))
    }

    /// Which profile (if any) already uses the system location for the given product.
    pub fn system_owner(&self, desktop: bool, except: Option<&str>) -> Option<String> {
        self.profiles
            .iter()
            .filter(|(name, _)| Some(name.as_str()) != except)
            .find(|(_, p)| {
                let storage = if desktop {
                    &p.desktop_storage
                } else {
                    &p.code_storage
                };
                *storage == Storage::System
            })
            .map(|(name, _)| name.clone())
    }

    pub fn display_name(&self, name: &str) -> String {
        match self.profiles.get(name) {
            Some(p) if !p.label.trim().is_empty() => p.label.clone(),
            _ => name.to_string(),
        }
    }
}

fn storage_dir(storage: &Storage, name: &str, product: &str) -> Result<Option<PathBuf>> {
    Ok(match storage {
        Storage::Isolated => Some(paths::profile_root(name)?.join(product)),
        Storage::System => None,
        Storage::Custom(path) => Some(expand_home(path)?),
    })
}

/// Expands a leading `~` so hand-edited configs can use it.
pub fn expand_home(path: &str) -> Result<PathBuf> {
    let trimmed = path.trim();
    if trimmed == "~" {
        return paths::home_dir();
    }
    if let Some(rest) = trimmed
        .strip_prefix("~/")
        .or_else(|| trimmed.strip_prefix("~\\"))
    {
        return Ok(paths::home_dir()?.join(rest));
    }
    Ok(PathBuf::from(trimmed))
}

/// Profile names become file names, shim names (`claude-<name>`) and CLI arguments.
pub fn validate_profile_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 32 {
        bail!("profile names must be 1–32 characters long");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        bail!(
            "use only lowercase letters, digits, '-' and '_' (for example: work, personal, client-a)"
        );
    }
    if name.starts_with('-') {
        bail!("profile names cannot start with '-'");
    }
    const RESERVED: [&str; 6] = ["all", "system", "default", "code", "desktop", "help"];
    if RESERVED.contains(&name) {
        bail!("'{name}' is reserved, pick another name");
    }
    Ok(())
}

/// Lower-cases and strips separators so paths from command lines and configs compare equal.
pub fn normalize_path(path: &Path) -> String {
    let resolved = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut text = resolved.to_string_lossy().to_string();
    if let Some(stripped) = text.strip_prefix(r"\\?\") {
        text = stripped.to_string();
    }
    let mut text = text.replace('\\', "/");
    while text.len() > 1 && text.ends_with('/') {
        text.pop();
    }
    if cfg!(windows) || cfg!(target_os = "macos") {
        text = text.to_lowercase();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_profile_names() {
        assert!(validate_profile_name("work").is_ok());
        assert!(validate_profile_name("client-a_2").is_ok());
        assert!(validate_profile_name("Work").is_err());
        assert!(validate_profile_name("with space").is_err());
        assert!(validate_profile_name("all").is_err());
        assert!(validate_profile_name("").is_err());
    }

    #[test]
    fn default_profile_sorts_first() {
        let mut cfg = Config::default();
        cfg.profiles.insert("alpha".into(), Profile::default());
        cfg.profiles.insert("work".into(), Profile::default());
        cfg.default_profile = Some("work".into());
        assert_eq!(cfg.profile_names(), vec!["work", "alpha"]);
    }

    #[test]
    fn normalizes_trailing_separators() {
        let a = normalize_path(Path::new("/nonexistent/dir/"));
        let b = normalize_path(Path::new("/nonexistent/dir"));
        assert_eq!(a, b);
    }
}
