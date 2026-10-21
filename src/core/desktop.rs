//! Claude Desktop: finding the installation, launching one instance per profile, and telling
//! running instances apart.
//!
//! Claude Desktop is an Electron app that never overrides its own data path, so Chromium's
//! `--user-data-dir` switch moves *everything* (session cookies, local storage, MCP config,
//! logs) and also keys the single-instance lock. Two instances with different directories are
//! therefore two independent apps that can run at the same time.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::config::{Config, LaunchMethod, expand_home, normalize_path};
use crate::core::env::{self, Target};
use crate::core::sys;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InstallKind {
    /// Microsoft Store / MSIX package.
    Msix,
    /// Classic per-user installer (`%LOCALAPPDATA%\AnthropicClaude`).
    Squirrel,
    MacApp,
    Linux,
    Custom,
}

impl InstallKind {
    pub fn describe(self) -> &'static str {
        match self {
            InstallKind::Msix => "Microsoft Store (MSIX)",
            InstallKind::Squirrel => "Windows installer",
            InstallKind::MacApp => "macOS app",
            InstallKind::Linux => "Linux package",
            InstallKind::Custom => "custom path",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopInstall {
    pub kind: InstallKind,
    pub exe: PathBuf,
    /// Every Desktop process runs from below this directory.
    pub root: PathBuf,
    pub version: Option<String>,
    /// MSIX: AppUserModelID (`<family>!<app id>`).
    pub aumid: Option<String>,
    /// MSIX: package family name.
    pub family: Option<String>,
    pub app_bundle: Option<PathBuf>,
}

const CACHE: &str = "desktop-install.json";

/// Detects Claude Desktop. Results are cached because the Store query takes ~1 s.
pub fn detect(cfg: &Config, refresh: bool) -> Option<DesktopInstall> {
    if let Some(custom) = cfg
        .desktop
        .exe_path
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        let exe = expand_home(custom).ok()?;
        let text = exe.to_string_lossy().to_lowercase();
        let kind = if text.contains("windowsapps") {
            InstallKind::Msix
        } else {
            InstallKind::Custom
        };
        let root = exe
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| exe.clone());
        return Some(DesktopInstall {
            kind,
            exe,
            root,
            version: None,
            aumid: cfg.desktop.aumid.clone(),
            family: None,
            app_bundle: None,
        });
    }
    if !refresh
        && let Some(cached) = sys::read_state::<DesktopInstall>(CACHE)
        && (cached.exe.exists() || cached.kind == InstallKind::Msix && cached.root.exists())
    {
        return Some(apply_overrides(cfg, cached));
    }
    let found = platform::detect()?;
    let _ = sys::write_state(CACHE, &found);
    Some(apply_overrides(cfg, found))
}

fn apply_overrides(cfg: &Config, mut install: DesktopInstall) -> DesktopInstall {
    if let Some(aumid) = cfg
        .desktop
        .aumid
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        install.aumid = Some(aumid.to_string());
    }
    install
}

/// Where Desktop keeps data when started normally (the "system" storage), most likely first.
pub fn stock_data_dirs(install: Option<&DesktopInstall>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if cfg!(windows) {
        if let (Some(install), Some(local)) = (install, dirs::data_local_dir())
            && let Some(family) = &install.family
        {
            out.push(
                local
                    .join("Packages")
                    .join(family)
                    .join(r"LocalCache\Roaming\Claude"),
            );
        }
        if let Some(roaming) = dirs::data_dir() {
            out.push(roaming.join("Claude"));
        }
    } else if cfg!(target_os = "macos") {
        if let Some(home) = dirs::home_dir() {
            out.push(home.join("Library/Application Support/Claude"));
        }
    } else if let Some(config) = dirs::config_dir() {
        out.push(config.join("Claude"));
    }
    out
}

pub fn stock_data_dir(install: Option<&DesktopInstall>) -> Option<PathBuf> {
    let candidates = stock_data_dirs(install);
    candidates
        .iter()
        .find(|p| p.exists())
        .cloned()
        .or_else(|| candidates.first().cloned())
}

/// The directory that holds a profile's Desktop data (resolving "system").
pub fn effective_data_dir(
    cfg: &Config,
    install: Option<&DesktopInstall>,
    name: &str,
) -> Option<PathBuf> {
    match cfg.desktop_dir(name).ok()? {
        Some(dir) => Some(dir),
        None => stock_data_dir(install),
    }
}

