# Kubernetes

Kubernetes collection is optional. Build with the `kubernetes` Cargo feature, attach a cluster, then create a Kubernetes monitor whose target identifies the resource to inspect.

```sh
cargo build --release --features kubernetes
monitra k8s attach --name production --kubeconfig /path/to/kubeconfig --context production --namespace default
monitra k8s list
monitra monitor add --name api --target production/default/api --kind k8s-deployment --interval 30
```

The CLI also accepts `k8s-stateful-set` and `k8s-service` kinds. Omitted context selects the kubeconfig current context; omitted namespace uses `default`. Cluster attachments are saved in XDG config and take effect on the next daemon start. `monitra k8s detach production` removes one.

The engine polls a collector at each monitor's interval. If the collector cannot be created or queried, it logs the error and records an unknown observation; it does not assert that the resource is down or stop the whole daemon. Without the `kubernetes` feature, no collector factory is available. See [Providers](providers.md).
