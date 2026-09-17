//! `monitra k8s …` — argument shape only.
//!
//! Attaching a Kubernetes cluster is a `Collector` provider (§4.1, ADR-008),
//! but gets its own command family rather than riding `monitra service
//! attach` — a cluster needs a kubeconfig path/context/namespace, which
//! doesn't fit cleanly into a single provider URL.

use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum K8sCommand {
    /// Attach a Kubernetes cluster as a `Collector`.
    Attach {
        #[arg(long)]
        name: String,
        #[arg(long)]
        kubeconfig: String,
        #[arg(long)]
        context: Option<String>,
        #[arg(long)]
        namespace: Option<String>,
    },
    /// List attached clusters.
    List,
    /// Detach a cluster.
    Detach { name: String },
}
