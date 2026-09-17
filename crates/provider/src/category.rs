//! The four provider categories (DESIGN.md §4.1, ADR-007, extended by ADR-008).

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderCategory {
    Store,
    Cache,
    Notifier,
    Collector,
}

impl fmt::Display for ProviderCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            ProviderCategory::Store => "store",
            ProviderCategory::Cache => "cache",
            ProviderCategory::Notifier => "notifier",
            ProviderCategory::Collector => "collector",
        };
        f.write_str(name)
    }
}
