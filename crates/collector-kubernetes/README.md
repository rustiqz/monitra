# collector-kubernetes

The `Collector` provider for direct Kubernetes API polling (DESIGN.md §4,
ADR-008). Gated behind the root `kubernetes` cargo feature. Has no default —
only exists once a cluster is attached via `monitra k8s attach`.

## Authentication (§11.12)

Two supported shapes, chosen by the configured cluster's `kubeconfig` value:

- **A kubeconfig file path.** Only bearer-token auth (`users[].user.token`)
  is supported. Client-certificate and exec-plugin auth users produce a
  named `UnsupportedAuth` error at startup for that cluster (never a silent
  partial failure).
- **The literal value `in-cluster`.** Uses the pod's own service-account
  token and CA cert from the standard
  `/var/run/secrets/kubernetes.io/serviceaccount/` paths and the
  `KUBERNETES_SERVICE_HOST`/`_PORT` env vars Kubernetes injects automatically.

A cluster whose auth fails to resolve at startup is logged and skipped —
its monitors report `Unknown` (never fails the daemon, §4.1) until the
config is fixed and `monitra start` is restarted.

## Minimum RBAC surface

Read-only `get`/`list` on exactly what's polled — nothing else, and no
write access to a cluster is ever needed (per DESIGN.md §11.12's standing
answer: Monitra introspects, it does not act):

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: monitra-collector
rules:
  - apiGroups: ["apps"]
    resources: ["deployments", "statefulsets"]
    verbs: ["get"]
  - apiGroups: [""]
    resources: ["services", "endpoints"]
    verbs: ["get"]
```

(`endpoints` read is what turns a `K8sService` check from "does the Service
object exist" into "does it actually have a ready backend" — see
`client.rs::service_health`.)

## Why plain `reqwest`, not `kube`/`k8s-openapi`

This crate reads four endpoint shapes: Deployment status, StatefulSet
status, Service existence, Endpoints readiness. The `kube` ecosystem crates
generate types for the entire Kubernetes API surface, which is a large
transitive dependency addition for four GETs — a real cost against §11.13's
still-unverified size budget. Revisit if a future phase needs more of the
API (watches, CRDs, write access).
