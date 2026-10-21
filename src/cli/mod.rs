//! Command-line interface: argument definitions here, one module per workflow.

pub mod code_cmd;
pub mod common;
pub mod desktop_cmd;
pub mod doctor;
pub mod editor;
pub mod install_cmd;
pub mod login;
pub mod menu;
pub mod profiles;
pub mod settings;
pub mod status;
pub mod wizard;

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(
    name = "cswitch",
    version,
    about = "Run work & personal Claude side by side — Claude Code and Claude Desktop, fully isolated.",
    long_about = "Claude Switch keeps several Claude accounts completely apart: every profile gets its own \
Claude Code config/credentials and its own Claude Desktop instance, both can run at the same time, \
and sign-in always opens the right browser profile and returns to the right app.\n\n\
Run `cswitch` with no arguments for the interactive menu.",
    after_help = "Examples:\n  cswitch                      interactive menu\n  cswitch setup                first-run wizard\n  cswitch code work            Claude Code as 'work' (or just: claude-work)\n  cswitch code personal -n     ...in a new terminal tab\n  cswitch desktop work         Claude Desktop as 'work'\n  cswitch login work           sign 'work' in (Code + Desktop)\n  cswitch status               who is signed in where"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Start Claude Code in a profile (arguments after the profile go to `claude`)
    #[command(visible_alias = "c")]
    Code(CodeArgs),
    /// Start (or focus) Claude Desktop in a profile
    #[command(visible_alias = "d")]
    Desktop(DesktopArgs),
    /// Sign a profile in: right browser profile, sign-in routed back to the right app
    Login(LoginArgs),
    /// Sign a profile out
    Logout(LogoutArgs),
    /// Show every profile: signed-in account, running Desktop, sign-in browser
    #[command(visible_alias = "ls")]
    Status(StatusArgs),
    /// Create, edit, rename and remove profiles
    #[command(visible_alias = "p")]
    Profile(ProfileArgs),
    /// Open a sub-shell in which `claude` runs as the profile
    Shell(ProfileOnly),
    /// Print the profile's environment as shell commands (for eval / scripts)
    Env(EnvArgs),
    /// Copy Claude Code / Desktop settings (agents, commands, MCP servers, ...) between profiles
    Copy(CopyArgs),
    /// Check everything and explain how to fix problems
    Doctor(DoctorArgs),
    /// First-run wizard: profiles, browsers, install, link router, shortcuts
    Setup,
    /// Install or update: binary, claude-<profile> commands, PATH, shortcuts, link router
    Install(InstallArgs),
    /// Remove Claude Switch from this computer
    Uninstall(UninstallArgs),
    /// Manage the claude:// link router used for Claude Desktop sign-in
    Router(RouterArgs),
    /// Settings, theme and the config file
    Config(ConfigArgs),
    /// Print shell completions (e.g. `cswitch completions powershell >> $PROFILE`)
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Open a URL in a profile's sign-in browser (used as $BROWSER)
    #[command(hide = true)]
    OpenUrl {
        #[arg(long)]
        profile: Option<String>,
        url: String,
    },
    /// Handle a claude:// link (registered with the OS)
    #[command(hide = true)]
    Route { url: String },
    #[command(hide = true, name = "__reassert")]
    Reassert,
    /// List processes whose name or path contains FILTER (diagnostics)
    #[command(hide = true, name = "__procs")]
    Procs {
        #[arg(default_value = "claude")]
        filter: String,
    },
}

#[derive(Args, Debug, Clone)]
pub struct CodeArgs {
    /// Profile name (asks when omitted)
    pub profile: Option<String>,
    /// Open in a new terminal tab / window instead of this one
    #[arg(short = 'n', long)]
    pub new_window: bool,
    /// Run in this terminal even when Settings say "open in a new window"
    #[arg(long, conflicts_with = "new_window")]
    pub here: bool,
    /// Working directory
    #[arg(short = 'C', long, value_name = "DIR")]
    pub dir: Option<PathBuf>,
    /// Arguments passed straight to `claude`
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub args: Vec<String>,
}

#[derive(Args, Debug, Clone)]
pub struct DesktopArgs {
    /// Profile name (asks when omitted)
    pub profile: Option<String>,
    /// Start every profile's Desktop
    #[arg(long, conflicts_with = "profile")]
    pub all: bool,
    /// Stop the profile's Desktop instead
    #[arg(long)]
    pub stop: bool,
    /// Stop and start again
    #[arg(long, conflicts_with = "stop")]
    pub restart: bool,
}

