//! Resolves a `K8sClusterConfig` (DESIGN.md §11.12) into the bare server
//! URL, CA cert, and bearer token a client needs — either from a kubeconfig
//! file or, when `kubeconfig` is the literal sentinel `"in-cluster"`, from
//! the standard in-cluster service-account paths.
//!
//! Deliberately narrow scope: only bearer-token auth is supported (the
//! common shape for CI/service-account-issued kubeconfigs). A
//! client-certificate or exec-plugin user is a named, clear error — never a
//! silent "works for some clusters, not others" (P1: never guess).

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::Deserialize;

use crate::error::CollectorKubernetesError as Error;

/// The literal `kubeconfig` value meaning "use the in-cluster service
/// account" instead of a file path — `K8sClusterConfig.kubeconfig` is a
/// required `String` (DESIGN.md §11.7/Phase 2), so this is the sentinel
/// that keeps "no file, use the pod's own identity" expressible without
/// changing that field to `Option`.
pub const IN_CLUSTER_SENTINEL: &str = "in-cluster";

#[derive(Debug)]
pub struct ClusterAuth {
    pub server: String,
    pub ca_cert_pem: Option<Vec<u8>>,
    pub token: String,
}

impl ClusterAuth {
    pub fn resolve(kubeconfig: &str, context_override: Option<&str>) -> Result<Self, Error> {
        if kubeconfig == IN_CLUSTER_SENTINEL {
            Self::in_cluster()
        } else {
            Self::from_kubeconfig_file(kubeconfig, context_override)
        }
    }

    fn in_cluster() -> Result<Self, Error> {
        let host = std::env::var("KUBERNETES_SERVICE_HOST").map_err(|_| Error::NotInCluster {
            var: "KUBERNETES_SERVICE_HOST",
        })?;
        let port = std::env::var("KUBERNETES_SERVICE_PORT").map_err(|_| Error::NotInCluster {
            var: "KUBERNETES_SERVICE_PORT",
        })?;

        const BASE: &str = "/var/run/secrets/kubernetes.io/serviceaccount";
        let token = read_file(&format!("{BASE}/token"))?;
        let ca_cert_pem = std::fs::read(format!("{BASE}/ca.crt")).map_err(|source| {
            Error::ServiceAccountRead {
                path: format!("{BASE}/ca.crt"),
                source,
            }
        })?;

        Ok(Self {
            server: format!("https://{host}:{port}"),
            ca_cert_pem: Some(ca_cert_pem),
            token: token.trim().to_string(),
        })
    }

    fn from_kubeconfig_file(path: &str, context_override: Option<&str>) -> Result<Self, Error> {
        let raw = std::fs::read_to_string(path).map_err(|source| Error::KubeconfigRead {
            path: path.to_string(),
            source,
        })?;
        let config: RawKubeConfig =
            serde_yaml::from_str(&raw).map_err(|source| Error::KubeconfigParse {
                path: path.to_string(),
                source,
            })?;

        let context_name = context_override
            .map(str::to_string)
            .or(config.current_context.clone())
            .ok_or_else(|| Error::ContextNotFound {
                path: path.to_string(),
                context: String::new(),
            })?;
        let context = config
            .contexts
            .iter()
            .find(|c| c.name == context_name)
            .map(|c| &c.context)
            .ok_or_else(|| Error::ContextNotFound {
                path: path.to_string(),
                context: context_name.clone(),
            })?;

        let cluster = config
            .clusters
            .iter()
            .find(|c| c.name == context.cluster)
            .map(|c| &c.cluster)
            .ok_or_else(|| Error::ClusterNotFound {
                path: path.to_string(),
                cluster: context.cluster.clone(),
            })?;

        let user = config
            .users
            .iter()
            .find(|u| u.name == context.user)
            .map(|u| &u.user)
            .ok_or_else(|| Error::UserNotFound {
                path: path.to_string(),
                user: context.user.clone(),
            })?;

        let token = user.token.clone().ok_or_else(|| Error::UnsupportedAuth {
            path: path.to_string(),
            user: context.user.clone(),
        })?;

        let ca_cert_pem = cluster
            .certificate_authority_data
            .as_deref()
            .map(|encoded| {
                BASE64
                    .decode(encoded)
                    .map_err(|source| Error::InvalidCaData {
                        path: path.to_string(),
                        source,
                    })
            })
            .transpose()?;

        Ok(Self {
            server: cluster.server.clone(),
            ca_cert_pem,
            token,
        })
    }
}

