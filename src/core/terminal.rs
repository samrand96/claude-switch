//! Opening Claude Code for a profile in a new terminal tab or window, so profiles run side by side.

use std::path::Path;
use std::process::Command;

use anyhow::Result;

use crate::config::{Config, paths};
#[cfg(not(target_os = "macos"))]
use crate::core::sys;

pub fn open_code_in_new_terminal(
    cfg: &Config,
    name: &str,
    cwd: &Path,
    extra: &[String],
) -> Result<String> {
    let exe = paths::stable_exe()?;
    let title = format!("Claude · {}", cfg.display_name(name));
    // `--here`: the new window must not obey `code_launch = "new-window"` and open yet another.
    let mut args: Vec<String> = vec!["code".into(), name.into(), "--here".into()];
    if !extra.is_empty() {
        args.push("--".into());
        args.extend(extra.iter().cloned());
    }
    let color = cfg.profile(name)?.color.clone();
    platform::open(&exe, &args, cwd, &title, &color)
}

mod platform {
    use super::*;

    #[cfg(windows)]
    pub fn open(
        exe: &Path,
        args: &[String],
        cwd: &Path,
        title: &str,
        color: &str,
    ) -> Result<String> {
        use std::os::windows::process::CommandExt;
        if sys::has_program("wt") {
            let mut cmd = Command::new("wt");
            cmd.args([
                "-w",
                "0",
                "new-tab",
                "--title",
                title,
                "--suppressApplicationTitle",
            ]);
            if let Some(rgb) = crate::ui::theme::Rgb::parse(color) {
                // wt only accepts #RRGGBB; hand-edited configs may omit the '#'.
                cmd.args(["--tabColor", &rgb.hex()]);
            }
            cmd.arg("-d").arg(cwd).arg(exe).args(args);
            sys::spawn_detached(&mut cmd)?;
            return Ok("new Windows Terminal tab".to_string());
        }
        let line = format!(
            "/c start {} /D {} {} {}",
            sys::quote_windows_arg(title),
            sys::quote_windows_arg(&cwd.to_string_lossy()),
            sys::quote_windows_arg(&exe.to_string_lossy()),
            sys::join_windows_args(args)
        );
        let mut cmd = Command::new("cmd");
        cmd.raw_arg(line);
        sys::spawn_detached(&mut cmd)?;
        Ok("new console window".to_string())
    }

    #[cfg(target_os = "macos")]
    pub fn open(
        exe: &Path,
        args: &[String],
        cwd: &Path,
        title: &str,
        _color: &str,
    ) -> Result<String> {
        let quoted: Vec<String> = args
            .iter()
            .map(|a| shell_words::quote(a).to_string())
            .collect();
        let line = format!(
            "cd {} && printf '\\033]0;{}\\007' && {} {}",
            shell_words::quote(&cwd.to_string_lossy()),
            title.replace('\'', ""),
            shell_words::quote(&exe.to_string_lossy()),
            quoted.join(" ")
        );
        let escaped = line.replace('\\', "\\\\").replace('"', "\\\"");
        let iterm = std::env::var("TERM_PROGRAM")
            .map(|t| t == "iTerm.app")
            .unwrap_or(false);
        let script = if iterm {
            format!(
                "tell application \"iTerm\"\ncreate window with default profile\ntell current session of current window to write text \"{escaped}\"\nend tell"
            )
        } else {
            format!("tell application \"Terminal\"\ndo script \"{escaped}\"\nactivate\nend tell")
        };
        let status = Command::new("osascript").args(["-e", &script]).status()?;
        if !status.success() {
            anyhow::bail!("could not open a new terminal window");
        }
        Ok(if iterm {
            "new iTerm window"
        } else {
            "new Terminal window"
        }
        .to_string())
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    pub fn open(
        exe: &Path,
        args: &[String],
        cwd: &Path,
        title: &str,
        _color: &str,
    ) -> Result<String> {
        let mut program: Vec<String> = vec![exe.to_string_lossy().to_string()];
        program.extend(args.iter().cloned());
        let preferred = std::env::var("TERMINAL").ok().filter(|t| !t.is_empty());
        let candidates: Vec<(&str, Vec<String>)> = vec![
            (
                "gnome-terminal",
                vec!["--title".into(), title.into(), "--".into()],
            ),
            (
                "konsole",
                vec!["-p".into(), format!("tabtitle={title}"), "-e".into()],
            ),
            ("kitty", vec!["--title".into(), title.into()]),
            (
                "alacritty",
                vec!["--title".into(), title.into(), "-e".into()],
            ),
            ("wezterm", vec!["start".into(), "--".into()]),
            ("foot", vec!["--title".into(), title.into()]),
            ("x-terminal-emulator", vec!["-e".into()]),
            ("xterm", vec!["-T".into(), title.into(), "-e".into()]),
        ];
        if let Some(term) = preferred {
            let mut cmd = Command::new(&term);
            cmd.arg("-e").args(&program).current_dir(cwd);
            if sys::spawn_detached(&mut cmd).is_ok() {
                return Ok(format!("new {term} window"));
            }
        }
        for (term, prefix) in candidates {
            if !sys::has_program(term) {
                continue;
            }
            let mut cmd = Command::new(term);
            cmd.args(&prefix).args(&program).current_dir(cwd);
            if sys::spawn_detached(&mut cmd).is_ok() {
                return Ok(format!("new {term} window"));
            }
        }
        anyhow::bail!("no supported terminal emulator found (set $TERMINAL)")
    }
}
