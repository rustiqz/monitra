//! Argument surface (DESIGN.md §4 `cli`).
//!
//! Owns the `clap` command tree and argument-shape validation only.
//! `Cli::parse()` produces a fully-typed command description; `main.rs`
//! decides what to do with it. No command execution, no I/O.

mod agent;
mod k8s;
mod monitor;
mod service;

pub use agent::AgentCommand;
pub use k8s::K8sCommand;
pub use monitor::{MonitorCommand, MonitorKindArg};
pub use service::ServiceCommand;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "monitra",
    version,
    about = "Single-binary, CLI-first uptime monitoring"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Print build info.
    Version,
    /// Run the daemon: engine + backend + storage.
    Start {
        #[arg(long)]
        config: Option<String>,
        #[arg(long)]
        bind: Option<String>,
    },
    /// Run the terminal dashboard. Attaches to a running daemon, or boots an
    /// embedded backend on loopback for a local, no-daemon session (ADR-009).
    Tui {
        #[arg(long)]
        url: Option<String>,
    },
    /// Interactive first-run wizard. Never required — `monitor add` must work
    /// on a pristine machine with no config (§11.7).
    Setup,
    /// Manage monitors.
    Monitor {
        #[command(subcommand)]
        command: MonitorCommand,
    },
    /// Manage remote agents (ADR-008).
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    /// Manage attached Kubernetes clusters (a `Collector` provider, ADR-008).
    K8s {
        #[command(subcommand)]
        command: K8sCommand,
    },
    /// Manage Store/Cache/Notifier providers (§4.1, ADR-007).
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn command_tree_is_valid() {
        Cli::command().debug_assert();
    }
}
