//! Finds installed browsers and their profiles, and opens URLs in the browser a Claude profile owns.
//!
//! This is what fixes "the sign-in page opened in the wrong account": Claude Code calls
//! `$BROWSER <url>`, `$BROWSER` points at us, and we open the URL in the profile's own browser
//! profile (or a sandboxed one that is used for nothing else).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::config::{BrowserChoice, BrowserKind, BrowserMode, Config, expand_home};
use crate::core::sys;

#[derive(Debug, Clone)]
pub struct Browser {
    pub kind: BrowserKind,
    pub exe: PathBuf,
    /// macOS: the `.app` bundle, launched through LaunchServices.
    pub app_bundle: Option<PathBuf>,
    /// Chromium "User Data" directory or Firefox root (where `profiles.ini` lives).
    pub data_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserProfile {
    /// Chromium profile directory (`Default`, `Profile 1`) or Firefox profile name.
    pub id: String,
    pub name: String,
    pub email: Option<String>,
}

impl BrowserProfile {
    pub fn describe(&self) -> String {
        match &self.email {
            Some(email) if !email.is_empty() => format!("{} — {} ({})", self.name, email, self.id),
            _ if self.name != self.id => format!("{} ({})", self.name, self.id),
            _ => self.name.clone(),
        }
    }
}

pub const DETECTABLE: [BrowserKind; 6] = [
    BrowserKind::Chrome,
    BrowserKind::Edge,
    BrowserKind::Brave,
    BrowserKind::Vivaldi,
    BrowserKind::Chromium,
    BrowserKind::Firefox,
];

pub fn detect_all() -> Vec<Browser> {
    DETECTABLE.iter().filter_map(|k| detect(*k)).collect()
}

pub fn detect(kind: BrowserKind) -> Option<Browser> {
    platform::detect(kind)
}

/// Resolves a choice to a concrete browser, honoring an `exe` override.
pub fn resolve(choice: &BrowserChoice) -> Option<Browser> {
    let mut browser = detect(choice.kind);
    if let Some(custom) = choice.exe.as_deref().filter(|s| !s.trim().is_empty()) {
        let exe = expand_home(custom).ok()?;
        match &mut browser {
            Some(b) => {
                b.exe = exe;
                b.app_bundle = None;
            }
            None => {
                browser = Some(Browser {
                    kind: choice.kind,
                    exe,
                    app_bundle: None,
                    data_dir: None,
                });
            }
        }
    }
    browser
}

pub fn profiles(browser: &Browser) -> Vec<BrowserProfile> {
    let Some(dir) = &browser.data_dir else {
        return Vec::new();
    };
    if browser.kind == BrowserKind::Firefox {
        firefox_profiles(dir)
    } else {
        chromium_profiles(dir)
    }
}

/// Parses Chromium's `Local State` → `profile.info_cache`.
pub fn chromium_profiles(user_data: &Path) -> Vec<BrowserProfile> {
    let Ok(raw) = fs::read_to_string(user_data.join("Local State")) else {
        return Vec::new();
    };
    parse_chromium_local_state(&raw)
}

pub fn parse_chromium_local_state(raw: &str) -> Vec<BrowserProfile> {
    let Ok(json) = serde_json::from_str::<Value>(raw) else {
        return Vec::new();
    };
    let Some(cache) = json
        .pointer("/profile/info_cache")
        .and_then(Value::as_object)
    else {
        return Vec::new();
    };
    let mut out: Vec<BrowserProfile> = cache
        .iter()
        .map(|(dir, info)| {
            let text = |k: &str| {
                info.get(k)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            };
            BrowserProfile {
                id: dir.clone(),
                name: text("name")
                    .or_else(|| text("gaia_name"))
                    .unwrap_or_else(|| dir.clone()),
                email: text("user_name"),
            }
        })
        .collect();
    out.sort_by_key(|p| (p.id != "Default", profile_number(&p.id), p.id.clone()));
    out
}

fn profile_number(id: &str) -> u32 {
    id.rsplit(' ')
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

/// Parses Firefox's `profiles.ini`.
pub fn firefox_profiles(root: &Path) -> Vec<BrowserProfile> {
    let Ok(raw) = fs::read_to_string(root.join("profiles.ini")) else {
        return Vec::new();
    };
    parse_profiles_ini(&raw)
}

pub fn parse_profiles_ini(raw: &str) -> Vec<BrowserProfile> {
    let mut out = Vec::new();
    let mut in_profile = false;
    for line in raw.lines().map(str::trim) {
        if line.starts_with('[') {
            in_profile = line.to_lowercase().starts_with("[profile");
            continue;
        }
        if in_profile && let Some(name) = line.strip_prefix("Name=") {
            out.push(BrowserProfile {
                id: name.to_string(),
                name: name.to_string(),
                email: None,
            });
        }
    }
    out
}

/// One-line description of a profile's browser choice, for menus and status.
pub fn describe(choice: &BrowserChoice) -> String {
    let kind = choice.kind;
    if kind == BrowserKind::System {
        return "System default browser".to_string();
    }
    if kind == BrowserKind::Custom {
        return format!("Custom: {}", choice.command.clone().unwrap_or_default());
    }
    let short = match kind {
        BrowserKind::Chrome => "Chrome",
        BrowserKind::Edge => "Edge",
        BrowserKind::Brave => "Brave",
        BrowserKind::Vivaldi => "Vivaldi",
        BrowserKind::Chromium => "Chromium",
        BrowserKind::Firefox => "Firefox",
        _ => kind.display(),
    };
    match choice.mode {
        BrowserMode::Sandbox => format!("{short} · private sandbox profile"),
        BrowserMode::Private => {
            let window = if kind == BrowserKind::Edge {
                "InPrivate"
            } else {
                "private"
            };
            format!("{short} · {window} window")
        }
        BrowserMode::Profile => {
            let label = choice.profile.as_deref().and_then(|id| {
                let browser = resolve(choice)?;
                profiles(&browser)
                    .into_iter()
                    .find(|p| p.id == id)
                    .map(|p| match p.email {
                        Some(e) => format!("{} <{e}>", p.name),
                        None => p.name,
                    })
            });
            match (label, &choice.profile) {
                (Some(l), _) => format!("{short} · {l}"),
                (None, Some(id)) => format!("{short} · {id}"),
                (None, None) => format!("{short} · last used profile"),
            }
        }
    }
}

/// Folder of the browser profile owned by Claude Switch (sandbox mode).
///
/// Snap-packaged browsers — Ubuntu's default Firefox and its only Chromium — may not touch
/// hidden folders in `$HOME` such as `~/.cswitch`, so theirs lives in the snap's own
/// `~/snap/<name>/common`, which every Firefox / Chromium build can use.
fn sandbox_dir(cfg: &Config, profile_name: &str, browser: &Browser, sub: &str) -> Result<PathBuf> {
    if let Some(snap) = snap_name(browser) {
        let home = dirs::home_dir().context("could not determine your home directory")?;
        return Ok(home
            .join("snap")
            .join(snap)
            .join("common")
            .join("cswitch")
            .join(profile_name));
    }
    Ok(cfg.sandbox_browser_dir(profile_name)?.join(sub))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn snap_name(browser: &Browser) -> Option<&'static str> {
    use std::io::Read;
    let name = match browser.kind {
        BrowserKind::Firefox => "firefox",
        BrowserKind::Chromium => "chromium",
        BrowserKind::Brave => "brave",
        _ => return None,
    };
    let exe = &browser.exe;
    // `/snap/bin/firefox` is a link to `/usr/bin/snap`.
    let resolved = fs::canonicalize(exe).unwrap_or_else(|_| exe.clone());
    if exe.starts_with("/snap/") || resolved.starts_with("/snap/") || resolved.ends_with("bin/snap")
    {
        return Some(name);
    }
    // Ubuntu's `/usr/bin/firefox` / `chromium-browser` are small scripts that exec the snap.
    let mut head = Vec::new();
    let is_snap_wrapper = Path::new("/snap").join(name).exists()
        && fs::File::open(&resolved)
            .and_then(|f| f.take(4096).read_to_end(&mut head))
            .is_ok()
        && head.starts_with(b"#!")
        && String::from_utf8_lossy(&head).contains("/snap/");
    is_snap_wrapper.then_some(name)
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn snap_name(_browser: &Browser) -> Option<&'static str> {
    None
}

/// The program + arguments that open `url` for this profile.
pub fn plan(cfg: &Config, profile_name: &str, url: &str) -> Result<(PathBuf, Vec<String>)> {
    let choice = &cfg.profile(profile_name)?.browser;
    match choice.kind {
        BrowserKind::System => Ok(system_opener(url)),
        BrowserKind::Custom => {
            let template = choice
                .command
                .as_deref()
                .filter(|c| !c.trim().is_empty())
                .context("custom browser selected but no command is configured")?;
            let mut parts = sys::split_command(template)
                .context("could not parse the custom browser command")?;
            if parts.is_empty() {
                bail!("custom browser command is empty");
            }
            let mut replaced = false;
            for part in parts.iter_mut() {
                if part.contains("{url}") {
                    *part = part.replace("{url}", url);
                    replaced = true;
                }
            }
            if !replaced {
                parts.push(url.to_string());
            }
            let program = PathBuf::from(parts.remove(0));
            Ok((program, parts))
        }
        kind => {
            let browser = resolve(choice).with_context(|| {
                format!(
                    "{} is not installed (or not where expected)",
                    kind.display()
                )
            })?;
            let mut args: Vec<String> = Vec::new();
            if kind == BrowserKind::Firefox {
                match choice.mode {
                    BrowserMode::Sandbox => {
                        let dir = sandbox_dir(cfg, profile_name, &browser, "firefox")?;
                        fs::create_dir_all(&dir)?;
                        args.extend(["-profile".into(), dir.to_string_lossy().to_string()]);
                        args.extend(["-new-tab".into(), url.to_string()]);
                    }
                    BrowserMode::Private => {
                        if let Some(p) = &choice.profile {
                            args.extend(["-P".into(), p.clone()]);
                        }
                        args.extend(["-private-window".into(), url.to_string()]);
                    }
                    BrowserMode::Profile => {
                        if let Some(p) = &choice.profile {
                            args.extend(["-P".into(), p.clone()]);
                        }
                        args.extend(["-new-tab".into(), url.to_string()]);
                    }
                }
            } else {
                match choice.mode {
                    BrowserMode::Sandbox => {
                        let dir = sandbox_dir(cfg, profile_name, &browser, "chromium")?;
                        fs::create_dir_all(&dir)?;
                        args.push(format!("--user-data-dir={}", dir.display()));
                        args.push("--no-first-run".into());
                        args.push("--no-default-browser-check".into());
                    }
                    BrowserMode::Private => {
                        if let Some(p) = &choice.profile {
                            args.push(format!("--profile-directory={p}"));
                        }
                        args.push(if kind == BrowserKind::Edge {
                            "--inprivate".into()
                        } else {
                            "--incognito".into()
                        });
                    }
                    BrowserMode::Profile => {
                        if let Some(p) = &choice.profile {
                            args.push(format!("--profile-directory={p}"));
                        }
                    }
                }
                args.push(url.to_string());
            }
            if let Some(bundle) = &browser.app_bundle {
                let mut open_args = vec![
                    "-n".to_string(),
                    "-a".to_string(),
                    bundle.to_string_lossy().to_string(),
                    "--args".to_string(),
                ];
                open_args.extend(args);
                return Ok((PathBuf::from("open"), open_args));
            }
            Ok((browser.exe, args))
        }
    }
}

fn system_opener(url: &str) -> (PathBuf, Vec<String>) {
    if cfg!(windows) {
        (
            PathBuf::from("rundll32"),
            vec!["url.dll,FileProtocolHandler".into(), url.into()],
        )
    } else if cfg!(target_os = "macos") {
        (PathBuf::from("open"), vec![url.into()])
    } else {
        (PathBuf::from("xdg-open"), vec![url.into()])
    }
}

/// Opens `url` in the profile's browser. Used as `$BROWSER` by Claude Code.
pub fn open(cfg: &Config, profile_name: &str, url: &str) -> Result<()> {
    let (program, args) = plan(cfg, profile_name, url)?;
    sys::log(format!(
        "open-url: profile={profile_name} via {}",
        program.display()
    ));
    let mut cmd = Command::new(&program);
    cmd.args(&args);
    sys::spawn_detached(&mut cmd)
        .with_context(|| format!("could not start {}", program.display()))?;
    Ok(())
}

/// Accepts `https://…`, `"https://…"` (older Claude Code builds quote it) and `'https://…'`.
pub fn clean_url_arg(raw: &str) -> String {
    raw.trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_string()
}

pub fn is_web_url(raw: &str) -> bool {
    let url = clean_url_arg(raw).to_lowercase();
    url.starts_with("https://") || url.starts_with("http://")
}

mod platform {
    use super::*;

    #[cfg(windows)]
    pub fn detect(kind: BrowserKind) -> Option<Browser> {
        let local = dirs::data_local_dir()?;
        let roaming = dirs::data_dir()?;
        let pf = std::env::var_os("ProgramFiles")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
        let pf86 = std::env::var_os("ProgramFiles(x86)")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)"));
        let (exe_name, candidates, data): (&str, Vec<PathBuf>, PathBuf) = match kind {
            BrowserKind::Chrome => (
                "chrome.exe",
                vec![
                    pf.join(r"Google\Chrome\Application\chrome.exe"),
                    pf86.join(r"Google\Chrome\Application\chrome.exe"),
                    local.join(r"Google\Chrome\Application\chrome.exe"),
                ],
                local.join(r"Google\Chrome\User Data"),
            ),
            BrowserKind::Edge => (
                "msedge.exe",
                vec![
                    pf86.join(r"Microsoft\Edge\Application\msedge.exe"),
                    pf.join(r"Microsoft\Edge\Application\msedge.exe"),
                ],
                local.join(r"Microsoft\Edge\User Data"),
            ),
            BrowserKind::Brave => (
                "brave.exe",
                vec![
                    pf.join(r"BraveSoftware\Brave-Browser\Application\brave.exe"),
                    local.join(r"BraveSoftware\Brave-Browser\Application\brave.exe"),
                ],
                local.join(r"BraveSoftware\Brave-Browser\User Data"),
            ),
            BrowserKind::Vivaldi => (
                "vivaldi.exe",
                vec![
                    local.join(r"Vivaldi\Application\vivaldi.exe"),
                    pf.join(r"Vivaldi\Application\vivaldi.exe"),
                ],
                local.join(r"Vivaldi\User Data"),
            ),
            BrowserKind::Chromium => (
                "chromium.exe",
                vec![local.join(r"Chromium\Application\chrome.exe")],
                local.join(r"Chromium\User Data"),
            ),
            BrowserKind::Firefox => (
                "firefox.exe",
                vec![
                    pf.join(r"Mozilla Firefox\firefox.exe"),
                    pf86.join(r"Mozilla Firefox\firefox.exe"),
                ],
                roaming.join(r"Mozilla\Firefox"),
            ),
            _ => return None,
        };
        let exe = app_path(exe_name).or_else(|| candidates.into_iter().find(|p| p.exists()))?;
        Some(Browser {
            kind,
            exe,
            app_bundle: None,
            data_dir: Some(data),
        })
    }

    #[cfg(windows)]
    fn app_path(exe_name: &str) -> Option<PathBuf> {
        use winreg::RegKey;
        use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
        let sub = format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\{exe_name}");
        for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            if let Ok(key) = RegKey::predef(hive).open_subkey(&sub)
                && let Ok(value) = key.get_value::<String, _>("")
            {
                let path = PathBuf::from(value.trim_matches('"'));
                if path.exists() {
                    return Some(path);
                }
            }
        }
        None
    }

