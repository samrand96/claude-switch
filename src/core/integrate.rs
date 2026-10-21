//! Installing into the system: the binary itself, `claude-<profile>` commands, PATH, shortcuts.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::config::{Config, paths};
use crate::core::desktop::DesktopInstall;
use crate::core::sys;
use crate::ui::theme::Rgb;

/// Copies the running binary to `~/.cswitch/bin` (and makes `cswitchw.exe` on Windows).
/// A binary installed by a system package (.deb, .rpm, .pkg …) is linked instead of copied, so
/// upgrading the package upgrades `cswitch` and every `claude-<profile>` command with it.
/// Returns the installed path.
pub fn install_binary() -> Result<PathBuf> {
    let bin = paths::bin_dir()?;
    fs::create_dir_all(&bin).with_context(|| format!("could not create {}", bin.display()))?;
    let current = std::env::current_exe().context("could not locate the running executable")?;
    let target = paths::installed_exe()?;
    let same = match (fs::canonicalize(&current), fs::canonicalize(&target)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if !same {
        if is_package_managed(&current) {
            link_replacing(&current, &target)?;
        } else {
            replace_file(&current, &target)?;
        }
    }
    clean_parked_files(&bin);
    if cfg!(windows) {
        let windowed = bin.join("cswitchw.exe");
        let bytes = fs::read(&target)?;
        let patched = make_windowed(&bytes)?;
        write_replacing(&windowed, &patched)?;
    }
    #[cfg(unix)]
    if !is_symlink(&target) {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&target, fs::Permissions::from_mode(0o755));
    }
    sys::log(format!("install: binary at {}", target.display()));
    Ok(target)
}

/// A root-owned binary run by a regular user was put there by a package manager (or an admin)
/// and is updated in place — link to it rather than freezing a copy.
#[cfg(unix)]
fn is_package_managed(exe: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let euid = unsafe { libc::geteuid() };
    euid != 0 && fs::metadata(exe).is_ok_and(|m| m.uid() == 0)
}

#[cfg(not(unix))]
fn is_package_managed(_exe: &Path) -> bool {
    false
}

fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink())
}

#[cfg(unix)]
fn link_replacing(original: &Path, link: &Path) -> Result<()> {
    remove_or_park(link)?;
    std::os::unix::fs::symlink(original, link).with_context(|| {
        format!(
            "could not link {} -> {}",
            link.display(),
            original.display()
        )
    })
}

#[cfg(not(unix))]
fn link_replacing(original: &Path, link: &Path) -> Result<()> {
    replace_file(original, link)
}

/// Removes executables parked by earlier updates (they may still have been running then).
fn clean_parked_files(dir: &Path) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().contains(".old-") {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

/// Deletes a file, or moves it aside when Windows keeps it locked because it is running.
fn remove_or_park(path: &Path) -> Result<()> {
    if !path.exists() && fs::symlink_metadata(path).is_err() {
        return Ok(());
    }
    if fs::remove_file(path).is_ok() {
        return Ok(());
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let parked = path.with_file_name(format!("{name}.old-{}", sys::now_unix()));
    fs::rename(path, &parked).with_context(|| format!("{} is in use", path.display()))?;
    Ok(())
}

fn replace_file(from: &Path, to: &Path) -> Result<()> {
    let bytes = fs::read(from).with_context(|| format!("could not read {}", from.display()))?;
    write_replacing(to, &bytes)
}

/// Writes an executable even if an older copy is currently running.
///
/// macOS / Linux: write a new file and rename it over the old one. The new inode matters —
/// rewriting a signed binary in place makes macOS kill it on its next launch ("Killed: 9"),
/// Linux refuses to write a running one, and a symlink (to a package-managed binary) is
/// replaced rather than written through.
#[cfg(unix)]
fn write_replacing(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.new-{}", std::process::id()));
    fs::write(&tmp, bytes).with_context(|| format!("could not write {}", tmp.display()))?;
    let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755));
    fs::rename(&tmp, path).map_err(|err| {
        let _ = fs::remove_file(&tmp);
        anyhow::Error::new(err).context(format!("could not replace {}", path.display()))
    })
}

