# Limitations

Current behavior has these boundaries:

1. PostgreSQL Store and Redis Cache are not implemented. Their Cargo features were removed; configured URLs fail startup with a named unsupported-provider error. Clear legacy values with `monitra service detach store` or `monitra service detach cache`.
2. Agent-side Kubernetes fallback polling is not yet implemented. Kubernetes collection uses the daemon's optional collector.
3. SIGINT/SIGTERM are wired into daemon shutdown, but shutdown still depends on task drain completing; inspect logs if a process does not exit promptly. The engine has a 10-second drain deadline.
4. The cache is currently used for health reporting; engine and backend do not yet use `Cache::get` or `Cache::set` for data caching.
5. Remote `tui` and `web` token resolution checks `--token`, then environment/config. An error suggests `monitra setup`, but setup does not generate a token; generation happens on backend start.
6. Hourly and daily rollups from DESIGN §5.4 are not yet implemented. Uptime percentages use retained raw samples only and are not a 90-day statistic.
7. `monitra monitor history <ID> --since <UNIX_SECS>` accepts Unix seconds without an upper bound check. Very far-future queries are not rejected in the CLI.
8. Kubernetes resources are polled on each monitor's interval, rather than a separate global collector interval.

See [Configuration](configuration.md) for supported defaults and [Providers](providers.md) for current provider availability.
