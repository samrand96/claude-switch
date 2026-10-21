//! `cswitch doctor`: checks every moving part and says exactly how to fix what is wrong.

use std::path::PathBuf;

use anyhow::Result;

use super::DoctorArgs;
use super::install_cmd;
use super::profiles::storage_conflicts;
use crate::config::{BrowserKind, BrowserMode, Config, paths};
use crate::core::code::{self, Credentials};
use crate::core::router::{self, RouterState};
use crate::core::{browser, desktop, env, integrate, sys};
use crate::ui;

#[derive(Default)]
struct Tally {
    problems: usize,
    warnings: usize,
}

impl Tally {
    fn fail(&mut self, msg: impl AsRef<str>, fix: impl AsRef<str>) {
        self.problems += 1;
        ui::fail(msg);
        if !fix.as_ref().is_empty() {
            ui::hint(format!("fix: {}", fix.as_ref()));
        }
    }

    fn warn(&mut self, msg: impl AsRef<str>, fix: impl AsRef<str>) {
        self.warnings += 1;
        ui::warn(msg);
        if !fix.as_ref().is_empty() {
            ui::hint(fix.as_ref());
        }
    }
}

/// Returns the number of problems found (`cswitch doctor` exits 1 when there are any).
pub fn run(cfg: &mut Config, args: DoctorArgs) -> Result<usize> {
    let mut t = Tally::default();
    if args.fix {
        fix(cfg)?;
    }

    ui::section("Claude Switch");
    ui::ok(format!("version {}", env!("CARGO_PKG_VERSION")));
    ui::ok(format!("config {}", paths::config_file()?.display()));
    let installed = paths::installed_exe()?;
    if installed.exists() {
        ui::ok(format!("installed at {}", installed.display()));
        if cfg!(windows) && !paths::bin_dir()?.join("cswitchw.exe").exists() {
            t.fail(
                "cswitchw.exe is missing (needed for links and sign-in pages)",
                "cswitch install",
            );
        }
    } else {
        t.warn(
            "not installed yet (running from a download folder)",
            "run `cswitch install`",
        );
    }
    if integrate::bin_on_path() {
        ui::ok("profile commands are on PATH");
    } else {
        t.warn(
            "~/.cswitch/bin is not on PATH, so claude-<profile> commands won't be found",
            "cswitch install",
        );
    }
    for name in cfg.profile_names() {
        if installed.exists() && !integrate::shim_path(&name)?.exists() {
            t.warn(
                format!("command claude-{name} is missing"),
                "cswitch install",
            );
        }
    }

    ui::section("Claude Code");
    match code::resolve_claude(cfg) {
        Ok(path) => {
            let version = code::version(&path).unwrap_or_else(|| "version unknown".into());
            ui::ok(format!("{} ({version})", path.display()));
        }
        Err(err) => t.fail(
            format!("{err:#}"),
            "install Claude Code: https://claude.com/claude-code",
        ),
    }
    let stripped = env::stripped_from_current(cfg, &[]);
    if stripped.is_empty() {
        ui::ok("no inherited Claude/Anthropic variables in this environment");
    } else {
        let shown: Vec<&str> = stripped.iter().take(6).map(String::as_str).collect();
        let more = stripped.len().saturating_sub(shown.len());
        ui::note(format!(
            "isolation strips {} inherited Claude/Anthropic variable(s) from every profile: {}{}",
            stripped.len(),
            ui::accent2(shown.join(", ")),
            if more > 0 {
                ui::dim(format!(" … and {more} more (see `cswitch env`)"))
            } else {
                String::new()
            }
        ));
        let leaks: Vec<&String> = stripped
            .iter()
            .filter(|k| {
                matches!(
                    k.to_uppercase().as_str(),
                    "ANTHROPIC_API_KEY"
                        | "CLAUDE_CODE_OAUTH_TOKEN"
                        | "ANTHROPIC_AUTH_TOKEN"
                        | "CLAUDE_CONFIG_DIR"
                )
            })
            .collect();
        if !leaks.is_empty() {
            t.warn(
                format!(
                    "{} is set globally — plain `claude` (outside Claude Switch) uses it and ignores your sign-in",
                    leaks.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
                ),
                "remove it from your system environment, or move it into one profile's environment variables",
            );
        }
    }
    for managed in managed_settings_paths() {
        if managed.exists() {
            ui::note(format!(
                "organization policy {} applies to every profile (that's expected for managed machines)",
                managed.display()
            ));
        }
    }

    ui::section("Claude Desktop");
    let install = desktop::detect(cfg, true);
    match &install {
        Some(i) => {
            ui::ok(format!(
                "{} {} — {}",
                i.kind.describe(),
                i.version.clone().unwrap_or_default(),
                i.exe.display()
            ));
            if let Some(aumid) = &i.aumid {
                ui::hint(format!(
                    "AppUserModelID {aumid} · launch method {}",
                    cfg.desktop.launch_method
                ));
            }
            if let Some(stock) = desktop::stock_data_dir(Some(i)) {
                ui::hint(format!("original data folder {}", stock.display()));
            }
            let instances = desktop::instances(cfg, i);
            for inst in &instances {
                let who = inst
                    .profile
                    .clone()
                    .map(|p| cfg.display_name(&p))
                    .unwrap_or_else(|| {
                        inst.data_dir
                            .as_ref()
                            .map(|d| format!("unmanaged ({})", d.display()))
                            .unwrap_or_else(|| "original (not a profile)".into())
                    });
                ui::note(format!("running: pid {} — {who}", inst.pid));
            }
        }
        None => ui::note("not installed — only Claude Code profiles are available"),
    }

    if install.is_some() {
        ui::section("Sign-in link router (claude://)");
        match router::state() {
            RouterState::Active => ui::ok("claude:// links reach Claude Switch and are routed per profile"),
            RouterState::NotDefault { current } => t.fail(
                format!("claude:// links open in {current}, so Desktop sign-in lands in the wrong window"),
                if cfg!(windows) {
                    "cswitch router settings → link type CLAUDE → \"Claude Switch\""
                } else {
                    "cswitch router register"
                },
            ),
            RouterState::NotRegistered if cfg.desktop.router_enabled => {
                t.fail("the claude:// router is not registered", "cswitch router register")
            }
            RouterState::NotRegistered => ui::note("router disabled in settings"),
            RouterState::Unsupported => ui::note("macOS: `cswitch login` signs in one profile at a time instead"),
        }
        if let Some(arm) = router::active_arm() {
            ui::note(format!(
                "sign-in routing → {} until {} UTC",
                cfg.display_name(&arm.profile),
                sys::timestamp(arm.until)
            ));
        }
        let recent: Vec<String> = sys::recent_log_lines(40)
            .into_iter()
            .filter(|l| l.contains("router:"))
            .rev()
            .take(4)
            .collect();
        for line in recent.into_iter().rev() {
            ui::hint(line);
        }
        if args.test_router && !install_cmd::test_router()? {
            t.problems += 1;
        }
    }

    ui::section("Profiles");
    if cfg.profiles.is_empty() {
        t.warn("no profiles", "cswitch setup");
    }
    let mut seen_accounts: Vec<(String, String)> = Vec::new();
    for name in cfg.profile_names() {
        let profile = cfg.profile(&name)?.clone();
        println!("  {}", ui::badge(&name, Some(&profile)));
        let code_dir = cfg.effective_code_dir(&name)?;
        let account = code::account(cfg, &name);
        match (code::credentials(cfg, &name), &account) {
            (Credentials::Missing, _) => t.warn(
                format!("Claude Code not signed in ({})", code_dir.display()),
                format!("cswitch login {name} --code"),
            ),
            (_, Some(a)) => {
                match code::email_matches(profile.email.as_deref(), a.email.as_deref()) {
                    Some(false) => t.fail(
                        format!(
                            "Claude Code is signed in as {} but should be {}",
                            a.summary(),
                            profile.email.clone().unwrap_or_default()
                        ),
                        format!("cswitch logout {name} --code && cswitch login {name} --code"),
                    ),
                    _ => ui::ok(format!("Claude Code: {}", a.summary())),
                }
                if let Some(email) = &a.email {
                    if let Some((other, _)) = seen_accounts
                        .iter()
                        .find(|(_, e)| e.eq_ignore_ascii_case(email))
                    {
                        t.fail(
                            format!(
                                "same account as profile '{other}' — the two are not separated"
                            ),
                            format!("sign one of them out: cswitch logout {name} --code"),
                        );
                    }
                    seen_accounts.push((name.clone(), email.clone()));
                }
            }
            _ => ui::ok("Claude Code: signed in"),
        }
        if let Some(i) = &install {
            match desktop::effective_data_dir(cfg, Some(i), &name) {
                Some(dir) if dir.exists() => {
                    ui::ok(format!("Claude Desktop data {}", dir.display()))
                }
                Some(dir) => ui::note(format!("Claude Desktop not used yet ({})", dir.display())),
                None => {}
            }
        }
        check_browser(&mut t, cfg, &name);
    }
    for problem in storage_conflicts(cfg) {
        t.fail(
            problem,
            "give each profile its own storage: cswitch profile edit",
        );
    }

    println!();
    if t.problems == 0 && t.warnings == 0 {
        ui::ok(ui::bold("Everything looks good."));
    } else {
        let summary = format!("{} problem(s), {} warning(s)", t.problems, t.warnings);
        if t.problems > 0 {
            ui::fail(summary)
        } else {
            ui::warn(summary)
        }
        if !args.fix {
            ui::hint(
                "`cswitch doctor --fix` repairs installation, PATH, commands and the link router.",
            );
        }
    }
    Ok(t.problems)
}