/// Windows locks running .exe files against writing, but allows renaming them out of the way.
#[cfg(not(unix))]
fn write_replacing(path: &Path, bytes: &[u8]) -> Result<()> {
    if is_symlink(path) {
        fs::remove_file(path).with_context(|| format!("could not replace {}", path.display()))?;
    }
    if path.exists() {
        if fs::write(path, bytes).is_ok() {
            return Ok(());
        }
        remove_or_park(path).with_context(|| {
            format!(
                "{} is in use — close other Claude Switch windows and retry",
                path.display()
            )
        })?;
    }
    fs::write(path, bytes).with_context(|| format!("could not write {}", path.display()))?;
    Ok(())
}

/// Turns a console-subsystem PE image into a GUI-subsystem one (what `editbin /SUBSYSTEM:WINDOWS`
/// does). The result never opens a console window — used for the link handler and `$BROWSER`.
pub fn make_windowed(image: &[u8]) -> Result<Vec<u8>> {
    let read_u32 = |at: usize| -> Option<u32> {
        image
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    if image.get(0..2) != Some(b"MZ") {
        bail!("not a Windows executable");
    }
    let pe = read_u32(0x3C).context("truncated executable")? as usize;
    if image.get(pe..pe + 4) != Some(b"PE\0\0") {
        bail!("missing PE signature");
    }
    // Optional header follows the 4-byte signature and the 20-byte COFF header; the Subsystem
    // field sits at offset 68 for both PE32 and PE32+.
    let subsystem = pe + 4 + 20 + 68;
    if image.len() < subsystem + 2 {
        bail!("truncated optional header");
    }
    let mut out = image.to_vec();
    out[subsystem] = 2; // IMAGE_SUBSYSTEM_WINDOWS_GUI
    out[subsystem + 1] = 0;
    Ok(out)
}

pub fn shim_path(name: &str) -> Result<PathBuf> {
    Ok(paths::bin_dir()?.join(paths::exe_name(&format!("claude-{name}"))))
}

/// Creates `claude-<profile>` for every profile and removes shims of deleted profiles.
pub fn sync_shims(cfg: &Config) -> Result<Vec<PathBuf>> {
    let bin = paths::bin_dir()?;
    fs::create_dir_all(&bin)?;
    let target = paths::installed_exe()?;
    if !target.exists() {
        bail!("Claude Switch is not installed yet — run `cswitch install` first");
    }
    let wanted: Vec<PathBuf> = cfg
        .profiles
        .keys()
        .map(|n| shim_path(n))
        .collect::<Result<_>>()?;
    for entry in fs::read_dir(&bin)?.flatten() {
        let path = entry.path();
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        if stem.starts_with("claude-") && !wanted.contains(&path) {
            let _ = fs::remove_file(&path);
        }
    }
    let mut created = Vec::new();
    for shim in wanted {
        remove_or_park(&shim)?;
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&target, &shim)
                .with_context(|| format!("could not create {}", shim.display()))?;
        }
        #[cfg(windows)]
        {
            if fs::hard_link(&target, &shim).is_err() {
                fs::copy(&target, &shim)
                    .with_context(|| format!("could not create {}", shim.display()))?;
            }
        }
        created.push(shim);
    }
    Ok(created)
}

pub fn bin_on_path() -> bool {
    let Ok(bin) = paths::bin_dir() else {
        return false;
    };
    let wanted = crate::config::normalize_path(&bin);
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| crate::config::normalize_path(&d) == wanted))
        .unwrap_or(false)
        || user_path_contains_bin()
}

#[cfg(windows)]
fn user_path_contains_bin() -> bool {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    let Ok(bin) = paths::bin_dir() else {
        return false;
    };
    let wanted = bin.to_string_lossy().to_lowercase();
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Environment")
        .and_then(|k| k.get_value::<String, _>("Path"))
        .map(|p| {
            p.split(';')
                .any(|d| d.trim().trim_end_matches('\\').to_lowercase() == wanted)
        })
        .unwrap_or(false)
}

