//! Everything that draws to the terminal: palette-aware styles, glyphs, messages, prompts, tables.

pub mod prompt;
pub mod table;
pub mod theme;

use std::sync::atomic::{AtomicBool, Ordering};

use console::{Style, measure_text_width};

use crate::config::Profile;
use theme::{Rgb, palette};

static FANCY: AtomicBool = AtomicBool::new(true);

pub fn set_fancy_glyphs(enabled: bool) {
    FANCY.store(enabled, Ordering::Relaxed);
}

pub fn fancy() -> bool {
    FANCY.load(Ordering::Relaxed)
}

/// A glyph with an ASCII fallback for consoles without good unicode fonts.
pub fn g(fancy: &'static str, plain: &'static str) -> &'static str {
    if self::fancy() { fancy } else { plain }
}

pub fn accent(text: impl AsRef<str>) -> String {
    palette()
        .accent
        .style()
        .bold()
        .apply_to(text.as_ref())
        .to_string()
}

pub fn accent2(text: impl AsRef<str>) -> String {
    palette()
        .accent2
        .style()
        .apply_to(text.as_ref())
        .to_string()
}

pub fn success(text: impl AsRef<str>) -> String {
    palette()
        .success
        .style()
        .apply_to(text.as_ref())
        .to_string()
}

pub fn warning(text: impl AsRef<str>) -> String {
    palette()
        .warning
        .style()
        .apply_to(text.as_ref())
        .to_string()
}

pub fn error(text: impl AsRef<str>) -> String {
    palette()
        .error
        .style()
        .bold()
        .apply_to(text.as_ref())
        .to_string()
}

pub fn info(text: impl AsRef<str>) -> String {
    palette().info.style().apply_to(text.as_ref()).to_string()
}

pub fn dim(text: impl AsRef<str>) -> String {
    palette().dim.style().apply_to(text.as_ref()).to_string()
}

pub fn bold(text: impl AsRef<str>) -> String {
    Style::new().bold().apply_to(text.as_ref()).to_string()
}

pub fn ok(msg: impl AsRef<str>) {
    println!("  {} {}", success(g("✔", "[ok]")), msg.as_ref());
}

pub fn warn(msg: impl AsRef<str>) {
    println!("  {} {}", warning(g("▲", "[!]")), msg.as_ref());
}

pub fn fail(msg: impl AsRef<str>) {
    println!("  {} {}", error(g("✖", "[x]")), msg.as_ref());
}

pub fn note(msg: impl AsRef<str>) {
    println!("  {} {}", info(g("›", ">")), msg.as_ref());
}

pub fn hint(msg: impl AsRef<str>) {
    println!("    {}", dim(msg.as_ref()));
}

pub fn blank() {
    println!();
}

pub fn section(title: impl AsRef<str>) {
    println!();
    println!("  {} {}", accent(g("◆", "*")), bold(title.as_ref()));
    let width = measure_text_width(title.as_ref()) + 2;
    println!("  {}", dim(g("─", "-").repeat(width)));
}

/// The colored dot + label used for a profile everywhere.
pub fn badge(name: &str, profile: Option<&Profile>) -> String {
    let (label, color, icon) = match profile {
        Some(p) => (
            if p.label.trim().is_empty() {
                name.to_string()
            } else {
                p.label.clone()
            },
            Rgb::parse(&p.color).unwrap_or(palette().accent),
            p.icon.clone(),
        ),
        None => (name.to_string(), palette().accent, String::new()),
    };
    let dot = color.style().apply_to(g("●", "*")).to_string();
    let text = color.style().bold().apply_to(label).to_string();
    if icon.trim().is_empty() {
        format!("{dot} {text}")
    } else {
        format!("{dot} {icon} {text}")
    }
}

pub fn banner(subtitle: &str) {
    let version = env!("CARGO_PKG_VERSION");
    let title = format!("claude-switch v{version}");
    let lines = [title.clone(), subtitle.to_string()];
    let inner = lines
        .iter()
        .map(|l| measure_text_width(l))
        .max()
        .unwrap_or(0)
        + 4;
    let (tl, tr, bl, br, h, v) = if fancy() {
        ("╭", "╮", "╰", "╯", "─", "│")
    } else {
        ("+", "+", "+", "+", "-", "|")
    };
    let pal = palette();
    let frame = |s: &str| pal.accent.style().apply_to(s).to_string();
    println!();
    println!("  {}", frame(&format!("{tl}{}{tr}", h.repeat(inner))));
    let logo = pal.accent.style().bold().apply_to(g("◆", "*")).to_string();
    let title_line = format!(
        "{logo} {} {}",
        pal.accent.style().bold().apply_to("claude-switch"),
        dim(format!("v{version}"))
    );
    let pad1 = inner.saturating_sub(measure_text_width(&title_line) + 2);
    println!(
        "  {} {title_line}{} {}",
        frame(v),
        " ".repeat(pad1),
        frame(v)
    );
    let sub = dim(subtitle);
    let pad2 = inner.saturating_sub(measure_text_width(subtitle) + 4);
    println!("  {}   {sub}{} {}", frame(v), " ".repeat(pad2), frame(v));
    println!("  {}", frame(&format!("{bl}{}{br}", h.repeat(inner))));
}

/// Sets the terminal/tab title (ignored by terminals that do not support it).
pub fn set_terminal_title(title: &str) {
    if console::user_attended() {
        print!("\x1b]0;{title}\x07");
    }
}
