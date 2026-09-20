//! `collector-kubernetes`'s error vocabulary (CLAUDE.md "errors name their
//! component").

#[derive(Debug, thiserror::Error)]
pub enum CollectorKubernetesError {
    #[error("collector-kubernetes: failed to read kubeconfig {path}: {source}")]
    KubeconfigRead {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("collector-kubernetes: {path} is not valid kubeconfig YAML: {source}")]
    KubeconfigParse {
        path: String,
        #[source]
        source: serde_yaml::Error,
    },

    #[error("collector-kubernetes: kubeconfig {path}: no context named {context:?} found")]
    ContextNotFound { path: String, context: String },

    #[error(
        "collector-kubernetes: kubeconfig {path}: cluster {cluster:?} referenced by context is not defined"
    )]
    ClusterNotFound { path: String, cluster: String },

    #[error(
        "collector-kubernetes: kubeconfig {path}: user {user:?} referenced by context is not defined"
    )]
    UserNotFound { path: String, user: String },

    #[error(
        "collector-kubernetes: kubeconfig {path}: user {user:?} has no supported auth method \
         (only a bearer `token` is supported — client-certificate and exec-plugin auth are not)"
    )]
    UnsupportedAuth { path: String, user: String },

    #[error(
        "collector-kubernetes: {path}: certificate-authority-data is not valid base64: {source}"
    )]
    InvalidCaData {
        path: String,
        #[source]
        source: base64::DecodeError,
    },

    #[error(
        "collector-kubernetes: {path}: certificate-authority-data is not a valid PEM certificate: {source}"
    )]
    InvalidCaCert {
        path: String,
        #[source]
        source: reqwest::Error,
    },

    #[error("collector-kubernetes: not running in-cluster (no {var} set)")]
    NotInCluster { var: &'static str },

    #[error("collector-kubernetes: in-cluster service account: failed to read {path}: {source}")]
    ServiceAccountRead {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("collector-kubernetes: failed to build HTTP client for cluster {cluster:?}: {source}")]
    ClientBuild {
        cluster: String,
        #[source]
        source: reqwest::Error,
    },

    #[error(
        "collector-kubernetes: unknown cluster {cluster:?} (not attached via `monitra k8s attach`)"
    )]
    UnknownCluster { cluster: String },

    #[error("collector-kubernetes: request to {url} failed: {source}")]
    Request {
        url: String,
        #[source]
        source: reqwest::Error,
    },

    #[error("collector-kubernetes: {url} returned unexpected status {status}: {body}")]
    UnexpectedStatus {
        url: String,
        status: reqwest::StatusCode,
        body: String,
    },
}
