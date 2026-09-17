//! `Collector` provider: direct Kubernetes API polling (DESIGN.md §4, ADR-008).
//!
//! Gated behind the `kubernetes` cargo feature (Cargo.toml, root). Polls the
//! API server (kubeconfig or in-cluster service account) for Deployment/
//! StatefulSet/Service status and on-demand pod-level breakdown. Has no
//! default — a `Collector` only exists because an operator configured one.
//!
//! Empty as of Phase 1 (scaffolding).
