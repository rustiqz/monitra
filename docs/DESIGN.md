# Monitra — Design Document

> **Status:** Living document
> **Version:** 0.3 (Phase 0 — Scaffolding complete; Phase 1 not started)
> **Last updated:** 2026-09-17

This document describes the intent, architecture, and design trade-offs behind Monitra. It is written to be the **contract we build against**, not a description of what already exists. Sections marked *(deferred)* describe planned behaviour that is not yet implemented.

---

## Table of Contents

1. [Vision & Scope](#1-vision--scope)
2. [Design Principles](#2-design-principles)
3. [System Architecture](#3-system-architecture)
4. [Crate Responsibilities](#4-crate-responsibilities)
5. [Data Model](#5-data-model)
6. [Concurrency & Scaling Model](#6-concurrency--scaling-model)
7. [Reliability & Failure Handling](#7-reliability--failure-handling)
8. [Single-Binary Strategy](#8-single-binary-strategy)
9. [Architecture Decision Records](#9-architecture-decision-records)
10. [Roadmap](#10-roadmap)
11. [Open Questions & Known Risks](#11-open-questions--known-risks)

---

## 1. Vision & Scope

### 1.1 The problem

Existing uptime monitoring tools fall into two camps, and both have a gap:

- **Hosted SaaS** (Pingdom, UptimeRobot, Better Stack) — excellent UX, but your monitoring lives on someone else's infrastructure, costs scale per-monitor, and you cannot monitor anything inside a private network without an agent.
- **Self-hosted platforms** (Uptime Kuma, Zabbix, Prometheus Blackbox) — powerful, but deployment is heavy: containers, databases, reverse proxies, config files. Getting from "I want to know if my API is down" to "I am being alerted" takes a weekend.

There is a gap for a tool where **the entire monitoring platform is one executable you can `scp` to a box and run**, with no external database, no container runtime, and no config-file archaeology — while still being genuinely capable at scale.

### 1.2 What Monitra is

Monitra is a **CLI-first uptime monitoring platform** that ships as a single, self-contained binary. It monitors HTTP, TCP, and ICMP targets; stores history locally; exposes a REST + WebSocket API; and provides a terminal dashboard as the primary interface, with an optional embedded web dashboard.

The design target is the engineer who owns infrastructure and lives in a terminal. The workflow we optimise for:

```
$ scp monitra server:/usr/local/bin/
$ ssh server
$ monitra monitor add api https://api.example.com --interval 30
$ monitra start
$ monitra tui        # from anywhere, over SSH
```

Nothing else to install. No `docker compose up`. No Postgres.

### 1.3 What Monitra is explicitly NOT

Scope discipline matters more than feature count. Monitra is **not**:

| Not this | Why | Where to go instead |
|---|---|---|
| A metrics/observability platform | Uptime ≠ time-series telemetry. Storing millions of arbitrary metrics is a fundamentally different storage problem. | Prometheus, VictoriaMetrics |
| An APM / tracing tool | Requires instrumenting application internals; out of scope for black-box probing. | Jaeger, Tempo, Datadog |
| A log aggregator | Different ingest, indexing, and retention model entirely. | Loki, Elasticsearch |
| A general alerting router | We will emit alerts, not become a routing/deduplication/on-call engine. | Alertmanager, PagerDuty |
| A multi-region latency prober | Probing the *same* target from many geographic vantage points to compare latency is a distinct design problem (cross-region aggregation) from watching a target's own internals. Still not a v1 constraint. | *(see Roadmap)* |

**On alerting — revised by ADR-007.** Monitra *does* emit alert events to attached notification sinks (§4 `provider`). It does not group, deduplicate, silence, escalate, or schedule them. The test for whether a proposed alerting feature is in scope: **if it needs to know who is on call, it belongs in Alertmanager or PagerDuty, not here.**

**On distributed monitoring — revised by ADR-008.** The original table row here read "a distributed multi-region prober... not a v1 constraint." That is reversed for target *introspection*: Monitra now reaches into architectures it does not run on — Kubernetes clusters, other machines — via direct API polling and lightweight per-host/per-cluster agents that push results back. This is still a single Monitra instance owning the view; agents extend its reach, they do not become independent regional probers with their own aggregation problem. The row above (probing the *same* target from multiple vantage points for latency comparison) is the part that remains genuinely out of scope — a different problem, not yet reconsidered.

Saying no to these keeps the binary small, the schema simple, and the concurrency model tractable.

### 1.4 Target users

| Persona | Need | How Monitra serves it |
|---|---|---|
| **Solo operator / homelab** | Monitor 5–50 services without running another stack | Single binary, SQLite, zero config to start |
| **Small platform team** | Monitor a few hundred internal + external endpoints, inside a VPC | Runs where the services are; no egress required |
| **Embedded / edge operator** | Monitoring on a resource-constrained device | Small static binary, low idle memory footprint |

The common thread: **people for whom deployment friction is the deciding factor.**

### 1.5 Success criteria

Monitra v1 is successful if:

1. A single static binary under ~25 MB runs on a stock Linux box with no dependencies.
2. It sustains 1,000 monitors at 30s intervals on 2 vCPU / 512 MB RAM without falling behind schedule.
3. Time from download to first monitor firing is under 60 seconds.
4. It survives an unclean shutdown (SIGKILL, power loss) without database corruption or lost configuration.

---

## 2. Design Principles

Every design decision is checked against these six rules. When two conflict, the earlier one wins.

### P1 — Reliability over features

A monitoring tool that lies is worse than no monitoring tool. If we are uncertain whether a target is down, we must say "unknown," not guess. A missed check is reported as a missed check. The monitor that watches everything else must be the most boring, most predictable component in the stack.

**Practical consequence:** no silent failure paths. Every error either propagates via `Result` or is explicitly logged and recorded as check metadata. `unwrap()` is banned outside of tests and provably-infallible cases.

### P2 — Single binary, always

Any feature that requires a sidecar process, external service, or runtime dependency is rejected or redesigned. This constraint is load-bearing for the entire product thesis — it is not a nice-to-have.

**Practical consequence:** SQLite is compiled in (`rusqlite` bundled feature), web assets are embedded via `rust-embed`, TLS uses `rustls` rather than linking system OpenSSL.

### P3 — CLI is the primary interface

The terminal is not a fallback for when the web UI is unavailable — it is the intended interface. The TUI and CLI get first-class design attention. The web dashboard is a convenience layer that consumes the same public API.

**Practical consequence:** every capability must be reachable from the CLI. If a feature only works in the web UI, it is incomplete.

### P4 — Modular crates, acyclic dependencies

Each concern is a separate crate with an explicitly documented contract. The dependency graph is a DAG, enforced by `models` having zero internal dependencies. This keeps compile times manageable and makes each layer independently testable.

**Practical consequence:** `storage` never imports `engine`. `engine` never imports `backend`. Shared types live in `models` or nowhere.

### P5 — Correct before fast

Optimise only against measurement. The scaling target (thousands of monitors) is a hypothesis to be validated with load tests, not a licence for speculative complexity in v1.

**Practical consequence:** the initial engine uses the simplest scheduler that could work. We replace it when a benchmark says we must, and the benchmark goes in the repo.

### P6 — Boring, observable operations

The tool should be diagnosable at 3 a.m. Structured logging, meaningful error messages that name the failing component, and a health endpoint that reports internal state rather than just `200 OK`.

**Practical consequence:** errors carry context (`thiserror` per-crate error enums, `anyhow` context at boundaries). Log lines identify monitor ID and check attempt.

---

## 3. System Architecture

### 3.1 Component map

```
┌──────────────────────────────────────────────────────────────────┐
│                     monitra (single binary)                      │
│                                                                  │
│   ┌──────────┐                                                   │
│   │   cli    │  parse argv → dispatch command                    │
│   └────┬─────┘                                                   │
│        │                                                         │
│        ├──────────────┬─────────────────┬────────────────┐       │
│        ▼              ▼                 ▼                ▼       │
│   ┌─────────┐   ┌──────────┐      ┌──────────┐    ┌──────────┐   │
│   │ engine  │   │ backend  │      │   tui    │    │ storage  │   │
│   │         │   │  (Axum)  │      │(Ratatui) │    │ (SQLite) │   │
│   │scheduler│   │ REST +   │      │ dashboard│    │  schema  │   │
│   │ probes  │   │    WS    │      │  widgets │    │  queries │   │
│   └────┬────┘   └────┬─────┘      └────┬─────┘    └────▲─────┘   │
│        │             │                 │               │         │
│        └─────────────┴─────────────────┴───────────────┘         │
│                              │                                   │
│                         ┌────▼─────┐                             │
│                         │  models  │  shared domain types        │
│                         └──────────┘  (zero internal deps)       │
│                                                                  │
│   ┌────────────────────────────────────────────────────────┐     │
│   │  embedded web assets (rust-embed) — Phase 10           │     │
│   └────────────────────────────────────────────────────────┘     │
└──────────────────────────────────────────────────────────────────┘
                    │                          │
                    ▼                          ▼
            ┌──────────────┐          ┌────────────────┐
            │ monitra.db   │          │ monitored      │
            │  (SQLite)    │          │ targets        │
            └──────────────┘          │ HTTP/TCP/ICMP  │
                                      └────────────────┘
```

### 3.2 Dependency graph

Strictly acyclic. Arrows point from dependent to dependency.

```
                         main (monitra)
        ┌──────┬──────┬──────┴───┬──────────┬──────────────┐
        │      │      │          │          │              │
        ▼      ▼      ▼          ▼          ▼              ▼
      cli   backend  tui      storage  store-postgres  cache-redis
        │      │      │          │          │          notify-*
        │      ▼      │          │          │              │
        │   engine    │          │          │              │
        │      │      │          │          │              │
        │      └──────┴────┬─────┴──────────┴──────────────┘
        │                  ▼
        │              provider          (traits + registry + config)
        └──────────────────┴──────────────► models
```

Added by ADR-008/ADR-009 (v0.3) — two new leaf crates, same DAG discipline as everything else:

```
        main (monitra)
              │
              ├──► agent                          (models only — push client, local host checks)
              │
              └──► provider ──► collector-kubernetes   (models, provider — a Collector impl)
```

Only the root binary knows which concrete providers exist. Every other consumer holds a trait object.

| Crate | May depend on | Must never depend on |
|---|---|---|
| `models` | *(nothing internal)* | everything |
| `provider` | `models` | every other internal crate |
| `storage` | `models`, `provider` | `engine`, `backend`, `tui`, `cli`, `agent`, sibling providers |
| `store-postgres` | `models`, `provider` | `storage`, `engine`, `backend`, `tui`, `cli`, `agent` |
| `cache-redis` | `models`, `provider` | `storage`, `engine`, `backend`, `tui`, `cli`, `agent` |
| `notify-*` | `models`, `provider` | `storage`, `engine`, `backend`, `tui`, `cli`, `agent` |
| `collector-kubernetes` | `models`, `provider` | `storage`, `engine`, `backend`, `tui`, `cli`, `agent`, sibling providers |
| `engine` | `models`, `provider` | `storage`, `backend`, `tui`, `cli`, `agent` |
| `backend` | `models`, `provider`, `engine` | `storage`, `tui`, `cli`, `agent` |
| `tui` | `models` | `provider`, `storage`, `engine`, `backend`, `cli`, `agent` |
| `agent` | `models` | `provider`, `storage`, `engine`, `backend`, `tui`, `cli` |
| `cli` | `models` | everything else |
| `monitra` (bin) | all | — |

**What changed in v0.2 and why it is an improvement:** `engine`, `backend`, and `tui` previously depended on `storage` directly. They now depend on the `provider` traits and receive an `Arc<dyn Store>` chosen by `main.rs`. `storage` becomes a leaf implementation crate that *nothing* imports except the binary. This is strictly stronger P4: the layers can no longer reach a concrete database even accidentally.

**What changed in v0.3 (ADR-008/ADR-009) and why it is an improvement:** `tui` drops even its `provider` dependency — it no longer reads storage in any mode, local or remote, only ever speaking the wire protocol over HTTP/WS (§3.2 no longer needs a "local vs remote" distinction inside `tui` at all; see §11.4). `agent` and `collector-kubernetes` enter the graph as new leaves at the same strictness as every existing one: `agent` mirrors `cli`'s position (models only, wired by `main.rs`), `collector-kubernetes` mirrors `store-postgres`/`cache-redis` (a `provider`-category implementation nothing else imports). `provider` itself gains a fourth category, `Collector`, alongside `Store`/`Cache`/`Notifier` (§4.1).

**Why `cli` depends only on `models`:** the CLI crate defines argument structure, not behaviour. Command *execution* is wired in `main.rs`, which has access to everything. This keeps `cli` trivially testable and prevents it becoming a god-crate.

**Why `agent` depends only on `models`:** same reasoning as `cli` — it is wired by `main.rs` (`monitra agent run`), needs the domain vocabulary to shape what it pushes, and must not be able to reach a concrete store, cache, or notifier directly. It talks to `backend`'s ingest endpoint over HTTP, never in-process.

**Why `tui` does not depend on `backend`, `provider`, or `storage`:** the TUI is a pure API client in every mode. When no daemon is already running, `main.rs` boots an embedded backend (engine + storage + API layer) on a loopback address and points the TUI's HTTP/WS client at it — one client implementation, not a local-storage path and a separate remote-HTTP path. A future `monitra tui --remote https://host` works without any change to `tui` itself, because it never knew the difference.

### 3.3 Runtime data flow

The steady-state loop once `monitra start` is running:

```
  ┌─ scheduler tick ─────────────────────────────────────────┐
  │                                                          │
  │  1. engine selects monitors whose next_check_at <= now    │
  │  2. acquires concurrency permit (semaphore)              │
  │  3. spawns probe task ──► HTTP / TCP / ICMP target       │
  │  4. probe returns CheckResult                            │
  │       │                                                  │
  │       ├──► storage: batched INSERT into check_results    │
  │       ├──► storage: UPDATE monitor status if changed     │
  │       └──► broadcast channel ──► WebSocket subscribers   │
  │                                        │                 │
  │  5. reschedule monitor: next_check_at = now + interval   │
  └────────────────────────────────────────┼─────────────────┘
                                           ▼
                              ┌────────────────────────┐
                              │ TUI / web dashboard    │
                              │ live status update     │
                              └────────────────────────┘
```

Key property: **the probe path and the read path are decoupled.** Dashboards never block probing. If every dashboard disconnects, monitoring is unaffected. If the database is momentarily slow, probes still execute and results queue in memory (bounded — see §7.3).

### 3.4 Process modes

One binary, several operating modes selected by sub-command:

| Command | Runs | Purpose |
|---|---|---|
| `monitra start` | engine + backend + storage | The daemon. Long-running. |
| `monitra tui` | tui + storage *(or HTTP client)* | Dashboard. Attaches to a running daemon or reads local DB. |
| `monitra monitor …` | storage *(or HTTP client)* | One-shot config management. Exits immediately. |
| `monitra version` | — | Build info. |

`monitor` sub-commands are deliberately one-shot: they should work whether or not the daemon is running, so bootstrapping a fresh install never requires starting a server first.

---

## 4. Crate Responsibilities

Each crate has an explicit contract. "Does not own" is as important as "owns."

### `models` — shared domain vocabulary

**Owns:** `Monitor`, `MonitorKind`, `MonitorStatus`, `CheckResult`, `MonitorId`, and their `Serialize`/`Deserialize` impls.

**Does not own:** persistence logic, validation that requires I/O, HTTP representations, business rules.

**Contract:** contains only plain data types and pure functions over them. Zero internal dependencies — this is what makes the graph acyclic. If two crates need to agree on a type, it goes here.

**Why a separate crate rather than a module:** it is depended on by everything. As a module inside a larger crate, any change would trigger recompilation of that whole crate. Isolated, it recompiles in milliseconds.

### `provider` — pluggable service contracts

**Owns:** the `Store`, `Cache`, `Notifier`, and `Collector` (added by ADR-008) traits; the provider registry (URL scheme → constructor); config parsing and resolution; the default-selection and availability rules of §4.1.

**Does not own:** any concrete implementation. `provider` knows that `postgres://` is a `Store` scheme; it does not know how to speak the Postgres wire protocol, and it does not know how to speak the Kubernetes API.

**Contract:** depends only on `models`. Every provider implementation crate depends on `provider`; `provider` depends on none of them. Registration happens in `main.rs`, gated by cargo features — this is what keeps the dependency arrow pointing the right way while still allowing a build to omit a provider entirely.

#### 4.1 Availability policy (ADR-007, extended by ADR-008)

"If the configured service is unavailable, fall back to the default" is **not** applied uniformly, because the consequences differ by category:

| Category | Default (always compiled in) | If configured service is unreachable |
|---|---|---|
| `Store` | SQLite (`storage`) | **Fail fast.** Refuse to start, naming the service and the error. |
| `Cache` | in-process map | Degrade to the in-process default, log at WARN, expose in `/health`. |
| `Notifier` | webhook (`notify-webhook`), logs instead of POSTing when no target is configured | Queue with bounded retry and backoff, log loudly. Never blocks a probe. |
| `Collector` | *(none)* | Per-resource, not daemon-wide: mark the affected Monitor's status as unknown (same honesty as agent-silence, §5.2), log at WARN, keep polling on schedule. Never fail the whole daemon over one unreachable cluster. |

**Why `Store` is different:** silently falling back from Postgres to SQLite would write history into a second database. The dashboard would then report uptime computed from a partial record — a direct P1 violation, and worse than not starting, because the operator would not know it happened. Refusing to boot with a clear message is the honest failure.

**Notifier default corrected at Phase 3:** this table originally named a "log sink" as the embedded default, but no such crate was ever scaffolded (Appendix) — `notify-webhook`, already planned as always-compiled in the root `Cargo.toml`, was. Rather than add a second trivial default crate, `notify-webhook` fills both roles: with no target URL configured it logs instead of sending, and becomes a real webhook sink once one is attached via `service attach webhook://…`. The actual HTTP-sending logic remains Phase 7 work, alongside the rest of the notifier sinks — only this identity/wording correction landed at Phase 3.

`Cache` and `Notifier` carry no such hazard: a cache miss costs latency, and a queued notification is still delivered.

**Why `Collector` has no default:** unlike `Store`/`Cache`/`Notifier`, there is nothing to fall back *to* — a `Collector` only exists because an operator configured a specific external system (a Kubernetes cluster) to introspect. No configuration means no collector-backed monitors, not a degraded default. Its failure mode is also scoped differently: it is one unreachable *resource* among possibly many configured, not a foundational service the whole daemon needs to boot.

### `storage` — persistence (default `Store` provider)

**Owns:** the SQLite implementation of the `Store` trait — schema definition, migrations, connection lifecycle, all SQL, query methods returning `models` types, retention/pruning.

**Does not own:** deciding *when* to write, business rules about status transitions, caching policy.

**Contract:** exposes a `Database` handle with typed methods (`list_monitors()`, `insert_check_result()`, `prune_older_than()`). SQL never leaks outside this crate. Callers cannot construct raw queries.

**Design note (revised v0.2):** the backend-agnostic API is no longer aspirational — it *is* the `Store` trait in `provider`. `storage` is one implementation of it, distinguished only by being the one that is always compiled in and requires nothing external.

### `engine` — the monitoring core

**Owns:** the scheduler, probe execution for each `MonitorKind` (including `Collector`-based direct Kubernetes polling, ADR-008), timeout enforcement, retry/backoff policy, concurrency limiting, status-transition logic (flap damping, **and agent-liveness watchdog** — heartbeat timeout on an `Agent` transitions its dependent Monitors to "unknown, agent unreachable," never silently to "down," §5.2), result broadcast, and emission of `AlertEvent`s to attached `Notifier`s on status transition.

**Does not own:** persistence details, HTTP API shape, presentation.

**Contract:** started via `EngineHandle::start()`, shut down gracefully via `shutdown()`. Emits `CheckResult` on a broadcast channel and writes through `storage`. Both pulled results (its own probes) and pushed results (relayed from `backend`'s agent-ingest endpoint) flow through the same bounded writer path (§6.2) — engine does not distinguish their origin once they arrive. This is the crate where P1 (reliability) matters most — it is the component whose correctness the entire product rests on.

### `backend` — API surface

**Owns:** Axum router, HTTP handlers, request/response DTOs, WebSocket upgrade and event fan-out, the authenticated agent-ingest endpoint (ADR-008), human-facing API authentication (ADR-009 — mechanism TBD, §11), middleware (logging, CORS, error mapping), embedded static asset serving for the web dashboard *(Phase 10)*.

**Does not own:** monitoring logic, database schema, scheduling.

**Contract:** a thin translation layer. Handlers validate input, call into `storage` or `engine`, and map results to HTTP. Any handler containing business logic is a design smell that belongs in `engine`. **Mutation handlers and `cli`'s command execution in `main.rs` call the same internal service functions** (ADR-009) — a mutation is never implemented twice.

**Why DTOs are separate from `models`:** the wire format must be able to evolve independently of the internal domain model. Coupling them means an internal refactor becomes a breaking API change.

### `tui` — terminal dashboard

**Owns:** terminal setup/teardown (raw mode, alternate screen), event loop, widget composition, key bindings, view state, an HTTP/WS client against `backend`'s API.

**Does not own:** data fetching policy beyond its own refresh cadence, monitoring logic, *any* persistence access — it never reads `storage` directly, in any mode (ADR-009).

**Contract:** `run_tui()` takes over the terminal and **must restore it on every exit path**, including panics. A panic that leaves the user's terminal in raw mode is a serious bug — a panic hook that restores terminal state is mandatory. `tui` is handed a base URL and knows nothing else about where it points — whether that URL is a remote daemon or an embedded backend `main.rs` booted on loopback for a no-daemon local session (§11.4) is invisible to this crate by design.

### `agent` — remote collection and local host checks

**Owns:** the `monitra agent run` mode: local host checks (systemd unit status, disk space, process liveness — no black-box network equivalent exists for these, ADR-008), the Kubernetes-fallback push path for clusters the central instance cannot reach directly, the push loop itself (retry/backoff, a bounded local buffer for when the backend is unreachable), and registration/token handling for authenticating its pushes.

**Does not own:** deciding *whether* a pushed result changes a Monitor's status — that is `engine`'s job once the result lands via `backend`'s ingest endpoint. `agent` reports; it does not interpret.

**Contract:** depends on `models` only (§3.2) — it is wired by `main.rs` exactly like `cli`, and talks to `backend` over HTTP, never in-process. A `monitra agent` that cannot reach its backend keeps running its local checks and buffering (bounded — P1 §7.3), it does not crash or block on connectivity.

### `collector-kubernetes` — direct Kubernetes API polling (a `Collector` provider)

**Owns:** the `Collector` trait implementation that polls the Kubernetes API server directly (kubeconfig or in-cluster service account) for Deployment/StatefulSet/Service status and on-demand pod-level breakdown.

**Does not own:** deciding what to do when the cluster is unreachable (that's the per-category policy in §4.1, enforced by `provider`/`engine`), persistence of pod-level detail — pod status is fetched live, never stored as its own `Monitor` (§5.1).

**Contract:** depends on `models` and `provider` only, same as `store-postgres`/`cache-redis`/`notify-*` — nothing else may import it. Gated by a cargo feature like every other non-default provider (§8).

### `cli` — argument surface

**Owns:** the `clap` command tree, argument types, validation of argument *shape*.

**Does not own:** command execution, I/O, business logic.

**Contract:** parsing only. `Cli::parse()` produces a fully-typed command description; `main.rs` decides what to do with it. This makes the entire CLI surface testable without side effects.

---

## 5. Data Model

### 5.1 Entities

Four entities as of v0.3 (ADR-008/009 added two — deliberately the exception, not the start of a trend; §5.1's original two remain the core).

**`Monitor`** — configuration. Low write volume, low row count (hundreds to low thousands). Read constantly by the scheduler. As of ADR-008, a `Monitor` may also represent an orchestrator resource (a Kubernetes Deployment/StatefulSet/Service) rather than a bare network endpoint — its identity is the resource, not any one pod backing it; individual pod status is fetched live as breakdown detail on demand, never persisted as its own `Monitor` row (pod identity churns on every reschedule/scale/rollout, which would defeat §5.4's retention model).

| Field | Type | Notes |
|---|---|---|
| `id` | `u64` | Primary key |
| `name` | `String` | Human label, shown in dashboards |
| `target` | `String` | URL, host:port, IP, or orchestrator-resource reference, depending on `kind` |
| `kind` | `MonitorKind` | `Http` \| `Tcp` \| `Icmp` \| `K8sDeployment` \| `K8sStatefulSet` \| `K8sService` \| `HostAgentCheck` (ADR-008 — exact variant set finalized at Phase 4) |
| `interval_secs` | `u64` | Check frequency |
| `status` | `MonitorStatus` | `Pending` \| `Up` \| `Down` \| `Paused` |

**`CheckResult`** — observation. High write volume, unbounded growth without retention policy. This is the table that determines whether the storage design holds. Populated by both pull (engine's own probes, including `Collector`-based K8s polling) and push (agent-relayed) paths through the same writer task (§6.2) — this table does not distinguish origin.

| Field | Type | Notes |
|---|---|---|
| `monitor_id` | `u64` | FK → `Monitor` |
| `checked_at` | `u64` | Unix seconds |
| `success` | `bool` | Did the probe pass |
| `latency_ms` | `u64` | Round-trip time |
| `message` | `Option<String>` | Status code or error text |

**`Agent`** *(new, ADR-008)* — a registered remote collector (a `monitra agent run` instance watching a host or a Kubernetes cluster it pushes into). Low row count, low write volume (heartbeats, not check data).

| Field | Type | Notes |
|---|---|---|
| `id` | `u64` | Primary key |
| `name` | `String` | Human label |
| `last_heartbeat_at` | `u64` | Unix seconds; drives the liveness watchdog (§4, `engine`) |
| `scope` | `String` | What it watches — a host, or a Kubernetes cluster/namespace reference |

An agent's own liveness is tracked separately from any `Monitor`'s status, for the same reason `Pending` exists (§5.2): "the agent went silent" and "the target is down" are different failure signals and must never be collapsed into one.

**`AlertEvent`** *(new, ADR-009)* — a persisted record of an emitted alert, so alert history is queryable by `cli`/`tui`/web rather than existing only as whatever a `Notifier` sink did with it. A deliberate exception to this section's "resist adding entities" discipline — without it, an alert-history view is not buildable at all.

| Field | Type | Notes |
|---|---|---|
| `id` | `u64` | Primary key |
| `monitor_id` | `u64` | FK → `Monitor` |
| `transitioned_to` | `MonitorStatus` | The status the transition landed on |
| `occurred_at` | `u64` | Unix seconds |
| `sinks_attempted` | `String` | Which `Notifier`s were sent this event (serialized list) |
| `delivery_outcome` | `String` | Per-sink delivery result, for diagnosing a stuck queue (§7.2) |

### 5.2 The `Pending` status

`MonitorStatus::Pending` exists specifically to serve P1. A monitor that has been created but never checked is **not** "up" and is **not** "down" — it is unknown. Collapsing this into either value would make the dashboard lie during the window between monitor creation and first check.

The same reasoning applies to the daemon restarting: on startup, monitors retain their last known status but the dashboard *(Phase 7)* will visually distinguish "confirmed 12s ago" from "last known, staleness unknown."

**A third case, added by ADR-008:** a `Monitor` fed by an unreachable `Agent` or `Collector` is neither confirmed-up nor confirmed-down — it is exactly the same "last known, staleness unknown" case as a daemon restart, just triggered by the feed going silent instead of the daemon restarting. Whether this is modeled as a staleness/confidence flag alongside `status` (extending the existing daemon-restart mechanism) or as distinct `MonitorStatus` variants is left to Phase 6, when the agent-liveness watchdog is actually built — not decided here, to avoid locking in a schema shape before the mechanism is implemented. The constraint that **is** locked in: whichever shape is chosen, it must never let "agent unreachable" render as "target down" (P1, same rule as §11.3's ICMP case).

### 5.3 Status transitions

```
                    ┌──────────┐
     created ──────►│ Pending  │
                    └────┬─────┘
                         │ first check completes
              ┌──────────┴──────────┐
              ▼                     ▼
         ┌────────┐            ┌────────┐
    ┌───►│   Up   │◄──────────►│  Down  │◄───┐
    │    └────┬───┘  N consec.  └───┬────┘    │
    │         │      failures /     │         │
    │         │      successes      │         │
    │         └──────────┬──────────┘         │
    │                    │ user pauses        │
    │               ┌────▼─────┐              │
    └───────────────┤  Paused  ├──────────────┘
       user resumes └──────────┘
       (→ Pending)
```

**Flap damping (deferred, Phase 6 — corrected; §10's roadmap table always placed this under Monitoring engine, this cross-reference was wrong):** transitions between `Up` and `Down` require *N* consecutive results in the new state, not a single one. A single dropped packet should not page anyone. Default N=2 for down-transitions, N=1 for recovery — asymmetric because we want to be slow to alarm and fast to reassure.

Resuming from `Paused` returns to `Pending`, not to the pre-pause status, for the same honesty reason as §5.2.

### 5.4 Retention

`check_results` grows at `monitors × (86400 / interval_secs)` rows/day. At 1,000 monitors on 30s intervals: **2.88 M rows/day**, roughly 150 MB/month uncompressed.

This is unsustainable without a policy. Plan *(Phase 4/5)*:

- **Raw results:** retained 7 days (configurable).
- **Hourly rollups:** min/max/avg latency, success count, total count. Retained 90 days.
- **Daily rollups:** same aggregates. Retained indefinitely (small).
- Pruning runs as a low-priority background task, not on the probe path.

Rollups are what make "uptime over the last 90 days" a cheap query instead of a table scan over 250 M rows.

---

## 6. Concurrency & Scaling Model

### 6.1 The hypothesis

**Claim:** a task-per-monitor model on Tokio sustains ~1,000 monitors at 30s intervals within 512 MB RAM and 2 vCPU.

This is stated as a hypothesis, not a fact. It is untested at Phase 1. Recording it explicitly means we can be proven wrong by evidence rather than discovering it in production.

**Reasoning behind the estimate:** a Tokio task idling on a timer costs roughly a few KB of state. 1,000 tasks is therefore single-digit MB — negligible. The real costs are (a) per-probe allocation, (b) TLS handshakes for HTTPS monitors, and (c) database write contention. (c) is the one I expect to bite first (see §11).

### 6.2 Structure

```
        ┌─────────────────────────────────────────┐
        │  Scheduler task                         │
        │  - owns monitor registry                │
        │  - tracks next_check_at per monitor     │
        │  - wakes on earliest deadline           │
        └────────────────┬────────────────────────┘
                         │ dispatch due monitors
                         ▼
        ┌─────────────────────────────────────────┐
        │  Semaphore (max_concurrent_probes)      │
        │  bounds in-flight probes, NOT monitors  │
        └────────────────┬────────────────────────┘
                         │ permit acquired
          ┌──────────────┼──────────────┐
          ▼              ▼              ▼
     ┌────────┐     ┌────────┐     ┌────────┐
     │ probe  │     │ probe  │     │ probe  │   spawned, short-lived
     │  task  │     │  task  │     │  task  │   hard timeout each
     └───┬────┘     └───┬────┘     └───┬────┘
         └──────────────┼──────────────┘
                        ▼
        ┌─────────────────────────────────────────┐
        │  Result channel (bounded mpsc)          │
        └────────────────┬────────────────────────┘
                         ├──► writer task: batched DB inserts
                         └──► broadcast: live subscribers
```

**Added by ADR-008 — a second arrival path feeds the same result channel:** `backend`'s authenticated agent-ingest endpoint hands pushed results into the identical bounded `mpsc` shown above, rather than a separate queue. Two producers (scheduler-dispatched probes, agent pushes), one bounded channel, one writer task — the batching and overflow-drop-and-log policy (§7.3) apply uniformly regardless of which path a result came from.

**Why a semaphore on probes rather than a fixed worker pool:** monitor count and concurrent-probe count are different quantities. 5,000 monitors on 5-minute intervals produce far less instantaneous load than 500 monitors on 10-second intervals. Bounding the thing that actually consumes resources (in-flight network operations, file descriptors) rather than the thing that is merely configured is the correct control point.

**Why a dedicated writer task:** SQLite permits one writer. Funnelling all writes through a single task that batches them turns thousands of individual transactions into a handful of batched ones. This is the single most important performance decision in the design.

### 6.3 Timing guarantees

Monitra promises **"checked at least every `interval_secs`, best effort"** — not exact periodicity. Under load, checks may drift later. What we guarantee:

1. A check never silently doesn't happen. If the scheduler falls behind, it logs and records the gap.
2. Drift does not accumulate unboundedly — scheduling is anchored to absolute deadlines, not `sleep(interval)` after completion, so a slow probe doesn't permanently shift the schedule.
3. Every probe has a hard timeout. A hung connection to one target cannot stall the scheduler.

Point 3 is why probes are spawned rather than awaited inline.

### 6.4 Falsification test

The hypothesis in §6.1 is tested, not assumed. Planned at Phase 6 (corrected — same pre-existing cross-reference error as §5.3's flap-damping note; the roadmap table always had benchmarks under Monitoring engine):

```
benches/scale.rs
  - spawn N local mock HTTP targets with configurable latency
  - register N monitors at interval I
  - run for 10 minutes
  - assert: p99 schedule drift < 2s
  - assert: RSS < 512 MB
  - assert: zero missed checks
  - report: CPU%, DB write latency p50/p99
```

Run at N = 100, 500, 1000, 2500, 5000. The point where any assertion fails is the documented, honest scaling limit — and it goes in the README as a number, not a marketing adjective.

---

## 7. Reliability & Failure Handling

P1 says reliability beats features. Concretely:

### 7.1 Error strategy

- **Per-crate error enums** via `thiserror`. Each crate defines its own error type describing failures *in its own vocabulary* (`StorageError::MigrationFailed`, `ProbeError::Timeout`).
- **`anyhow` at boundaries** — `main.rs` and command handlers use `anyhow::Result` with `.context()` to build a readable chain.
- **No `unwrap()` / `expect()`** outside tests and provably-infallible cases. A panic in the monitoring engine takes down monitoring for everything.
- **Errors name the component.** `"storage: failed to open monitra.db: permission denied"` beats `"error: permission denied"` at 3 a.m.

### 7.2 Failure taxonomy

| Failure | Blast radius | Handling |
|---|---|---|
| Single probe times out | One check | Record as failed with `message`; apply flap damping before status change |
| Target DNS fails | One monitor | Same as above — a resolution failure *is* a legitimate down signal |
| Probe task panics | One check | Caught via `JoinHandle` error; recorded as an internal error, distinct from target-down |
| DB write fails transiently | Buffered results | Retry with backoff; if buffer fills, drop oldest and log loudly (never block probes) |
| DB corrupt / unopenable | Whole daemon | Fail fast at startup with a clear message. Refusing to start beats pretending to monitor. |
| Scheduler falls behind | Timing accuracy | Log drift, expose in health endpoint; checks still run, just late |
| WebSocket client stalls | That client | Bounded per-client buffer; slow clients are dropped, never back-pressure the engine |
| Unclean shutdown (SIGKILL) | In-flight results | SQLite WAL guarantees no corruption; at most the last unflushed batch is lost |

### 7.3 Bounded everything

Every queue, buffer, and channel has an explicit bound. An unbounded channel between a fast producer (probes) and a slow consumer (database) is an OOM waiting to happen. When a bound is hit, the policy is **drop and log loudly** — degraded observation beats a dead process, and a silent drop violates P1.

### 7.4 Graceful shutdown

On SIGINT/SIGTERM:
1. Stop scheduling new probes.
2. Await in-flight probes with a deadline (default 10s).
3. Flush the result buffer to disk.
4. Close WebSocket connections with a proper close frame.
5. Checkpoint and close the database.

If step 2 exceeds its deadline, proceed anyway — a shutdown that hangs forever is worse than losing a few in-flight results.

---

## 8. Single-Binary Strategy

P2 is load-bearing. Here is how each potential external dependency is eliminated:

| Dependency | Normally requires | Monitra's approach |
|---|---|---|
| Database | Postgres/MySQL server | `rusqlite` with `bundled` — SQLite compiled into the binary from C source |
| TLS | System OpenSSL | `rustls` — pure Rust, no system library linkage |
| Web assets | Nginx / static file server | `rust-embed` — HTML/JS/CSS embedded at compile time *(Phase 10)* |
| Migrations | Separate migration tool | Embedded SQL, applied at startup by `storage` |
| Config | Config file management | Sensible defaults; CLI flags and env vars override; file optional |

**Fully static builds:** targeting `x86_64-unknown-linux-musl` produces a binary with no libc dependency, runnable on any Linux including `scratch` containers and Alpine.

**Size budget:** target under 25 MB stripped. Managed via `opt-level = "z"` consideration, `lto = true`, `codegen-units = 1`, `strip = true`, and `panic = "abort"` in the release profile — with the caveat that `panic = "abort"` interacts with how we catch probe-task panics (§7.2), so this needs verification before adoption.

**Feature gating (ADR-007).** Pluggable providers threaten the size budget: Postgres, Redis, and SMTP clients are not free. They are therefore behind cargo features and **off by default**.

| Build | Providers | Budget |
|---|---|---|
| `cargo build --release` (default) | SQLite store, in-process cache, log + webhook notify | **< 25 MB** — the number quoted in §1.5 and the README |
| `--features postgres,redis,slack,smtp` | all of the above | unbudgeted; documented as-measured |

The README quotes the default build. Quoting the smallest possible build while shipping the fattest would be the kind of marketing adjective §6.4 rejects.

**Cost accepted:** the bundled SQLite means first compile takes ~60s longer. This is a one-time developer cost traded for a permanent operator benefit. Correct trade.

---

## 9. Architecture Decision Records

### ADR-001 — Cargo workspace with six member crates

**Status:** Accepted — **revised by ADR-007** (crate count is no longer six)

**Context:** Need a structure that supports incremental development across nine phases without becoming a monolith.

**Decision:** Workspace with `models`, `storage`, `engine`, `backend`, `tui`, `cli` as libraries plus a thin root binary.

**Alternatives:**
- *Single crate with modules* — simplest, but every change recompiles everything, and module boundaries are not enforced by the compiler. Rejected: boundaries that are only conventions get violated.
- *Separate repositories* — maximum isolation but painful version coordination for a single-binary product. Rejected as overkill.

**Consequences:** ✅ Enforced boundaries, parallel + incremental compilation, independently testable layers. ❌ More `Cargo.toml` files; shared dependency versions must be centralised (done via `[workspace.dependencies]`).

---

### ADR-002 — SQLite first, Postgres later

**Status:** **Superseded by ADR-007.** Retained because the reasoning about *why* SQLite is the correct default survives the supersession intact; only "Postgres later" is reversed.

**Context:** Need persistence that does not violate P2.

**Decision:** SQLite via bundled `rusqlite`, with `storage` exposing a backend-agnostic API.

**Alternatives:**
- *Postgres from day one* — better concurrent writes, but requires a server. Directly violates P2. Rejected.
- *Embedded KV (sled, redb)* — no server, but we lose SQL aggregation which the rollup queries (§5.4) depend on. Rejected.
- *In-memory only* — no history. Defeats the purpose.

**Consequences:** ✅ Zero-dependency deployment, real SQL, WAL gives crash safety. ❌ Single writer — mitigated by the batched writer task (§6.2), but this is the design's most likely breaking point (§11.1).

---

### ADR-003 — Tokio task-per-monitor scheduling

**Status:** Accepted, unvalidated

**Context:** Need to schedule thousands of independent periodic checks.

**Decision:** Central scheduler dispatching to spawned probe tasks, bounded by a semaphore.

**Alternatives:**
- *Fixed thread pool + blocking I/O* — simpler mental model, but thousands of threads is untenable. Rejected.
- *Single-threaded event loop, no spawn* — a slow probe stalls everything. Violates §6.3.3. Rejected.
- *Timer wheel* — likely better at very high monitor counts, but more complex. Deferred until §6.4 benchmarks justify it (P5).

**Consequences:** ✅ Cheap concurrency, natural timeout handling, isolation between probes. ❌ Unvalidated at the top of the target range. Explicitly to be tested, not assumed.

---

### ADR-004 — TUI as primary interface, web as optional

**Status:** Accepted

**Context:** Must decide which interface receives primary design investment.

**Decision:** Terminal dashboard is primary; web dashboard is an optional embedded convenience consuming the same public API.

**Alternatives:**
- *Web-first* — broader appeal, but pulls toward a heavier product and weakens the SSH-native workflow that is the core thesis. Rejected.
- *TUI only* — cleanest, but the web view is genuinely useful for sharing status with non-terminal colleagues. Rejected as unnecessarily austere.

**Consequences:** ✅ Excellent remote/SSH experience, no browser required, reinforces P3. ❌ Two interfaces to maintain; the web dashboard must never gain capabilities the CLI lacks.

---

### ADR-005 — `models` has zero internal dependencies

**Status:** Accepted

**Context:** Dependency cycles in Rust workspaces are a hard compile error, and shared types are the usual cause.

**Decision:** `models` contains only plain data types and depends on nothing internal. Every other crate may depend on it.

**Consequences:** ✅ Acyclic graph guaranteed structurally; `models` recompiles instantly. ❌ Some logic that "feels like" it belongs on a domain type must live elsewhere — accepted deliberately, since anything requiring I/O belongs in a layer that owns I/O.

---

### ADR-006 — Build persistence before the API

**Status:** Accepted (Phase 0)

**Context:** The v0.1 roadmap ordered Backend API at Phase 3 and Database at Phase 4. But §3.2 has `backend` depending on the store. Building the API first means writing handlers against a store that does not exist.

**Decision:** Persistence lands before the API. The v0.2 roadmap orders provider layer → storage → backend.

**Alternatives:**
- *Keep the original order, stub the handlers* — Phase 3 would ship handlers wired to in-memory stubs, then rewrite every one of them at Phase 4. Worse, the Phase 3 test gate could only assert routing and status codes. A gate that cannot fail is not a gate. Rejected.

**Consequences:** ✅ Every handler is written against a real store and gets a behavioural integration test the day it is written. ❌ Nothing runs end-to-end until later in the sequence; the first externally visible milestone moves right.

---

### ADR-007 — Pluggable service providers with embedded defaults

**Status:** Accepted (Phase 0). Supersedes ADR-002; revises ADR-001 and §1.3.

**Context:** Operators want to attach their own infrastructure — an existing Postgres, an existing Redis, their own notification channels — rather than accept whatever the tool embeds. But P2 forbids *requiring* any of it.

**Decision:** Three provider categories (`Store`, `Cache`, `Notifier`) defined as traits in a new `provider` crate. Each has an embedded default requiring nothing external. Users attach alternatives by URL in config, produced by an interactive `monitra setup` wizard or by flags. Providers are registered at compile time and gated by cargo features. Availability failures are handled per-category (§4.1).

**Alternatives:**
- *Dynamic `.so` plugins* — maximum extensibility, but the binary stops being self-contained. Directly violates P2, which §2 declares load-bearing. Rejected.
- *Runtime hot attach/detach of any provider* — attractive for notifiers, but swapping a live `Store` means draining in-flight writes and reconciling migration state mid-flight. Deferred, not rejected; see §11.8.
- *Uniform fallback to defaults for every category* — simpler rule, but silently splits history across two stores when the configured store blips. Rejected on P1 grounds (§4.1).
- *Keep providers out of v1 entirely* — smallest scope, but retrofitting an abstraction after `engine` and `backend` have hard-wired SQLite is markedly more expensive than building it now.

**Consequences:** ✅ Operators attach their own infrastructure; the zero-config path is untouched; `engine`/`backend`/`tui` can no longer reach a concrete database. ❌ Crate count rises from 6 to 10+; the feature matrix must be built and tested in CI, or feature combinations rot; `monitra setup` is new surface area that must stay strictly optional (§11.7).

---

### ADR-008 — Distributed agent architecture for target introspection

**Status:** Accepted (pre-Phase 1). Revises §1.3 and ADR-005's crate list; extends §4.1.

**Context:** Monitra's v1 scope was a single-node, black-box prober — reachable-over-the-network targets only. That leaves a real gap: it cannot tell whether a Kubernetes Deployment is actually healthy (as opposed to its Service's ClusterIP merely being reachable), and it cannot check host-local facts — a systemd unit's state, disk space, process liveness — that have no network-visible signal at all. The ask was to "track all deployments no matter the architecture," which black-box probing alone cannot satisfy.

**Decision:** Two new mechanisms, chosen deliberately over stronger and weaker alternatives:

1. Kubernetes: poll the cluster's API server directly by default (new `Collector` provider category in `provider`, implemented by `collector-kubernetes`), with a lightweight agent push path as the fallback for clusters the central instance cannot reach directly.
2. Bare-machine facts with no network signal (systemd unit status, disk space, process liveness): agent-only. A new `agent` crate runs `monitra agent run` — the same binary, a different mode, not a separate artifact — performing local checks and pushing results to `backend`'s authenticated ingest endpoint, with its own retry/backoff and a bounded local buffer when the backend is unreachable.

The persisted `Monitor` for a Kubernetes target is the orchestrator resource (Deployment/StatefulSet/Service) — a stable identity — not any individual pod; pod-level status is fetched live as breakdown detail, never persisted (§5.1). A new `Agent` entity (§5.1) tracks each registered collector's own liveness, separately from any `Monitor`'s status — `engine` owns the state-transition logic for what an unreachable agent means for the monitors it feeds (§5.2), on the same honesty grounds as `Pending` and the ICMP-permission case (§11.3): a silent agent is never rendered as a down target.

**Alternatives:**
- *Kubernetes via agent-only, no direct polling* — simpler (one mechanism, not two), but needlessly requires installing something inside every cluster even when the API server is directly reachable. Rejected as the sole mechanism; kept as the fallback.
- *Bare-machine checks via SSH-exec, no agent to install* — fits the existing "scp the binary, ssh in" thesis with zero footprint on the target, and was the initial recommendation. Rejected in favor of an agent: an agent supports richer local checks and doesn't require inbound SSH access to be configured on every host.
- *Individual Kubernetes pods as persisted `Monitor` rows* — finer-grained history, but pod identity churns on every reschedule, scale event, and rolling update; this would defeat §5.4's retention model by turning `check_results` growth into an unbounded-identity problem, not just an unbounded-time one. Rejected.
- *Keep Monitra a pure black-box prober, leave orchestrator/host introspection to `kubectl` and existing host tooling* — smallest scope, most consistent with the original v1 thesis. Rejected because it does not answer what was actually asked: whether a deployment is healthy, not merely reachable.

**Consequences:** ✅ Monitra can answer "is this Deployment actually healthy" and "is this host out of disk," not just "is this port open"; the mechanism (agent as a mode of the same binary) preserves P2. ❌ `provider` gains a fourth category with no honest default (§4.1); `backend` gains a new authenticated ingest surface that is real attack surface; `engine` gains a second, structurally different kind of "we don't know" state to represent correctly; the crate count grows again (two more leaves); §1.3's "not a distributed multi-region prober" line no longer holds as originally written and had to be narrowed rather than simply deleted (see the revised §1.3 text).

---

### ADR-009 — Backend-first client architecture; web and TUI as symmetric API clients

**Status:** Accepted (pre-Phase 1). Supersedes ADR-004; adds `AlertEvent` to §5.1; resolves §11.4.

**Context:** ADR-004 made the web dashboard permanently subordinate to the TUI in capability, and `tui` itself had two different data-access implementations (local: read `storage` directly; remote: speak HTTP) with the abstraction reconciling them left undesigned (§11.4). Once the decision was made that web should no longer be capability-subordinate to TUI, and that the backend should be built out to support every capability identified for the dashboards (both the "must" and "could" feature sets worked out for the TUI — fleet/agent/K8s views, health, alert history, and more) before either frontend catches up, the TUI's dual-implementation problem became actively worse, not better, if left as-is: a third (web) implementation would just add a second axis of duplication.

**Decision:**

1. `backend` becomes the sole source of truth. `tui` and the web dashboard are both pure HTTP/WS API clients, in *every* mode — `tui` no longer reads `storage` directly even when running locally with no daemon present (§3.2 tightens accordingly: `tui` depends on `models` only, not `provider`).
2. When `monitra tui` or `monitra web` runs with no daemon already up, `main.rs` boots an embedded backend (engine + storage + API layer) in-process on a loopback TCP address with an ephemeral port, and points the client at it exactly as it would point at a remote daemon. One client implementation exists, not two — this resolves §11.4 by construction rather than by designing an abstraction to paper over two implementations.
3. ADR-004 is **superseded**, not merely revised: once TUI and web are structurally symmetric clients of the same API, "web must never exceed TUI" is not a rule that needs enforcing, it is a rule that no longer applies. This does not touch the separate, still-standing hard rule (P3, CLAUDE.md) that every capability must be reachable from the CLI — that constraint is independent of ADR-004 and is unaffected by its supersession.
4. A new `AlertEvent` entity (§5.1) persists emitted alerts as queryable history — a deliberate, named exception to §5.1's general discipline against adding entities, made because an alert-history view is not buildable at all without it.
5. The backend's human-facing HTTP/WS surface requires authentication from the start, separate from agent-push tokens (§11.10) — because web is now a real network-facing equal client rather than a TUI-parity convenience assumed to sit behind an SSH tunnel. The exact mechanism (API key vs. session login) is left open (§11.11); only the requirement is decided here.
6. CLI command execution (in `main.rs`) and backend HTTP handlers for the same mutation (add/pause/resume/remove a monitor, etc.) call the same internal service functions — a mutation's logic exists in exactly one place, per the existing `backend` contract's principle that business logic belongs in `engine`, not in a handler.

**Alternatives:**
- *Keep `tui`'s dual local/remote implementation and add web as a third* — rejected: this compounds the §11.4 problem instead of resolving it.
- *Defer authentication to a later phase, since this is currently a solo/small-team project* — rejected once web became a real equal-capability client: an unauthenticated network-facing API contradicts "build the backend for every capability" the moment the web dashboard is actually deployed somewhere reachable, and retrofitting auth onto an already-built API surface is more expensive than building it in from the first handler.
- *Keep ADR-004's ordering but simply raise web's ceiling to match TUI* — rejected: the ceiling itself was the wrong model once both are pure API clients; there is nothing left for a ceiling to constrain.

**Consequences:** ✅ §11.4 is resolved rather than merely scheduled; TUI and web share one client mental model and one set of integration tests against the API; the backend becomes buildable and testable well ahead of either frontend, matching ADR-006's precedent of not writing a layer against something that doesn't exist yet. ❌ `tui`'s DAG position gets *stricter*, not looser, which means any future "quick local read" temptation must go through the embedded-backend path rather than a shortcut; the backend now carries a real authentication surface (and its failure modes) that did not exist before; local-mode startup now always pays the cost of booting a full embedded backend, even for a single `monitor add` glance.

---

## 10. Roadmap

Revised in v0.2 by ADR-006 (persistence before API) and ADR-007 (provider layer); resequenced in v0.3 by ADR-008 (distributed agents) and ADR-009 (backend-first clients). Phases 0–7 keep their v0.2 numbering and gates unchanged in substance — each just gained scope from the two new ADRs, listed below. Phase 8 is new; the old Phase 8/9/10 (TUI/Web/Bundling) shift to 9/10/11.

| Phase | Deliverable | Gate | Status |
|---|---|---|---|
| 0 | Scaffolding — `CLAUDE.md`, phase skills, dep-check, git | dep-check runs; DESIGN.md reflects ADR-006/007/008/009 | ✅ Complete |
| 1 | Project setup — workspace, **all crates including `agent` and `collector-kubernetes` stubbed from day one** | builds clean; clippy `-D warnings`; dep-DAG passes with the full crate set present; `monitra version` | ✅ Complete |
| 2 | CLI base — command tree, monitor CRUD, **agent management, K8s cluster attach**, `setup`, `service` | parse tests for every command form incl. new ones; `--help` snapshot; **no execution** | ✅ Complete |
| 3 | Provider layer — Store/Cache/Notifier/**Collector** traits, registry, config, `monitra setup` | fake providers exercise **all four** §4.1 policies; zero-config path still works | ✅ Complete |
| 4 | Storage — SQLite `Store` impl, schema, migrations, retention, **+ `Agent`, `AlertEvent`, orchestrator-resource `MonitorKind`s** | migrations on fresh DB cover all entities incl. new ones; round-trip; prune; WAL asserted on | ⬜ |
| 5 | Backend API — Axum router, REST handlers, health endpoint, **human-facing auth built in from the first handler** | integration tests on ephemeral port against a real store; **401 without credentials / 200 with**; health reports internal state | ⬜ |
| 6 | Monitoring engine — scheduler, probes, **Collector-based K8s direct-poll**, flap damping, **agent-liveness watchdog**, benchmarks | §6.4 falsification harness; flap tests; **agent-heartbeat-timeout test**; monotonic-clock test; hard-timeout test | ⬜ |
| 7 | Events — WebSocket fan-out + notifier sinks, **agent-ingest endpoint, `AlertEvent` emission on transition** | slow client dropped without back-pressuring engine; sink retry/backoff; **ingest queue bounded-drop-and-log test**; `AlertEvent` row created on every transition | ⬜ |
| 8 | **Agent binary** (new, ADR-008) — local host checks, push loop with retry/backoff and a bounded local buffer, registration/token handling, K8s-fallback push | local checks produce correct payloads standalone (no backend needed); push loop delivers to a real backend; survives the backend being unreachable without crashing or blocking local checks | ⬜ |
| 9 | TUI dashboard — Ratatui event loop, widgets, **pure API client only (§11.4 resolved by ADR-009)**, embedded-local-backend bootstrap | panic restores terminal (subprocess test); widget snapshots; local-embedded and remote modes exercise the same client code path | ⬜ |
| 10 | Web dashboard — React SPA, embedded via `rust-embed`, **consumes the identical API as TUI, no ADR-004 capability ceiling** | embedded server serves index; SPA exercises the same auth and full API surface TUI does | ⬜ |
| 11 | Bundling — feature matrix (**`collector-kubernetes` gated, `agent` mode always in the default binary**), static musl, size, release CI | default build size re-verified against the added surface (§11.12) rather than assumed at the original 25 MB figure; `ldd` static; feature combos build in CI; §11.6 resolved | ⬜ |

### Beyond v1 (not committed)

- ~~Alerting integrations (webhook, email, Slack)~~ — pulled into v1 as notifier providers (ADR-007), sinks only
- ~~Postgres backend for multi-instance deployments~~ — pulled into v1 as a store provider (ADR-007)
- ~~Kubernetes/host introspection via agents~~ — pulled into v1 by ADR-008
- Multi-region *latency* probing from multiple geographic vantage points (distinct from ADR-008's target introspection — see the revised §1.3 table)
- Alert routing, deduplication, and on-call schedules — explicitly out, see §1.3
- SSL certificate expiry monitoring
- Status pages
- Maintenance windows

Each of these is a plausible direction, none is a v1 constraint, and each must be checked against §1.3 before being accepted.

---

## 11. Open Questions & Known Risks

These are unresolved. Documenting them prevents a future reader from mistaking an unknown for a settled decision.

### 11.1 SQLite write contention *(highest risk)*

**The concern:** at 1,000 monitors × 30s intervals, that is ~33 writes/second sustained, with bursts when many monitors align. SQLite allows one writer. The batched writer task (§6.2) is the mitigation, but batch size and flush interval are untuned guesses.

**Why this is the top risk:** it is the interaction between the two headline requirements (single binary → SQLite; thousands of monitors → high write volume). If the design breaks anywhere, most likely here.

**Resolution path:** §6.4 benchmarks must report DB write latency p50/p99 specifically. If contention dominates, options in order of preference: (a) tune batch size and WAL checkpoint interval, (b) write rollups only and sample raw results, (c) reconsider ADR-002 for high-scale deployments.

### 11.2 Scheduler design at the top of the range

Task-per-monitor is unvalidated above ~1,000. A timer wheel may be necessary at 5,000+. Deliberately deferred under P5 — but flagged so that if benchmarks disappoint, the alternative is already identified rather than discovered under pressure.

### 11.3 ICMP requires elevated privileges

Raw sockets need root or `CAP_NET_RAW`. This conflicts with the frictionless-deployment thesis. Options: unprivileged ICMP via `SOCK_DGRAM` where the kernel allows it, document the capability requirement, or degrade ICMP monitors to a clear "unavailable — requires CAP_NET_RAW" state rather than failing silently. **Undecided.** P1 requires that whatever we choose, we never report an ICMP monitor as "down" when the real cause is a permissions failure on our side.

### 11.4 TUI remote mode boundary — **resolved by ADR-009**

Originally: §3.2 stated `tui` reads via `storage` locally or HTTP remotely, with the abstraction reconciling the two left undesigned. ADR-009 resolves this by construction rather than by designing that abstraction: `tui` never reads `storage` in any mode. When no daemon is running, `main.rs` boots an embedded backend on loopback and `tui` talks to it exactly as it would a remote daemon. One client implementation. Left here, struck rather than deleted, per the document's own convention of keeping resolved reasoning visible.

### 11.5 Clock changes and DST

Scheduling anchored to absolute deadlines (§6.3.2) must use a monotonic clock, while `checked_at` timestamps must use wall time. Mixing these produces either mass simultaneous checks or scrambled history when the system clock steps. Straightforward to get right, easy to get wrong silently.

### 11.6 `panic = "abort"` vs. probe panic recovery

§8 wants `panic = "abort"` for binary size; §7.2 wants to catch probe-task panics and continue. These are in tension. Must be resolved before Phase 11 (Bundling — the phase that actually turns on the release profile's `panic = "abort"`; a prior version of this note pointed at the old Phase 9, which was TUI, not Bundling — corrected during the v0.3 renumbering audit) — likely by ensuring probe code cannot panic in the first place rather than relying on catching it.

### 11.7 `monitra setup` must not become mandatory

ADR-007 introduces a config file and a wizard. §1.2 promises first monitor firing in under 60 seconds with nothing to install and no config archaeology. These pull in opposite directions the moment any code path assumes config exists.

**The rule:** `monitra monitor add …` on a machine with no config file, no wizard run, and no environment variables must work, using embedded defaults throughout. If that test ever needs relaxing, ADR-007 was a mistake.

**Resolved at Phase 3 — config file location and precedence:** `$XDG_CONFIG_HOME/monitra/config.toml` (falling back to `$HOME/.config/monitra/config.toml` when unset), overridden by `./monitra.toml`, overridden by `MONITRA_*` env vars, overridden by CLI flags. `provider::config::resolve` implements this precedence as a pure function over an already-gathered `ConfigSources`, so it's tested without touching real env vars or `$HOME`. `setup`/`service attach|detach`/`k8s attach|detach` write only to the XDG path — the project-local file and env/flags are read-only override layers, never written by a command.

**Test-location note:** the pristine-`$HOME` assertion this section originally scheduled as a Phase 3 `monitor add` end-to-end test instead landed as a resolver-level test (`provider::tests::pristine_environment_resolves_to_defaults_throughout`) — `monitor add` has nothing real to execute against yet, since `storage`'s SQLite `Store` impl doesn't land until Phase 4. The literal end-to-end version of this assertion belongs in Phase 4, once there's a real `Store` for it to add against.

### 11.8 Live attach/detach of providers — **resolved at Phase 2**

ADR-007 registers providers at compile time and resolves them at startup. `monitra service attach slack://…` taking effect on a running daemon is deferred. Notifiers could support it easily; stores cannot without solving in-flight write drain and mid-flight migration state. **Risk:** if the CLI surface designed at Phase 2 implies liveness that Phase 3 does not deliver, the command names will be wrong. Decide the wording at Phase 2, not Phase 7.

**Decision:** `monitra service attach <url>` / `service detach <name>` / `service list`, generic across the `Store`/`Cache`/`Notifier` categories by URL scheme — the wording this section already used, kept deliberately. It does not imply liveness: the command edits config, and every category except possibly `Notifier` requires a `monitra start` restart to take effect, which the command's own help text says explicitly rather than leaving it implied. Nothing about live attach/detach on a running daemon is built by this wording; it only avoids naming a command in a way Phase 3 would have to contradict.

`Collector`/Kubernetes attach did **not** ride `service` — it got its own `monitra k8s attach/list/detach` family instead, because a cluster's real configuration (kubeconfig path, context, namespace/label scope) doesn't fit a single provider URL the way `postgres://`/`redis://`/`slack://` do. This also reads correctly given `Collector` has no default and no fallback (§4.1) — it is a distinct enough category from the URL-scheme three that a shared verb across all four would have forced an awkward encoding for no benefit.

Parse-only shape for both landed in Phase 2 (`crates/cli/src/service.rs`, `crates/cli/src/k8s.rs`); actual attach/detach behavior is still Phase 3's to build.

### 11.9 Feature-combination rot

Ten crates behind cargo features means the number of buildable configurations grows fast, and combinations nobody builds stop compiling silently. CI must build at minimum: default, all-features, and each provider feature alone. Not hard — just easy to skip until it breaks a release. (ADR-008 adds another feature-gated crate, `collector-kubernetes` — same discipline applies to it.)

### 11.10 Agent transport and wire protocol

ADR-008 decided *that* agents push results to `backend`'s ingest endpoint and authenticate doing so, not the specifics: whether that's plain HTTP+JSON, gRPC, or something else; the exact token/registration flow; whether an agent can watch more than one host or cluster per process. **Undecided**, deliberately — these are Phase 7/8 implementation questions, not architecture, and answering them now would be guessing ahead of the code.

### 11.11 Human-facing API auth mechanism

ADR-009 requires the backend's HTTP/WS surface to be authenticated but does not choose how: a static API key, a session/login flow, something else. **Undecided.** Needs a decision before Phase 5, the same way §11.7 flags config precedence needing a decision before Phase 3 — don't let Phase 5's handlers get built against a guessed-at auth shape.

### 11.12 Kubernetes RBAC and kubeconfig handling

`collector-kubernetes` needs a kubeconfig or in-cluster service account, and the minimum RBAC surface it requires (read-only on Deployments/StatefulSets/Services/Pods, presumably) is not yet specified. **Undecided** until Phase 3/6. Also unresolved: whether Monitra ever needs write access to a cluster for anything (current answer, until reconsidered: no — it introspects, it does not act).

### 11.13 Size budget under the expanded surface

§1.5 and §8 both quote "<25 MB stripped" as a success criterion, set before ADR-008/009 added a `Collector` provider, an `agent` mode, an authenticated API surface, and an `AlertEvent` table. **Undecided whether the number still holds.** Phase 11's gate should measure the actual default build, not assume the original figure — if it no longer holds, that is a finding to record honestly (per §6.4's own "report the real number, not a marketing adjective" ethos), not a reason to quietly redefine "default build."

---

## Appendix — Repository Layout

```
monitra/
├── Cargo.toml                  # workspace manifest + root binary
├── src/
│   └── main.rs                 # entry point, command dispatch
├── docs/
│   └── DESIGN.md               # this document
└── crates/
    ├── models/                 # shared domain types (zero internal deps)
    │   └── src/{lib,monitor,check_result,agent,alert_event}.rs
    ├── provider/               # Store/Cache/Notifier/Collector traits, registry, config
    │   └── src/{lib,store,cache,notifier,collector,registry,config}.rs
    ├── storage/                # default Store: SQLite
    │   └── src/{lib,db}.rs
    ├── store-postgres/         # Store impl        [feature: postgres]
    ├── cache-redis/            # Cache impl        [feature: redis]
    ├── notify-webhook/         # Notifier impl     (default build)
    ├── notify-slack/           # Notifier impl     [feature: slack]
    ├── collector-kubernetes/   # Collector impl     [feature: kubernetes] (ADR-008)
    ├── engine/                 # async monitoring engine
    │   └── src/{lib,runner}.rs
    ├── backend/                # Axum HTTP + WebSocket API, agent ingest, auth
    │   └── src/{lib,server}.rs
    ├── cli/                    # clap argument definitions
    │   └── src/{lib,args}.rs
    ├── tui/                    # Ratatui terminal dashboard (pure API client, ADR-009)
    │   └── src/{lib,dashboard}.rs
    └── agent/                  # `monitra agent run` — local checks + push client (ADR-008)
        └── src/{lib,checks,push}.rs

# No separate crate for the web dashboard (Phase 10) — static SPA assets
# embedded via rust-embed and served by `backend`, consuming the same API
# `tui` does. See ADR-009.
```

---

*This document is revised at the end of each phase. When a decision recorded here is reversed, the ADR is superseded rather than deleted — the reasoning behind an abandoned choice is often more valuable than the choice itself.*