#[derive(Debug, Clone)]
pub struct Instance {
    pub pid: u32,
    pub data_dir: Option<PathBuf>,
    pub profile: Option<String>,
}

/// Running Desktop main processes, mapped to profiles.
pub fn instances(cfg: &Config, install: &DesktopInstall) -> Vec<Instance> {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_cmd(UpdateKind::Always)
            .with_exe(UpdateKind::Always),
    );
    let root = normalize_path(&install.root);
    let target = normalize_path(&install.exe);
    let dedicated_root = matches!(
        install.kind,
        InstallKind::Msix | InstallKind::Squirrel | InstallKind::MacApp
    );
    let mut out = Vec::new();
    for (pid, process) in system.processes() {
        let cmd: Vec<String> = process
            .cmd()
            .iter()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        if cmd.iter().any(|a| a.starts_with("--type=")) {
            continue; // renderer / GPU / utility helpers
        }
        // On macOS / Linux Electron's crash reporter runs as `chrome_crashpad_handler` without
        // `--type=` or `--user-data-dir`; counting it would make a "system" profile look running.
        if process
            .name()
            .to_string_lossy()
            .to_lowercase()
            .contains("crashpad")
        {
            continue;
        }
        let exe = process.exe().map(normalize_path).unwrap_or_default();
        let belongs = if dedicated_root {
            !exe.is_empty() && exe.starts_with(&root)
        } else {
            // Linux / custom paths often live in shared folders such as /usr/bin, and may be
            // wrapper scripts run by an interpreter: match the executable itself, not its folder.
            exe == target
                || cmd
                    .iter()
                    .take(3)
                    .any(|a| normalize_path(Path::new(a)) == target)
                || (install.kind == InstallKind::Linux && is_linux_desktop_process(&exe, &cmd))
        };
        if !belongs {
            continue;
        }
        let data_dir = user_data_dir_arg(&cmd).map(PathBuf::from);
        let profile = profile_for_data_dir(cfg, data_dir.as_deref());
        out.push(Instance {
            pid: pid.as_u32(),
            data_dir,
            profile,
        });
    }
    // Every matching process is kept (a stub launcher or a forwarding second instance can
    // briefly coexist with the real one) so that `stop` gets all of them.
    out.sort_by_key(|i| i.pid);
    out
}

/// Community Linux builds run as `claude-desktop`, from `/opt/claude/`, or as
/// `electron …/claude-desktop/app.asar` (also via a shell wrapper). Only the program and the
/// script it runs count — not every argument — so `tail claude-desktop.log` or
/// `apt install claude-desktop` are never mistaken for (and stopped as) Claude.
fn is_linux_desktop_process(exe: &str, cmd: &[String]) -> bool {
    let ours = |s: &str| {
        let s = s.to_lowercase();
        s.contains("claude-desktop") || s.contains("/opt/claude/")
    };
    if ours(exe) || cmd.first().is_some_and(|a| ours(a)) {
        return true;
    }
    let launcher = cmd
        .first()
        .and_then(|a| Path::new(a).file_name())
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let interpreter = launcher.starts_with("electron")
        || matches!(launcher.as_str(), "sh" | "bash" | "dash" | "zsh" | "node");
    interpreter && cmd.get(1).is_some_and(|a| ours(a))
}

pub fn user_data_dir_arg(cmd: &[String]) -> Option<String> {
    let mut iter = cmd.iter();
    while let Some(arg) = iter.next() {
        if let Some(value) = arg.strip_prefix("--user-data-dir=") {
            return Some(value.trim_matches('"').to_string());
        }
        if arg == "--user-data-dir" {
            return iter.next().map(|v| v.trim_matches('"').to_string());
        }
    }
    None
}

fn profile_for_data_dir(cfg: &Config, data_dir: Option<&Path>) -> Option<String> {
    match data_dir {
        Some(dir) => {
            let wanted = normalize_path(dir);
            cfg.profiles.keys().find(
                |name| matches!(cfg.desktop_dir(name), Ok(Some(d)) if normalize_path(&d) == wanted),
            )
        }
        None => cfg
            .profiles
            .keys()
            .find(|name| matches!(cfg.desktop_dir(name), Ok(None))),
    }
    .cloned()
}