/// PATH configured in a shell startup file counts too (it applies to every new terminal).
#[cfg(not(windows))]
fn user_path_contains_bin() -> bool {
    let Ok(home) = paths::home_dir() else {
        return false;
    };
    [
        ".zshrc",
        ".bashrc",
        ".bash_profile",
        ".config/fish/conf.d/claude-switch.fish",
    ]
    .iter()
    .any(|rc| {
        std::fs::read_to_string(home.join(rc))
            .map(|t| t.contains(">>> claude-switch >>>") || t.contains("fish_add_path"))
            .unwrap_or(false)
    })
}

/// Adds `~/.cswitch/bin` to the user's PATH. Returns a human description of what changed.
pub fn add_bin_to_path() -> Result<String> {
    let bin = paths::bin_dir()?;
    #[cfg(windows)]
    {
        use winreg::enums::{HKEY_CURRENT_USER, KEY_ALL_ACCESS, RegType};
        use winreg::{RegKey, RegValue};
        let env = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags("Environment", KEY_ALL_ACCESS)?;
        let current: String = env.get_value("Path").unwrap_or_default();
        let entry = bin.to_string_lossy().to_string();
        if current
            .split(';')
            .any(|d| d.trim().trim_end_matches('\\').eq_ignore_ascii_case(&entry))
        {
            return Ok("already on your PATH".to_string());
        }
        let updated = if current.trim().is_empty() {
            entry.clone()
        } else {
            format!("{};{entry}", current.trim_end_matches(';'))
        };
        let wide: Vec<u8> = updated
            .encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(|u| u.to_le_bytes())
            .collect();
        env.set_raw_value(
            "Path",
            &RegValue {
                bytes: wide.into(),
                vtype: RegType::REG_EXPAND_SZ,
            },
        )?;
        sys::win::broadcast_environment_change();
        Ok(format!(
            "added {entry} to your user PATH (open a new terminal to use it)"
        ))
    }
    #[cfg(unix)]
    {
        let home = paths::home_dir()?;
        let shell = std::env::var("SHELL").unwrap_or_default();
        let line_posix = format!("export PATH=\"{}:$PATH\"", bin.display());
        let (file, body) = if shell.ends_with("fish") {
            let file = home.join(".config/fish/conf.d/claude-switch.fish");
            (file, format!("fish_add_path {}\n", bin.display()))
        } else if shell.ends_with("zsh") {
            (
                home.join(".zshrc"),
                format!("\n# >>> claude-switch >>>\n{line_posix}\n# <<< claude-switch <<<\n"),
            )
        } else if cfg!(target_os = "macos") {
            (
                home.join(".bash_profile"),
                format!("\n# >>> claude-switch >>>\n{line_posix}\n# <<< claude-switch <<<\n"),
            )
        } else {
            (
                home.join(".bashrc"),
                format!("\n# >>> claude-switch >>>\n{line_posix}\n# <<< claude-switch <<<\n"),
            )
        };
        let existing = fs::read_to_string(&file).unwrap_or_default();
        if existing.contains(">>> claude-switch >>>")
            || existing.contains(&bin.display().to_string())
        {
            return Ok(format!("already configured in {}", file.display()));
        }
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut content = existing;
        content.push_str(&body);
        fs::write(&file, content)?;
        Ok(format!(
            "added {} to PATH in {} (open a new terminal to use it)",
            bin.display(),
            file.display()
        ))
    }
}

