//! Arrow-key prompts with the active palette. `Esc` returns `None` wherever that makes sense.

use anyhow::Result;
use console::style;
use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, FuzzySelect, Input, MultiSelect, Select};

use super::g;
use super::theme::palette;

pub fn theme() -> ColorfulTheme {
    let pal = palette();
    ColorfulTheme {
        defaults_style: pal.dim.style(),
        prompt_style: pal.text.style().bold(),
        prompt_prefix: pal.accent.style().bold().apply_to(g("?", "?").to_string()),
        prompt_suffix: pal.dim.style().apply_to(g("›", ">").to_string()),
        success_prefix: pal.success.style().apply_to(g("✔", "+").to_string()),
        success_suffix: pal.dim.style().apply_to(g("·", ":").to_string()),
        error_prefix: pal.error.style().apply_to(g("✖", "x").to_string()),
        error_style: pal.error.style(),
        hint_style: pal.dim.style(),
        values_style: pal.accent2.style(),
        active_item_style: pal.accent.style().bold(),
        inactive_item_style: pal.text.style(),
        active_item_prefix: pal.accent.style().bold().apply_to(g("❯", ">").to_string()),
        inactive_item_prefix: style(" ".to_string()),
        checked_item_prefix: pal.success.style().apply_to(g("◉", "[x]").to_string()),
        unchecked_item_prefix: pal.dim.style().apply_to(g("○", "[ ]").to_string()),
        picked_item_prefix: pal.accent.style().apply_to(g("❯", ">").to_string()),
        unpicked_item_prefix: style(" ".to_string()),
        fuzzy_cursor_style: pal.accent.style().reverse(),
        fuzzy_match_highlight_style: pal.accent.style().bold(),
    }
}

/// Single choice; `None` when the user presses Esc or `q`.
pub fn select<T: ToString>(prompt: &str, items: &[T], default: usize) -> Result<Option<usize>> {
    let theme = theme();
    let default = default.min(items.len().saturating_sub(1));
    Ok(Select::with_theme(&theme)
        .with_prompt(prompt)
        .items(items.iter().map(ToString::to_string))
        .default(default)
        .max_length(20)
        .interact_opt()?)
}

/// Like `select` but with type-to-filter, for long lists such as browser profiles.
pub fn fuzzy<T: ToString>(prompt: &str, items: &[T], default: usize) -> Result<Option<usize>> {
    let theme = theme();
    let default = default.min(items.len().saturating_sub(1));
    Ok(FuzzySelect::with_theme(&theme)
        .with_prompt(prompt)
        .items(items.iter().map(ToString::to_string))
        .default(default)
        .max_length(12)
        .interact_opt()?)
}

pub fn multi<T: ToString>(
    prompt: &str,
    items: &[T],
    defaults: &[bool],
) -> Result<Option<Vec<usize>>> {
    let theme = theme();
    Ok(MultiSelect::with_theme(&theme)
        .with_prompt(prompt)
        .items(items.iter().map(ToString::to_string))
        .defaults(defaults)
        .interact_opt()?)
}

pub fn confirm(prompt: &str, default: bool) -> Result<bool> {
    let theme = theme();
    Ok(Confirm::with_theme(&theme)
        .with_prompt(prompt)
        .default(default)
        .interact_opt()?
        .unwrap_or(false))
}

/// Free text, pre-filled with `current` so it can be kept (Enter), edited or cleared.
/// (A dialoguer *default* would be returned for empty input, making "empty to clear" impossible.)
pub fn text(prompt: &str, current: &str) -> Result<String> {
    let theme = theme();
    let mut input = Input::<String>::with_theme(&theme).with_prompt(prompt);
    if !current.is_empty() {
        input = input.with_initial_text(current);
    }
    Ok(input.allow_empty(true).interact_text()?.trim().to_string())
}

pub fn text_validated<F>(prompt: &str, default: &str, mut validate: F) -> Result<String>
where
    F: FnMut(&String) -> std::result::Result<(), String>,
{
    let theme = theme();
    let mut input = Input::<String>::with_theme(&theme).with_prompt(prompt);
    if !default.is_empty() {
        input = input.default(default.to_string());
    }
    Ok(input
        .validate_with(move |v: &String| validate(v))
        .interact_text()?
        .trim()
        .to_string())
}

pub fn pause(message: &str) {
    let theme = theme();
    let _ = Input::<String>::with_theme(&theme)
        .with_prompt(message)
        .allow_empty(true)
        .show_default(false)
        .default(String::new())
        .interact_text();
}