/// Raw process listing for `cswitch __procs` — helps diagnose detection on unusual installs.
pub fn debug_processes(filter: &str) -> Vec<String> {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_cmd(UpdateKind::Always)
            .with_exe(UpdateKind::Always),
    );
    let needle = filter.to_lowercase();
    let mut out: Vec<String> = system
        .processes()
        .iter()
        .filter(|(_, p)| {
            let name = p.name().to_string_lossy().to_lowercase();
            let exe = p
                .exe()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            name.contains(&needle) || exe.contains(&needle)
        })
        .map(|(pid, p)| {
            format!(
                "pid={} name={:?} exe={:?} normalized={:?} cmd={:?}",
                pid.as_u32(),
                p.name(),
                p.exe(),
                p.exe().map(normalize_path),
                p.cmd()
            )
        })
        .collect();
    out.sort();
    out
}

pub fn running_profiles(cfg: &Config, install: &DesktopInstall) -> Vec<String> {
    let mut names: Vec<String> = instances(cfg, install)
        .into_iter()
        .filter_map(|i| i.profile)
        .collect();
    names.sort();
    names.dedup();
    names
}

#[derive(Debug, Clone)]
pub struct LaunchReport {
    pub method: &'static str,
    pub pid: Option<u32>,
    pub already_running: bool,
}

#[derive(Serialize, Deserialize, Default)]
struct LastLaunched {
    profile: String,
    at: u64,
}

pub fn last_launched() -> Option<String> {
    sys::read_state::<LastLaunched>("last-desktop.json").map(|l| l.profile)
}

/// Starts (or focuses) the profile's Desktop instance, optionally handing it a `claude://` link.
pub fn launch(
    cfg: &Config,
    install: &DesktopInstall,
    name: &str,
    url: Option<&str>,
) -> Result<LaunchReport> {
    let data_dir = cfg.desktop_dir(name)?;
    if let Some(dir) = &data_dir {
        fs::create_dir_all(dir).with_context(|| format!("could not create {}", dir.display()))?;
    }
    let running = instances(cfg, install)
        .into_iter()
        .find(|i| i.profile.as_deref() == Some(name));

    // macOS Claude Desktop has no single-instance lock: a second process on the same data
    // directory would corrupt it. Focus the existing window instead (links arrive via Apple Events).
    if cfg!(target_os = "macos")
        && let Some(instance) = &running
    {
        focus_pid(instance.pid);
        return Ok(LaunchReport {
            method: "focus",
            pid: Some(instance.pid),
            already_running: true,
        });
    }

    let mut args: Vec<String> = Vec::new();
    if let Some(dir) = &data_dir {
        args.push(format!("--user-data-dir={}", dir.display()));
    }
    args.extend(cfg.desktop.extra_args.iter().cloned());
    if let Some(link) = url {
        // Claude Desktop reads the *last* argument as the link when it is already running.
        args.push(link.to_string());
    }
    let plan = env::plan(cfg, name, Target::Desktop)?;

    let wants_activation = install.kind == InstallKind::Msix
        && match cfg.desktop.launch_method {
            LaunchMethod::Activate => true,
            LaunchMethod::Direct => false,
            // Without --user-data-dir the Store app must run with its package identity, otherwise
            // it would look for its data outside the package's private AppData and appear signed out.
            LaunchMethod::Auto => data_dir.is_none(),
        };

    let report = if wants_activation {
        activate(install, &args)?
    } else {
        let mut cmd = Command::new(&install.exe);
        cmd.args(&args);
        plan.apply(&mut cmd);
        let store_auto =
            install.kind == InstallKind::Msix && cfg.desktop.launch_method == LaunchMethod::Auto;
        let direct = |pid: u32, already_running: bool| LaunchReport {
            method: "direct",
            pid: Some(pid),
            already_running,
        };
        match sys::spawn_detached_child(&mut cmd) {
            Ok(mut child) if store_auto && running.is_none() => match watch_start(&mut child) {
                Started::Running => direct(child.id(), false),
                Started::HandedOff => {
                    // Never re-send: sign-in links work once, and a second copy makes the
                    // instance that just used it fail the sign-in.
                    sys::log(
                        "desktop: handed over to an instance of this profile that was already \
                         running",
                    );
                    direct(child.id(), true)
                }
                Started::Failed(_)
                    if instances(cfg, install)
                        .iter()
                        .any(|i| i.profile.as_deref() == Some(name)) =>
                {
                    direct(child.id(), true)
                }
                Started::Failed(code) => {
                    // Some Store builds refuse to run without their package identity and exit at
                    // once; start them through package activation instead.
                    sys::log(format!(
                        "desktop: direct launch failed (exit code {}), falling back to activation \
                         (the profile's environment variables do not reach an activated Store app)",
                        code.map_or_else(|| "unknown".to_string(), |c| c.to_string())
                    ));
                    activate(install, &args)?
                }
            },
            Ok(child) => direct(child.id(), running.is_some()),
            Err(err) if store_auto => {
                sys::log(format!(
                    "desktop: direct launch failed ({err:#}), falling back to activation \
                     (the profile's environment variables do not reach an activated Store app)"
                ));
                activate(install, &args)?
            }
            Err(err) => return Err(err),
        }
    };
    let report = LaunchReport {
        already_running: running.is_some() || report.already_running,
        ..report
    };

    sys::log(format!(
        "desktop: launched profile={name} method={} link={}",
        report.method,
        url.map(redact_link).unwrap_or_default()
    ));
    let _ = sys::write_state(
        "last-desktop.json",
        &LastLaunched {
            profile: name.to_string(),
            at: sys::now_unix(),
        },
    );
    if cfg.desktop.router_enabled && !cfg!(target_os = "macos") {
        crate::core::router::schedule_reassert();
    }
    Ok(report)
}

