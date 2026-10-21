//! TOML schema for `~/.cswitch/config.toml`. Types only — loading and saving live in `store.rs`.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

pub const CONFIG_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    /// Profile used when a command needs one and none was given.
    pub default_profile: Option<String>,
    /// Terminal theme name (see `ui::theme`).
    pub theme: String,
    pub settings: Settings,
    pub code: CodeSettings,
    pub desktop: DesktopSettings,
    pub profiles: BTreeMap<String, Profile>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            default_profile: None,
            theme: "claude".to_string(),
            settings: Settings::default(),
            code: CodeSettings::default(),
            desktop: DesktopSettings::default(),
            profiles: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// What to do with a `claude://` link when it is not obvious which profile it belongs to.
    pub deep_link_policy: DeepLinkPolicy,
    /// How long a "sign in to profile X" request keeps routing `claude://` links to X.
    pub login_window_minutes: u64,
    /// How aggressively inherited environment variables are stripped before launching a profile.
    pub env_isolation: EnvIsolation,
    /// Variables that always survive isolation (e.g. `CLAUDE_CODE_GIT_BASH_PATH` on Windows).
    pub env_passthrough: Vec<String>,
    /// Where `cswitch code` opens Claude Code by default.
    pub code_launch: CodeLaunch,
    /// Use unicode glyphs in menus (turn off for very old consoles).
    pub fancy_glyphs: bool,
    /// Print the profile badge before Claude Code starts.
    pub show_badge: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            deep_link_policy: DeepLinkPolicy::Ask,
            login_window_minutes: 10,
            env_isolation: EnvIsolation::Strict,
            env_passthrough: vec!["CLAUDE_CODE_GIT_BASH_PATH".to_string()],
            code_launch: CodeLaunch::Here,
            fancy_glyphs: true,
            show_badge: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CodeSettings {
    /// Explicit path to the `claude` executable. Empty = auto-detect.
    pub claude_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DesktopSettings {
    /// Explicit path to the Claude Desktop executable. Empty = auto-detect.
    pub exe_path: Option<String>,
    /// Windows Store (MSIX) installs only: how instances are started.
    pub launch_method: LaunchMethod,
    /// Windows Store (MSIX) installs only: AppUserModelID override.
    pub aumid: Option<String>,
    /// Extra command-line switches passed to every Desktop instance.
    pub extra_args: Vec<String>,
    /// Whether `claude://` links are routed through Claude Switch.
    pub router_enabled: bool,
}

impl Default for DesktopSettings {
    fn default() -> Self {
        Self {
            exe_path: None,
            launch_method: LaunchMethod::Auto,
            aumid: None,
            extra_args: Vec::new(),
            router_enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub label: String,
    /// Hex color such as `#3B82F6`.
    pub color: String,
    /// Optional short symbol or emoji shown next to the label.
    pub icon: String,
    /// Account this profile is meant for; pre-fills sign-in and flags mismatches.
    pub email: Option<String>,
    pub login: LoginKind,
    /// Where Claude Code keeps this profile's config, history and credentials.
    pub code_storage: Storage,
    /// Where Claude Desktop keeps this profile's session, cookies and MCP config.
    pub desktop_storage: Storage,
    /// Point Claude Desktop's built-in Claude Code at this profile's Claude Code storage.
    pub desktop_shares_code: bool,
    /// Browser used for this profile's sign-in pages.
    pub browser: BrowserChoice,
    /// Extra environment variables for this profile (Claude Code and Desktop).
    pub env: BTreeMap<String, String>,
    /// Extra inherited variables this profile keeps despite isolation.
    pub env_passthrough: Vec<String>,
    /// Arguments always passed to `claude` for this profile.
    pub claude_args: Vec<String>,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            label: String::new(),
            color: "#D97757".to_string(),
            icon: String::new(),
            email: None,
            login: LoginKind::ClaudeAi,
            code_storage: Storage::Isolated,
            desktop_storage: Storage::Isolated,
            desktop_shares_code: true,
            browser: BrowserChoice::default(),
            env: BTreeMap::new(),
            env_passthrough: Vec::new(),
            claude_args: Vec::new(),
        }
    }
}

/// Storage location for one product inside a profile.
///
/// Serialized as `"isolated"`, `"system"` or an absolute path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(from = "String", into = "String")]
pub enum Storage {
    /// `~/.cswitch/profiles/<name>/<product>` — private to this profile.
    #[default]
    Isolated,
    /// The product's normal location (`~/.claude`, `%APPDATA%\Claude`, ...). Keeps an existing login.
    System,
    Custom(String),
}

impl From<String> for Storage {
    fn from(value: String) -> Self {
        match value.trim() {
            "" | "isolated" => Storage::Isolated,
            "system" | "default" => Storage::System,
            other => Storage::Custom(other.to_string()),
        }
    }
}

impl From<Storage> for String {
    fn from(value: Storage) -> Self {
        match value {
            Storage::Isolated => "isolated".to_string(),
            Storage::System => "system".to_string(),
            Storage::Custom(path) => path,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum LoginKind {
    /// Claude subscription (Pro / Max / Team / Enterprise) via claude.ai.
    #[default]
    ClaudeAi,
    /// Single sign-on (SAML / OIDC) for organizations.
    Sso,
    /// Anthropic Console (API billing).
    Console,
}

impl LoginKind {
    pub const ALL: [LoginKind; 3] = [LoginKind::ClaudeAi, LoginKind::Sso, LoginKind::Console];

    pub fn describe(self) -> &'static str {
        match self {
            LoginKind::ClaudeAi => "Claude account (Pro / Max / Team / Enterprise)",
            LoginKind::Sso => "Company single sign-on (SSO)",
            LoginKind::Console => "Anthropic Console (API billing)",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct BrowserChoice {
    pub kind: BrowserKind,
    pub mode: BrowserMode,
    /// Chromium profile directory (`Default`, `Profile 2`, ...) or Firefox profile name.
    pub profile: Option<String>,
    /// Executable override.
    pub exe: Option<String>,
    /// For `custom`: command line, `{url}` is replaced by the address.
    pub command: Option<String>,
}

impl Default for BrowserChoice {
    fn default() -> Self {
        Self {
            kind: BrowserKind::System,
            mode: BrowserMode::Profile,
            profile: None,
            exe: None,
            command: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum BrowserKind {
    /// Whatever the operating system opens links with.
    #[default]
    System,
    Chrome,
    Edge,
    Brave,
    Vivaldi,
    Chromium,
    Firefox,
    Custom,
}

impl BrowserKind {
    pub fn display(self) -> &'static str {
        match self {
            BrowserKind::System => "System default browser",
            BrowserKind::Chrome => "Google Chrome",
            BrowserKind::Edge => "Microsoft Edge",
            BrowserKind::Brave => "Brave",
            BrowserKind::Vivaldi => "Vivaldi",
            BrowserKind::Chromium => "Chromium",
            BrowserKind::Firefox => "Firefox",
            BrowserKind::Custom => "Custom command",
        }
    }

    pub fn is_chromium(self) -> bool {
        matches!(
            self,
            BrowserKind::Chrome
                | BrowserKind::Edge
                | BrowserKind::Brave
                | BrowserKind::Vivaldi
                | BrowserKind::Chromium
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum BrowserMode {
    /// Open in an existing browser profile.
    #[default]
    Profile,
    /// Open in a browser profile owned by Claude Switch, used for nothing else.
    Sandbox,
    /// Open in a private / incognito window.
    Private,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum DeepLinkPolicy {
    /// Ask which profile should receive the link.
    #[default]
    Ask,
    /// Send it to the profile whose Desktop was launched most recently.
    LastLaunched,
    /// Send it to the default profile.
    DefaultProfile,
}

impl DeepLinkPolicy {
    pub const ALL: [DeepLinkPolicy; 3] = [
        DeepLinkPolicy::Ask,
        DeepLinkPolicy::LastLaunched,
        DeepLinkPolicy::DefaultProfile,
    ];

    pub fn describe(self) -> &'static str {
        match self {
            DeepLinkPolicy::Ask => "Ask me which profile",
            DeepLinkPolicy::LastLaunched => "Use the most recently launched profile",
            DeepLinkPolicy::DefaultProfile => "Use the default profile",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum EnvIsolation {
    /// Remove every inherited `ANTHROPIC_*`, `CLAUDE*` and `BROWSER` variable (except passthrough).
    #[default]
    Strict,
    /// Remove only variables that carry identity, credentials or config location.
    Standard,
    /// Remove nothing.
    Off,
}

impl EnvIsolation {
    pub const ALL: [EnvIsolation; 3] = [
        EnvIsolation::Strict,
        EnvIsolation::Standard,
        EnvIsolation::Off,
    ];

    pub fn describe(self) -> &'static str {
        match self {
            EnvIsolation::Strict => "Strict — strip every inherited ANTHROPIC_* / CLAUDE* variable",
            EnvIsolation::Standard => {
                "Standard — strip only credentials and config-location variables"
            }
            EnvIsolation::Off => "Off — inherit everything (not isolated!)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum CodeLaunch {
    /// Replace the current terminal with Claude Code.
    #[default]
    Here,
    /// Open a new terminal tab or window.
    NewWindow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum LaunchMethod {
    /// Pick the best method for the detected installation.
    #[default]
    Auto,
    /// Start the executable directly (keeps per-profile environment variables).
    Direct,
    /// Windows Store apps: start through the package activation API.
    Activate,
}

impl fmt::Display for LaunchMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            LaunchMethod::Auto => "auto",
            LaunchMethod::Direct => "direct",
            LaunchMethod::Activate => "activate",
        };
        f.write_str(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_round_trips_through_strings() {
        assert_eq!(Storage::from("isolated".to_string()), Storage::Isolated);
        assert_eq!(Storage::from("system".to_string()), Storage::System);
        assert_eq!(Storage::from(String::new()), Storage::Isolated);
        assert_eq!(
            Storage::from("/tmp/x".to_string()),
            Storage::Custom("/tmp/x".to_string())
        );
        assert_eq!(String::from(Storage::System), "system");
    }

    #[test]
    fn config_parses_minimal_toml() {
        let cfg: Config = toml::from_str(
            r##"
default_profile = "work"
[profiles.work]
label = "Work"
color = "#3B82F6"
code_storage = "isolated"
desktop_storage = "system"
[profiles.work.browser]
kind = "chrome"
mode = "sandbox"
"##,
        )
        .expect("parse");
        let work = &cfg.profiles["work"];
        assert_eq!(work.desktop_storage, Storage::System);
        assert_eq!(work.browser.kind, BrowserKind::Chrome);
        assert_eq!(work.browser.mode, BrowserMode::Sandbox);
        assert_eq!(cfg.settings.login_window_minutes, 10);
    }
}