fn check_browser(t: &mut Tally, cfg: &Config, name: &str) {
    let Ok(profile) = cfg.profile(name) else {
        return;
    };
    let choice = &profile.browser;
    match choice.kind {
        BrowserKind::System => ui::warn(format!(
            "sign-in pages open in the system browser — fine if you pick the right account there; {}",
            ui::dim("a sandbox or browser profile is safer")
        )),
        BrowserKind::Custom => ui::ok(format!("sign-in browser: {}", browser::describe(choice))),
        kind => match browser::resolve(choice) {
            None => t.fail(
                format!(
                    "{} is configured for sign-in but was not found",
                    kind.display()
                ),
                format!("cswitch profile edit {name} → Sign-in browser"),
            ),
            Some(found) => {
                if choice.mode == BrowserMode::Profile
                    && let Some(id) = &choice.profile
                {
                    let exists = browser::profiles(&found).iter().any(|p| &p.id == id);
                    if !exists {
                        t.warn(
                            format!("{} profile '{id}' no longer exists", kind.display()),
                            format!("cswitch profile edit {name} → Sign-in browser"),
                        );
                        return;
                    }
                }
                ui::ok(format!("sign-in browser: {}", browser::describe(choice)));
            }
        },
    }
}

fn managed_settings_paths() -> Vec<PathBuf> {
    if cfg!(windows) {
        vec![
            PathBuf::from(r"C:\Program Files\ClaudeCode\managed-settings.json"),
            PathBuf::from(r"C:\ProgramData\ClaudeCode\managed-settings.json"),
        ]
    } else if cfg!(target_os = "macos") {
        vec![PathBuf::from(
            "/Library/Application Support/ClaudeCode/managed-settings.json",
        )]
    } else {
        vec![PathBuf::from("/etc/claude-code/managed-settings.json")]
    }
}

fn fix(cfg: &mut Config) -> Result<()> {
    ui::section("Repairing");
    integrate::install_binary()?;
    ui::ok("binary installed");
    if !cfg.profiles.is_empty() {
        integrate::sync_shims(cfg)?;
        ui::ok("profile commands refreshed");
    }
    if !integrate::bin_on_path() {
        ui::ok(integrate::add_bin_to_path()?);
    }
    for name in cfg.profile_names() {
        if let Some(dir) = cfg.code_dir(&name)? {
            std::fs::create_dir_all(dir)?;
        }
    }
    if desktop::detect(cfg, true).is_some()
        && cfg.desktop.router_enabled
        && !cfg!(target_os = "macos")
    {
        router::register()?;
        ui::ok("claude:// router registered");
        if let RouterState::NotDefault { .. } = router::state()
            && cfg!(windows)
            && super::common::interactive()
        {
            super::login::ensure_router(true)?;
        }
    }
    Ok(())
}
