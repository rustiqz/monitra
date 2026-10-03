# CLI reference

All commands start with `monitra`. Use `monitra <command> --help` for parser help. Arguments in angle brackets are required; square brackets indicate optional values. There are no implicit defaults for required `monitor add` flags.

| Command | Arguments | Action |
|---|---|---|
| `monitra version` | — | Print build version. |
| `monitra start` | `[--config <PATH>] [--bind <ADDR>] [--retention-days <DAYS>]` | Run the daemon; default bind `127.0.0.1:8080`. `--config` replaces `./monitra.toml` in the project config layer. Raw-result retention defaults to 7 days. |
| `monitra setup` | — | Run the optional config wizard. |
| `monitra tui` | `[--url <URL>] [--token <TOKEN>]` | Run the terminal client; without `--url`, start a local backend. |
| `monitra web` | `[--url <URL>] [--token <TOKEN>]` | Print the browser URL and token; without `--url`, serve a local backend. |

`tui` and `web` resolve remote tokens from `--token`, then resolved configuration (including `MONITRA_API_TOKEN`). Their `--token` flag is ignored in embedded mode.

## Monitors

| Command | Arguments | Action |
|---|---|---|
| `monitra monitor add` | `--name <NAME> --target <TARGET> --kind <KIND> --interval <SECS> [--agent-id <ID>]` | Add a monitor. Kinds: `http`, `tcp`, `icmp`, `k8s-deployment`, `k8s-stateful-set`, `k8s-service`, `host-agent-check`. Supply `--agent-id` for agent-fed checks. Intervals must be 1..=86,400 seconds. |
| `monitra monitor list` | — | List monitors. |
| `monitra monitor show` | `<ID>` | Show one monitor. |
| `monitra monitor edit` | `<ID> [--name <NAME>] [--target <TARGET>] [--interval <SECS>] [--agent-id <ID>]` | Change a monitor. |
| `monitra monitor remove` | `<ID>` | Delete a monitor. |
| `monitra monitor pause` | `<ID>` | Pause checks. |
| `monitra monitor resume` | `<ID>` | Resume into `Pending`. |
| `monitra monitor history` | `<ID> [--since <UNIX_SECS>]` | Show check results at or after the timestamp, capped at the retention cutoff. |
| `monitra monitor regions` | `[--since <UNIX_SECS>]` | Compare regional latency and failures since the timestamp, capped at the retention cutoff. |

## Agents, collectors, and services

| Command | Arguments | Action |
|---|---|---|
| `monitra agent register` | `--name <NAME> --scope <SCOPE> [--region <REGION>]` | Register an agent and issue its push token. |
| `monitra agent list` | — | List agents. |
| `monitra agent remove` | `<ID>` | Deregister an agent. |
| `monitra agent run` | `--name <NAME> --scope <SCOPE> --backend-url <URL> --agent-id <ID> [--token <TOKEN> \| --token-file <PATH>] [--config <PATH>]` | Run local checks and push results. Token or token file is required at runtime. |
| `monitra k8s attach` | `--name <NAME> --kubeconfig <PATH> [--context <CONTEXT>] [--namespace <NAMESPACE>]` | Attach a cluster; omitted context uses kubeconfig's current context, omitted namespace uses `default`. |
| `monitra k8s list` | — | List attached clusters. |
| `monitra k8s detach` | `<NAME>` | Detach a cluster. |
| `monitra service attach` | `<URL>` | Configure a Store, Cache, or Notifier provider by URL. |
| `monitra service detach` | `<NAME>` | Clear `store`, `cache`, or `notifier` configuration. |
| `monitra service list` | — | List effective provider configuration and sources. |
| `monitra alert list` | — | Show recorded alert history, newest first. |

Configuration commands write the XDG config and take effect on the next `monitra start`. `monitra service` manages providers, not an operating system service. See [Providers](providers.md).
