//! Pluggable service contracts (DESIGN.md §4 `provider`).
//!
//! Owns the `Store`, `Cache`, `Notifier`, and `Collector` (ADR-008) traits,
//! the provider registry (URL scheme → category), config parsing/precedence
//! resolution, and the per-category availability policies of §4.1. Depends
//! only on `models`; knows nothing about any concrete implementation —
//! `storage`, `store-postgres`, `cache-redis`, `notify-webhook`,
//! `notify-slack`, and `collector-kubernetes` depend on this crate, never
//! the other way around.

mod cache;
mod category;
mod config;
mod error;
mod policy;
mod registry;
mod token;
mod traits;

pub use cache::InProcessCache;
pub use category::ProviderCategory;
pub use config::{
    ConfigFile, ConfigSource, ConfigSources, FlagOverrides, K8sClusterConfig, ResolvedConfig,
    ResolvedField, default_db_path, gather_env, load_file, project_config_path, resolve,
    write_file, xdg_config_path, xdg_data_path,
};
pub use error::ProviderError;
pub use policy::{DegradingCache, RetryingNotifier, poll_collector_safely, resolve_store};
pub use registry::category_for_scheme;
pub use token::generate_api_token;
pub use traits::{Cache, Collector, CollectorStatus, Notifier, Store};

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn pristine_environment_resolves_to_defaults_throughout() {
        // No files, no env, no flags — the §11.7 machine-with-nothing-installed
        // case. Resolution must succeed and every field must be `Default`.
        let sources = ConfigSources::default();
        let resolved = resolve(&sources);

        assert_eq!(
            resolved.store,
            ResolvedField {
                value: None,
                source: ConfigSource::Default
            }
        );
        assert_eq!(
            resolved.cache,
            ResolvedField {
                value: None,
                source: ConfigSource::Default
            }
        );
        assert_eq!(
            resolved.notifier,
            ResolvedField {
                value: None,
                source: ConfigSource::Default
            }
        );
        assert!(resolved.k8s.is_empty());
    }

    #[test]
    fn project_overrides_xdg() {
        let sources = ConfigSources {
            xdg: Some(ConfigFile {
                store: Some("postgres://xdg".to_string()),
                ..Default::default()
            }),
            project: Some(ConfigFile {
                store: Some("postgres://project".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let resolved = resolve(&sources);
        assert_eq!(resolved.store.value.as_deref(), Some("postgres://project"));
        assert_eq!(resolved.store.source, ConfigSource::Project);
    }

    #[test]
    fn env_overrides_project() {
        let mut env = HashMap::new();
        env.insert("MONITRA_STORE".to_string(), "postgres://env".to_string());
        let sources = ConfigSources {
            project: Some(ConfigFile {
                store: Some("postgres://project".to_string()),
                ..Default::default()
            }),
            env,
            ..Default::default()
        };
        let resolved = resolve(&sources);
        assert_eq!(resolved.store.value.as_deref(), Some("postgres://env"));
        assert_eq!(resolved.store.source, ConfigSource::Env);
    }

    #[test]
    fn flag_overrides_env() {
        let mut env = HashMap::new();
        env.insert("MONITRA_STORE".to_string(), "postgres://env".to_string());
        let sources = ConfigSources {
            env,
            flags: FlagOverrides {
                store: Some("postgres://flag".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let resolved = resolve(&sources);
        assert_eq!(resolved.store.value.as_deref(), Some("postgres://flag"));
        assert_eq!(resolved.store.source, ConfigSource::Flag);
    }

    #[test]
    fn missing_config_file_is_not_an_error() {
        let dir =
            std::env::temp_dir().join(format!("monitra-provider-test-{}", std::process::id()));
        let path = dir.join("does-not-exist.toml");
        let result = load_file(&path);
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn malformed_config_file_is_an_error() {
        let dir = std::env::temp_dir().join(format!(
            "monitra-provider-test-malformed-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "not valid toml =====").unwrap();

        let result = load_file(&path);
        assert!(result.is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_then_load_round_trips() {
        let dir = std::env::temp_dir().join(format!(
            "monitra-provider-test-roundtrip-{}",
            std::process::id()
        ));
        let path = dir.join("config.toml");
        let file = ConfigFile {
            store: Some("postgres://host/db".to_string()),
            cache: None,
            notifier: Some("slack://hooks/xyz".to_string()),
            api_token: Some("deadbeef".to_string()),
            k8s: vec![K8sClusterConfig {
                name: "prod".to_string(),
                kubeconfig: "/home/user/.kube/config".to_string(),
                context: Some("prod-ctx".to_string()),
                namespace: None,
            }],
        };

        write_file(&path, &file).unwrap();
        let loaded = load_file(&path)
            .unwrap()
            .expect("just-written file must exist");
        assert_eq!(loaded, file);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn known_schemes_map_to_their_category() {
        assert_eq!(
            category_for_scheme("postgres://host/db").unwrap(),
            ProviderCategory::Store
        );
        assert_eq!(
            category_for_scheme("redis://host:6379").unwrap(),
            ProviderCategory::Cache
        );
        assert_eq!(
            category_for_scheme("slack://hooks/xyz").unwrap(),
            ProviderCategory::Notifier
        );
        assert_eq!(
            category_for_scheme("webhook://host/path").unwrap(),
            ProviderCategory::Notifier
        );
    }

    #[test]
    fn unknown_scheme_is_a_named_error_not_a_panic() {
        let result = category_for_scheme("ftp://host");
        assert!(matches!(result, Err(ProviderError::UnknownScheme { .. })));
    }
}
