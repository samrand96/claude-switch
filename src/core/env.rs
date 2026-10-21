//! Builds the environment a profile runs in.
//!
//! Isolation is not only about directories: an inherited `ANTHROPIC_API_KEY`,
//! `CLAUDE_CODE_OAUTH_TOKEN` or `CLAUDE_CONFIG_DIR` silently overrides whatever account the
//! profile is signed into. Every launch therefore starts by stripping those variables.

use std::ffi::OsString;
use std::process::Command;

use anyhow::Result;

use crate::config::{Config, EnvIsolation, paths};

/// Variables that carry identity, credentials, provider routing or config location.
pub const SENSITIVE: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_CUSTOM_HEADERS",
    "ANTHROPIC_BEDROCK_BASE_URL",
    "ANTHROPIC_VERTEX_BASE_URL",
    "ANTHROPIC_VERTEX_PROJECT_ID",
    "ANTHROPIC_FOUNDRY_API_KEY",
    "ANTHROPIC_FOUNDRY_BASE_URL",
    "ANTHROPIC_FOUNDRY_RESOURCE",
    "AWS_BEARER_TOKEN_BEDROCK",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_OAUTH_REFRESH_TOKEN",
    "CLAUDE_CODE_SESSION_ACCESS_TOKEN",
    "CLAUDE_SESSION_INGRESS_TOKEN_FILE",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_ACCOUNT_UUID",
    "CLAUDE_CODE_USER_EMAIL",
    "CLAUDE_CODE_ORGANIZATION_UUID",
    "CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST",
    "CLAUDE_CONFIG_DIR",
    "CLAUDE_SECURESTORAGE_CONFIG_DIR",
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "BROWSER",
    "CSWITCH_PROFILE",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Code,
    Desktop,
}

#[derive(Debug, Default, Clone)]
pub struct EnvPlan {
    pub remove: Vec<String>,
    pub set: Vec<(String, String)>,
}

impl EnvPlan {
    pub fn apply(&self, cmd: &mut Command) {
        for key in &self.remove {
            cmd.env_remove(key);
        }
        for (key, value) in &self.set {
            cmd.env(key, value);
        }
    }

    /// Shell-agnostic view used by `cswitch env`.
    pub fn exports(&self) -> Vec<(String, Option<String>)> {
        let mut out: Vec<(String, Option<String>)> =
            self.remove.iter().map(|k| (k.clone(), None)).collect();
        out.extend(self.set.iter().map(|(k, v)| (k.clone(), Some(v.clone()))));
        out
    }
}

fn upper(key: &OsString) -> String {
    key.to_string_lossy().to_uppercase()
}

fn is_claude_family(key: &str) -> bool {
    key.starts_with("ANTHROPIC_")
        || key.starts_with("CLAUDE")
        || key == "BROWSER"
        || key == "CSWITCH_PROFILE"
}

/// Whether an inherited variable (upper-cased name) is removed at this isolation level.
fn strips(level: EnvIsolation, key: &str) -> bool {
    match level {
        // Strict is a superset of Standard: SENSITIVE also lists non-Claude names such as
        // AWS_BEARER_TOKEN_BEDROCK.
        EnvIsolation::Strict => is_claude_family(key) || SENSITIVE.contains(&key),
        EnvIsolation::Standard => SENSITIVE.contains(&key),
        EnvIsolation::Off => false,
    }
}

/// Names of inherited variables the given isolation level strips.
pub fn stripped_from_current(cfg: &Config, extra_passthrough: &[String]) -> Vec<String> {
    let passthrough: Vec<String> = cfg
        .settings
        .env_passthrough
        .iter()
        .chain(extra_passthrough.iter())
        .map(|s| s.to_uppercase())
        .collect();
    let mut names: Vec<String> = std::env::vars_os()
        .map(|(k, _)| k)
        .filter(|k| {
            let key = upper(k);
            if passthrough.contains(&key) {
                return false;
            }
            strips(cfg.settings.env_isolation, &key)
        })
        .map(|k| k.to_string_lossy().to_string())
        .collect();
    names.sort();
    names.dedup();
    names
}

pub fn plan(cfg: &Config, name: &str, target: Target) -> Result<EnvPlan> {
    let profile = cfg.profile(name)?;
    let mut plan = EnvPlan {
        remove: stripped_from_current(cfg, &profile.env_passthrough),
        set: Vec::new(),
    };

    let code_dir = cfg.code_dir(name)?;
    let wants_config_dir = match target {
        Target::Code => true,
        Target::Desktop => profile.desktop_shares_code,
    };
    if wants_config_dir {
        if let Some(dir) = &code_dir {
            plan.set.push((
                "CLAUDE_CONFIG_DIR".into(),
                dir.to_string_lossy().to_string(),
            ));
        } else if cfg.settings.env_isolation != EnvIsolation::Off {
            // "system" storage: make sure an inherited CLAUDE_CONFIG_DIR cannot redirect it.
            if !plan
                .remove
                .iter()
                .any(|k| k.eq_ignore_ascii_case("CLAUDE_CONFIG_DIR"))
            {
                plan.remove.push("CLAUDE_CONFIG_DIR".into());
            }
        }
    }

    // Sign-in pages opened by Claude Code go through our opener, which picks this profile's browser.
    let helper = paths::helper_exe()?;
    plan.set
        .push(("BROWSER".into(), helper.to_string_lossy().to_string()));
    plan.set.push(("CSWITCH_PROFILE".into(), name.to_string()));

    for (key, value) in &profile.env {
        plan.set.push((key.clone(), value.clone()));
    }
    // A variable that is explicitly set must not also be removed.
    let set_keys: Vec<String> = plan.set.iter().map(|(k, _)| k.to_uppercase()).collect();
    plan.remove
        .retain(|k| !set_keys.contains(&k.to_uppercase()));
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_family_detection() {
        assert!(is_claude_family("ANTHROPIC_API_KEY"));
        assert!(is_claude_family("CLAUDECODE"));
        assert!(is_claude_family("CLAUDE_CODE_OAUTH_TOKEN"));
        assert!(is_claude_family("BROWSER"));
        assert!(!is_claude_family("CSWITCH_HOME"));
        assert!(!is_claude_family("PATH"));
    }

    #[test]
    fn strict_strips_everything_standard_does() {
        for key in SENSITIVE {
            assert!(
                strips(EnvIsolation::Strict, key),
                "{key} survives strict isolation"
            );
        }
        assert!(strips(EnvIsolation::Strict, "CLAUDE_CODE_SOMETHING_NEW"));
        assert!(!strips(EnvIsolation::Standard, "CLAUDE_CODE_SOMETHING_NEW"));
        assert!(!strips(EnvIsolation::Off, "ANTHROPIC_API_KEY"));
        assert!(!strips(EnvIsolation::Strict, "PATH"));
    }
}
