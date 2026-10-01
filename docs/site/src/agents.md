# Agents

An agent pushes local host checks and regional network probes to a Monitra backend. Register it first, then save the issued push token. Re-registering the same name issues a new token and revokes the old one.

```sh
monitra agent register --name host-a --scope host --region eu-west
monitra agent list
monitra agent run --name host-a --scope host --backend-url http://127.0.0.1:8080 --agent-id 1 --token-file /path/to/token --config /path/to/checks.toml
```

`--token` and `--token-file` are mutually exclusive. The check file is optional; an agent with no local checks still pushes an empty batch as a heartbeat. A supplied but missing check file is an error. A check file can define disk, systemd, and process checks tied to existing `host-agent-check` monitor IDs:

```toml
interval_secs = 30

[[checks]]
kind = "disk"
monitor_id = 1
path = "/"
min_free_pct = 10.0
```

Create that monitor with `monitra monitor add --name root-disk --target / --kind host-agent-check --interval 30 --agent-id 1`. The agent fetches regional assignments and probes HTTP, TCP, and ICMP targets from its location. `monitra monitor regions` compares observations from region-tagged agents. Omitting `--region` excludes an agent from regional aggregation; repeating registration without it clears a prior region. Kubernetes monitors are not regional assignments.

Failed pushes are buffered up to 256 results by default; overflow drops the oldest with a warning. The agent attempts a final flush on SIGINT/SIGTERM. Host agent Kubernetes fallback polling is not yet implemented. See [Limitations](limitations.md).
