//! `monitra service …` — argument shape only.
//!
//! Generic attach/detach across the `Store`/`Cache`/`Notifier` provider
//! categories (§4.1, ADR-007) by URL scheme. Deliberately does not imply
//! liveness on a running daemon (§11.8): ADR-007 resolves providers at
//! startup, and only `Notifier` could plausibly support a true live
//! attach/detach later — `Store`/`Cache` need a restart to take effect.

use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum ServiceCommand {
    /// Configure a provider by URL (e.g. `postgres://…`, `redis://…`, `slack://…`).
    /// Takes effect on the next `monitra start`.
    Attach { url: String },
    /// Remove a configured provider, reverting that category to its embedded default.
    Detach { name: String },
    /// List configured providers.
    List,
}