enum Started {
    /// Still up after a few seconds: a new instance.
    Running,
    /// Exited cleanly: Electron's second instance, which passes its arguments (and the link) to
    /// the instance already running on the same data directory, then quits with code 0.
    HandedOff,
    /// Exited with an error, e.g. a Store build that refuses to run without its package identity.
    Failed(Option<i32>),
}

/// Watches a freshly started Desktop process for a few seconds.
fn watch_start(child: &mut Child) -> Started {
    for _ in 0..12 {
        std::thread::sleep(Duration::from_millis(250));
        match child.try_wait() {
            Ok(None) => {}
            Ok(Some(status)) if status.success() => return Started::HandedOff,
            Ok(Some(status)) => return Started::Failed(status.code()),
            Err(_) => return Started::Running,
        }
    }
    Started::Running
}

fn activate(install: &DesktopInstall, args: &[String]) -> Result<LaunchReport> {
    #[cfg(windows)]
    {
        let aumid = install.aumid.as_deref().context(
            "this Store installation has no AppUserModelID; set desktop.aumid in the config",
        )?;
        let pid = sys::win::activate_packaged_app(aumid, &sys::join_windows_args(args))?;
        Ok(LaunchReport {
            method: "activate",
            pid: Some(pid),
            already_running: false,
        })
    }
    #[cfg(not(windows))]
    {
        let _ = (install, args);
        anyhow::bail!("package activation is only available on Windows")
    }
}

/// Stops a profile's Desktop instance (main process and its helpers).
pub fn stop(cfg: &Config, install: &DesktopInstall, name: &str) -> Result<usize> {
    let targets: Vec<u32> = instances(cfg, install)
        .into_iter()
        .filter(|i| i.profile.as_deref() == Some(name))
        .map(|i| i.pid)
        .collect();
    if targets.is_empty() {
        return Ok(0);
    }
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
    // The whole tree: MCP servers are often grandchildren (npx.cmd → cmd.exe → node) and would
    // otherwise be orphaned, then duplicated by the next start.
    let mut doomed: std::collections::HashSet<u32> = targets.iter().copied().collect();
    loop {
        let before = doomed.len();
        for (pid, process) in system.processes() {
            if process
                .parent()
                .is_some_and(|parent| doomed.contains(&parent.as_u32()))
            {
                doomed.insert(pid.as_u32());
            }
        }
        if doomed.len() == before {
            break;
        }
    }
    let mut killed = 0;
    for (pid, process) in system.processes() {
        if doomed.contains(&pid.as_u32()) && process.kill() {
            killed += 1;
        }
    }
    sys::log(format!(
        "desktop: stopped profile={name} ({killed} processes)"
    ));
    std::thread::sleep(Duration::from_millis(600));
    Ok(killed)
}

#[cfg(target_os = "macos")]
fn focus_pid(pid: u32) {
    let script = format!(
        "tell application \"System Events\" to set frontmost of (first process whose unix id is {pid}) to true"
    );
    let _ = Command::new("osascript").args(["-e", &script]).output();
}

#[cfg(not(target_os = "macos"))]
fn focus_pid(_pid: u32) {}

/// Login links carry one-time codes; never write them to logs in full.
pub fn redact_link(url: &str) -> String {
    match url.find(['?', '#']) {
        Some(i) => format!("{}…", &url[..i]),
        None => url.to_string(),
    }
}

