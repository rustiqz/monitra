//! `Collector` provider: direct Kubernetes API polling (DESIGN.md §4, ADR-008).
//!
//! Gated behind the `kubernetes` cargo feature (Cargo.toml, root). Polls the
//! API server (kubeconfig or in-cluster service account) for Deployment/
//! StatefulSet/Service status. Has no default — a `Collector` only exists
//! because an operator configured one, via `monitra k8s attach`.
//!
//! Deliberately built on plain `reqwest` calls against the K8s REST API
//! rather than the `kube`/`k8s-openapi` crates (Phase 6 decision): those
//! pull in a large transitive dependency tree for generated types covering
//! the entire API surface when this crate only ever reads four endpoint
//! shapes. Revisit if a future phase needs more of the API (watches,
//! CRDs, …) — see §11.13 on the size-budget risk this sidesteps for now.

mod client;
mod error;
mod kubeconfig;

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use monitra_models::MonitorKind;
use monitra_provider::{
    Collector, CollectorStatus, K8sClusterConfig, K8sCollectorFactory, ProviderCategory,
    ProviderError,
};

use client::{K8sApiClient, ResourceHealth};
pub use error::CollectorKubernetesError;
pub use kubeconfig::IN_CLUSTER_SENTINEL;

pub struct K8sCollector {
    client: Arc<K8sApiClient>,
    namespace: String,
    name: String,
    kind: MonitorKind,
}

#[async_trait]
impl Collector for K8sCollector {
    fn name(&self) -> &'static str {
        "kubernetes"
    }

    async fn poll(&self) -> Result<CollectorStatus, ProviderError> {
        let health = match self.kind {
            MonitorKind::K8sDeployment => {
                self.client
                    .deployment_health(&self.namespace, &self.name)
                    .await
            }
            MonitorKind::K8sStatefulSet => {
                self.client
                    .stateful_set_health(&self.namespace, &self.name)
                    .await
            }
            MonitorKind::K8sService => {
                self.client
                    .service_health(&self.namespace, &self.name)
                    .await
            }
            other => {
                return Err(ProviderError::Operation {
                    category: ProviderCategory::Collector,
                    detail: format!(
                        "collector-kubernetes: not a Kubernetes MonitorKind: {other:?}"
                    ),
                });
            }
        };

        match health {
            Ok(ResourceHealth::Healthy) => Ok(CollectorStatus::Healthy),
            Ok(ResourceHealth::Unhealthy { reason }) => Ok(CollectorStatus::Unhealthy { reason }),
            Err(source) => Err(ProviderError::Unavailable {
                category: ProviderCategory::Collector,
                detail: source.to_string(),
            }),
        }
    }
}

/// Builds a `K8sCollector` per (cluster, namespace, name, kind), reusing one
/// [`K8sApiClient`] (and its connection pool) per cluster across every
/// monitor that references it.
pub struct Factory {
    clusters: HashMap<String, Arc<K8sApiClient>>,
}

impl Factory {
    /// Eagerly resolves auth for every attached cluster at startup. A
    /// cluster whose kubeconfig fails to resolve is logged and skipped, not
    /// fatal to the others (§4.1: `Collector` failures are per-resource,
    /// never daemon-fatal) — monitors referencing it report `Unknown` until
    /// the config is fixed and the daemon restarted.
    pub fn new(clusters: &[K8sClusterConfig]) -> Self {
        let mut resolved = HashMap::new();
        for cluster in clusters {
            let client =
                kubeconfig::ClusterAuth::resolve(&cluster.kubeconfig, cluster.context.as_deref())
                    .map_err(|source| source.to_string())
                    .and_then(|auth| {
                        K8sApiClient::new(auth, &cluster.name).map_err(|source| source.to_string())
                    });
            match client {
                Ok(client) => {
                    resolved.insert(cluster.name.clone(), Arc::new(client));
                }
                Err(message) => {
                    tracing::warn!(
                        cluster = %cluster.name,
                        error = %message,
                        "collector-kubernetes: failed to initialize cluster, its monitors will report unknown until fixed and restarted"
                    );
                }
            }
        }
        Self { clusters: resolved }
    }
}

impl K8sCollectorFactory for Factory {
    fn collector_for(
        &self,
        cluster: &str,
        namespace: &str,
        name: &str,
        kind: MonitorKind,
    ) -> Result<Arc<dyn Collector>, ProviderError> {
        let client =
            self.clusters
                .get(cluster)
                .cloned()
                .ok_or_else(|| ProviderError::Unavailable {
                    category: ProviderCategory::Collector,
                    detail: format!(
                        "collector-kubernetes: unknown cluster '{cluster}' (not attached via \
                     `monitra k8s attach`, or its kubeconfig failed to resolve at startup)"
                    ),
                })?;
        Ok(Arc::new(K8sCollector {
            client,
            namespace: namespace.to_string(),
            name: name.to_string(),
            kind,
        }))
    }
}
