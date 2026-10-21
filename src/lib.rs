//! Claude Switch — run several fully isolated Claude accounts (Claude Code + Claude Desktop) side by side.
//!
//! Layers (see ARCHITECTURE.md):
//! * `config` — TOML schema, paths and persistence.
//! * `core`   — platform logic: environment isolation, Claude Code, Claude Desktop, browsers,
//!   the `claude://` router, system integration.
//! * `cli`    — commands and interactive flows.
//! * `ui`     — themed terminal output and prompts.

pub mod app;
pub mod cli;
pub mod config;
pub mod core;
pub mod ui;
