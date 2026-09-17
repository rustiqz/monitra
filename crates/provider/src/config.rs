//! Config parsing and precedence resolution (DESIGN.md §4 `provider`, §11.7).
//!
//! Precedence, lowest to highest: `$XDG_CONFIG_HOME/monitra/config.toml` →
//! `./monitra.toml` → `MONITRA_*` env vars → CLI flags. Missing files are not
//! an error — a pristine machine with no config resolves to embedded
//! defaults throughout (§11.7); a malformed file is.
//!
//! `resolve` itself touches neither the filesystem nor the environment — it
//! takes a fully-gathered `ConfigSources` and layers it deterministically.
//! That's what makes precedence testable without mutating process env vars
//! or a real `$HOME`. `gather_from_environment` is the one function that
//! reads the real world, and it is intentionally thin enough not to need
//! its own tests.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::ProviderError;

/// One Kubernetes cluster attached as a `Collector` (§11.8 — its own command
/// family, not `service attach`, because a cluster's identity doesn't fit a
/// single provider URL).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct K8sClusterConfig {
    pub name: String,
    pub kubeconfig: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

/// The on-disk shape of `config.toml`. Every field optional — an absent file
/// or an empty one both mean "use embedded defaults throughout" (§11.7).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notifier: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub k8s: Vec<K8sClusterConfig>,
}

/// Where a resolved field's value ultimately came from — used by
/// `service list` to show the *effective* config honestly (P6), not just
/// what one file contains.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
    Default,
    Xdg,
    Project,
    Env,
    Flag,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedField {
    pub value: Option<String>,
    pub source: ConfigSource,
}

impl ResolvedField {
    fn default_value() -> Self {
        ResolvedField {
            value: None,
            source: ConfigSource::Default,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedConfig {
    pub store: ResolvedField,
    pub cache: ResolvedField,
    pub notifier: ResolvedField,
    pub k8s: Vec<K8sClusterConfig>,
}

/// Explicit CLI flag overrides — the highest-precedence layer. Distinct
/// fields (not a `ConfigFile`) because flags never carry `k8s` entries;
/// those go through `monitra k8s attach`, not `service`/flags.
#[derive(Debug, Clone, Default)]
pub struct FlagOverrides {
    pub store: Option<String>,
    pub cache: Option<String>,
    pub notifier: Option<String>,
}

/// Everything `resolve` needs, already gathered from the outside world.
#[derive(Debug, Clone, Default)]
pub struct ConfigSources {
    pub xdg: Option<ConfigFile>,
    pub project: Option<ConfigFile>,
    pub env: HashMap<String, String>,
    pub flags: FlagOverrides,
}

fn layer(
    current: ResolvedField,
    file: &Option<ConfigFile>,
    source: ConfigSource,
    pick: impl Fn(&ConfigFile) -> Option<String>,
) -> ResolvedField {
    match file.as_ref().and_then(pick) {
        Some(value) => ResolvedField {
            value: Some(value),
            source,
        },
        None => current,
    }
}

/// Layer `xdg` → `project` → `env` → `flags` onto embedded defaults. Pure:
/// no I/O, no env access — everything it needs is already in `sources`.
pub fn resolve(sources: &ConfigSources) -> ResolvedConfig {
    let mut store = ResolvedField::default_value();
    let mut cache = ResolvedField::default_value();
    let mut notifier = ResolvedField::default_value();

    store = layer(store, &sources.xdg, ConfigSource::Xdg, |c| c.store.clone());
    cache = layer(cache, &sources.xdg, ConfigSource::Xdg, |c| c.cache.clone());
    notifier = layer(notifier, &sources.xdg, ConfigSource::Xdg, |c| {
        c.notifier.clone()
    });

    store = layer(store, &sources.project, ConfigSource::Project, |c| {
        c.store.clone()
    });
    cache = layer(cache, &sources.project, ConfigSource::Project, |c| {
        c.cache.clone()
    });
    notifier = layer(notifier, &sources.project, ConfigSource::Project, |c| {
        c.notifier.clone()
    });

    if let Some(value) = sources.env.get("MONITRA_STORE") {
        store = ResolvedField {
            value: Some(value.clone()),
            source: ConfigSource::Env,
        };
    }
    if let Some(value) = sources.env.get("MONITRA_CACHE") {
        cache = ResolvedField {
            value: Some(value.clone()),
            source: ConfigSource::Env,
        };
    }
    if let Some(value) = sources.env.get("MONITRA_NOTIFIER") {
        notifier = ResolvedField {
            value: Some(value.clone()),
            source: ConfigSource::Env,
        };
    }

    if let Some(value) = &sources.flags.store {
        store = ResolvedField {
            value: Some(value.clone()),
            source: ConfigSource::Flag,
        };
    }
    if let Some(value) = &sources.flags.cache {
        cache = ResolvedField {
            value: Some(value.clone()),
            source: ConfigSource::Flag,
        };
    }
    if let Some(value) = &sources.flags.notifier {
        notifier = ResolvedField {
            value: Some(value.clone()),
            source: ConfigSource::Flag,
        };
    }

    let k8s = sources
        .xdg
        .iter()
        .chain(sources.project.iter())
        .flat_map(|c| c.k8s.iter().cloned())
        .collect();

    ResolvedConfig {
        store,
        cache,
        notifier,
        k8s,
    }
}

/// `$XDG_CONFIG_HOME/monitra/config.toml`, falling back to
/// `$HOME/.config/monitra/config.toml` when `XDG_CONFIG_HOME` is unset, per
/// the XDG base directory spec's own fallback rule.
pub fn xdg_config_path() -> Option<PathBuf> {
    if let Ok(xdg) = env::var("XDG_CONFIG_HOME")
        && !xdg.is_empty()
    {
        return Some(PathBuf::from(xdg).join("monitra").join("config.toml"));
    }
    env::var("HOME").ok().map(|home| {
        PathBuf::from(home)
            .join(".config")
            .join("monitra")
            .join("config.toml")
    })
}

/// `./monitra.toml` relative to the current working directory.
pub fn project_config_path() -> PathBuf {
    PathBuf::from("monitra.toml")
}

/// Reads and parses a config file. `Ok(None)` when the file does not exist —
/// that is the pristine-machine case, not an error (§11.7).
pub fn load_file(path: &Path) -> Result<Option<ConfigFile>, ProviderError> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(ProviderError::ConfigRead {
                path: path.to_path_buf(),
                source: err,
            });
        }
    };
    let file: ConfigFile =
        toml::from_str(&contents).map_err(|source| ProviderError::ConfigParse {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(Some(file))
}

/// Writes a config file, creating parent directories as needed.
pub fn write_file(path: &Path, config: &ConfigFile) -> Result<(), ProviderError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ProviderError::ConfigWrite {
            path: path.to_path_buf(),
            source,
        })?;
    }
    let contents =
        toml::to_string_pretty(config).map_err(|source| ProviderError::ConfigSerialize {
            path: path.to_path_buf(),
            source,
        })?;
    fs::write(path, contents).map_err(|source| ProviderError::ConfigWrite {
        path: path.to_path_buf(),
        source,
    })
}

/// Gathers real env vars into the map `resolve` expects. The one function in
/// this module that touches process state — kept thin deliberately so the
/// precedence logic it feeds stays pure and testable.
pub fn gather_env() -> HashMap<String, String> {
    ["MONITRA_STORE", "MONITRA_CACHE", "MONITRA_NOTIFIER"]
        .into_iter()
        .filter_map(|key| env::var(key).ok().map(|value| (key.to_string(), value)))
        .collect()
}
