//! `monitra agent …` — argument shape only (DESIGN.md §4 `agent`, ADR-008).

use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum AgentCommand {
    /// Register a new remote agent (a host or a Kubernetes cluster it will watch).
    Register {
        #[arg(long)]
        name: String,
        /// What it watches — a host, or a Kubernetes cluster/namespace reference.
        #[arg(long)]
        scope: String,
    },
    /// List registered agents.
    List,
    /// Deregister an agent.
    Remove { id: u64 },
    /// Run as an agent process: local host checks + push loop (§4 `agent`).
    Run {
        #[arg(long)]
        name: String,
        #[arg(long)]
        scope: String,
        #[arg(long)]
        backend_url: String,
        #[arg(long, conflicts_with = "token_file")]
        token: Option<String>,
        #[arg(long, conflicts_with = "token")]
        token_file: Option<String>,
        #[arg(long)]
        config: Option<String>,
    },
}
