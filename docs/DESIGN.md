# Monitra — Design Document

> **Status:** Living document
> **Version:** 0.3 (Phase 7 — Events complete; Phase 8 not started)
> **Last updated:** 2026-09-20

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
| ~~A multi-region latency prober~~ *(reversed by ADR-011)* | Probing the *same* target from many geographic vantage points to compare latency was a distinct design problem (cross-region aggregation) from watching a target's own internals, and out of scope through v0.3. ADR-011 brings it into v1, built on the existing `Agent` mechanism rather than a new regional-prober concept. | Phase 11 |

**On alerting — revised by ADR-007.** Monitra *does* emit alert events to attached notification sinks (§4 `monitra-provider`). It does not group, deduplicate, silence, escalate, or schedule them. The test for whether a proposed alerting feature is in scope: **if it needs to know who is on call, it belongs in Alertmanager or PagerDuty, not here.**

**On distributed monitoring — revised by ADR-008, extended by ADR-011.** The original table row here read "a distributed multi-region prober... not a v1 constraint." ADR-008 reversed that for target *introspection*: Monitra reaches into architectures it does not run on — Kubernetes clusters, other machines — via direct API polling and lightweight per-host/per-cluster agents that push results back. At that point this was still a single Monitra instance owning the view; agents extended its reach without becoming independent regional probers with their own aggregation problem. ADR-011 now reverses the remaining half — probing the *same* target from multiple vantage points for latency comparison — by reusing that same `Agent` mechanism as the vantage point rather than inventing a second one: multi-region coverage is ordinary Monitors sharing a target, each linked to a region-tagged Agent, aggregated read-side. Nothing here creates an independent regional-probing subsystem; it is the same single-instance-owns-the-view model ADR-008 established, extended one field further.

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

Each concern is a separate crate with an explicitly documented contract. The dependency graph is a DAG, enforced by `monitra-models` having zero internal dependencies. This keeps compile times manageable and makes each layer independently testable.

**Practical consequence:** `monitra-storage` never imports `monitra-engine`. `monitra-engine` never imports `monitra-backend`. Shared types live in `monitra-models` or nowhere.

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
        ┌──────────┬──────────┬──────────┴───────┬──────────┬──────────────┐
        │          │          │                   │          │              │
        ▼          ▼          ▼                   ▼          ▼              ▼
 monitra-cli monitra-backend monitra-tui   monitra-storage store-postgres cache-redis
        │          │          │                   │          │          notify-*
        │          ▼          │                   │          │              │
        │   monitra-engine    │                   │          │              │
        │          │          │                   │          │              │
        │          └──────────┴──────┬─────────────┴──────────┴──────────────┘
        │                            ▼
        │                    monitra-provider    (traits + registry + config)
        └────────────────────────────┴──────────────► monitra-models
```

Added by ADR-008/ADR-009 (v0.3) — two new leaf crates, same DAG discipline as everything else:

```
        main (monitra)
              │
              ├──► monitra-agent                  (monitra-models only — push client, local host checks)
              │
              └──► monitra-provider ──► collector-kubernetes   (monitra-models, monitra-provider — a Collector impl)
```

Added by ADR-011 (v0.4) — one new leaf crate, shared by two existing ones rather than wired fresh from `main`:

```
        monitra-engine ──┐
                          ├──► monitra-probe   (monitra-models only — HTTP/TCP/ICMP probe execution)
        monitra-agent ────┘
