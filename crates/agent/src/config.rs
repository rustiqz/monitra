//! Local check-config parsing (DESIGN.md §4 `agent`, `--config`).
//!
//! The agent never discovers its monitors from the backend — no
//! "list my monitors" endpoint exists, deliberately (§11.10 leaves
//! `HostAgentCheck` target interpretation to whichever crate consumes it,
//! and the scheduler already never dispatches `HostAgentCheck` monitors —
//! `engine/src/scheduler.rs` — because they are agent-fed only). Instead
//! the operator creates the `Monitor` row (`monitra monitor add --kind
//! host-agent-check ...`) and this file, and the two are tied together by
//! `monitor_id`.

use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::error::AgentError;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CheckDefinition {
    Disk {
        monitor_id: u64,
        path: String,
        #[serde(default = "default_min_free_pct")]
        min_free_pct: f64,
    },
    Systemd {
        monitor_id: u64,
        unit: String,
    },
    Process {
        monitor_id: u64,
        name: String,
    },
}

impl CheckDefinition {
    pub fn monitor_id(&self) -> u64 {
        match self {
            CheckDefinition::Disk { monitor_id, .. }
            | CheckDefinition::Systemd { monitor_id, .. }
            | CheckDefinition::Process { monitor_id, .. } => *monitor_id,
        }
    }
}

fn default_min_free_pct() -> f64 {
    10.0
}

fn default_interval_secs() -> u64 {
    30
}

#[derive(Debug, Clone, Deserialize)]
pub struct CheckConfig {
    #[serde(default = "default_interval_secs")]
    pub interval_secs: u64,
    #[serde(default, rename = "checks")]
    pub checks: Vec<CheckDefinition>,
}

impl Default for CheckConfig {
    fn default() -> Self {
        CheckConfig {
            interval_secs: default_interval_secs(),
            checks: Vec::new(),
        }
    }
}

/// Loads and parses `path`. No "missing file means defaults" case here
/// (unlike `monitra-provider`'s config, §11.7) — an agent invoked with
/// `--config` but nothing at that path is a plain user error, not a
/// pristine-machine default: there is no honest embedded default for "what
/// should this agent check."
pub fn load(path: &Path) -> Result<CheckConfig, AgentError> {
    let contents = fs::read_to_string(path).map_err(|source| AgentError::ConfigRead {
        path: path.to_path_buf(),
        source,
    })?;
    toml::from_str(&contents).map_err(|source| AgentError::ConfigParse {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_three_check_kinds() {
        let toml = r#"
            interval_secs = 15

            [[checks]]
            kind = "disk"
            monitor_id = 1
            path = "/"
            min_free_pct = 5.0

            [[checks]]
            kind = "systemd"
            monitor_id = 2
            unit = "nginx.service"

            [[checks]]
            kind = "process"
            monitor_id = 3
            name = "postgres"
        "#;
        let config: CheckConfig = toml::from_str(toml).expect("valid config");
        assert_eq!(config.interval_secs, 15);
        assert_eq!(config.checks.len(), 3);
        assert_eq!(config.checks[0].monitor_id(), 1);
        assert_eq!(config.checks[1].monitor_id(), 2);
        assert_eq!(config.checks[2].monitor_id(), 3);
    }

    #[test]
    fn disk_min_free_pct_defaults_when_absent() {
        let toml = r#"
            [[checks]]
            kind = "disk"
            monitor_id = 1
            path = "/"
        "#;
        let config: CheckConfig = toml::from_str(toml).expect("valid config");
        assert!(matches!(
            config.checks[0],
            CheckDefinition::Disk { min_free_pct, .. } if min_free_pct == 10.0
        ));
    }

    #[test]
    fn interval_secs_defaults_when_absent() {
        let config: CheckConfig = toml::from_str("").expect("empty config is valid");
        assert_eq!(config.interval_secs, 30);
        assert!(config.checks.is_empty());
    }
}