/// Extracts `attr="value"` from an XML start tag.
pub fn xml_attr(tag: &str, attr: &str) -> Option<String> {
    let needle = format!("{attr}=\"");
    let mut search = 0;
    while let Some(pos) = tag[search..].find(&needle) {
        let start = search + pos;
        let ok_boundary = start == 0 || tag[..start].ends_with(|c: char| c.is_whitespace());
        let value_start = start + needle.len();
        if ok_boundary {
            let end = tag[value_start..].find('"')? + value_start;
            return Some(tag[value_start..end].to_string());
        }
        search = value_start;
    }
    None
}

/// Finds the first `<Application …>` element of an AppxManifest and returns (Id, Executable).
pub fn parse_appx_application(manifest: &str) -> Option<(String, String)> {
    let start = manifest.find("<Application ")?;
    let end = manifest[start..].find('>')? + start;
    let tag = &manifest[start..end];
    Some((xml_attr(tag, "Id")?, xml_attr(tag, "Executable")?))
}

mod platform {
    use super::*;

    #[cfg(windows)]
    pub fn detect() -> Option<DesktopInstall> {
        detect_msix().or_else(detect_squirrel)
    }

    #[cfg(windows)]
    fn detect_msix() -> Option<DesktopInstall> {
        // Get-AppxPackageManifest reads the manifest through the package API, so this works even
        // where the WindowsApps folder itself is not readable.
        let script = "$ErrorActionPreference='SilentlyContinue'; \
            $r=@(Get-AppxPackage -Name '*Claude*' | ForEach-Object { \
              $m = Get-AppxPackageManifest -Package $_.PackageFullName; \
              $app = @($m.Package.Applications.Application)[0]; \
              [pscustomobject]@{ Name=$_.Name; Family=$_.PackageFamilyName; Location=$_.InstallLocation; \
                Version=\"$($_.Version)\"; AppId=$app.Id; Executable=$app.Executable; \
                Publisher=\"$($m.Package.Properties.PublisherDisplayName) $($_.Publisher)\" } }); \
            ConvertTo-Json -InputObject $r -Compress";
        let out = sys::powershell(script).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let packages: Vec<serde_json::Value> = serde_json::from_str(text.trim()).ok()?;
        for pkg in packages {
            let field = |k: &str| {
                pkg.get(k)
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(str::to_string)
            };
            let (Some(family), Some(location)) = (field("Family"), field("Location")) else {
                continue;
            };
            let location = PathBuf::from(location);
            let manifest =
                fs::read_to_string(location.join("AppxManifest.xml")).unwrap_or_default();
            let by_anthropic = field("Publisher").is_some_and(|p| p.contains("Anthropic"))
                || manifest.contains("Anthropic");
            if !by_anthropic {
                continue;
            }
            let from_manifest = parse_appx_application(&manifest);
            let (Some(app_id), Some(executable)) = (
                field("AppId").or_else(|| from_manifest.as_ref().map(|m| m.0.clone())),
                field("Executable").or_else(|| from_manifest.as_ref().map(|m| m.1.clone())),
            ) else {
                continue;
            };
            return Some(DesktopInstall {
                kind: InstallKind::Msix,
                exe: location.join(executable.replace('/', "\\")),
                root: location,
                version: field("Version"),
                aumid: Some(format!("{family}!{app_id}")),
                family: Some(family),
                app_bundle: None,
            });
        }
        None
    }

    #[cfg(windows)]
    fn detect_squirrel() -> Option<DesktopInstall> {
        let root = dirs::data_local_dir()?.join("AnthropicClaude");
        let exe = root.join("claude.exe");
        if !exe.exists() {
            return None;
        }
        let version = fs::read_dir(&root)
            .ok()?
            .flatten()
            .filter_map(|e| {
                e.file_name()
                    .to_string_lossy()
                    .strip_prefix("app-")
                    .map(str::to_string)
            })
            .max_by(|a, b| compare_versions(a, b));
        Some(DesktopInstall {
            kind: InstallKind::Squirrel,
            exe,
            root,
            version,
            aumid: None,
            family: None,
            app_bundle: None,
        })
    }

