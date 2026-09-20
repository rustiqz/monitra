import { fixtureSnapshot } from "../fixtures";
import { ApiError } from "./types";
import type {
  Agent,
  AgentDto,
  AlertEventDto,
  CheckResultDto,
  DashboardSnapshot,
  DashboardSource,
  HealthResponse,
  MonitorDto,
  Provider,
} from "./types";

const TOKEN_KEY = "monitra.web.token";

// Browser storage can throw (private mode, blocked site data) — guarded so
// a storage failure degrades to "ask for the token again," never a crash.
export function getToken(): string | null {
  try {
    return window.localStorage.getItem(TOKEN_KEY);
  } catch {
    return null;
  }
}

export function setToken(token: string): void {
  try {
    window.localStorage.setItem(TOKEN_KEY, token);
  } catch {
    // Session still works; it just re-prompts on the next load.
  }
}

export function clearToken(): void {
  try {
    window.localStorage.removeItem(TOKEN_KEY);
  } catch {
    // see setToken
  }
}

/** Used by the token-entry gate to check a token before storing it. */
export async function verifyToken(token: string): Promise<boolean> {
  const response = await fetch("/monitors", { headers: { Authorization: `Bearer ${token}` } });
  return response.ok;
}

async function getJson<T>(path: string): Promise<T> {
  const token = getToken();
  const response = await fetch(path, {
    headers: token ? { Authorization: `Bearer ${token}` } : {},
  });
  if (!response.ok) throw new ApiError(response.status, path);
  return response.json() as Promise<T>;
}

// Server-side truth for when an agent's monitors actually go `Stale` lives
// in `engine::watchdog`'s `heartbeat_timeout` (default 90s,
// `crates/engine/src/watchdog.rs`) — the API exposes only the raw
// `last_heartbeat_at`, not that configured value, so this mirrors the
// *default* rather than reading the real one. A display heuristic, not the
// source of truth.
const HEARTBEAT_TIMEOUT_SECS = 90;

function toAgent(dto: AgentDto): Agent {
  const ageSecs = Math.max(0, Math.floor(Date.now() / 1000) - dto.last_heartbeat_at);
  return {
    id: dto.id,
    name: dto.name,
    scope: dto.scope,
    liveness: ageSecs > HEARTBEAT_TIMEOUT_SECS ? "unknown" : "up",
    heartbeatAgeSecs: ageSecs,
  };
}

// Only what `/health` actually reports (DESIGN.md §4.1) — no fabricated
// endpoint URLs or byte counts the API doesn't expose.
function providersFromHealth(health: HealthResponse): Provider[] {
  const providers: Provider[] = [
    {
      slot: "Store",
      implementation: health.store.name,
      state: health.store.reachable ? "ok" : "degraded",
      endpoint: "",
      detail: health.store.reachable ? "reachable" : "unreachable — never silently falls back (§4.1)",
    },
    {
      slot: "Cache",
      implementation: health.cache.name,
      state: health.cache.degraded ? "degraded" : "ok",
      endpoint: "",
      detail: health.cache.degraded ? "degraded to the embedded default" : "active",
    },
    {
      slot: "Notifier",
      implementation: health.notifier.name,
      state: health.notifier.queue_len > 0 ? "throttled" : "ok",
      endpoint: "",
      detail: `${health.notifier.queue_len} queued`,
    },
  ];
  for (const cluster of health.k8s_clusters) {
    providers.push({ slot: "Collector", implementation: "Kubernetes", state: "ok", endpoint: cluster, detail: "attached" });
  }
  return providers;
}

/**
 * The real API client (ADR-009) — replaces the design branch's
 * `FixtureDashboardSource` behind the same `DashboardSource` interface, per
 * `web/README.md`'s original plan. `regions` stays fixture data: real
 * per-region metrics need `Agent.region` and per-region aggregation, both
 * Phase 11 / ADR-011 work (see `Region`'s doc comment in `./types`).
 */
class HttpDashboardSource implements DashboardSource {
  async getSnapshot(): Promise<DashboardSnapshot> {
    const [monitors, agentDtos, alerts, health] = await Promise.all([
      getJson<MonitorDto[]>("/monitors"),
      getJson<AgentDto[]>("/agents"),
      getJson<AlertEventDto[]>("/alerts"),
      getJson<HealthResponse>("/health"),
    ]);
    return {
      generatedAt: new Date().toISOString(),
      monitors,
      agents: agentDtos.map(toAgent),
      alerts,
      health,
      providers: providersFromHealth(health),
      regions: fixtureSnapshot.regions,
    };
  }

  async getMonitorHistory(id: number): Promise<CheckResultDto[]> {
    return getJson<CheckResultDto[]>(`/monitors/${id}/history`);
  }

  /**
   * Mirrors `monitra-tui`'s `WsFeed` (`crates/tui/src/data.rs`): `/ws`
   * pushes only trigger an early re-poll (`onMessage`), never patch state
   * in place — one derivation path for the snapshot, matching TUI/ADR-009's
   * "one client mental model." Reconnects on close with the same 3s delay
   * TUI's `WS_RETRY_DELAY` uses, unless `unsubscribe` was already called.
   */
  subscribeLive(onMessage: () => void): () => void {
    let stopped = false;
    let socket: WebSocket | null = null;

    const connect = () => {
      if (stopped) return;
      const token = getToken();
      if (!token) return;
      const wsUrl = `${window.location.origin.replace(/^http/, "ws")}/ws`;
      socket = new WebSocket(wsUrl, [token]);
      socket.onmessage = () => onMessage();
      socket.onclose = () => {
        if (!stopped) setTimeout(connect, 3000);
      };
      socket.onerror = () => socket?.close();
    };
    connect();

    return () => {
      stopped = true;
      socket?.close();
    };
  }
}

export const dashboardSource: DashboardSource = new HttpDashboardSource();
