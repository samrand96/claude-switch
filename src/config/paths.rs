//! Filesystem layout of Claude Switch.
//!
//! ```text
//! ~/.cswitch/                     (override with CSWITCH_HOME)
//!   config.toml
//!   bin/                          installed binary, cswitchw.exe, claude-<profile> shims
//!   profiles/<name>/code/         CLAUDE_CONFIG_DIR for the profile
//!   profiles/<name>/desktop/      Claude Desktop --user-data-dir for the profile
//!   profiles/<name>/browser/      sandbox browser profile for sign-in pages
//!   state/                        router state, caches and logs
//! ```
//!
//! The data lives under the home folder rather than `AppData` on purpose: Windows Store (MSIX)
//! apps get their `AppData` writes redirected into a private package folder, which would make the
//! Desktop profiles invisible to everything else.

use std::path::PathBuf;

use anyhow::{Context, Result};

pub fn home_dir() -> Result<PathBuf> {
    dirs::home_dir().context("could not determine your home directory")
}

pub fn cswitch_home() -> Result<PathBuf> {
    if let Some(custom) = std::env::var_os("CSWITCH_HOME").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(custom));
    }
    Ok(home_dir()?.join(".cswitch"))
}

pub fn config_file() -> Result<PathBuf> {
    Ok(cswitch_home()?.join("config.toml"))
}

pub fn bin_dir() -> Result<PathBuf> {
    Ok(cswitch_home()?.join("bin"))
}

pub fn state_dir() -> Result<PathBuf> {
    Ok(cswitch_home()?.join("state"))
}

pub fn profile_root(name: &str) -> Result<PathBuf> {
    Ok(cswitch_home()?.join("profiles").join(name))
}

/// Claude Code's own default config directory (`~/.claude`).
pub fn system_code_dir() -> Result<PathBuf> {
    Ok(home_dir()?.join(".claude"))
}

/// Claude Code's own default global state file (`~/.claude.json`).
pub fn system_code_state_file() -> Result<PathBuf> {
    Ok(home_dir()?.join(".claude.json"))
}

pub fn exe_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
}

/// The installed console binary (`~/.cswitch/bin/cswitch[.exe]`).
pub fn installed_exe() -> Result<PathBuf> {
    Ok(bin_dir()?.join(exe_name("cswitch")))
}

/// The binary used as a background helper: browser opener and `claude://` handler.
///
/// On Windows this is `cswitchw.exe`, a GUI-subsystem copy that never flashes a console window.
/// Elsewhere it is the regular binary.
pub fn helper_exe() -> Result<PathBuf> {
    if cfg!(windows) {
        let windowed = bin_dir()?.join("cswitchw.exe");
        if windowed.exists() {
            return Ok(windowed);
        }
    }
    let installed = installed_exe()?;
    if installed.exists() {
        return Ok(installed);
    }
    std::env::current_exe().context("could not locate the running executable")
}

/// Path of the binary that should be referenced from shortcuts and shims.
pub fn stable_exe() -> Result<PathBuf> {
    let installed = installed_exe()?;
    if installed.exists() {
        return Ok(installed);
    }
    std::env::current_exe().context("could not locate the running executable")
}
