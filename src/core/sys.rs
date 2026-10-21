//! Small cross-platform helpers: detached processes, state files, logging, Windows APIs.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::config::paths;

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `YYYY-MM-DD HH:MM:SS` in UTC, without pulling in a date crate.
pub fn timestamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Appends a line to `state/cswitch.log` (kept under ~256 KB).
pub fn log(event: impl AsRef<str>) {
    let Ok(dir) = paths::state_dir() else { return };
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("cswitch.log");
    if fs::metadata(&path)
        .map(|m| m.len() > 256 * 1024)
        .unwrap_or(false)
    {
        let _ = fs::rename(&path, dir.join("cswitch.log.old"));
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{} {}", timestamp(now_unix()), event.as_ref());
    }
}

pub fn recent_log_lines(count: usize) -> Vec<String> {
    let Ok(path) = paths::state_dir().map(|d| d.join("cswitch.log")) else {
        return Vec::new();
    };
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let lines: Vec<String> = text.lines().map(str::to_string).collect();
    lines[lines.len().saturating_sub(count)..].to_vec()
}

pub fn write_state<T: Serialize>(name: &str, value: &T) -> Result<()> {
    let dir = paths::state_dir()?;
    fs::create_dir_all(&dir).with_context(|| format!("could not create {}", dir.display()))?;
    let path = dir.join(name);
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

pub fn read_state<T: DeserializeOwned>(name: &str) -> Option<T> {
    let path = paths::state_dir().ok()?.join(name);
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn remove_state(name: &str) {
    if let Ok(dir) = paths::state_dir() {
        let _ = fs::remove_file(dir.join(name));
    }
}

/// Starts a process that outlives us, with no inherited console or stdio.
pub fn spawn_detached(cmd: &mut Command) -> Result<u32> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    let child = cmd
        .spawn()
        .with_context(|| format!("could not start {:?}", cmd.get_program()))?;
    Ok(child.id())
}

/// Quotes one argument for a Windows command line (CommandLineToArgvW rules).
pub fn quote_windows_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.chars().any(|c| matches!(c, ' ' | '\t' | '"')) {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

pub fn join_windows_args(args: &[String]) -> String {
    args.iter()
        .map(|a| quote_windows_arg(a))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Opens a folder or file with the platform's file manager / default app.
pub fn reveal(path: &Path) -> Result<()> {
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("explorer");
        c.arg(path);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(path);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(path);
        c
    };
    spawn_detached(&mut cmd)?;
    Ok(())
}

/// Opens a file in the user's editor ($VISUAL / $EDITOR, else a sensible default) and waits.
pub fn edit_file(path: &Path) -> Result<()> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| {
            if cfg!(windows) {
                "notepad".to_string()
            } else if cfg!(target_os = "macos") {
                "open -W -t".to_string()
            } else {
                "nano".to_string()
            }
        });
    let mut parts = split_command(&editor).unwrap_or_else(|_| vec![editor.clone()]);
    if parts.is_empty() {
        parts.push("notepad".to_string());
    }
    let program = parts.remove(0);
    // `which` honours PATHEXT, so editors installed as `code.cmd` and similar are found too.
    let resolved = which::which(&program).unwrap_or_else(|_| program.clone().into());
    let status = Command::new(&resolved)
        .args(&parts)
        .arg(path)
        .status()
        .with_context(|| format!("could not start editor '{program}'"))?;
    if !status.success() {
        anyhow::bail!("editor exited with {status}");
    }
    Ok(())
}

pub fn copy_dir_recursive(from: &Path, to: &Path) -> Result<u64> {
    copy_tree(from, to, 0)
}

fn copy_tree(from: &Path, to: &Path, depth: usize) -> Result<u64> {
    if depth > 32 {
        anyhow::bail!("{} is nested too deeply (a symlink loop?)", from.display());
    }
    let mut copied = 0;
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let src = entry.path();
        let dst = to.join(entry.file_name());
        let mut kind = entry.file_type()?;
        if kind.is_symlink() {
            // Skills, agents and commands are often symlinked in: copy what they point to.
            match fs::metadata(&src) {
                Ok(meta) => kind = meta.file_type(),
                Err(_) => continue, // dangling link
            }
        }
        if kind.is_dir() {
            copied += copy_tree(&src, &dst, depth + 1)?;
        } else if kind.is_file() {
            fs::copy(&src, &dst).with_context(|| {
                format!("could not copy {} -> {}", src.display(), dst.display())
            })?;
            copied += 1;
        }
    }
    Ok(copied)
}

