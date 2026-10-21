//! Claude Code (the `claude` CLI): locating it, running it inside a profile, reading who is signed in.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::config::{Config, LoginKind, expand_home, paths};
use crate::core::env::{self, Target};

/// Finds the `claude` executable, skipping our own shims and Windows Store app aliases
/// (the Claude Desktop package may publish a `claude.exe` alias of its own).
pub fn resolve_claude(cfg: &Config) -> Result<PathBuf> {
    if let Some(custom) = cfg
        .code
        .claude_path
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        let path = expand_home(custom)?;
        if path.exists() {
            return Ok(path);
        }
        bail!("configured claude_path does not exist: {}", path.display());
    }
    let me = std::env::current_exe()
        .ok()
        .and_then(|p| fs::canonicalize(p).ok());
    let usable = |p: &Path| {
        let text = p.to_string_lossy().to_lowercase();
        if text.contains("\\windowsapps\\") || text.contains(".cswitch") {
            return false;
        }
        match (&me, fs::canonicalize(p).ok()) {
            (Some(me), Some(real)) => &real != me,
            _ => true,
        }
    };
    if let Ok(found) = which::which_all("claude") {
        for candidate in found {
            if usable(&candidate) {
                return Ok(candidate);
            }
        }
    }
    let home = paths::home_dir()?;
    let mut fallbacks = vec![
        home.join(".local")
            .join("bin")
            .join(paths::exe_name("claude")),
        home.join(".claude")
            .join("local")
            .join(paths::exe_name("claude")),
    ];
    if cfg!(windows) {
        if let Some(appdata) = dirs::data_dir() {
            fallbacks.push(appdata.join("npm").join("claude.cmd"));
        }
    } else {
        fallbacks.push(PathBuf::from("/opt/homebrew/bin/claude"));
        fallbacks.push(PathBuf::from("/usr/local/bin/claude"));
    }
    for candidate in fallbacks {
        if candidate.exists() && usable(&candidate) {
            return Ok(candidate);
        }
    }
    bail!(
        "Claude Code (`claude`) was not found. Install it from https://claude.com/claude-code \
         or set its path with `cswitch config` → Settings"
    )
}

