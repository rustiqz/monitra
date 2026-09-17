//! URL scheme → provider category (DESIGN.md §4 `provider`, §11.8).
//!
//! `monitra service attach <url>` is generic across `Store`/`Cache`/
//! `Notifier` by scheme — this is the lookup that makes that possible
//! without a separate `--category` flag. `Collector`/Kubernetes doesn't ride
//! this at all (§11.8): it has its own `monitra k8s attach` family instead.

use crate::category::ProviderCategory;
use crate::error::ProviderError;

pub fn category_for_scheme(url: &str) -> Result<ProviderCategory, ProviderError> {
    let scheme = url
        .split_once("://")
        .map(|(scheme, _)| scheme)
        .unwrap_or(url);
    match scheme {
        "postgres" | "postgresql" => Ok(ProviderCategory::Store),
        "redis" => Ok(ProviderCategory::Cache),
        "slack" | "webhook" => Ok(ProviderCategory::Notifier),
        other => Err(ProviderError::UnknownScheme {
            scheme: other.to_string(),
        }),
    }
}
