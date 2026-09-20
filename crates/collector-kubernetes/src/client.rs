//! A minimal, direct K8s API client (DESIGN.md §11.12) — plain `reqwest`
//! calls against the API server's REST surface rather than the full
//! `kube`/`k8s-openapi` crates. Deliberately narrow: read-only GETs against
//! Deployments/StatefulSets/Services/Endpoints, the exact surface §11.12's
//! RBAC note requires and nothing more. See the crate README for why.

use serde_json::Value;

use crate::error::CollectorKubernetesError as Error;
use crate::kubeconfig::ClusterAuth;

pub struct K8sApiClient {
    http: reqwest::Client,
    server: String,
    token: String,
}

pub enum ResourceHealth {
    Healthy,
    Unhealthy { reason: String },
}

impl K8sApiClient {
    pub fn new(auth: ClusterAuth, cluster_name: &str) -> Result<Self, Error> {
        let mut builder = reqwest::Client::builder();
        if let Some(pem) = &auth.ca_cert_pem {
            let cert =
                reqwest::Certificate::from_pem(pem).map_err(|source| Error::InvalidCaCert {
                    path: cluster_name.to_string(),
                    source,
                })?;
            builder = builder.add_root_certificate(cert);
        }
        let http = builder.build().map_err(|source| Error::ClientBuild {
            cluster: cluster_name.to_string(),
            source,
        })?;
        Ok(Self {
            http,
            server: auth.server,
            token: auth.token,
        })
    }

    pub async fn deployment_health(
        &self,
        namespace: &str,
        name: &str,
    ) -> Result<ResourceHealth, Error> {
        let path = format!("/apis/apps/v1/namespaces/{namespace}/deployments/{name}");
        self.replica_health(&path).await
    }

    pub async fn stateful_set_health(
        &self,
        namespace: &str,
        name: &str,
    ) -> Result<ResourceHealth, Error> {
        let path = format!("/apis/apps/v1/namespaces/{namespace}/statefulsets/{name}");
        self.replica_health(&path).await
    }

    /// Healthy means the `Service` exists *and* has at least one ready
    /// endpoint address — existence alone would silently under-report a
    /// service with zero backing pods as "up" (P1: never guess).
    pub async fn service_health(
        &self,
        namespace: &str,
        name: &str,
    ) -> Result<ResourceHealth, Error> {
        let svc_path = format!("/api/v1/namespaces/{namespace}/services/{name}");
        if self.get(&svc_path).await?.is_none() {
            return Ok(not_found(&svc_path));
        }

        let ep_path = format!("/api/v1/namespaces/{namespace}/endpoints/{name}");
        let endpoints = match self.get(&ep_path).await? {
            None => return Ok(not_found(&ep_path)),
            Some(body) => body,
        };

        let has_ready_address = endpoints["subsets"]
            .as_array()
            .map(|subsets| {
                subsets.iter().any(|subset| {
                    subset["addresses"]
                        .as_array()
                        .is_some_and(|addrs| !addrs.is_empty())
                })
            })
            .unwrap_or(false);

        if has_ready_address {
            Ok(ResourceHealth::Healthy)
        } else {
            Ok(ResourceHealth::Unhealthy {
                reason: "service has no ready endpoint addresses".to_string(),
            })
        }
    }

    async fn replica_health(&self, path: &str) -> Result<ResourceHealth, Error> {
        let body = match self.get(path).await? {
            None => return Ok(not_found(path)),
            Some(body) => body,
        };
        let desired = body["spec"]["replicas"].as_i64().unwrap_or(0);
        let ready = body["status"]["readyReplicas"].as_i64().unwrap_or(0);
        if ready >= desired {
            Ok(ResourceHealth::Healthy)
        } else {
            Ok(ResourceHealth::Unhealthy {
                reason: format!("{ready}/{desired} replicas ready"),
            })
        }
    }

    /// `Ok(None)` on a confirmed 404 — the API server gave a definitive
    /// answer, not a connectivity/auth problem, so callers treat it as a
    /// real health signal rather than an `Unknown`/`Err`.
    async fn get(&self, path: &str) -> Result<Option<Value>, Error> {
        let url = format!("{}{path}", self.server);
        let response = self
            .http
            .get(&url)
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|source| Error::Request {
                url: url.clone(),
                source,
            })?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(Error::UnexpectedStatus { url, status, body });
        }

        response
            .json()
            .await
            .map(Some)
            .map_err(|source| Error::Request { url, source })
    }
}

fn not_found(path: &str) -> ResourceHealth {
    ResourceHealth::Unhealthy {
        reason: format!("{path} not found"),
    }
}