pub fn remove_bin_from_path() -> Result<()> {
    let bin = paths::bin_dir()?;
    #[cfg(windows)]
    {
        use winreg::enums::{HKEY_CURRENT_USER, KEY_ALL_ACCESS, RegType};
        use winreg::{RegKey, RegValue};
        let env = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags("Environment", KEY_ALL_ACCESS)?;
        let current: String = env.get_value("Path").unwrap_or_default();
        let entry = bin.to_string_lossy().to_string();
        let kept: Vec<&str> = current
            .split(';')
            .filter(|d| !d.trim().trim_end_matches('\\').eq_ignore_ascii_case(&entry))
            .collect();
        let updated = kept.join(";");
        if updated != current {
            let wide: Vec<u8> = updated
                .encode_utf16()
                .chain(std::iter::once(0))
                .flat_map(|u| u.to_le_bytes())
                .collect();
            env.set_raw_value(
                "Path",
                &RegValue {
                    bytes: wide.into(),
                    vtype: RegType::REG_EXPAND_SZ,
                },
            )?;
            sys::win::broadcast_environment_change();
        }
    }
    #[cfg(unix)]
    {
        let home = paths::home_dir()?;
        let _ = fs::remove_file(home.join(".config/fish/conf.d/claude-switch.fish"));
        for rc in [".zshrc", ".bashrc", ".bash_profile"] {
            let file = home.join(rc);
            let Ok(text) = fs::read_to_string(&file) else {
                continue;
            };
            if let (Some(start), Some(end)) = (
                text.find("\n# >>> claude-switch >>>"),
                text.find("# <<< claude-switch <<<\n"),
            ) {
                let cleaned = format!(
                    "{}{}",
                    &text[..start],
                    &text[end + "# <<< claude-switch <<<\n".len()..]
                );
                fs::write(&file, cleaned)?;
            }
        }
        let _ = bin;
    }
    Ok(())
}

/// A 64×64 ICO: a disc in the profile's color with a white "C" — tells profiles apart in
/// the Start menu and taskbar pins.
pub fn profile_icon(color: Rgb) -> Vec<u8> {
    const SIZE: usize = 64;
    let mut pixels = vec![0u8; SIZE * SIZE * 4];
    let center = (SIZE as f32 - 1.0) / 2.0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let dist = (dx * dx + dy * dy).sqrt();
            let disc = (31.0 - dist).clamp(0.0, 1.0);
            if disc <= 0.0 {
                continue;
            }
            // Ring between radius 13 and 20, opened on the right (the "C").
            let ring = ((20.0 - dist).clamp(0.0, 1.0)) * ((dist - 13.0).clamp(0.0, 1.0));
            let angle = dy.atan2(dx).abs();
            let gap = ((angle - 0.75) * 8.0).clamp(0.0, 1.0);
            let white = ring * gap;
            let mix = |c: u8| (c as f32 * (1.0 - white) + 255.0 * white).round() as u8;
            // ICO bitmaps are stored bottom-up, BGRA.
            let row = SIZE - 1 - y;
            let i = (row * SIZE + x) * 4;
            pixels[i] = mix(color.2);
            pixels[i + 1] = mix(color.1);
            pixels[i + 2] = mix(color.0);
            pixels[i + 3] = (disc * 255.0).round() as u8;
        }
    }
    let and_mask = vec![0u8; SIZE * SIZE / 8];
    let image_len = 40 + pixels.len() + and_mask.len();
    let mut out = Vec::with_capacity(22 + image_len);
    out.extend_from_slice(&[0, 0, 1, 0, 1, 0]); // ICONDIR: reserved, type=icon, count=1
    out.extend_from_slice(&[SIZE as u8, SIZE as u8, 0, 0]);
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bit count
    out.extend_from_slice(&(image_len as u32).to_le_bytes());
    out.extend_from_slice(&22u32.to_le_bytes()); // offset
    out.extend_from_slice(&40u32.to_le_bytes()); // BITMAPINFOHEADER
    out.extend_from_slice(&(SIZE as i32).to_le_bytes());
    out.extend_from_slice(&((SIZE * 2) as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0u8; 24]);
    out.extend_from_slice(&pixels);
    out.extend_from_slice(&and_mask);
    out
}

fn icons_dir() -> Result<PathBuf> {
    Ok(paths::cswitch_home()?.join("icons"))
}