pub fn dir_size(path: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            Ok(_) => e.metadata().map(|m| m.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Splits a user-written command line. Windows paths keep their backslashes (only double quotes
/// group words); elsewhere POSIX shell rules apply.
pub fn split_command(line: &str) -> Result<Vec<String>> {
    if cfg!(windows) {
        split_windows_command(line)
    } else {
        shell_words::split(line).context("could not parse the command line")
    }
}

/// The inverse of [`split_command`], for showing a stored argument list as editable text.
pub fn join_command(args: &[String]) -> String {
    if cfg!(windows) {
        args.iter()
            .map(|a| {
                if a.is_empty() || a.chars().any(char::is_whitespace) {
                    format!("\"{a}\"")
                } else {
                    a.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        shell_words::join(args)
    }
}

pub fn split_windows_command(line: &str) -> Result<Vec<String>> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut has_token = false;
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                has_token = true;
            }
            c if c.is_whitespace() && !quoted => {
                if has_token {
                    parts.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            c => {
                current.push(c);
                has_token = true;
            }
        }
    }
    if quoted {
        anyhow::bail!("unbalanced quotes in the command line");
    }
    if has_token {
        parts.push(current);
    }
    Ok(parts)
}

/// Standard base64 (RFC 4648) — small enough not to warrant a dependency.
pub fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// A hidden `powershell -EncodedCommand …` for `script`. Encoding sidesteps every quoting rule.
pub fn powershell(script: &str) -> Command {
    let utf16: Vec<u8> = script
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    let mut cmd = Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-EncodedCommand",
        &base64(&utf16),
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Does a program exist on PATH?
pub fn has_program(name: &str) -> bool {
    which::which(name).is_ok()
}

#[cfg(windows)]
pub mod win {
    //! Thin wrappers over the handful of Win32 calls we need.

    use anyhow::{Result, anyhow};
    use windows::Win32::System::Com::{
        CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    };
    use windows::Win32::System::Console::{
        AllocConsole, FreeConsole, GetConsoleProcessList, SetConsoleCtrlHandler, SetConsoleTitleW,
    };
    use windows::Win32::UI::Shell::{
        AO_NONE, ApplicationActivationManager, IApplicationActivationManager, SHCNE_ASSOCCHANGED,
        SHCNF_IDLIST, SHChangeNotify,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        HWND_BROADCAST, MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MB_SETFOREGROUND, MessageBoxW,
        SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
    };
    use windows::core::{HSTRING, PCWSTR};

    /// Gives a GUI-subsystem process a console window so it can show an interactive prompt.
    pub fn alloc_console(title: &str) -> bool {
        unsafe {
            if AllocConsole().is_err() {
                return false;
            }
            let _ = SetConsoleTitleW(&HSTRING::from(title));
        }
        true
    }

    pub fn free_console() {
        unsafe {
            let _ = FreeConsole();
        }
    }

    /// True when this process is the only one attached to its console, i.e. it was started by
    /// double-clicking in Explorer and the window will vanish as soon as we exit.
    pub fn owns_console() -> bool {
        let mut ids = [0u32; 4];
        let count = unsafe { GetConsoleProcessList(&mut ids) };
        count == 1
    }

    /// Lets Ctrl+C reach the child process without killing the parent first.
    ///
    /// A handler routine rather than `SetConsoleCtrlHandler(None, true)`: that "ignore Ctrl+C"
    /// flag is inherited by child processes, so `claude -p …` could not be interrupted.
    pub fn ignore_ctrl_c() {
        unsafe extern "system" fn survive(ctrl_type: u32) -> windows::core::BOOL {
            // CTRL_C_EVENT = 0, CTRL_BREAK_EVENT = 1: the child handles them; we keep waiting.
            windows::core::BOOL::from(ctrl_type <= 1)
        }
        unsafe {
            let _ = SetConsoleCtrlHandler(Some(survive), true);
        }
    }

    pub fn message_box(title: &str, text: &str, is_error: bool) {
        let icon = if is_error {
            MB_ICONERROR
        } else {
            MB_ICONINFORMATION
        };
        unsafe {
            MessageBoxW(
                None,
                &HSTRING::from(text),
                &HSTRING::from(title),
                MB_OK | icon | MB_SETFOREGROUND,
            );
        }
    }

    /// Tells Explorer and other apps that user environment variables changed.
    pub fn broadcast_environment_change() {
        let name = HSTRING::from("Environment");
        unsafe {
            let _ = SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                windows::Win32::Foundation::WPARAM(0),
                windows::Win32::Foundation::LPARAM(PCWSTR(name.as_ptr()).0 as isize),
                SMTO_ABORTIFHUNG,
                3000,
                None,
            );
        }
    }

    /// Tells the shell that file / URL associations changed.
    pub fn notify_association_change() {
        unsafe {
            SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
        }
    }

    /// Starts a packaged (Microsoft Store / MSIX) app with arguments and returns its PID.
    pub fn activate_packaged_app(aumid: &str, arguments: &str) -> Result<u32> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let manager: IApplicationActivationManager =
                CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_LOCAL_SERVER)
                    .map_err(|e| anyhow!("could not create ApplicationActivationManager: {e}"))?;
            manager
                .ActivateApplication(&HSTRING::from(aumid), &HSTRING::from(arguments), AO_NONE)
                .map_err(|e| anyhow!("Windows refused to start {aumid}: {e}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_timestamps() {
        assert_eq!(timestamp(0), "1970-01-01 00:00:00");
        assert_eq!(timestamp(1_700_000_000), "2023-11-14 22:13:20");
    }

    #[test]
    fn quotes_windows_args() {
        assert_eq!(quote_windows_arg("plain"), "plain");
        assert_eq!(quote_windows_arg("has space"), "\"has space\"");
        assert_eq!(quote_windows_arg(r"C:\a b\"), r#""C:\a b\\""#);
        assert_eq!(quote_windows_arg(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(quote_windows_arg(""), "\"\"");
    }

    #[test]
    fn splits_windows_commands_without_eating_backslashes() {
        let parts =
            split_windows_command(r#""C:\Program Files\B\b.exe" --new-window {url}"#).unwrap();
        assert_eq!(
            parts,
            vec![r"C:\Program Files\B\b.exe", "--new-window", "{url}"]
        );
        let bare = split_windows_command(r"C:\Tools\b.exe {url}").unwrap();
        assert_eq!(bare, vec![r"C:\Tools\b.exe", "{url}"]);
        assert!(split_windows_command("\"unbalanced").is_err());
    }

    #[test]
    fn encodes_base64() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn formats_sizes() {
        assert_eq!(human_size(10), "10 B");
        assert_eq!(human_size(2048), "2.0 KB");
    }

    #[test]
    fn joined_commands_split_back_unchanged() {
        let args: Vec<String> = ["--add-dir", r"C:\work\my proj", "--model", "opus"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let round_trip = if cfg!(windows) {
            split_windows_command(&join_command(&args)).unwrap()
        } else {
            split_command(&join_command(&args)).unwrap()
        };
        assert_eq!(round_trip, args);
        // Backslashes survive on Windows (POSIX rules would eat them).
        assert_eq!(
            split_windows_command(r"--add-dir C:\work\proj").unwrap(),
            vec!["--add-dir", r"C:\work\proj"]
        );
    }

    #[test]
    fn copies_trees_through_symlinks() {
        let base = std::env::temp_dir().join(format!("cswitch-copy-test-{}", std::process::id()));
        let src = base.join("src");
        fs::create_dir_all(src.join("skills/one")).unwrap();
        fs::write(src.join("skills/one/SKILL.md"), "x").unwrap();
        let copied = copy_dir_recursive(&src, &base.join("dst")).unwrap();
        assert_eq!(copied, 1);
        assert!(base.join("dst/skills/one/SKILL.md").is_file());
        let _ = fs::remove_dir_all(&base);
    }
}