#[derive(Args, Debug, Clone)]
pub struct LoginArgs {
    pub profile: Option<String>,
    /// Only Claude Code
    #[arg(long)]
    pub code: bool,
    /// Only Claude Desktop
    #[arg(long)]
    pub desktop: bool,
    /// Don't wait for confirmation; keep Desktop sign-in routing active for the configured minutes
    #[arg(long)]
    pub no_wait: bool,
}

#[derive(Args, Debug, Clone)]
pub struct LogoutArgs {
    pub profile: Option<String>,
    /// Only Claude Code
    #[arg(long)]
    pub code: bool,
    /// Only Claude Desktop (wipes the profile's Desktop data)
    #[arg(long)]
    pub desktop: bool,
    /// Don't ask for confirmation
    #[arg(short, long)]
    pub yes: bool,
}

#[derive(Args, Debug, Clone)]
pub struct StatusArgs {
    /// Machine-readable output
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct ProfileOnly {
    pub profile: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct ProfileArgs {
    #[command(subcommand)]
    pub action: Option<ProfileAction>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum ProfileAction {
    /// List profiles
    List,
    /// Create a profile (interactive)
    Add { name: Option<String> },
    /// Edit a profile (interactive)
    Edit { name: Option<String> },
    /// Show a profile's resolved settings and paths
    Show { name: Option<String> },
    /// Rename a profile (moves its data)
    Rename { from: String, to: String },
    /// Make a profile the default
    Default { name: String },
    /// Delete a profile
    Remove {
        name: Option<String>,
        /// Also delete its Claude Code / Desktop data
        #[arg(long)]
        purge: bool,
        #[arg(short, long)]
        yes: bool,
    },
    /// Open a profile's data folder
    Open { name: Option<String> },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum EnvShell {
    Bash,
    Zsh,
    Fish,
    Powershell,
    Cmd,
}

#[derive(Args, Debug, Clone)]
pub struct EnvArgs {
    pub profile: Option<String>,
    /// Output syntax (auto-detected when omitted)
    #[arg(long, value_enum)]
    pub shell: Option<EnvShell>,
}

#[derive(Args, Debug, Clone)]
pub struct CopyArgs {
    /// Source profile, or `system` for your original ~/.claude and Claude Desktop data
    #[arg(long)]
    pub from: Option<String>,
    /// Destination profile
    #[arg(long)]
    pub to: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct DoctorArgs {
    /// Try to fix what can be fixed automatically
    #[arg(long)]
    pub fix: bool,
    /// Send a test claude:// link through the OS and check that it reaches Claude Switch
    #[arg(long)]
    pub test_router: bool,
}

#[derive(Args, Debug, Clone)]
pub struct InstallArgs {
    /// Don't touch PATH
    #[arg(long)]
    pub no_path: bool,
    /// Don't create Start menu / app shortcuts
    #[arg(long)]
    pub no_shortcuts: bool,
    /// Don't register the claude:// link router
    #[arg(long)]
    pub no_router: bool,
}

#[derive(Args, Debug, Clone)]
pub struct UninstallArgs {
    /// Also delete all profiles and their data (Claude Code + Desktop sessions)
    #[arg(long)]
    pub purge: bool,
    #[arg(short, long)]
    pub yes: bool,
}

#[derive(Args, Debug, Clone)]
pub struct RouterArgs {
    #[command(subcommand)]
    pub action: Option<RouterAction>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum RouterAction {
    /// Show whether claude:// links reach Claude Switch
    Status,
    /// Register Claude Switch as the claude:// handler
    Register,
    /// Restore the previous claude:// handler
    Unregister,
    /// Send a test link through the OS
    Test,
    /// Open the OS settings page for default link handlers
    Settings,
    /// Route all claude:// links to a profile for a while
    Arm {
        profile: String,
        #[arg(long, default_value_t = 10)]
        minutes: u64,
    },
    /// Stop routing links to a fixed profile
    Disarm,
}

#[derive(Args, Debug, Clone)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub action: Option<ConfigAction>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum ConfigAction {
    /// Print the config file location
    Path,
    /// Print the config file
    Show,
    /// Open the config file in your editor
    Edit,
    /// Pick a color theme
    Theme { name: Option<String> },
}