pub fn version(claude: &Path) -> Option<String> {
    let out = Command::new(claude).arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Account {
    pub email: Option<String>,
    pub organization: Option<String>,
    pub display_name: Option<String>,
}

impl Account {
    pub fn summary(&self) -> String {
        match (&self.email, &self.organization) {
            (Some(e), Some(o)) if !o.is_empty() => format!("{e} · {o}"),
            (Some(e), _) => e.clone(),
            (None, Some(o)) => o.clone(),
            (None, None) => self
                .display_name
                .clone()
                .unwrap_or_else(|| "signed in".to_string()),
        }
    }
}

/// Account recorded by Claude Code in the profile's `.claude.json` (no network, no secrets).
pub fn account(cfg: &Config, name: &str) -> Option<Account> {
    let file = cfg.code_state_file(name).ok()?;
    account_from_file(&file)
}

pub fn account_from_file(file: &Path) -> Option<Account> {
    let raw = fs::read_to_string(file).ok()?;
    let json: Value = serde_json::from_str(&raw).ok()?;
    let oauth = json.get("oauthAccount")?;
    let field = |key: &str| oauth.get(key).and_then(Value::as_str).map(str::to_string);
    let account = Account {
        email: field("emailAddress").or_else(|| field("email")),
        organization: field("organizationName"),
        display_name: field("displayName"),
    };
    (account != Account::default()).then_some(account)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Credentials {
    Present,
    Missing,
    Unknown,
}

/// Whether the profile has stored sign-in credentials.
///
/// Windows and Linux keep them in `<config dir>/.credentials.json`. macOS keeps them in the
/// login keychain under `Claude Code-credentials` plus, when `CLAUDE_CONFIG_DIR` is set, a
/// `-<first 8 hex chars of sha256(dir)>` suffix — which is exactly why profiles do not collide.
pub fn credentials(cfg: &Config, name: &str) -> Credentials {
    let Ok(code_dir) = cfg.code_dir(name) else {
        return Credentials::Unknown;
    };
    if cfg!(target_os = "macos") {
        let service = keychain_service(code_dir.as_deref());
        return match Command::new("security")
            .args(["find-generic-password", "-s", &service])
            .output()
        {
            Ok(out) if out.status.success() => Credentials::Present,
            Ok(_) => Credentials::Missing,
            Err(_) => Credentials::Unknown,
        };
    }
    let dir = match code_dir {
        Some(dir) => dir,
        None => match paths::system_code_dir() {
            Ok(d) => d,
            Err(_) => return Credentials::Unknown,
        },
    };
    if dir.join(".credentials.json").exists() {
        Credentials::Present
    } else {
        Credentials::Missing
    }
}

pub fn keychain_service(config_dir: Option<&Path>) -> String {
    match config_dir {
        None => "Claude Code-credentials".to_string(),
        Some(dir) => {
            let digest = Sha256::digest(dir.to_string_lossy().as_bytes());
            let hex: String = digest.iter().take(4).map(|b| format!("{b:02x}")).collect();
            format!("Claude Code-credentials-{hex}")
        }
    }
}

/// A `claude` command prepared for the profile: isolated env, config dir, browser opener.
pub fn command(cfg: &Config, name: &str, args: &[String]) -> Result<Command> {
    let claude = resolve_claude(cfg)?;
    if let Some(dir) = cfg.code_dir(name)? {
        fs::create_dir_all(&dir).with_context(|| format!("could not create {}", dir.display()))?;
    }
    let profile = cfg.profile(name)?;
    let mut cmd = Command::new(&claude);
    cmd.args(&profile.claude_args).args(args);
    env::plan(cfg, name, Target::Code)?.apply(&mut cmd);
    Ok(cmd)
}

/// Runs Claude Code in the foreground. On Unix the current process is replaced.
pub fn run_foreground(
    cfg: &Config,
    name: &str,
    args: &[String],
    cwd: Option<&Path>,
) -> Result<i32> {
    let mut cmd = command(cfg, name, args)?;
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    crate::core::sys::log(format!("code: profile={name} args={args:?}"));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = cmd.exec();
        Err(anyhow::Error::new(err).context("could not start claude"))
    }
    #[cfg(windows)]
    {
        crate::core::sys::win::ignore_ctrl_c();
        let status = cmd.status().context("could not start claude")?;
        Ok(status.code().unwrap_or(1))
    }
}

/// Runs `claude <args>` for the profile, waits, and returns the exit status (stdio inherited).
pub fn run_wait(cfg: &Config, name: &str, args: &[String]) -> Result<bool> {
    let mut cmd = command(cfg, name, args)?;
    #[cfg(windows)]
    crate::core::sys::win::ignore_ctrl_c();
    Ok(cmd.status().context("could not start claude")?.success())
}

pub fn login_args(cfg: &Config, name: &str) -> Result<Vec<String>> {
    let profile = cfg.profile(name)?;
    let mut args = vec!["auth".to_string(), "login".to_string()];
    match profile.login {
        LoginKind::ClaudeAi => args.push("--claudeai".into()),
        LoginKind::Sso => args.push("--sso".into()),
        LoginKind::Console => args.push("--console".into()),
    }
    if let Some(email) = profile.email.as_deref().filter(|e| !e.trim().is_empty()) {
        args.push("--email".into());
        args.push(email.trim().to_string());
    }
    Ok(args)
}

/// `claude auth status --json` for the profile.
pub fn auth_status(cfg: &Config, name: &str) -> Result<Value> {
    let out = command(
        cfg,
        name,
        &["auth".into(), "status".into(), "--json".into()],
    )?
    .output()
    .context("could not run claude auth status")?;
    let text = String::from_utf8_lossy(&out.stdout);
    let start = text.find('{').unwrap_or(0);
    serde_json::from_str(&text[start..]).context("unexpected output from claude auth status")
}

/// Compares the signed-in account against the profile's expected email.
pub fn email_matches(expected: Option<&str>, actual: Option<&str>) -> Option<bool> {
    let expected = expected?.trim().to_lowercase();
    let actual = actual?.trim().to_lowercase();
    if expected.is_empty() || actual.is_empty() {
        return None;
    }
    Some(expected == actual)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keychain_service_matches_claude_code_scheme() {
        assert_eq!(keychain_service(None), "Claude Code-credentials");
        let svc = keychain_service(Some(Path::new("/Users/me/.cswitch/profiles/work/code")));
        assert!(svc.starts_with("Claude Code-credentials-"));
        assert_eq!(svc.len(), "Claude Code-credentials-".len() + 8);
    }

    #[test]
    fn reads_oauth_account() {
        let dir = std::env::temp_dir().join(format!("cswitch-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join(".claude.json");
        fs::write(
            &file,
            r#"{"oauthAccount":{"emailAddress":"a@b.com","organizationName":"Acme"}}"#,
        )
        .unwrap();
        let acct = account_from_file(&file).unwrap();
        assert_eq!(acct.summary(), "a@b.com · Acme");
        fs::write(&file, r#"{"numStartups":3}"#).unwrap();
        assert!(account_from_file(&file).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn email_comparison() {
        assert_eq!(email_matches(Some("A@b.com"), Some("a@B.com")), Some(true));
        assert_eq!(email_matches(Some("a@b.com"), Some("c@b.com")), Some(false));
        assert_eq!(email_matches(None, Some("c@b.com")), None);
    }
}