    #[cfg(target_os = "macos")]
    pub fn detect() -> Option<DesktopInstall> {
        let home = dirs::home_dir()?;
        for root in [PathBuf::from("/Applications"), home.join("Applications")] {
            let bundle = root.join("Claude.app");
            let exe = bundle.join("Contents/MacOS/Claude");
            if exe.exists() {
                let version = fs::read_to_string(bundle.join("Contents/Info.plist"))
                    .ok()
                    .and_then(|plist| plist_string(&plist, "CFBundleShortVersionString"));
                return Some(DesktopInstall {
                    kind: InstallKind::MacApp,
                    exe,
                    root: bundle.clone(),
                    version,
                    aumid: None,
                    family: None,
                    app_bundle: Some(bundle),
                });
            }
        }
        None
    }

    #[cfg(target_os = "macos")]
    fn plist_string(plist: &str, key: &str) -> Option<String> {
        let at = plist.find(&format!("<key>{key}</key>"))?;
        let rest = &plist[at..];
        let start = rest.find("<string>")? + "<string>".len();
        let end = rest[start..].find("</string>")? + start;
        Some(rest[start..end].trim().to_string())
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    pub fn detect() -> Option<DesktopInstall> {
        let candidates = [
            which::which("claude-desktop").ok(),
            Some(PathBuf::from("/usr/bin/claude-desktop")),
            Some(PathBuf::from("/opt/Claude/claude")),
            Some(PathBuf::from("/usr/lib/claude-desktop/claude-desktop")),
        ];
        let exe = candidates.into_iter().flatten().find(|p| p.exists())?;
        let root = exe
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| exe.clone());
        Some(DesktopInstall {
            kind: InstallKind::Linux,
            exe,
            root,
            version: None,
            aumid: None,
            family: None,
            app_bundle: None,
        })
    }

    #[cfg(windows)]
    fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
        let parse = |v: &str| {
            v.split('.')
                .map(|p| p.parse::<u64>().unwrap_or(0))
                .collect::<Vec<_>>()
        };
        parse(a).cmp(&parse(b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &str) -> Vec<String> {
        line.split(' ').map(str::to_string).collect()
    }

    #[test]
    fn matches_linux_desktop_processes_by_program_only() {
        assert!(is_linux_desktop_process(
            "/usr/lib/claude-desktop/claude-desktop",
            &args("/usr/lib/claude-desktop/claude-desktop --user-data-dir=/x")
        ));
        assert!(is_linux_desktop_process(
            "/usr/lib/electron/electron",
            &args("electron /usr/lib/claude-desktop/app.asar")
        ));
        assert!(is_linux_desktop_process(
            "/usr/bin/bash",
            &args("/bin/bash /usr/bin/claude-desktop")
        ));
        assert!(!is_linux_desktop_process(
            "/usr/bin/tail",
            &args("tail -f /var/log/claude-desktop.log")
        ));
        assert!(!is_linux_desktop_process(
            "/usr/bin/apt",
            &args("apt install claude-desktop")
        ));
    }

    #[test]
    fn finds_user_data_dir_in_both_forms() {
        let a = vec![
            "Claude.exe".to_string(),
            "--user-data-dir=C:\\x y\\d".to_string(),
        ];
        assert_eq!(user_data_dir_arg(&a).as_deref(), Some("C:\\x y\\d"));
        let b = vec![
            "claude".into(),
            "--user-data-dir".into(),
            "/tmp/p".into(),
            "claude://x".into(),
        ];
        assert_eq!(user_data_dir_arg(&b).as_deref(), Some("/tmp/p"));
        assert_eq!(user_data_dir_arg(&["x".to_string()]), None);
    }

    #[test]
    fn parses_appx_manifest_application() {
        let manifest = r#"<Package><Applications>
            <Application Id="Claude" Executable="app\Claude.exe" EntryPoint="Windows.FullTrustApplication">
            </Application></Applications></Package>"#;
        assert_eq!(
            parse_appx_application(manifest),
            Some(("Claude".to_string(), "app\\Claude.exe".to_string()))
        );
        // Attribute names that merely end in "Id" must not match.
        let tricky = r#"<Application AppId="nope" Id="Real" Executable="a.exe">"#;
        assert_eq!(parse_appx_application(tricky).unwrap().0, "Real");
    }

    #[test]
    fn redacts_login_codes() {
        assert_eq!(
            redact_link("claude://login/google-auth?code=SECRET"),
            "claude://login/google-auth…"
        );
        assert_eq!(
            redact_link("claude://claude.ai/magic-link#a:b"),
            "claude://claude.ai/magic-link…"
        );
    }
}