fn read_file(path: &str) -> Result<String, Error> {
    std::fs::read_to_string(path).map_err(|source| Error::ServiceAccountRead {
        path: path.to_string(),
        source,
    })
}

#[derive(Deserialize)]
struct RawKubeConfig {
    #[serde(default)]
    clusters: Vec<NamedCluster>,
    #[serde(default)]
    contexts: Vec<NamedContext>,
    #[serde(default)]
    users: Vec<NamedUser>,
    #[serde(rename = "current-context", default)]
    current_context: Option<String>,
}

#[derive(Deserialize)]
struct NamedCluster {
    name: String,
    cluster: ClusterDetail,
}

#[derive(Deserialize)]
struct ClusterDetail {
    server: String,
    #[serde(rename = "certificate-authority-data")]
    certificate_authority_data: Option<String>,
}

#[derive(Deserialize)]
struct NamedContext {
    name: String,
    context: ContextDetail,
}

#[derive(Deserialize)]
struct ContextDetail {
    cluster: String,
    user: String,
}

#[derive(Deserialize)]
struct NamedUser {
    name: String,
    user: UserDetail,
}

#[derive(Deserialize, Default)]
struct UserDetail {
    #[serde(default)]
    token: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn write_temp(contents: &str) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("kubeconfig.yaml");
        let mut file = std::fs::File::create(&path).expect("create");
        file.write_all(contents.as_bytes()).expect("write");
        (dir, path.to_string_lossy().to_string())
    }

    const SAMPLE: &str = r#"
apiVersion: v1
kind: Config
current-context: prod-ctx
clusters:
- name: prod
  cluster:
    server: https://prod.example.com:6443
    certificate-authority-data: aGVsbG8=
contexts:
- name: prod-ctx
  context:
    cluster: prod
    user: prod-user
users:
- name: prod-user
  user:
    token: deadbeef
"#;

    #[test]
    fn resolves_current_context_by_default() {
        let (_dir, path) = write_temp(SAMPLE);
        let auth = ClusterAuth::resolve(&path, None).expect("resolve");
        assert_eq!(auth.server, "https://prod.example.com:6443");
        assert_eq!(auth.token, "deadbeef");
        assert_eq!(auth.ca_cert_pem.as_deref(), Some(b"hello".as_slice()));
    }

    const MULTI_CONTEXT_SAMPLE: &str = r#"
apiVersion: v1
kind: Config
current-context: prod-ctx
clusters:
- name: prod
  cluster:
    server: https://prod.example.com:6443
- name: staging
  cluster:
    server: https://staging.example.com:6443
contexts:
- name: prod-ctx
  context:
    cluster: prod
    user: shared-user
- name: staging-ctx
  context:
    cluster: staging
    user: shared-user
users:
- name: shared-user
  user:
    token: deadbeef
"#;

    #[test]
    fn context_override_wins_over_current_context() {
        let (_dir, path) = write_temp(MULTI_CONTEXT_SAMPLE);
        let auth = ClusterAuth::resolve(&path, Some("staging-ctx")).expect("resolve");
        assert_eq!(auth.server, "https://staging.example.com:6443");
    }

    #[test]
    fn unknown_context_is_a_named_error() {
        let (_dir, path) = write_temp(SAMPLE);
        let err = ClusterAuth::resolve(&path, Some("does-not-exist")).unwrap_err();
        assert!(matches!(err, Error::ContextNotFound { .. }));
    }

    #[test]
    fn client_certificate_only_user_is_an_unsupported_auth_error() {
        let cert_only = SAMPLE.replace(
            "    token: deadbeef",
            "    client-certificate-data: aGVsbG8=",
        );
        let (_dir, path) = write_temp(&cert_only);
        let err = ClusterAuth::resolve(&path, None).unwrap_err();
        assert!(matches!(err, Error::UnsupportedAuth { .. }));
    }

    #[test]
    fn in_cluster_without_env_vars_is_a_named_error() {
        // SAFETY: test-only env mutation of a var these tests own exclusively.
        unsafe {
            std::env::remove_var("KUBERNETES_SERVICE_HOST");
        }
        let err = ClusterAuth::resolve(IN_CLUSTER_SENTINEL, None).unwrap_err();
        assert!(matches!(err, Error::NotInCluster { .. }));
    }
}
