# Alerting

Monitra emits an alert when a monitor's evaluated status changes. The engine formats a transition message, passes it to the configured notifier, and records an alert event with the monitor ID, new status, time, attempted sink, and delivery outcome.

```sh
monitra alert list
```

The default notifier logs messages. A configured `webhook://hooks.example.com/path` sends a JSON `message` over HTTPS. With the `slack` Cargo feature, `slack://hooks.example.com/services/path` sends a Slack `text` payload. See [Providers](providers.md).

The alert request channel holds 256 entries; if full, it drops the newest request and logs a warning. The notifier retry queue also holds 256 entries; on overflow it drops the oldest and logs it. Failed deliveries are recorded as queued for retry, with retries on a 30-second interval. Alert history remains queryable even when delivery fails. `monitra alert list` shows all recorded events, newest first; monitor detail also has per-monitor alert history.