```

`monitra-probe` holds exactly what `crates/engine/src/probe/{http,tcp,icmp}.rs` held before ADR-011 — nothing new, just relocated so `monitra-agent` (which can never depend on `monitra-engine`, ADR-008) can run the same probes `monitra-engine` runs, for agent-executed regional checks (Phase 11).

Only the root binary knows which concrete providers exist. Every other consumer holds a trait object.

| Crate | May depend on | Must never depend on |
|---|---|---|
| `monitra-models` | *(nothing internal)* | everything |
| `monitra-provider` | `monitra-models` | every other internal crate |
| `monitra-storage` | `monitra-models`, `monitra-provider` | `monitra-engine`, `monitra-backend`, `monitra-tui`, `monitra-cli`, `monitra-agent`, sibling providers |
| `store-postgres` | `monitra-models`, `monitra-provider` | `monitra-storage`, `monitra-engine`, `monitra-backend`, `monitra-tui`, `monitra-cli`, `monitra-agent` |
| `cache-redis` | `monitra-models`, `monitra-provider` | `monitra-storage`, `monitra-engine`, `monitra-backend`, `monitra-tui`, `monitra-cli`, `monitra-agent` |
| `notify-*` | `monitra-models`, `monitra-provider` | `monitra-storage`, `monitra-engine`, `monitra-backend`, `monitra-tui`, `monitra-cli`, `monitra-agent` |
| `collector-kubernetes` | `monitra-models`, `monitra-provider` | `monitra-storage`, `monitra-engine`, `monitra-backend`, `monitra-tui`, `monitra-cli`, `monitra-agent`, sibling providers |
| `monitra-probe` *(new, ADR-011)* | `monitra-models` | every other internal crate |
| `monitra-engine` | `monitra-models`, `monitra-provider`, `monitra-probe` | `monitra-storage`, `monitra-backend`, `monitra-tui`, `monitra-cli`, `monitra-agent` |
| `monitra-backend` | `monitra-models`, `monitra-provider`, `monitra-engine` | `monitra-storage`, `monitra-tui`, `monitra-cli`, `monitra-agent` |
| `monitra-tui` | `monitra-models` | `monitra-provider`, `monitra-storage`, `monitra-engine`, `monitra-backend`, `monitra-cli`, `monitra-agent` |
| `monitra-agent` | `monitra-models`, `monitra-probe` | `monitra-provider`, `monitra-storage`, `monitra-engine`, `monitra-backend`, `monitra-tui`, `monitra-cli` |
| `monitra-cli` | `monitra-models` | everything else |
| `monitra` (bin) | all | — |

**What changed in v0.2 and why it is an improvement:** `monitra-engine`, `monitra-backend`, and `monitra-tui` previously depended on `monitra-storage` directly. They now depend on the `monitra-provider` traits and receive an `Arc<dyn Store>` chosen by `main.rs`. `monitra-storage` becomes a leaf implementation crate that *nothing* imports except the binary. This is strictly stronger P4: the layers can no longer reach a concrete database even accidentally.

**What changed in v0.3 (ADR-008/ADR-009) and why it is an improvement:** `monitra-tui` drops even its `monitra-provider` dependency — it no longer reads storage in any mode, local or remote, only ever speaking the wire protocol over HTTP/WS (§3.2 no longer needs a "local vs remote" distinction inside `monitra-tui` at all; see §11.4). `monitra-agent` and `collector-kubernetes` enter the graph as new leaves at the same strictness as every existing one: `monitra-agent` mirrors `monitra-cli`'s position (models only, wired by `main.rs`), `collector-kubernetes` mirrors `store-postgres`/`cache-redis` (a `monitra-provider`-category implementation nothing else imports). `monitra-provider` itself gains a fourth category, `Collector`, alongside `Store`/`Cache`/`Notifier` (§4.1).

**What changed in v0.4 (ADR-011) and why it is an improvement:** probe execution (HTTP/TCP/ICMP) moves out of `monitra-engine` and into a new leaf, `monitra-probe`, depending on `monitra-models` only. `monitra-engine` keeps using it exactly as before; `monitra-agent` gains it too, which is the entire point — it is the only way `monitra-agent` can run real network probes without violating its ADR-008 DAG position (`monitra-models` only, never `monitra-engine`/`monitra-provider`). Two crates sharing one leaf for behavior, not a trait object for a swappable implementation, is a new shape in this graph — deliberately different from the provider pattern, because there is nothing here an operator attaches or swaps; it is the same code running in two processes.

**Why `monitra-cli` depends only on `monitra-models`:** the CLI crate defines argument structure, not behaviour. Command *execution* is wired in `main.rs`, which has access to everything. This keeps `monitra-cli` trivially testable and prevents it becoming a god-crate.

**Why `monitra-agent` depends only on `monitra-models`:** same reasoning as `monitra-cli` — it is wired by `main.rs` (`monitra agent run`), needs the domain vocabulary to shape what it pushes, and must not be able to reach a concrete store, cache, or notifier directly. It talks to `monitra-backend`'s ingest endpoint over HTTP, never in-process.

**Why `monitra-tui` does not depend on `monitra-backend`, `monitra-provider`, or `monitra-storage`:** the TUI is a pure API client in every mode. When no daemon is already running, `main.rs` boots an embedded backend (engine + storage + API layer) on a loopback address and points the TUI's HTTP/WS client at it — one client implementation, not a local-storage path and a separate remote-HTTP path. A future `monitra tui --remote https://host` works without any change to `monitra-tui` itself, because it never knew the difference.

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
| `monitra tui` | tui client + embedded backend *(or remote via `--url`)* | Terminal dashboard. Pure API client in every mode (ADR-009) — never reads storage directly. |
| `monitra web` | embedded backend *(or prints a remote daemon's URL + token via `--url`)* | Browser dashboard. Same embedded-or-remote bootstrap as `tui`; the remote case doesn't need to stay running itself, since the remote daemon already serves the same SPA assets at `/`. |
| `monitra monitor …` | storage *(or HTTP client)* | One-shot config management. Exits immediately. |
| `monitra version` | — | Build info. |

`monitor` sub-commands are deliberately one-shot: they should work whether or not the daemon is running, so bootstrapping a fresh install never requires starting a server first.

---

## 4. Crate Responsibilities

Each crate has an explicit contract. "Does not own" is as important as "owns."

### `monitra-models` — shared domain vocabulary

**Owns:** `Monitor`, `MonitorKind`, `MonitorStatus`, `CheckResult`, `MonitorId`, and their `Serialize`/`Deserialize` impls.

**Does not own:** persistence logic, validation that requires I/O, HTTP representations, business rules.

**Contract:** contains only plain data types and pure functions over them. Zero internal dependencies — this is what makes the graph acyclic. If two crates need to agree on a type, it goes here.

**Why a separate crate rather than a module:** it is depended on by everything. As a module inside a larger crate, any change would trigger recompilation of that whole crate. Isolated, it recompiles in milliseconds.

### `monitra-provider` — pluggable service contracts

**Owns:** the `Store`, `Cache`, `Notifier`, and `Collector` (added by ADR-008) traits; the provider registry (URL scheme → constructor); config parsing and resolution; the default-selection and availability rules of §4.1.

**Does not own:** any concrete implementation. `monitra-provider` knows that `postgres://` is a `Store` scheme; it does not know how to speak the Postgres wire protocol, and it does not know how to speak the Kubernetes API.

**Contract:** depends only on `monitra-models`. Every provider implementation crate depends on `monitra-provider`; `monitra-provider` depends on none of them. Registration happens in `main.rs`, gated by cargo features — this is what keeps the dependency arrow pointing the right way while still allowing a build to omit a provider entirely.

#### 4.1 Availability policy (ADR-007, extended by ADR-008)

"If the configured service is unavailable, fall back to the default" is **not** applied uniformly, because the consequences differ by category:

| Category | Default (always compiled in) | If configured service is unreachable |
|---|---|---|
| `Store` | SQLite (`monitra-storage`) | **Fail fast.** Refuse to start, naming the service and the error. |
| `Cache` | in-process map | Degrade to the in-process default, log at WARN, expose in `/health`. |
| `Notifier` | webhook (`notify-webhook`), logs instead of POSTing when no target is configured | Queue with bounded retry and backoff, log loudly. Never blocks a probe. |
| `Collector` | *(none)* | Per-resource, not daemon-wide: mark the affected Monitor's status as unknown (same honesty as agent-silence, §5.2), log at WARN, keep polling on schedule. Never fail the whole daemon over one unreachable cluster. |

**Why `Store` is different:** silently falling back from Postgres to SQLite would write history into a second database. The dashboard would then report uptime computed from a partial record — a direct P1 violation, and worse than not starting, because the operator would not know it happened. Refusing to boot with a clear message is the honest failure.

**Notifier default corrected at Phase 3:** this table originally named a "log sink" as the embedded default, but no such crate was ever scaffolded (Appendix) — `notify-webhook`, already planned as always-compiled in the root `Cargo.toml`, was. Rather than add a second trivial default crate, `notify-webhook` fills both roles: with no target URL configured it logs instead of sending, and becomes a real webhook sink once one is attached via `service attach webhook://…`. The HTTP-sending logic (both `notify-webhook` and the optional `notify-slack`) landed at Phase 7, wrapped uniformly in `monitra-provider`'s `RetryingNotifier` (bounded queue, fixed-interval backoff redelivery) regardless of which sink is underneath — only this identity/wording correction landed at Phase 3.

`Cache` and `Notifier` carry no such hazard: a cache miss costs latency, and a queued notification is still delivered.

**Why `Collector` has no default:** unlike `Store`/`Cache`/`Notifier`, there is nothing to fall back *to* — a `Collector` only exists because an operator configured a specific external system (a Kubernetes cluster) to introspect. No configuration means no collector-backed monitors, not a degraded default. Its failure mode is also scoped differently: it is one unreachable *resource* among possibly many configured, not a foundational service the whole daemon needs to boot.

### `monitra-storage` — persistence (default `Store` provider)

**Owns:** the SQLite implementation of the `Store` trait — schema definition, migrations, connection lifecycle, all SQL, query methods returning `monitra-models` types, retention/pruning.

**Does not own:** deciding *when* to write, business rules about status transitions, caching policy.

**Contract:** exposes a `Database` handle with typed methods (`list_monitors()`, `insert_check_result()`, `prune_older_than()`). SQL never leaks outside this crate. Callers cannot construct raw queries.

**Design note (revised v0.2):** the backend-agnostic API is no longer aspirational — it *is* the `Store` trait in `monitra-provider`. `monitra-storage` is one implementation of it, distinguished only by being the one that is always compiled in and requires nothing external.

### `monitra-probe` — HTTP/TCP/ICMP probe execution *(new, ADR-011)*

**Owns:** the actual network calls for each black-box `MonitorKind` — HTTP request + status/latency, TCP connect timing, ICMP echo — and the `ProbeOutcome` type they report (`Success`/`Failure`/`Unavailable`, §5.2/ADR-010). Extracted from `monitra-engine` at Phase 11 so `monitra-agent` can run the same probes from its own vantage point (ADR-011) without depending on `monitra-engine` itself.

**Does not own:** scheduling (when a probe runs), timeout *policy* (how long is too long — that is `monitra-engine`'s call, `monitra-probe` just respects whatever deadline it is given), flap damping, persistence, deciding which agent probes which target.

**Contract:** depends on `monitra-models` only, same discipline as every other leaf. Pure functions/short-lived tasks over an already-decided target and timeout, returning a `ProbeOutcome` — no scheduling loop, no I/O beyond the one probe. `monitra-engine` and `monitra-agent` are both callers, never the other way around.

### `monitra-engine` — the monitoring core

**Owns:** the scheduler, dispatching probe execution (via `monitra-probe`, ADR-011) for each `MonitorKind` (including `Collector`-based direct Kubernetes polling, ADR-008), timeout enforcement, retry/backoff policy, concurrency limiting, status-transition logic (flap damping, **and agent-liveness watchdog** — heartbeat timeout on an `Agent` transitions its dependent Monitors to "unknown, agent unreachable," never silently to "down," §5.2), result broadcast, and emission of `AlertEvent`s to attached `Notifier`s on status transition.

**Does not own:** persistence details, HTTP API shape, presentation, the probe calls themselves (`monitra-probe`, ADR-011).

**Contract:** started via `EngineHandle::start()`, shut down gracefully via `shutdown()`. Emits `CheckResult` on a broadcast channel and writes through `monitra-storage`. Pulled results (dispatched to `monitra-probe`), pushed results (relayed from `monitra-backend`'s agent-ingest endpoint, including agent-executed regional probes as of Phase 11), and `Collector` polls all flow through the same bounded writer path (§6.2) — engine does not distinguish their origin once they arrive. This is the crate where P1 (reliability) matters most — it is the component whose correctness the entire product rests on.

### `monitra-backend` — API surface

**Owns:** Axum router, HTTP handlers, request/response DTOs, WebSocket upgrade and event fan-out, the authenticated agent-ingest endpoint (ADR-008), human-facing API authentication (ADR-009 — a static bearer token, §11.11, Phase 5), middleware (logging, CORS, error mapping), embedded static asset serving for the web dashboard *(Phase 10)*.

**Does not own:** monitoring logic, database schema, scheduling.

**Contract:** a thin translation layer. Handlers validate input, call into `monitra-storage` or `monitra-engine`, and map results to HTTP. Any handler containing business logic is a design smell that belongs in `monitra-engine`. **Mutation handlers and `monitra-cli`'s command execution in `main.rs` call the same internal service functions** (ADR-009) — a mutation is never implemented twice. Landed at Phase 5 as `monitra_backend::service`: plain functions over `Arc<dyn Store>` that both this crate's Axum handlers and `main.rs`'s one-shot CLI execution (`monitra monitor …`, `monitra agent …`) call directly — `monitra-cli` itself still owns only argument shape (§4 `monitra-cli`), never execution.

**Why DTOs are separate from `monitra-models`:** the wire format must be able to evolve independently of the internal domain model. Coupling them means an internal refactor becomes a breaking API change.

### `monitra-tui` — terminal dashboard

**Owns:** terminal setup/teardown (raw mode, alternate screen), event loop, widget composition, key bindings, view state, an HTTP/WS client against `monitra-backend`'s API.

**Does not own:** data fetching policy beyond its own refresh cadence, monitoring logic, *any* persistence access — it never reads `monitra-storage` directly, in any mode (ADR-009).

**Contract:** `run_tui()` takes over the terminal and **must restore it on every exit path**, including panics. A panic that leaves the user's terminal in raw mode is a serious bug — a panic hook that restores terminal state is mandatory. `monitra-tui` is handed a base URL and knows nothing else about where it points — whether that URL is a remote daemon or an embedded backend `main.rs` booted on loopback for a no-daemon local session (§11.4) is invisible to this crate by design.

### `monitra-agent` — remote collection, local host checks, and regional probing

**Owns:** the `monitra agent run` mode: local host checks (systemd unit status, disk space, process liveness — no black-box network equivalent exists for these, ADR-008), the Kubernetes-fallback push path for clusters the central instance cannot reach directly, **agent-executed network probes (HTTP/TCP/ICMP) against a target for multi-region latency comparison, via `monitra-probe` (new, ADR-011, Phase 11)** — this is what gives a probe a distinct geographic vantage point, the push loop itself (retry/backoff, a bounded local buffer for when the backend is unreachable), and registration/token handling for authenticating its pushes.

**Does not own:** deciding *whether* a pushed result changes a Monitor's status — that is `monitra-engine`'s job once the result lands via `monitra-backend`'s ingest endpoint. `monitra-agent` reports; it does not interpret. Nor does it decide *which* monitors it is assigned to probe regionally — that assignment is central-engine scheduling (ADR-011), the agent only executes what it is told.

**Contract:** depends on `monitra-models` and `monitra-probe` only (§3.2, ADR-011 revises the original models-only position to add the one shared leaf) — it is wired by `main.rs` exactly like `monitra-cli`, and talks to `monitra-backend` over HTTP, never in-process. A `monitra agent` that cannot reach its backend keeps running its local checks and buffering (bounded — P1 §7.3), it does not crash or block on connectivity.

### `collector-kubernetes` — direct Kubernetes API polling (a `Collector` provider)

**Owns:** the `Collector` trait implementation that polls the Kubernetes API server directly (kubeconfig or in-cluster service account, §11.12) for Deployment/StatefulSet health (`readyReplicas` vs `replicas`) and Service health (existence *and* at least one ready Endpoints address). Built on plain `reqwest` calls against those four REST shapes, not `kube`/`k8s-openapi` (§11.12).

**Does not own:** deciding what to do when the cluster is unreachable (that's the per-category policy in §4.1, enforced by `monitra-provider`/`monitra-engine`). **On-demand pod-level breakdown, mentioned here before Phase 6, was not built** — the `Collector` trait's `poll()` is per-resource-not-per-pod, and nothing in the backend/web/TUI surface yet asks for pod-level detail; revisit when something does, rather than build it speculatively now (P5).

**Contract:** depends on `monitra-models` and `monitra-provider` only, same as `store-postgres`/`cache-redis`/`notify-*` — nothing else may import it. Gated by a cargo feature like every other non-default provider (§8).

### `monitra-cli` — argument surface

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
| `status` | `MonitorStatus` | `Pending` \| `Up` \| `Down` \| `Paused` \| `Stale` (`Stale` added at Phase 6 — §5.2) |
| `agent_id` | `Option<u64>` | FK → `Agent` (added at Phase 6, migration `0005_monitor_agent_id`). The `Agent` this monitor's check data depends on; `None` for monitors the engine probes directly. Drives the agent-liveness watchdog (§4 `engine`) |

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
| `last_heartbeat_at` | `u64` | Unix seconds; drives the liveness watchdog (§4, `monitra-engine`) |
| `scope` | `String` | What it watches — a host, or a Kubernetes cluster/namespace reference |
| `token` | `String` | Push-auth credential for `POST /agents/{id}/ingest` (§11.10, Phase 7, migration `0006_agent_token`) — distinct from the human `api_token` (§11.11). Reissued on every `agent register`, including a repeat registration under the same name |
| `region` | `Option<String>` | Geographic vantage point this agent probes from (ADR-011, Phase 11, migration `0007_agent_region`). Nullable, never defaulted or guessed — an agent with no region is excluded from regional aggregation, not folded into some implicit default. `agent register` fully overwrites it on every call, same as `scope`/`token`: omitting `--region` on a repeat registration clears a previously stored value rather than preserving it |

An agent's own liveness is tracked separately from any `Monitor`'s status, for the same reason `Pending` exists (§5.2): "the agent went silent" and "the target is down" are different failure signals and must never be collapsed into one.

**`AlertEvent`** *(new, ADR-009)* — a persisted record of an emitted alert, so alert history is queryable by `monitra-cli`/`monitra-tui`/web rather than existing only as whatever a `Notifier` sink did with it. A deliberate exception to this section's "resist adding entities" discipline — without it, an alert-history view is not buildable at all.

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

The same reasoning applies to the daemon restarting: on startup, monitors retain their last known status but the dashboard *(Phase 9)* will visually distinguish "confirmed 12s ago" from "last known, staleness unknown."

**A third case, added by ADR-008, resolved at Phase 6:** a `Monitor` fed by an unreachable `Agent` or `Collector` is neither confirmed-up nor confirmed-down — it is exactly the same "last known, staleness unknown" case as a daemon restart, just triggered by the feed going silent instead of the daemon restarting. Modeled as a distinct `MonitorStatus::Stale` variant, not a side flag: storage already stores `status` as `TEXT` (`codec.rs`), so the addition was a codec match-arm, not a schema migration to the column itself. A monitor moves to `Stale` in two cases, both bypassing flap damping entirely (damping is for "is the target actually failing," not "can we even tell right now"):
- A network/collector probe reports `ProbeOutcome::Unavailable`/`CollectorStatus::Unknown` (e.g. ICMP without `CAP_NET_RAW`, §11.3; a K8s cluster unreachable) — immediate, no consecutive-failure count needed, since this isn't a claim about the target at all.
- The agent-liveness watchdog (`engine::watchdog`) finds `Monitor.agent_id`'s `Agent.last_heartbeat_at` older than `heartbeat_timeout` — every monitor referencing that agent moves to `Stale` on the next sweep. `Monitor.agent_id` (migration `0005_monitor_agent_id`, nullable FK → `Agent`) is itself a Phase 6 addition — discovered mid-implementation that nothing in the Phase 4 schema linked a `Monitor` to the `Agent` it depends on, which the watchdog needs to know which monitors to demote.

The constraint that was already locked in held: "agent/collector unreachable" never renders as "target down."

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

**Added by ADR-008 — a second arrival path feeds the same result channel:** `monitra-backend`'s authenticated agent-ingest endpoint hands pushed results into the identical bounded `mpsc` shown above, rather than a separate queue. Two producers (scheduler-dispatched probes, agent pushes), one bounded channel, one writer task — the batching and overflow-drop-and-log policy (§7.3) apply uniformly regardless of which path a result came from.

**Why a semaphore on probes rather than a fixed worker pool:** monitor count and concurrent-probe count are different quantities. 5,000 monitors on 5-minute intervals produce far less instantaneous load than 500 monitors on 10-second intervals. Bounding the thing that actually consumes resources (in-flight network operations, file descriptors) rather than the thing that is merely configured is the correct control point.

**Why a dedicated writer task:** SQLite permits one writer. Funnelling all writes through a single task that batches them turns thousands of individual transactions into a handful of batched ones. This is the single most important performance decision in the design.

### 6.3 Timing guarantees

Monitra promises **"checked at least every `interval_secs`, best effort"** — not exact periodicity. Under load, checks may drift later. What we guarantee:

1. A check never silently doesn't happen. If the scheduler falls behind, it logs and records the gap.
2. Drift does not accumulate unboundedly — scheduling is anchored to absolute deadlines, not `sleep(interval)` after completion, so a slow probe doesn't permanently shift the schedule.
3. Every probe has a hard timeout. A hung connection to one target cannot stall the scheduler.

Point 3 is why probes are spawned rather than awaited inline.

### 6.4 Falsification test — harness built at Phase 6, full sweep still outstanding

The hypothesis in §6.1 is tested, not assumed. Built at Phase 6 as `tests/scale.rs` **at the workspace root**, not `crates/engine/benches/` as originally sketched here — it needs a real `SqliteStore` wired to a real `EngineHandle` end to end, and `monitra-engine` is correctly forbidden by `scripts/dep-check.py` from depending on `monitra-storage` (only the root binary is allowed to know about both, CLAUDE.md's dependency DAG). It's a `#[tokio::test]`-based integration test, not a `[[bench]]`:

```
tests/scale.rs
  - spawn N local mock HTTP targets (bare TCP, not axum — N=5000 app
    instances would add real overhead of its own) with configurable latency
  - register N monitors at interval I against a real SqliteStore, wrapped to
    time every batched insert_check_results call
  - run for the configured duration
  - assert: p99 schedule drift < 2s
  - assert: RSS < 512 MB
  - assert: zero missed checks
  - report: DB write latency p50/p99 (the actual §11.1 number)
```

`scale_smoke` (N=10, 9s) runs in the routine gate and proves the harness mechanism itself — drift tracking, missed-check accounting, DB-write timing — is correct; it is not evidence about the hypothesis at any real N. The literal protocol — N = 100, 500, 1000, 2500, 5000, 10 minutes each, ~50 minutes total — is `#[ignore]`d and run explicitly: `cargo test --release --test scale -- --ignored --nocapture`. **That full sweep has not been run as of this Phase 6 landing** — the point where any assertion fails, once it has been, is the documented, honest scaling limit, and belongs in the README as a number, not a marketing adjective. A shortened, non-authoritative `preliminary_n500` check (real run, 60s not 600s) also exists for a fast sanity read before committing to the full sweep; its numbers are explicitly not the §6.4 figure either. One real (release-profile) run of it at Phase 6 landing: N=500, ran 60.0s, p99 drift 0.0ms (max 0.8ms), RSS 8.6→15.5MB, zero missed checks, DB write p50/p99 1.76/2.20ms — encouraging, but 60 seconds at N=500 says nothing about the 10-minute/N=5000 regime §11.1 actually worries about. No root `README.md` exists in this repo yet to hold the eventual authoritative table; that's a gap the person running the full sweep should close at the same time, not a Phase 6 decision to make unprompted.

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
| Migrations | Separate migration tool | Embedded SQL, applied at startup by `monitra-storage` |
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

**Decision:** Workspace with `monitra-models`, `monitra-storage`, `monitra-engine`, `monitra-backend`, `monitra-tui`, `monitra-cli` as libraries plus a thin root binary.

**Alternatives:**
- *Single crate with modules* — simplest, but every change recompiles everything, and module boundaries are not enforced by the compiler. Rejected: boundaries that are only conventions get violated.
- *Separate repositories* — maximum isolation but painful version coordination for a single-binary product. Rejected as overkill.

**Consequences:** ✅ Enforced boundaries, parallel + incremental compilation, independently testable layers. ❌ More `Cargo.toml` files; shared dependency versions must be centralised (done via `[workspace.dependencies]`).

---

### ADR-002 — SQLite first, Postgres later

**Status:** **Superseded by ADR-007.** Retained because the reasoning about *why* SQLite is the correct default survives the supersession intact; only "Postgres later" is reversed.

**Context:** Need persistence that does not violate P2.

**Decision:** SQLite via bundled `rusqlite`, with `monitra-storage` exposing a backend-agnostic API.

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

### ADR-005 — `monitra-models` has zero internal dependencies

**Status:** Accepted

**Context:** Dependency cycles in Rust workspaces are a hard compile error, and shared types are the usual cause.

**Decision:** `monitra-models` contains only plain data types and depends on nothing internal. Every other crate may depend on it.

**Consequences:** ✅ Acyclic graph guaranteed structurally; `monitra-models` recompiles instantly. ❌ Some logic that "feels like" it belongs on a domain type must live elsewhere — accepted deliberately, since anything requiring I/O belongs in a layer that owns I/O.

---

### ADR-006 — Build persistence before the API

**Status:** Accepted (Phase 0)

**Context:** The v0.1 roadmap ordered Backend API at Phase 3 and Database at Phase 4. But §3.2 has `monitra-backend` depending on the store. Building the API first means writing handlers against a store that does not exist.

**Decision:** Persistence lands before the API. The v0.2 roadmap orders provider layer → storage → backend.

**Alternatives:**
- *Keep the original order, stub the handlers* — Phase 3 would ship handlers wired to in-memory stubs, then rewrite every one of them at Phase 4. Worse, the Phase 3 test gate could only assert routing and status codes. A gate that cannot fail is not a gate. Rejected.

**Consequences:** ✅ Every handler is written against a real store and gets a behavioural integration test the day it is written. ❌ Nothing runs end-to-end until later in the sequence; the first externally visible milestone moves right.

---

### ADR-007 — Pluggable service providers with embedded defaults

**Status:** Accepted (Phase 0). Supersedes ADR-002; revises ADR-001 and §1.3.

**Context:** Operators want to attach their own infrastructure — an existing Postgres, an existing Redis, their own notification channels — rather than accept whatever the tool embeds. But P2 forbids *requiring* any of it.

**Decision:** Three provider categories (`Store`, `Cache`, `Notifier`) defined as traits in a new `monitra-provider` crate. Each has an embedded default requiring nothing external. Users attach alternatives by URL in config, produced by an interactive `monitra setup` wizard or by flags. Providers are registered at compile time and gated by cargo features. Availability failures are handled per-category (§4.1).

**Alternatives:**
- *Dynamic `.so` plugins* — maximum extensibility, but the binary stops being self-contained. Directly violates P2, which §2 declares load-bearing. Rejected.
- *Runtime hot attach/detach of any provider* — attractive for notifiers, but swapping a live `Store` means draining in-flight writes and reconciling migration state mid-flight. Deferred, not rejected; see §11.8.
- *Uniform fallback to defaults for every category* — simpler rule, but silently splits history across two stores when the configured store blips. Rejected on P1 grounds (§4.1).
- *Keep providers out of v1 entirely* — smallest scope, but retrofitting an abstraction after `monitra-engine` and `monitra-backend` have hard-wired SQLite is markedly more expensive than building it now.

**Consequences:** ✅ Operators attach their own infrastructure; the zero-config path is untouched; `monitra-engine`/`monitra-backend`/`monitra-tui` can no longer reach a concrete database. ❌ Crate count rises from 6 to 10+; the feature matrix must be built and tested in CI, or feature combinations rot; `monitra setup` is new surface area that must stay strictly optional (§11.7).

---

### ADR-008 — Distributed agent architecture for target introspection

**Status:** Accepted (pre-Phase 1). Revises §1.3 and ADR-005's crate list; extends §4.1.

**Context:** Monitra's v1 scope was a single-node, black-box prober — reachable-over-the-network targets only. That leaves a real gap: it cannot tell whether a Kubernetes Deployment is actually healthy (as opposed to its Service's ClusterIP merely being reachable), and it cannot check host-local facts — a systemd unit's state, disk space, process liveness — that have no network-visible signal at all. The ask was to "track all deployments no matter the architecture," which black-box probing alone cannot satisfy.

**Decision:** Two new mechanisms, chosen deliberately over stronger and weaker alternatives:

1. Kubernetes: poll the cluster's API server directly by default (new `Collector` provider category in `monitra-provider`, implemented by `collector-kubernetes`), with a lightweight agent push path as the fallback for clusters the central instance cannot reach directly.
2. Bare-machine facts with no network signal (systemd unit status, disk space, process liveness): agent-only. A new `monitra-agent` crate runs `monitra agent run` — the same binary, a different mode, not a separate artifact — performing local checks and pushing results to `monitra-backend`'s authenticated ingest endpoint, with its own retry/backoff and a bounded local buffer when the backend is unreachable.

The persisted `Monitor` for a Kubernetes target is the orchestrator resource (Deployment/StatefulSet/Service) — a stable identity — not any individual pod; pod-level status is fetched live as breakdown detail, never persisted (§5.1). A new `Agent` entity (§5.1) tracks each registered collector's own liveness, separately from any `Monitor`'s status — `monitra-engine` owns the state-transition logic for what an unreachable agent means for the monitors it feeds (§5.2), on the same honesty grounds as `Pending` and the ICMP-permission case (§11.3): a silent agent is never rendered as a down target.

**Alternatives:**
- *Kubernetes via agent-only, no direct polling* — simpler (one mechanism, not two), but needlessly requires installing something inside every cluster even when the API server is directly reachable. Rejected as the sole mechanism; kept as the fallback.
- *Bare-machine checks via SSH-exec, no agent to install* — fits the existing "scp the binary, ssh in" thesis with zero footprint on the target, and was the initial recommendation. Rejected in favor of an agent: an agent supports richer local checks and doesn't require inbound SSH access to be configured on every host.
- *Individual Kubernetes pods as persisted `Monitor` rows* — finer-grained history, but pod identity churns on every reschedule, scale event, and rolling update; this would defeat §5.4's retention model by turning `check_results` growth into an unbounded-identity problem, not just an unbounded-time one. Rejected.
- *Keep Monitra a pure black-box prober, leave orchestrator/host introspection to `kubectl` and existing host tooling* — smallest scope, most consistent with the original v1 thesis. Rejected because it does not answer what was actually asked: whether a deployment is healthy, not merely reachable.

**Consequences:** ✅ Monitra can answer "is this Deployment actually healthy" and "is this host out of disk," not just "is this port open"; the mechanism (agent as a mode of the same binary) preserves P2. ❌ `monitra-provider` gains a fourth category with no honest default (§4.1); `monitra-backend` gains a new authenticated ingest surface that is real attack surface; `monitra-engine` gains a second, structurally different kind of "we don't know" state to represent correctly; the crate count grows again (two more leaves); §1.3's "not a distributed multi-region prober" line no longer holds as originally written and had to be narrowed rather than simply deleted (see the revised §1.3 text).

---

### ADR-009 — Backend-first client architecture; web and TUI as symmetric API clients

**Status:** Accepted (pre-Phase 1). Supersedes ADR-004; adds `AlertEvent` to §5.1; resolves §11.4.

**Context:** ADR-004 made the web dashboard permanently subordinate to the TUI in capability, and `monitra-tui` itself had two different data-access implementations (local: read `monitra-storage` directly; remote: speak HTTP) with the abstraction reconciling them left undesigned (§11.4). Once the decision was made that web should no longer be capability-subordinate to TUI, and that the backend should be built out to support every capability identified for the dashboards (both the "must" and "could" feature sets worked out for the TUI — fleet/agent/K8s views, health, alert history, and more) before either frontend catches up, the TUI's dual-implementation problem became actively worse, not better, if left as-is: a third (web) implementation would just add a second axis of duplication.

**Decision:**

1. `monitra-backend` becomes the sole source of truth. `monitra-tui` and the web dashboard are both pure HTTP/WS API clients, in *every* mode — `monitra-tui` no longer reads `monitra-storage` directly even when running locally with no daemon present (§3.2 tightens accordingly: `monitra-tui` depends on `monitra-models` only, not `monitra-provider`).
2. When `monitra tui` or `monitra web` runs with no daemon already up, `main.rs` boots an embedded backend (engine + storage + API layer) in-process on a loopback TCP address with an ephemeral port, and points the client at it exactly as it would point at a remote daemon. One client implementation exists, not two — this resolves §11.4 by construction rather than by designing an abstraction to paper over two implementations.
3. ADR-004 is **superseded**, not merely revised: once TUI and web are structurally symmetric clients of the same API, "web must never exceed TUI" is not a rule that needs enforcing, it is a rule that no longer applies. This does not touch the separate, still-standing hard rule (P3, CLAUDE.md) that every capability must be reachable from the CLI — that constraint is independent of ADR-004 and is unaffected by its supersession.
4. A new `AlertEvent` entity (§5.1) persists emitted alerts as queryable history — a deliberate, named exception to §5.1's general discipline against adding entities, made because an alert-history view is not buildable at all without it.
5. The backend's human-facing HTTP/WS surface requires authentication from the start, separate from agent-push tokens (§11.10) — because web is now a real network-facing equal client rather than a TUI-parity convenience assumed to sit behind an SSH tunnel. The exact mechanism (API key vs. session login) is left open (§11.11); only the requirement is decided here.
6. CLI command execution (in `main.rs`) and backend HTTP handlers for the same mutation (add/pause/resume/remove a monitor, etc.) call the same internal service functions — a mutation's logic exists in exactly one place, per the existing `monitra-backend` contract's principle that business logic belongs in `monitra-engine`, not in a handler.

**Alternatives:**
- *Keep `monitra-tui`'s dual local/remote implementation and add web as a third* — rejected: this compounds the §11.4 problem instead of resolving it.
- *Defer authentication to a later phase, since this is currently a solo/small-team project* — rejected once web became a real equal-capability client: an unauthenticated network-facing API contradicts "build the backend for every capability" the moment the web dashboard is actually deployed somewhere reachable, and retrofitting auth onto an already-built API surface is more expensive than building it in from the first handler.
- *Keep ADR-004's ordering but simply raise web's ceiling to match TUI* — rejected: the ceiling itself was the wrong model once both are pure API clients; there is nothing left for a ceiling to constrain.

**Consequences:** ✅ §11.4 is resolved rather than merely scheduled; TUI and web share one client mental model and one set of integration tests against the API; the backend becomes buildable and testable well ahead of either frontend, matching ADR-006's precedent of not writing a layer against something that doesn't exist yet. ❌ `monitra-tui`'s DAG position gets *stricter*, not looser, which means any future "quick local read" temptation must go through the embedded-backend path rather than a shortcut; the backend now carries a real authentication surface (and its failure modes) that did not exist before; local-mode startup now always pays the cost of booting a full embedded backend, even for a single `monitor add` glance.

---

### ADR-010 — Agent push protocol extended to tri-state outcomes; K8s-fallback push deferred out of Phase 8

**Status:** Accepted (Phase 8)

**Context:** ADR-008 committed the Phase-8 agent binary to three things at once: local host checks, the push loop, and K8s-fallback push. §11.10 (Phase 7) had already fixed the ingest wire shape as `{monitor_id, success, latency_ms, message}` for the backend side, deliberately leaving the agent binary itself — and anything that shape didn't yet need to express — to Phase 8. Building the agent surfaced two problems neither earlier phase could have seen, because neither had agent-side check logic to look at yet: (1) `monitra-agent`'s DAG position is `{monitra-models}` only (ADR-008, enforced on dev-dependencies too by `scripts/dep-check.py`) — it can never depend on `monitra-provider`, so it cannot reuse `collector-kubernetes`'s Kubernetes-API polling code, and the roadmap's own Gate column for Phase 8 never actually tested K8s-fallback push, only host checks and the push loop; (2) the flat `success: bool` wire shape gives an agent no way to report "this check itself could not run" (permission denied reading disk stats, `systemctl`/dbus unreachable) distinctly from "I ran the check and the target is down" — exactly the honesty gap P1/§11.3 already closed on the pull path, where `ProbeOutcome::Unavailable` routes to `MonitorStatus::Stale` instead of being collapsed into `Down`.

**Decision:**

1. **K8s-fallback push is deferred out of Phase 8.** It needs its own from-scratch Kubernetes-API client inside `monitra-agent` (not a reuse of `collector-kubernetes`), which is materially separate work from host checks and the push loop. Phase 8 ships local host checks (disk/systemd/process) and the push loop only. Tracked at §11.15, not silently dropped — ADR-008's Kubernetes story is not complete until a later phase picks this up.
2. **The agent-push wire protocol is extended to a tagged, tri-state `outcome`.** `IngestRequest`/`PushedResultDto` (`crates/backend/src/ingest.rs`) now carry `outcome: "success" { latency_ms } | "failure" { message } | "unavailable" { message }` instead of a flat `success: bool`. `engine::PushedResult` (`crates/engine/src/scheduler.rs`) now carries a `ProbeOutcome` directly — the same type the pull path already uses — instead of a flat bool, and a pushed `Unavailable` routes through the existing `mark_stale_and_record` path (bypassing flap damping), exactly as an unreachable `Collector` already does. `monitra-engine` re-exports `ProbeOutcome` (`crates/engine/src/lib.rs`) so `monitra-backend` can construct it without a new shared type crossing the DAG. This revises §11.10's "resolved at Phase 7" wire shape; the revision is recorded at §11.16, not treated as if Phase 7 had gotten it right the first time.

**Alternatives:**
- *Keep `success: bool`; an agent simply skips pushing a result for a check it could not run* — rejected: the monitor's status is left silently frozen at its last value, with nothing to distinguish "confirmed recently" from "this check has been broken for days," and the agent's own heartbeat staying healthy actively masks the problem rather than surfacing it.
- *Keep `success: bool`; annotate the failure message with the real cause* — rejected: the wire value is still `success: false`, which still renders as `Down` on every consumer. Same category of dishonesty P1 already ruled out for the pull path before `Stale` existed; a human-readable annotation doesn't change what the status field claims.
- *Build K8s-fallback push in Phase 8 as originally scoped* — rejected: duplicates `collector-kubernetes`'s REST-shape logic from scratch (DAG forbids reuse), and the roadmap's own Gate never asked for it, making "finish Phase 8" and "build K8s-fallback push" two different-sized commitments accidentally bundled under one phase number.

**Consequences:** ✅ Push and pull paths share one honesty model end to end — `Unavailable`/`Stale` means the same thing regardless of which path produced it, and `engine::scheduler`'s pushed-result handling is a direct mirror of its network-probe handling rather than a parallel, weaker implementation. ❌ The wire protocol changed shape after being called "resolved" at Phase 7 — a real instance of a phase gate not anticipating a later phase's needs, worth remembering the next time a wire shape gets marked resolved before every consumer of it exists. ADR-008's Kubernetes story is now explicitly incomplete rather than implicitly assumed done by Phase 8 finishing; whichever phase builds K8s-fallback push has to write a second, independent Kubernetes API client rather than reusing existing code.

---

### ADR-011 — Multi-region latency probing promoted to v1 scope; probe execution extracted into `monitra-probe`

**Status:** Accepted (Phase 9 planning)

**Context:** §1.3 and §10 have named multi-region latency probing — comparing the same target's latency from multiple geographic vantage points — as explicitly out of v1 scope since the original design, and ADR-008 reaffirmed the boundary even while bringing distributed *target introspection* (Kubernetes, host agents) into v1: "probing the same target from many vantage points... is a distinct design problem... still not a v1 constraint." That boundary is reconsidered here, prompted by Phase 9 TUI design work (a "Globe" region-heatmap screen, found already sketched on a design branch) that has no data behind it. Decide whether to build it, and if so, how it fits the existing Agent/engine architecture without reopening the aggregation problem ADR-008 deliberately avoided.

**Decision:** Multi-region latency probing enters v1 scope, built on the existing `Agent` mechanism rather than a new "regional prober" concept:

1. `Agent` gains an additive `region: Option<String>` field (nullable — an agent with no declared region is simply excluded from regional aggregation, never guessed, per P1).
2. Multi-region coverage of one target is N ordinary `Monitor` rows sharing the same `target`/`kind`, each `agent_id`-linked to a different region-tagged `Agent` — not a new "regional target" entity. Aggregation (p95, failure rate, probe volume per region) is a read-side query grouping Monitors by `target` and their agent's `region`, never persisted as its own row — keeps §5.1's "resist adding entities" discipline intact.
3. `monitra-agent` gains the ability to execute outbound network probes (HTTP/TCP/ICMP) against a target from wherever it runs, alongside its existing local host checks (ADR-008) — this is what gives a probe a real vantage point. The probe-execution logic (currently `crates/engine/src/probe/{http,tcp,icmp}.rs`, owned solely by `monitra-engine`) is extracted into a new leaf crate, `monitra-probe`, depending only on `monitra-models`. Both `monitra-engine` and `monitra-agent` depend on it — the only way `monitra-agent` can share real probe code, since its DAG position forbids depending on `monitra-engine`/`monitra-provider` (ADR-008).
4. Scheduling authority stays central: the engine still decides when a check is due and assigns network-probe monitors to a region-tagged agent instead of only running them itself; the agent pulls its assignment and pushes results back through the existing push/ingest path (ADR-008/ADR-010).
5. Recorded as a new roadmap phase, **Phase 11 — Multi-region latency probing**, inserted before Bundling (which becomes Phase 12). Implementation is out of scope for this ADR and for Phase 9 — this ADR only settles that it is happening and how it fits.

**Alternatives:**
- *A dedicated "vantage point" entity, independent of `Agent`* — cleaner separation, but duplicates liveness/scope/push-auth `Agent` already provides. Rejected: recreates ADR-008's machinery a second time for no real gain.
- *Central engine probes every region itself, varying only source IP/interface* — no new crate or field, simplest to build. Rejected: one process/network path cannot honestly claim multiple *geographic* vantage points; it would be measuring routing from one location, not from the region — exactly the "lying dashboard" P1 forbids.
- *Duplicate the HTTP/TCP/ICMP calls inside `monitra-agent` instead of extracting a shared crate* — avoids a new crate. Rejected: duplicated timeout/`ProbeOutcome` semantics drifting apart between engine and agent is a correctness risk with no offsetting benefit; extracting one shared crate is strictly safer and mirrors what `monitra-provider` already does for shared behavior.
- *Cut the Globe screen, leave multi-region out* — considered and rejected in favor of committing to build it, tracked as its own phase.

**Consequences:** ✅ Multi-region latency comparison becomes buildable without inventing new persisted entities or a second scheduling authority; extracting `monitra-probe` is also a general win — `monitra-engine`'s own pull-path probing and any future in-process probe consumer share one tested implementation instead of one owned by the crate that happened to need it first. ❌ The crate count grows again (13 → 14); `monitra-agent`'s stated ADR-008 contract ("local host checks, no network-visible signal has no black-box equivalent") is now half-true — it also runs genuine network probes for the regional case, which the ADR-008 text undersells until Phase 11 lands; `Agent.region` is one more nullable field to keep honest — "unset" must never be silently read as "no region" *or* "the operator's own region," which the eventual Phase 11 phase-start must state as a hard rule, not leave implicit.

---

### ADR-012 — Release profile keeps `panic = "unwind"`; §11.6 resolved

**Status:** Accepted (Phase 12)

**Context:** §8 wants `panic = "abort"` in the release profile purely for binary size. §7.2's failure taxonomy promises something specific and stronger: a probe task that panics is caught via its `JoinHandle`/`JoinError`, recorded as an internal error distinct from target-down, and never takes the rest of the daemon with it. §11.6 flagged these as being in tension and required a decision before Phase 12 turned the release profile on for the first time (no `[profile.release]` existed in `Cargo.toml` before this phase), noting the tension might resolve itself "by ensuring probe code cannot panic in the first place." An audit of `monitra-probe` (`http.rs`/`tcp.rs`/`icmp.rs`) and the scheduler's dispatch path (`crates/engine/src/scheduler.rs`) done as part of this phase found exactly that: zero `unwrap()`/`expect()`/panic-capable arithmetic in the actual probe-execution path today — the two `.expect()` calls present are on a semaphore permit acquisition that a code comment already documents as provably never closed. The same audit also surfaced a real, independent bug: the scheduler's `tasks.join_next()` was silently discarding the `Err(JoinError)` case — a task that panicked (or, hypothetically, was cancelled) vanished with no log line and no recorded `CheckResult` at all, which is a silent drop (violates P1/§7.3) and directly contradicts §7.2's own table.

**Decision:** `[profile.release]` sets `panic = "unwind"` explicitly (not `"abort"`), so a future panic in probe-adjacent code — introduced by a later change, not present today — still isolates to the one monitor whose task panicked rather than aborting the whole daemon, matching §7.2's literal promise. Separately, and regardless of the abort/unwind choice, `crates/engine/src/scheduler.rs` now tracks which monitor owns each spawned task (a `TaskOwner` map keyed by `tokio::task::Id`, populated at every `dispatch_network`/`dispatch_k8s` spawn site) and uses `JoinSet::join_next_with_id` instead of `join_next`. On `Err`, it logs loudly (`monitor_id`, task id, and the panic payload extracted via `JoinError::try_into_panic`) and routes a synthesized `ProbeOutcome::Unavailable` / `CollectorStatus::Unknown` through the same `handle_outcome` path a real result would take — landing on `Stale`, never `Down`, exactly like the existing "collector unavailable" case. Covered by `scheduler::tests::a_panicked_probe_task_is_recorded_as_stale_not_dropped`.

**Alternatives:**
- *`panic = "abort"`, accepting the blast-radius regression* — matches §8's size intent most directly, and the audit gives real evidence the probe path shouldn't panic in practice. Rejected: the CLAUDE.md no-`unwrap`/no-`expect` rule is enforced by convention and clippy review, not by the type system — a future contributor's mistake (or a new transitive dependency panicking internally) would then take down monitoring for every target at once, the exact §7.2 failure §11.6 exists to prevent. The measured default build (7.0 MB stripped, see §11.13) has enough headroom under the 25 MB budget that the size argument for `abort` isn't load-bearing here.
- *Catch panics manually inside each spawned future (e.g. `FutureExt::catch_unwind`) and keep `panic = "abort"` for everything else* — would preserve isolation even under abort. Rejected: `catch_unwind` cannot cross an `.await` point cleanly without a new dependency (`futures`) and non-trivial unsafety around unwind-safety bounds, for a benefit `panic = "unwind"` already provides for free at the profile level.
- *Leave `join_next`'s silent drop as-is, treat it as out of scope for this ADR* — it's arguably a separate bug from the abort/unwind question. Rejected: it directly contradicts the §7.2 row this ADR is about, so resolving §11.6 without fixing it would leave the promise unmet either way the profile setting went.

**Consequences:** ✅ §7.2's failure-taxonomy table is now true in code, not just in the document — a probe/collector task panic is observable (loud log, recorded `Stale` status) instead of a silent gap in a monitor's history. §11.6 is resolved without weakening P1's blast-radius guarantee. ❌ The release binary forgoes `abort`'s size and (modest) performance benefit; if a future size audit finds the unwind tables material to the budget, this ADR is where that trade gets reopened, not a place to quietly flip the flag.

---

## 10. Roadmap

Revised in v0.2 by ADR-006 (persistence before API) and ADR-007 (provider layer); resequenced in v0.3 by ADR-008 (distributed agents) and ADR-009 (backend-first clients). Phases 0–7 keep their v0.2 numbering and gates unchanged in substance — each just gained scope from the two new ADRs, listed below. Phase 8 is new; the old Phase 8/9/10 (TUI/Web/Bundling) shift to 9/10/11. Resequenced again in v0.4 by ADR-011 (multi-region latency probing): a new Phase 11 is inserted for it, and the old Phase 11 (Bundling) shifts to 12.

| Phase | Deliverable | Gate | Status |
|---|---|---|---|
| 0 | Scaffolding — `CLAUDE.md`, phase skills, dep-check, git | dep-check runs; DESIGN.md reflects ADR-006/007/008/009 | ✅ Complete |
| 1 | Project setup — workspace, **all crates including `monitra-agent` and `collector-kubernetes` stubbed from day one** | builds clean; clippy `-D warnings`; dep-DAG passes with the full crate set present; `monitra version` | ✅ Complete |
| 2 | CLI base — command tree, monitor CRUD, **agent management, K8s cluster attach**, `setup`, `service` | parse tests for every command form incl. new ones; `--help` snapshot; **no execution** | ✅ Complete |
| 3 | Provider layer — Store/Cache/Notifier/**Collector** traits, registry, config, `monitra setup` | fake providers exercise **all four** §4.1 policies; zero-config path still works | ✅ Complete |
| 4 | Storage — SQLite `Store` impl, schema, migrations, retention, **+ `Agent`, `AlertEvent`, orchestrator-resource `MonitorKind`s** | migrations on fresh DB cover all entities incl. new ones; round-trip; prune; WAL asserted on | ✅ Complete |
| 5 | Backend API — Axum router, REST handlers, health endpoint, **human-facing auth built in from the first handler** | integration tests on ephemeral port against a real store; **401 without credentials / 200 with**; health reports internal state | ✅ Complete |
| 6 | Monitoring engine — scheduler, probes, **Collector-based K8s direct-poll**, flap damping, **agent-liveness watchdog**, benchmarks | §6.4 falsification harness; flap tests; **agent-heartbeat-timeout test**; monotonic-clock test; hard-timeout test | ✅ Complete (§6.4's full N=100–5000/10-minute sweep still needs a dedicated run — see §6.4 note) |
| 7 | Events — WebSocket fan-out + notifier sinks, **agent-ingest endpoint, `AlertEvent` emission on transition** | slow client dropped without back-pressuring engine; sink retry/backoff; **ingest queue bounded-drop-and-log test**; `AlertEvent` row created on every transition | ✅ Complete |
| 8 | **Agent binary** (new, ADR-008) — local host checks (disk/systemd/process), push loop with cadence-based retry and a bounded local buffer, token handling. **K8s-fallback push split out to a tracked follow-up, not built this phase** — see ADR-010 | local checks produce correct payloads standalone (no backend needed); push loop delivers to a real backend; survives the backend being unreachable without crashing or blocking local checks | ✅ Complete |
| 9 | TUI dashboard — Ratatui event loop, widgets, **pure API client only (§11.4 resolved by ADR-009)**, embedded-local-backend bootstrap | panic restores terminal (subprocess test); widget snapshots; local-embedded and remote modes exercise the same client code path | ✅ Complete (also closed a pre-existing P3 gap: `monitra alert list`/`monitor history` didn't exist in the CLI even though the data did) |
| 10 | Web dashboard — React SPA, embedded via `rust-embed`, **consumes the identical API as TUI, no ADR-004 capability ceiling** | embedded server serves index; SPA exercises the same auth and full API surface TUI does | ✅ Complete (Globe view is a fixture/preview panel, not wired to live data — same reason TUI dropped its Globe screen entirely at Phase 9: the region data pipeline doesn't exist until Phase 11/ADR-011. Kubernetes view's pod-breakdown panel from the original design mockup was dropped for the same "no backing endpoint yet" reason, matching TUI's Kubernetes screen) |
| 11 | **Multi-region latency probing** (new, ADR-011) — `Agent.region`, `monitra-probe` extracted from `monitra-engine` and shared with `monitra-agent`, agent-executed network probes, read-side per-region aggregation | `monitra-probe` produces identical `ProbeOutcome`s in both `monitra-engine` and `monitra-agent`; a target monitored from N region-tagged agents aggregates correctly; an agent with no declared region is excluded from regional views, never defaulted | ✅ Complete (also closed a pre-existing bug: `dispatch_due` ignored `Monitor.agent_id` for every network-probe kind, dispatching them all centrally regardless — fixed as part of this phase, not left for later, since ADR-011 needed the routing anyway. Also unified `monitra-agent`'s `CheckOutcome` onto `monitra_probe::ProbeOutcome`, deleting the former: the DAG constraint that justified the duplicate is gone. TUI/web wiring for regions is still out of scope per this row's own gate — `monitra monitor regions` and `GET /regions` are the only surfaces) |
| 12 | Bundling — feature matrix (**`collector-kubernetes` gated, `monitra-agent` mode always in the default binary**), static musl, size, release CI | default build size re-verified against the added surface (§11.12) rather than assumed at the original 25 MB figure; `ldd` static; feature combos build in CI; §11.6 resolved | ✅ Complete (default release build measured at 7.0 MB stripped, static musl at 6.8 MB — both well under the 25 MB budget, §11.13 resolved; `[profile.release]` added to the workspace `Cargo.toml` for the first time this phase, keeping `panic = "unwind"` per ADR-012 rather than `"abort"`; that same audit found and fixed a real bug — the scheduler was silently dropping a panicked probe/collector task's result instead of recording it as `Unavailable`/`Unknown`, see ADR-012) |

### Beyond v1 (not committed)

- ~~Alerting integrations (webhook, email, Slack)~~ — pulled into v1 as notifier providers (ADR-007), sinks only
- ~~Postgres backend for multi-instance deployments~~ — pulled into v1 as a store provider (ADR-007)
- ~~Kubernetes/host introspection via agents~~ — pulled into v1 by ADR-008
- ~~Multi-region *latency* probing from multiple geographic vantage points~~ — pulled into v1 by ADR-011, tracked as Phase 11
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

### 11.3 ICMP requires elevated privileges — **resolved at Phase 6**

Raw sockets need root or `CAP_NET_RAW`. This conflicts with the frictionless-deployment thesis.

**Decision:** unprivileged ICMP via `SOCK_DGRAM`, falling back to a clear `Stale`/"unavailable" state rather than `Down` if even that fails. In practice this cost nothing to implement: `surge-ping`'s default `Config` already requests `SOCK_DGRAM` first and only falls back to `SOCK_RAW` (needs `CAP_NET_RAW`) if the kernel refuses it — `engine::probe::icmp::IcmpProber` just uses that default and, if `Client::new` fails outright (neither path available), never fails engine startup over it: every future ICMP probe on that host reports `ProbeOutcome::Unavailable`, which the scheduler maps to `MonitorStatus::Stale`, never `Down` (P1 requirement held).

### 11.4 TUI remote mode boundary — **resolved by ADR-009**

Originally: §3.2 stated `monitra-tui` reads via `monitra-storage` locally or HTTP remotely, with the abstraction reconciling the two left undesigned. ADR-009 resolves this by construction rather than by designing that abstraction: `monitra-tui` never reads `monitra-storage` in any mode. When no daemon is running, `main.rs` boots an embedded backend on loopback and `monitra-tui` talks to it exactly as it would a remote daemon. One client implementation. Left here, struck rather than deleted, per the document's own convention of keeping resolved reasoning visible.

### 11.5 Clock changes and DST

Scheduling anchored to absolute deadlines (§6.3.2) must use a monotonic clock, while `checked_at` timestamps must use wall time. Mixing these produces either mass simultaneous checks or scrambled history when the system clock steps. Straightforward to get right, easy to get wrong silently.

### 11.6 `panic = "abort"` vs. probe panic recovery — **resolved at Phase 12 (ADR-012)**

§8 wants `panic = "abort"` for binary size; §7.2 wants to catch probe-task panics and continue. These are in tension. Must be resolved before Phase 12 (Bundling — the phase that actually turns on the release profile's `panic = "abort"`; this note pointed at the old Phase 9 after v0.2, then Phase 11 after v0.3 — corrected to 12 during the v0.4/ADR-011 renumbering audit) — likely by ensuring probe code cannot panic in the first place rather than relying on catching it.

**Decision (ADR-012): `panic = "unwind"` is kept**, not `"abort"`. An audit at Phase 12 found the probe-execution path (`monitra-probe`, `crates/engine/src/scheduler.rs`'s dispatch) already free of panics in practice, but `panic = "unwind"` is kept anyway so a *future* panic there still isolates to one monitor rather than aborting the whole daemon — §7.2's promise stays literally true, not true "as long as nobody ever adds a bug." The same Phase-12 audit also found and fixed a real, independent bug: the scheduler's `join_next()` silently dropped the `Err(JoinError)` case (a panicked task vanished with no log and no recorded result). It now tracks task ownership by `tokio::task::Id` and records a proper `Unavailable`/`Unknown` outcome (→ `Stale`, never `Down`) on that path, logged loudly. See ADR-012 for the full alternatives considered.

### 11.7 `monitra setup` must not become mandatory

ADR-007 introduces a config file and a wizard. §1.2 promises first monitor firing in under 60 seconds with nothing to install and no config archaeology. These pull in opposite directions the moment any code path assumes config exists.

**The rule:** `monitra monitor add …` on a machine with no config file, no wizard run, and no environment variables must work, using embedded defaults throughout. If that test ever needs relaxing, ADR-007 was a mistake.

**Resolved at Phase 3 — config file location and precedence:** `$XDG_CONFIG_HOME/monitra/config.toml` (falling back to `$HOME/.config/monitra/config.toml` when unset), overridden by `./monitra.toml`, overridden by `MONITRA_*` env vars, overridden by CLI flags. `monitra_provider::config::resolve` implements this precedence as a pure function over an already-gathered `ConfigSources`, so it's tested without touching real env vars or `$HOME`. `setup`/`service attach|detach`/`k8s attach|detach` write only to the XDG path — the project-local file and env/flags are read-only override layers, never written by a command.

**Test-location note:** the pristine-`$HOME` assertion this section originally scheduled as a Phase 3 `monitor add` end-to-end test instead landed as a resolver-level test (`monitra_provider::tests::pristine_environment_resolves_to_defaults_throughout`) — `monitor add` has nothing real to execute against yet, since `monitra-storage`'s SQLite `Store` impl doesn't land until Phase 4. The literal end-to-end version of this assertion belongs in Phase 4, once there's a real `Store` for it to add against.

### 11.8 Live attach/detach of providers — **resolved at Phase 2**

ADR-007 registers providers at compile time and resolves them at startup. `monitra service attach slack://…` taking effect on a running daemon is deferred. Notifiers could support it easily; stores cannot without solving in-flight write drain and mid-flight migration state. **Risk:** if the CLI surface designed at Phase 2 implies liveness that Phase 3 does not deliver, the command names will be wrong. Decide the wording at Phase 2, not Phase 7.

**Decision:** `monitra service attach <url>` / `service detach <name>` / `service list`, generic across the `Store`/`Cache`/`Notifier` categories by URL scheme — the wording this section already used, kept deliberately. It does not imply liveness: the command edits config, and every category except possibly `Notifier` requires a `monitra start` restart to take effect, which the command's own help text says explicitly rather than leaving it implied. Nothing about live attach/detach on a running daemon is built by this wording; it only avoids naming a command in a way Phase 3 would have to contradict.

`Collector`/Kubernetes attach did **not** ride `service` — it got its own `monitra k8s attach/list/detach` family instead, because a cluster's real configuration (kubeconfig path, context, namespace/label scope) doesn't fit a single provider URL the way `postgres://`/`redis://`/`slack://` do. This also reads correctly given `Collector` has no default and no fallback (§4.1) — it is a distinct enough category from the URL-scheme three that a shared verb across all four would have forced an awkward encoding for no benefit.

Parse-only shape for both landed in Phase 2 (`crates/cli/src/service.rs`, `crates/cli/src/k8s.rs`); actual attach/detach behavior is still Phase 3's to build.

### 11.9 Feature-combination rot

Ten crates behind cargo features means the number of buildable configurations grows fast, and combinations nobody builds stop compiling silently. CI must build at minimum: default, all-features, and each provider feature alone. Not hard — just easy to skip until it breaks a release. (ADR-008 adds another feature-gated crate, `collector-kubernetes` — same discipline applies to it.)

### 11.10 Agent transport and wire protocol — **resolved at Phase 7, batch payload revised at Phase 8 (ADR-010, §11.16)**

ADR-008 decided *that* agents push results to `monitra-backend`'s ingest endpoint and authenticate doing so, not the specifics: whether that's plain HTTP+JSON, gRPC, or something else; the exact token/registration flow; whether an agent can watch more than one host or cluster per process. Deliberately left undecided at Phase 6 — these were Phase 7/8 implementation questions, not architecture, and answering them earlier would have been guessing ahead of the code.

**Decision, made at Phase 7 (backend side; the agent binary itself is Phase 8):**

- **Wire protocol: plain HTTP+JSON**, on the same Axum router as every other endpoint — `POST /agents/{id}/ingest`. Not gRPC: CLAUDE.md rules out a new heavyweight dependency (`tonic`/`prost`) for this, and every other surface in the project already speaks HTTP+JSON.
- **Auth: a per-agent revocable token**, not a reuse of the human `api_token` (§11.11) and not one shared secret across every agent. `Agent` gained a `token: String` field (migration `0006_agent_token`); `agent register` (CLI or `POST /agents`) always issues a fresh token — including on a repeat registration under the same name — shown exactly once in the response, mirroring §11.11's "generated once, never re-shown" pattern. Checked via `Authorization: Bearer <token>` against that one agent's own row; an unknown agent id and a wrong token both answer 401, never 404, so an unauthenticated caller can't use the route to enumerate agent ids. A leaked agent token only ever exposes that one agent's push path, and re-running `register` is how it gets rotated.
- **Batch payload**: one `POST /agents/{id}/ingest` carries `{"results": [{monitor_id, outcome}, …]}`, where `outcome` is a tagged `success{latency_ms} | failure{message} | unavailable{message}` (revised at Phase 8, ADR-010, §11.16 — originally a flat `success: bool`) — an agent performing several local checks per cycle (disk, systemd, process liveness) sends them together, plus one heartbeat update per push, not one HTTP call per check. Each result's `monitor_id` is cross-checked against `Monitor.agent_id` server-side before being forwarded; a mismatched or unknown `monitor_id` is logged and dropped, not trusted blindly from an authenticated-but-untrusted-content push (P1).
- **One process per agent, one `scope` per registration — confirmed at Phase 8.** Unchanged from the Phase 2 CLI shape (`agent register --scope`, `agent run --scope`); the agent binary built at Phase 8 watches exactly the one host or cluster its `--scope` names, one process per registration. Revisit only if a real need for one process watching several scopes shows up (P5) — nothing in Phase 8 needed it.

Pushed results are handed to `monitra-engine` through a new bounded `IngestHandle` (§6.2's "two producers, one bounded channel" — the scheduler's `select!` loop treats a pushed result exactly like a dispatched probe's `Outcome`, running it through the same flap-damping/persist/alert/writer/broadcast path) rather than writing to storage directly from `backend` — `backend` still never contains monitoring business logic (§4 `backend`'s "thin translation layer" contract).

### 11.11 Human-facing API auth mechanism — **resolved at Phase 5**

ADR-009 requires the backend's HTTP/WS surface to be authenticated but does not choose how: a static API key, a session/login flow, something else. Needed a decision before Phase 5, the same way §11.7 flagged config precedence needing a decision before Phase 3 — don't let Phase 5's handlers get built against a guessed-at auth shape.

**Decision: a single static bearer token per instance, not a session/login flow.** §1.4's target users are solo operators and small teams running one instance each, not a multi-tenant deployment — per-user identity, password hashing, and session expiry would be real surface area with no user this document names to justify it. Generated on first `monitra start` if none is configured, persisted to the XDG config (`api_token`, same precedence machinery as `store`/`cache`/`notifier`: xdg → project → `MONITRA_API_TOKEN` env), printed once, and never re-shown. Checked via `Authorization: Bearer <token>` against every route except `/health`, which must answer even when the token has been lost (P6). Compared in constant time (hand-rolled — a dependency the size of `subtle` didn't justify itself for one 64-byte compare). If a real multi-user deployment ever becomes a v1 target, this section is where that reversal gets recorded.

**Addendum, Phase 10 — browser `/ws` auth.** The web dashboard's `/ws` client is a browser `WebSocket`, which cannot set an `Authorization` header on the upgrade request (the TUI's `tokio-tungstenite` client can, and keeps doing so unchanged). `crates/backend/src/auth.rs`'s `require_token_ws` — used only by `/ws`, every other route keeps `require_token` — accepts the same static token via `Sec-WebSocket-Protocol` instead, and `ws::upgrade` echoes it back as the accepted subprotocol per the handshake spec. Considered and rejected: a query-string token (logged in server access logs and browser history — the static, long-lived token is exactly the credential that shouldn't end up there) and a short-lived one-time ticket minted by a new authenticated endpoint (more secure, but new state and a new endpoint for one route, not justified at this scale — §1.4 again). The token is hex (`provider::token::generate_api_token`), which is always a valid subprotocol token value with no escaping to worry about.

### 11.12 Kubernetes RBAC and kubeconfig handling — **resolved at Phase 6**

`collector-kubernetes` needs a kubeconfig or in-cluster service account, and the minimum RBAC surface it requires was unspecified.

**Decision:** two supported auth shapes, both bearer-token only (client-certificate and exec-plugin kubeconfig users produce a named `UnsupportedAuth` error, not a silent failure) — a kubeconfig file path, or the literal sentinel value `"in-cluster"` (`K8sClusterConfig.kubeconfig` is a required `String`, not `Option`, so this is the sentinel that keeps "no file, use the pod's own service account" expressible). RBAC surface is exactly `get` on Deployments/StatefulSets (`apps/v1`) and Services/Endpoints (`v1`) — Pods turned out not to be needed: Deployment/StatefulSet health is read from `status.readyReplicas` vs `spec.replicas` on the resource itself, and Service health additionally checks its Endpoints for at least one ready address rather than existence alone (a Service with zero backing pods must not read as healthy, P1). Full ClusterRole in `crates/collector-kubernetes/README.md`. Write access remains unneeded, confirming the standing answer.

**Also decided:** built on plain `reqwest` calls against the four REST endpoint shapes above, not the `kube`/`k8s-openapi` crates — those generate types for the entire API surface, a large transitive-dependency cost (relevant to §11.13's still-open size question) for four GETs. Revisit if a later phase needs more of the API (watches, CRDs).

### 11.13 Size budget under the expanded surface — **resolved at Phase 12**

§1.5 and §8 both quote "<25 MB stripped" as a success criterion, set before ADR-008/009 added a `Collector` provider, an `monitra-agent` mode, an authenticated API surface, and an `AlertEvent` table — and ADR-011 adds a `monitra-probe` crate and agent-executed network probes on top of that. **Undecided whether the number still holds.** Phase 12's gate should measure the actual default build, not assume the original figure — if it no longer holds, that is a finding to record honestly (per §6.4's own "report the real number, not a marketing adjective" ethos), not a reason to quietly redefine "default build."

**Measured at Phase 12, with the `[profile.release]` this phase added** (`opt-level = "z"`, `lto = true`, `codegen-units = 1`, `strip = true` — none of which existed in `Cargo.toml` before this phase; the default build previously used plain `cargo build --release` defaults):

- Default build (`cargo build --release`, default features — SQLite store, webhook notifier, `monitra-agent` mode, TUI, and the embedded web dashboard all compiled in): **7.0 MB stripped**, well inside the 25 MB budget with substantial headroom.
- Static build (`cargo build --release --target x86_64-unknown-linux-musl`): **6.8 MB stripped**, verified fully static (`readelf -d` shows no `NEEDED` entries; runs standalone).

The budget holds, with room to spare even after ADR-008/009/011's added surface — no redefinition needed. CI now reports the default build's size on every PR (`.github/workflows/ci.yml`) so a future regression is caught immediately rather than rediscovered at the next size audit.

### 11.14 Default SQLite database location — **resolved at Phase 5**

Nothing before Phase 5 specified where `monitra.db` lives by default — §3.1's diagram names the file but not its directory, and no phase needed a real answer until `monitra start` had to actually open one.

**Decision:** `$XDG_DATA_HOME/monitra/monitra.db`, falling back to `$HOME/.local/share/monitra/monitra.db` when unset — the same XDG fallback rule §11.7 already uses for config, but the *data* half of the spec rather than the *config* half, since a database is state, not configuration. `provider::default_db_path()` implements this; `main.rs` creates the directory if absent (the one thing `SqliteStore::open` itself doesn't do) before opening. No CLI override flag yet — `Start` has no `--db`; add one if a real need shows up rather than speculatively now (P5).

### 11.15 K8s-fallback push — deferred out of Phase 8 (ADR-010)

ADR-008's Phase-8 deliverable text originally listed "K8s-fallback push" alongside host checks, but the roadmap's own Gate column never tested it, and `monitra-agent` may never depend on `monitra-provider` (DAG) — so it cannot reuse `collector-kubernetes`'s REST-shape polling and would need an entirely separate, from-scratch Kubernetes-API client. Phase 8 built host checks (disk/systemd/process) and the push loop only; see ADR-010. **Undecided:** which phase picks this up, and whether it lives in `monitra-agent` as a self-contained K8s poller or is reconsidered as a different mechanism entirely (e.g. a lightweight relay that still goes through `collector-kubernetes`'s logic some other way). Revisit before claiming ADR-008's Kubernetes story is complete.

### 11.16 Agent-push wire protocol — extended to tri-state at Phase 8

§11.10's Phase-7 batch payload (`{monitor_id, success, latency_ms, message}`) had no way for a `HostAgentCheck` to report "I could not run this check" (permission denied, `systemctl`/dbus unreachable) distinctly from "I ran it and the target is down" — the exact honesty gap P1/§11.3 already closed on the pull path via `ProbeOutcome::Unavailable`. Phase 8 (ADR-010) extended `IngestRequest`/`PushedResultDto` to a tagged `outcome` (`success`/`failure`/`unavailable`), and `engine::PushedResult` now carries a `ProbeOutcome` directly instead of a flat `success: bool` — an agent-side `unavailable` routes to `MonitorStatus::Stale` through the same `mark_stale_and_record` path the pull side already used, bypassing flap damping. Resolved, not left open — recorded here per this document's convention of keeping resolved reasoning visible (§11.4's precedent).

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
    │   ├── build.rs             # npm ci && npm run build against ../../web (Phase 10)
    │   └── src/{lib,server,assets}.rs
    ├── cli/                    # clap argument definitions
    │   └── src/{lib,args}.rs
    ├── tui/                    # Ratatui terminal dashboard (pure API client, ADR-009)
    │   └── src/{lib,dashboard}.rs
    └── agent/                  # `monitra agent run` — local checks + push client (ADR-008)
        └── src/{lib,checks,push}.rs

# No separate crate for the web dashboard (Phase 10) — it's a React SPA at
# web/ (workspace root, sibling of crates/), built by npm and embedded via
# rust-embed into `monitra-backend` (`crates/backend/build.rs`), consuming
# the same API `monitra-tui` does. See ADR-009.
#
# web/                          # React 19 + Vite + TS SPA (Phase 10)
#   └── src/{App,auth,fixtures,main}.tsx, api/{client,types}.ts, globe/SciFiGlobe.tsx
```

---

*This document is revised at the end of each phase. When a decision recorded here is reversed, the ADR is superseded rather than deleted — the reasoning behind an abandoned choice is often more valuable than the choice itself.*
