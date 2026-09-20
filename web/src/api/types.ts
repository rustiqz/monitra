// Wire DTOs mirror `crates/backend/src/{monitors,agents,alerts,health}.rs`
// exactly (same field names/shapes the TUI's `crates/tui/src/client.rs`
// mirrors independently, per ADR-009 — "wire format independent of the
// internal model", and independent of each other by design: the DAG
// forbids the web dashboard depending on any Rust crate).
//
// PascalCase, not lowercase — `monitra_models::MonitorStatus` has no
// `#[serde(rename_all)]`, so serde emits the Rust variant name verbatim
// (confirmed against a real `/monitors` response, not assumed).
export type MonitorStatus = "Pending" | "Up" | "Down" | "Paused" | "Stale";
export type MonitorKind =
  | "Http"
  | "Tcp"
  | "Icmp"
  | "K8sDeployment"
  | "K8sStatefulSet"
  | "K8sService"
  | "HostAgentCheck";

export interface MonitorDto {
  id: number;
  name: string;
  target: string;
  kind: MonitorKind;
  interval_secs: number;
  status: MonitorStatus;
  agent_id: number | null;
}

export interface CheckResultDto {
  checked_at: number;
  success: boolean;
  latency_ms: number;
  message: string | null;
}

export interface AgentDto {
  id: number;
  name: string;
  last_heartbeat_at: number;
  scope: string;
}

export interface AlertEventDto {
  id: number;
  monitor_id: number;
  transitioned_to: MonitorStatus;
  occurred_at: number;
  sinks_attempted: string;
  delivery_outcome: string;
}

export interface HealthResponse {
  status: string;
  version: string;
  store: { name: string; reachable: boolean };
  cache: { name: string; degraded: boolean };
  notifier: { name: string; queue_len: number };
  k8s_clusters: string[];
}

// View-layer status, lowercase to match the existing CSS (`.status-up`,
// `.status-down`, …) and broader than `MonitorStatus`: a `Monitor` from the
// API is never "unknown" (the wire enum has no such variant — an unchecked
// monitor is `Pending`, a silent one's monitors are `Stale`, ADR-008 §5.2),
// but an `Agent`'s liveness and the (fixture-only, see `Region` below)
// Globe preview's regions both need a "signal lost" state that has no
// server-computed equivalent to read. `toSignal` converts a real
// `MonitorStatus` into this; `unknown` is only ever produced client-side.
export type Signal = "pending" | "up" | "down" | "paused" | "stale" | "unknown";

export function toSignal(status: MonitorStatus): Signal {
  switch (status) {
    case "Pending":
      return "pending";
    case "Up":
      return "up";
    case "Down":
      return "down";
    case "Paused":
      return "paused";
    case "Stale":
      return "stale";
  }
}
export type ViewId = "fleet" | "monitor" | "agents" | "kubernetes" | "alerts" | "health" | "services" | "globe";
export type GlobeMetric = "failure" | "latency" | "volume";

export interface Agent {
  id: number;
  name: string;
  scope: string;
  /// Client-derived from `last_heartbeat_at` at the same 90s threshold
  /// `engine::watchdog`'s own `heartbeat_timeout` default uses
  /// (`crates/engine/src/watchdog.rs`) — not re-read from the API (nothing
  /// exposes the daemon's configured value), so this is a display
  /// heuristic that happens to match the common case, not the source of
  /// truth for what actually goes `Stale` server-side.
  liveness: "up" | "unknown";
  heartbeatAgeSecs: number;
}

// Fixture-only preview data — see `fixtures.ts` and `GlobeView`'s banner in
// `App.tsx`. Real per-region data needs `Agent.region` and per-region
// aggregation, both Phase 11 / ADR-011 work; TUI (Phase 9) dropped this
// screen entirely for the same reason rather than fake it — the web
// dashboard keeps a clearly-labeled preview instead (Phase 10 brief).
export interface Region {
  code: string;
  site: string;
  longitude: number;
  latitude: number;
  status: Signal;
  monitors: number;
  p95: number | null;
  failure: number | null;
  volume: number | null;
  hourly: number[];
}

export interface Provider {
  slot: string;
  implementation: string;
  state: "ok" | "degraded" | "throttled";
  endpoint: string;
  detail: string;
}

export interface DashboardSnapshot {
  generatedAt: string;
  monitors: MonitorDto[];
  agents: Agent[];
  alerts: AlertEventDto[];
  // Never null once a `DashboardSnapshot` exists: `getSnapshot()` fetches
  // all four endpoints via `Promise.all` (`api/client.ts`), so any one
  // failing — including this one — fails the whole snapshot and keeps the
  // last good one on screen (P1: no partial, silently-stale field).
  health: HealthResponse;
  providers: Provider[];
  regions: Region[];
}

export interface DashboardSource {
  getSnapshot(): Promise<DashboardSnapshot>;
  getMonitorHistory(id: number): Promise<CheckResultDto[]>;
  /** Live `CheckResult` fan-out — used only to trigger an early re-poll, matching `monitra-tui`'s `WsFeed` (`crates/tui/src/data.rs`): the poll loop remains the single source of derived state, `/ws` just shortens the wait for the next one. */
  subscribeLive(onMessage: () => void): () => void;
}

export class ApiError extends Error {
  constructor(
    public status: number,
    url: string,
  ) {
    super(`request to ${url} failed with ${status}`);
    this.name = "ApiError";
  }
}