pub fn shortcuts_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        dirs::data_dir().map(|d| d.join(r"Microsoft\Windows\Start Menu\Programs\Claude Switch"))
    } else if cfg!(target_os = "macos") {
        dirs::home_dir().map(|h| h.join("Applications/Claude Switch"))
    } else {
        dirs::data_dir().map(|d| d.join("applications"))
    }
}

/// Creates launchers: "Claude Desktop (Work)", "Claude Code (Work)" and "Claude Switch".
pub fn create_shortcuts(cfg: &Config, install: Option<&DesktopInstall>) -> Result<Vec<String>> {
    let dir = shortcuts_dir().context("could not find the shortcuts folder")?;
    fs::create_dir_all(&dir)?;
    remove_shortcuts();
    fs::create_dir_all(&dir)?;
    let exe = paths::stable_exe()?;
    let helper = paths::helper_exe()?;
    // Created up front: the "Claude Switch" shortcut needs it even when there are no profiles yet.
    let icons = icons_dir()?;
    fs::create_dir_all(&icons)?;
    let mut made = Vec::new();
    for name in cfg.profile_names() {
        let profile = cfg.profile(&name)?;
        // Labels are free text but end up in file names (`.lnk`, `.app`, `.desktop`).
        let label = file_name_safe(&cfg.display_name(&name));
        let color = Rgb::parse(&profile.color).unwrap_or(Rgb(0xD9, 0x77, 0x57));
        let icon = icons.join(format!("{name}.ico"));
        fs::write(&icon, profile_icon(color))?;
        if install.is_some() {
            platform_shortcut(
                &dir,
                &format!("Claude Desktop ({label})"),
                &helper,
                &["desktop", &name],
                &icon,
                false,
                install,
            )?;
            made.push(format!("Claude Desktop ({label})"));
        }
        platform_shortcut(
            &dir,
            &format!("Claude Code ({label})"),
            &exe,
            &["code", &name, "--here"],
            &icon,
            true,
            None,
        )?;
        made.push(format!("Claude Code ({label})"));
    }
    if cfg!(windows) {
        let icon = icons.join("cswitch.ico");
        fs::write(&icon, profile_icon(Rgb(0xD9, 0x77, 0x57)))?;
        platform_shortcut(&dir, "Claude Switch", &exe, &[], &icon, true, None)?;
        made.push("Claude Switch".to_string());
    }
    Ok(made)
}

