# Providers

Monitra has four provider categories. The default build needs no external service. `monitra service attach <URL>` writes a Store, Cache, or Notifier URL to the XDG config; Kubernetes collectors use `monitra k8s attach` instead.

| Category | Default | Configured option and current status | Failure behavior |
|---|---|---|---|
| Store | SQLite | `postgres://` or `postgresql://` is recognized but PostgreSQL is not yet implemented, including with the optional `postgres` feature. | Startup fails with a named error; history is never silently moved to another store. |
| Cache | In-process | `redis://` is recognized but Redis is not yet implemented, including with the optional `redis` feature. | Startup warns and degrades to in-process cache. Current daemon use is limited to health reporting. |
| Notifier | Log sink | `webhook://host/path` posts JSON to `https://host/path`. `slack://host/path` posts to `https://host/path` when built with optional `slack` feature. Both are implemented. | Failed sends enter a bounded retry queue; a build without `slack` rejects that URL at startup. |
| Collector | None | Kubernetes resource collector requires optional `kubernetes` feature and an attached cluster. | Collector errors become unknown/stale for affected monitors and are logged. |

`webhook://` and `slack://` are configuration markers, not transport protocols. The notifier converts each to HTTPS. Without a notifier URL, alerts are logged instead of sent. Feature flags are Cargo features: `postgres`, `redis`, `slack`, and `kubernetes`. See [Alerting](alerting.md) and [Kubernetes](kubernetes.md).
