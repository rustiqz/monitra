# Providers

Monitra has four provider categories. The default build needs no external service. `monitra service attach <URL>` writes a Store, Cache, or Notifier URL to the XDG config; Kubernetes collectors use `monitra k8s attach` instead.

| Category | Default | Configured option and current status | Failure behavior |
|---|---|---|---|
| Store | SQLite | PostgreSQL is not implemented; `postgres://` and `postgresql://` are unsupported. | Startup fails with a named error; history is never silently moved to another store. |
| Cache | In-process | Redis is not implemented; `redis://` is unsupported. Current daemon use is limited to health reporting. | Unsupported configuration fails startup. An implemented cache that later becomes unreachable would degrade under §4.1. |
| Notifier | Log sink | `webhook://host/path` posts JSON to `https://host/path`. `slack://host/path` posts to `https://host/path` when built with optional `slack` feature. Both are implemented. | Failed sends enter a bounded retry queue; a build without `slack` rejects that URL at startup. |
| Collector | None | Kubernetes resource collector requires optional `kubernetes` feature and an attached cluster. | Collector errors become unknown/stale for affected monitors and are logged. |

`webhook://` and `slack://` are configuration markers, not transport protocols. The notifier converts each to HTTPS. Without a notifier URL, alerts are logged instead of sent. Optional Cargo features are `slack` and `kubernetes`. See [Alerting](alerting.md) and [Kubernetes](kubernetes.md).