/// Replaces characters that Windows, macOS or Linux reject (or treat specially) in file names.
fn file_name_safe(label: &str) -> String {
    let cleaned: String = label
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches('.').trim();
    if trimmed.is_empty() {
        "Profile".to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn remove_shortcuts() {
    let Some(dir) = shortcuts_dir() else { return };
    if !cfg!(any(windows, target_os = "macos")) {
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("cswitch-")
                    && name.ends_with(".desktop")
                    && name != "cswitch-router.desktop"
                {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
    } else {
        let _ = fs::remove_dir_all(dir);
    }
}

#[cfg(windows)]
fn platform_shortcut(
    dir: &Path,
    title: &str,
    target: &Path,
    args: &[&str],
    icon: &Path,
    _terminal: bool,
    _install: Option<&DesktopInstall>,
) -> Result<()> {
    let q = |s: &str| s.replace('\'', "''");
    let lnk = dir.join(format!("{title}.lnk"));
    let home = paths::home_dir()?;
    let script = format!(
        "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}'); $s.TargetPath='{}'; $s.Arguments='{}'; \
         $s.IconLocation='{},0'; $s.WorkingDirectory='{}'; $s.Description='{}'; $s.Save()",
        q(&lnk.to_string_lossy()),
        q(&target.to_string_lossy()),
        q(&args.join(" ")),
        q(&icon.to_string_lossy()),
        q(&home.to_string_lossy()),
        q(title),
    );
    let out = sys::powershell(&script)
        .output()
        .context("could not run PowerShell to create shortcuts")?;
    if !out.status.success() {
        bail!(
            "creating '{title}' failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn platform_shortcut(
    dir: &Path,
    title: &str,
    target: &Path,
    args: &[&str],
    _icon: &Path,
    terminal: bool,
    install: Option<&DesktopInstall>,
) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let bundle = dir.join(format!("{title}.app"));
    let macos = bundle.join("Contents/MacOS");
    let resources = bundle.join("Contents/Resources");
    fs::create_dir_all(&macos)?;
    fs::create_dir_all(&resources)?;
    let quoted: Vec<String> = args
        .iter()
        .map(|a| shell_words::quote(a).to_string())
        .collect();
    let command = format!(
        "{} {}",
        shell_words::quote(&target.to_string_lossy()),
        quoted.join(" ")
    );
    let script = if terminal {
        // AppleScript string first, then quote each `-e` script for sh: paths with spaces are
        // already single-quoted inside `command`, so a bare '…' wrapper would break.
        let applescript = format!(
            "tell application \"Terminal\" to do script \"{}\"",
            command.replace('\\', "\\\\").replace('"', "\\\"")
        );
        format!(
            "#!/bin/sh\nosascript -e {} -e {}\n",
            shell_words::quote(&applescript),
            shell_words::quote("tell application \"Terminal\" to activate")
        )
    } else {
        format!("#!/bin/sh\nexec {command}\n")
    };
    let launcher = macos.join("launcher");
    fs::write(&launcher, script)?;
    fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755))?;
    let mut icon_entry = String::new();
    if let Some(app) = install.and_then(|i| i.app_bundle.clone())
        && let Ok(entries) = fs::read_dir(app.join("Contents/Resources"))
        && let Some(icns) = entries
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "icns"))
        && fs::copy(&icns, resources.join("icon.icns")).is_ok()
    {
        icon_entry = "<key>CFBundleIconFile</key><string>icon</string>".to_string();
    }
    let id: String = title
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    let title = title
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\"><dict><key>CFBundleName</key><string>{title}</string>\
         <key>CFBundleIdentifier</key><string>dev.claude-switch.{id}</string>\
         <key>CFBundleExecutable</key><string>launcher</string><key>CFBundlePackageType</key><string>APPL</string>\
         <key>LSUIElement</key><true/>{icon_entry}</dict></plist>\n"
    );
    fs::write(bundle.join("Contents/Info.plist"), plist)?;
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_shortcut(
    dir: &Path,
    title: &str,
    target: &Path,
    args: &[&str],
    icon: &Path,
    terminal: bool,
    _install: Option<&DesktopInstall>,
) -> Result<()> {
    let slug: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let png_free_icon = icon.to_string_lossy();
    let exec = format!("\"{}\" {}", target.display(), args.join(" "));
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName={title}\nExec={exec}\nIcon={png_free_icon}\nTerminal={terminal}\nCategories=Development;\n"
    );
    fs::write(dir.join(format!("cswitch-{slug}.desktop")), entry)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_pe() -> Vec<u8> {
        let mut image = vec![0u8; 512];
        image[0] = b'M';
        image[1] = b'Z';
        image[0x3C..0x40].copy_from_slice(&128u32.to_le_bytes());
        image[128..132].copy_from_slice(b"PE\0\0");
        image[128 + 24 + 68] = 3; // console
        image
    }

    #[test]
    fn patches_pe_subsystem() {
        let patched = make_windowed(&fake_pe()).unwrap();
        assert_eq!(patched[128 + 24 + 68], 2);
        assert!(make_windowed(b"not an exe").is_err());
    }

    #[test]
    fn makes_labels_safe_for_file_names() {
        assert_eq!(file_name_safe("Work"), "Work");
        assert_eq!(file_name_safe("Client: A/B?"), "Client- A-B-");
        assert_eq!(file_name_safe("dots..."), "dots");
        assert_eq!(file_name_safe("  "), "Profile");
    }

    #[test]
    fn icon_has_valid_header() {
        let ico = profile_icon(Rgb(0x3B, 0x82, 0xF6));
        assert_eq!(&ico[0..6], &[0, 0, 1, 0, 1, 0]);
        assert_eq!(ico.len(), 22 + 40 + 64 * 64 * 4 + 64 * 64 / 8);
    }
}