    #[cfg(target_os = "macos")]
    pub fn detect(kind: BrowserKind) -> Option<Browser> {
        let home = dirs::home_dir()?;
        let support = home.join("Library/Application Support");
        let (app, binary, data) = match kind {
            BrowserKind::Chrome => (
                "Google Chrome.app",
                "Google Chrome",
                support.join("Google/Chrome"),
            ),
            BrowserKind::Edge => (
                "Microsoft Edge.app",
                "Microsoft Edge",
                support.join("Microsoft Edge"),
            ),
            BrowserKind::Brave => (
                "Brave Browser.app",
                "Brave Browser",
                support.join("BraveSoftware/Brave-Browser"),
            ),
            BrowserKind::Vivaldi => ("Vivaldi.app", "Vivaldi", support.join("Vivaldi")),
            BrowserKind::Chromium => ("Chromium.app", "Chromium", support.join("Chromium")),
            BrowserKind::Firefox => ("Firefox.app", "firefox", support.join("Firefox")),
            _ => return None,
        };
        for root in [PathBuf::from("/Applications"), home.join("Applications")] {
            let bundle = root.join(app);
            let exe = bundle.join("Contents/MacOS").join(binary);
            if exe.exists() {
                return Some(Browser {
                    kind,
                    exe,
                    app_bundle: Some(bundle),
                    data_dir: Some(data),
                });
            }
        }
        None
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    pub fn detect(kind: BrowserKind) -> Option<Browser> {
        let home = dirs::home_dir()?;
        let config = dirs::config_dir().unwrap_or_else(|| home.join(".config"));
        let (names, data): (&[&str], Vec<PathBuf>) = match kind {
            BrowserKind::Chrome => (
                &["google-chrome-stable", "google-chrome"],
                vec![config.join("google-chrome")],
            ),
            BrowserKind::Edge => (
                &["microsoft-edge-stable", "microsoft-edge"],
                vec![config.join("microsoft-edge")],
            ),
            BrowserKind::Brave => (
                &["brave-browser", "brave"],
                vec![config.join("BraveSoftware/Brave-Browser")],
            ),
            BrowserKind::Vivaldi => (&["vivaldi-stable", "vivaldi"], vec![config.join("vivaldi")]),
            BrowserKind::Chromium => (
                &["chromium", "chromium-browser"],
                vec![
                    config.join("chromium"),
                    home.join("snap/chromium/common/chromium"),
                ],
            ),
            BrowserKind::Firefox => (
                &["firefox"],
                vec![
                    home.join(".mozilla/firefox"),
                    home.join("snap/firefox/common/.mozilla/firefox"),
                    // Newer Firefox releases follow XDG on fresh installs.
                    config.join("mozilla/firefox"),
                ],
            ),
            _ => return None,
        };
        let exe = names.iter().find_map(|n| which::which(n).ok())?;
        let data_dir = data
            .iter()
            .find(|d| d.exists())
            .cloned()
            .or_else(|| data.first().cloned());
        Some(Browser {
            kind,
            exe,
            app_bundle: None,
            data_dir,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_local_state() {
        let raw = r#"{"profile":{"info_cache":{
            "Profile 2":{"name":"Work","user_name":"me@corp.com"},
            "Default":{"name":"Person 1","user_name":""},
            "Profile 10":{"name":"","gaia_name":"Side"}
        }}}"#;
        let profiles = parse_chromium_local_state(raw);
        assert_eq!(profiles[0].id, "Default");
        assert_eq!(profiles[0].email, None);
        assert_eq!(profiles[1].id, "Profile 2");
        assert_eq!(profiles[1].email.as_deref(), Some("me@corp.com"));
        assert_eq!(profiles[2].name, "Side");
        assert!(profiles[1].describe().contains("me@corp.com"));
    }

    #[test]
    fn parses_profiles_ini() {
        let raw = "[Install4F96D1932A9F858E]\nDefault=abc.default-release\n\n[Profile1]\nName=work\nIsRelative=1\nPath=x.work\n\n[Profile0]\nName=default-release\n\n[General]\nVersion=2\n";
        let names: Vec<String> = parse_profiles_ini(raw)
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, vec!["work", "default-release"]);
    }

    #[test]
    fn cleans_quoted_urls() {
        assert_eq!(
            clean_url_arg("\"https://claude.ai/x?a=1&b=2\""),
            "https://claude.ai/x?a=1&b=2"
        );
        assert!(is_web_url("'https://a'"));
        assert!(!is_web_url("claude://login"));
    }

    #[test]
    fn custom_command_substitutes_url() {
        let mut cfg = Config::default();
        let mut p = crate::config::Profile::default();
        p.browser.kind = BrowserKind::Custom;
        p.browser.command = Some("my-browser --flag \"{url}\"".into());
        cfg.profiles.insert("work".into(), p);
        let (prog, args) = plan(&cfg, "work", "https://x.y/z").unwrap();
        assert_eq!(prog, PathBuf::from("my-browser"));
        assert_eq!(args, vec!["--flag", "https://x.y/z"]);
    }
}
