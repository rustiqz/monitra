import { useEffect, useState, type ReactNode } from "react";
import { clearToken, dashboardSource, getToken } from "./api/client";
import { TokenGate } from "./auth";
import { SciFiGlobe } from "./globe/SciFiGlobe";
import {
  ApiError,
  toSignal,
  type AlertEventDto,
  type CheckResultDto,
  type DashboardSnapshot,
  type GlobeMetric,
  type MonitorKind,
  type MonitorStatus,
  type Provider,
  type Region,
  type Signal,
  type ViewId,
} from "./api/types";

// Matches `monitra-tui`'s `POLL_INTERVAL` (`crates/tui/src/data.rs`) — one
// client mental model, one polling cadence, per ADR-009. `/ws` pushes just
// trigger an early poll rather than patching state incrementally.
const POLL_INTERVAL_MS = 2000;

const navigation: Array<{ id: ViewId; label: string; icon: string }> = [
  { id: "fleet", label: "Fleet", icon: "⌁" },
  { id: "monitor", label: "Monitor", icon: "⌇" },
  { id: "agents", label: "Agents", icon: "◇" },
  { id: "kubernetes", label: "Kubernetes", icon: "⬡" },
  { id: "alerts", label: "Alerts", icon: "△" },
  { id: "health", label: "Health", icon: "✣" },
  { id: "services", label: "Services", icon: "⛓" },
  { id: "globe", label: "Globe", icon: "◎" },
];

function initialView(): ViewId {
  const candidate = window.location.hash.slice(1);
  return navigation.some((item) => item.id === candidate) ? (candidate as ViewId) : "fleet";
}

const statusMeta: Record<Signal, { glyph: string; label: string }> = {
  up: { glyph: "●", label: "Up" },
  down: { glyph: "✕", label: "Down" },
  pending: { glyph: "○", label: "Pending" },
  stale: { glyph: "◐", label: "Unconfirmed" },
  unknown: { glyph: "?", label: "Unknown" },
  paused: { glyph: "Ⅱ", label: "Paused" },
};

function kindLabel(kind: MonitorKind): string {
  switch (kind) {
    case "Http":
      return "HTTP";
    case "Tcp":
      return "TCP";
    case "Icmp":
      return "ICMP";
    case "K8sDeployment":
      return "K8S:DEP";
    case "K8sStatefulSet":
      return "K8S:STS";
    case "K8sService":
      return "K8S:SVC";
    case "HostAgentCheck":
      return "AGENT";
  }
}

function formatTime(unixSecs: number): string {
  return new Date(unixSecs * 1000).toLocaleTimeString([], { hour12: false });
}

function formatDuration(secs: number): string {
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m`;
  return `${Math.floor(secs / 3600)}h`;
}

function ago(unixSecs: number): string {
  return `${formatDuration(Math.max(0, Math.floor(Date.now() / 1000) - unixSecs))} ago`;
}

export function App() {
  const [authed, setAuthed] = useState(() => getToken() !== null);
  if (!authed) {
    return <TokenGate onAuthenticated={() => setAuthed(true)} />;
  }
  return <Dashboard onAuthError={() => setAuthed(false)} />;
}

function Dashboard({ onAuthError }: { onAuthError: () => void }) {
  const [snapshot, setSnapshot] = useState<DashboardSnapshot | null>(null);
  const [lastError, setLastError] = useState<string | null>(null);
  const [activeView, setActiveView] = useState<ViewId>(initialView);
  const [search, setSearch] = useState("");
  const [selectedMonitorId, setSelectedMonitorId] = useState<number | null>(null);
  const [mobileNav, setMobileNav] = useState(false);

  useEffect(() => {
    let current = true;
    const refresh = () => {
      dashboardSource.getSnapshot().then(
        (next) => {
          if (!current) return;
          setSnapshot(next);
          setLastError(null);
        },
        (error: unknown) => {
          if (!current) return;
          if (error instanceof ApiError && error.status === 401) {
            clearToken();
            onAuthError();
            return;
          }
          // A transient failure (daemon restart, network blip) keeps the
          // last snapshot on screen rather than blanking it — but the
          // connection indicator below stops claiming "Live" (P1: never
          // imply confidence a display doesn't have).
          setLastError(error instanceof Error ? error.message : "request failed");
        },
      );
    };
    refresh();
    const interval = window.setInterval(refresh, POLL_INTERVAL_MS);
    const unsubscribe = dashboardSource.subscribeLive(refresh);
    return () => {
      current = false;
      window.clearInterval(interval);
      unsubscribe();
    };
  }, [onAuthError]);

  if (!snapshot) {
    return (
      <div className="boot-screen">
        <span className="brand-mark">M</span>
        <p>Connecting to Monitra…</p>
      </div>
    );
  }

  const openView = (view: ViewId) => {
    setActiveView(view);
    window.history.replaceState(null, "", `#${view}`);
    setMobileNav(false);
  };

  const selectMonitor = (id: number) => {
    setSelectedMonitorId(id);
    openView("monitor");
  };

  return (
    <div className="app-shell">
      <Sidebar active={activeView} open={mobileNav} onNavigate={openView} live={lastError === null} />
      <main className="workspace">
        <Topbar snapshot={snapshot} search={search} onSearch={setSearch} onMenu={() => setMobileNav((v) => !v)} live={lastError === null} />
        <div className="page-scroll">
          {activeView === "fleet" && <FleetView snapshot={snapshot} search={search} onSelect={selectMonitor} onOpenGlobe={() => openView("globe")} />}
          {activeView === "monitor" && <MonitorView snapshot={snapshot} selectedId={selectedMonitorId} onSelect={selectMonitor} />}
          {activeView === "agents" && <AgentsView snapshot={snapshot} />}
          {activeView === "kubernetes" && <KubernetesView snapshot={snapshot} />}
          {activeView === "alerts" && <AlertsView snapshot={snapshot} />}
          {activeView === "health" && <HealthView snapshot={snapshot} />}
          {activeView === "services" && <ServicesView providers={snapshot.providers} />}
          {activeView === "globe" && <GlobeView regions={snapshot.regions} />}
        </div>
      </main>
    </div>
  );
}

function Sidebar({ active, open, onNavigate, live }: { active: ViewId; open: boolean; onNavigate: (view: ViewId) => void; live: boolean }) {
  return (
    <aside className={`sidebar ${open ? "sidebar-open" : ""}`}>
      <div className="identity"><span className="brand-mark">M</span><div><strong>MONITRA</strong><small>MISSION CONTROL</small></div></div>
      <nav aria-label="Primary navigation">
        {navigation.map((item) => (
          <button key={item.id} className={active === item.id ? "nav-item active" : "nav-item"} onClick={() => onNavigate(item.id)}>
            <span className="nav-icon">{item.icon}</span><span>{item.label}</span>
          </button>
        ))}
      </nav>
      <div className="sidebar-foot">
        <div className="connection"><span className={live ? "pulse" : "pulse pulse-down"} /><div><strong>{live ? "Live stream" : "Connection lost"}</strong><small>{live ? `polling every ${POLL_INTERVAL_MS / 1000}s` : "showing last known state"}</small></div></div>
      </div>
    </aside>
  );
}

function Topbar({ snapshot, search, onSearch, onMenu, live }: { snapshot: DashboardSnapshot; search: string; onSearch: (value: string) => void; onMenu: () => void; live: boolean }) {
  return (
    <header className="topbar">
      <button className="menu-button" onClick={onMenu} aria-label="Toggle navigation">☰</button>
      <label className="global-search"><span>⌕</span><input value={search} onChange={(event) => onSearch(event.target.value)} placeholder="Search monitors, targets…" /><kbd>/</kbd></label>
      <div className="topbar-status"><span className={live ? "live-dot" : "live-dot live-dot-down"} /><span>{live ? "Live" : "Reconnecting"}</span><i /><span className="clock">{new Date(snapshot.generatedAt).toLocaleTimeString([], { hour12: false })}</span></div>
    </header>
  );
}

function PageHeader({ eyebrow, title, description, actions }: { eyebrow: string; title: string; description: string; actions?: ReactNode }) {
  return <div className="page-header"><div><span className="eyebrow">{eyebrow}</span><h1>{title}</h1><p>{description}</p></div>{actions && <div className="header-actions">{actions}</div>}</div>;
}

function Panel({ title, subtitle, action, className = "", children }: { title: string; subtitle?: string; action?: ReactNode; className?: string; children: ReactNode }) {
  return <section className={`panel ${className}`}><header><div><h2>{title}</h2>{subtitle && <p>{subtitle}</p>}</div>{action}</header><div className="panel-body">{children}</div></section>;
}

function StatusPill({ status, compact = false }: { status: Signal; compact?: boolean }) {
  const meta = statusMeta[status];
  return <span className={`status-pill status-${status} ${compact ? "compact" : ""}`}><b>{meta.glyph}</b>{!compact && meta.label}</span>;
}

function MetricCard({ label, value, status, detail }: { label: string; value: number; status: Signal; detail: string }) {
  return <article className={`metric-card metric-${status}`}><div className="metric-top"><span>{label}</span><StatusPill status={status} compact /></div><strong className="metric-value">{String(value).padStart(2, "0")}</strong><div className="metric-foot"><span>{detail}</span></div></article>;
}

function FleetView({ snapshot, search, onSelect, onOpenGlobe }: { snapshot: DashboardSnapshot; search: string; onSelect: (id: number) => void; onOpenGlobe: () => void }) {
  const filtered = snapshot.monitors.filter((monitor) => `${monitor.name} ${monitor.kind} ${monitor.target}`.toLowerCase().includes(search.toLowerCase()));
  const total = snapshot.monitors.length || 1;
  const count = (status: MonitorStatus) => snapshot.monitors.filter((monitor) => monitor.status === status).length;
  const pct = (n: number) => `${Math.round((n / total) * 100)}% of fleet`;

  const silentAgentIds = new Set(snapshot.agents.filter((agent) => agent.liveness === "unknown").map((agent) => agent.id));
  const affectedBySilentAgents = snapshot.monitors.filter((m) => m.agent_id !== null && silentAgentIds.has(m.agent_id)).length;
  const pendingCount = count("Pending");

  return <>
    <PageHeader eyebrow="Overview · all systems" title="Fleet command" description="One honest view of every probe, collector, and attached resource." />
    <div className="metric-grid">
      <MetricCard label="Confirmed up" value={count("Up")} status="up" detail={pct(count("Up"))} />
      <MetricCard label="Confirmed down" value={count("Down")} status="down" detail={pct(count("Down"))} />
      <MetricCard label="Unconfirmed" value={count("Stale")} status="stale" detail={pct(count("Stale"))} />
      <MetricCard label="Never checked" value={pendingCount} status="pending" detail={pct(pendingCount)} />
    </div>
    <div className="content-grid fleet-grid">
      <Panel title="Monitor fleet" subtitle={`${filtered.length} visible of ${snapshot.monitors.length} · confidence is never inferred`} className="fleet-table-panel">
        <div className="table-wrap"><table><thead><tr><th>Status</th><th>Monitor</th><th>Kind</th><th>Target</th><th>Fed by</th></tr></thead><tbody>
          {filtered.map((monitor) => <tr key={monitor.id} onClick={() => onSelect(monitor.id)} tabIndex={0}><td><StatusPill status={toSignal(monitor.status)} /></td><td><strong>{monitor.name}</strong></td><td><span className="kind-tag">{kindLabel(monitor.kind)}</span></td><td className="target-cell">{monitor.target}</td><td className="confidence">{monitor.agent_id !== null ? `agent #${monitor.agent_id}` : "direct"}</td></tr>)}
        </tbody></table></div>
      </Panel>
      <div className="side-stack">
        <Panel title="Probe geography" subtitle={`${snapshot.regions.length} source regions · preview`} action={<button className="text-button" onClick={onOpenGlobe}>Open globe →</button>}>
          <p className="truth-note" style={{ padding: "0 2px" }}>Regional latency probing lands with Phase 11 (ADR-011). This is a design preview, not live data.</p>
        </Panel>
        <Panel title="Why some are not results" className="explain-panel">
          <div className="explain-row amber"><b>{pendingCount}</b><div><strong>Never checked</strong><span>Awaiting first probe</span></div></div>
          <div className="explain-row violet"><b>{affectedBySilentAgents}</b><div><strong>Source signal lost</strong><span>Fed by an agent silent &gt;90s</span></div></div>
          <p className="truth-note">None of these are reported as down.</p>
        </Panel>
      </div>
    </div>
  </>;
}

function MonitorView({ snapshot, selectedId, onSelect }: { snapshot: DashboardSnapshot; selectedId: number | null; onSelect: (id: number) => void }) {
  const monitor = snapshot.monitors.find((item) => item.id === selectedId) ?? snapshot.monitors[0];
  const [history, setHistory] = useState<CheckResultDto[]>([]);

  useEffect(() => {
    if (!monitor) return;
    let current = true;
    dashboardSource.getMonitorHistory(monitor.id).then(
      (results) => {
        if (current) setHistory(results);
      },
      () => {
        if (current) setHistory([]);
      },
    );
    return () => {
      current = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [monitor?.id]);

  if (!monitor) {
    return <PageHeader eyebrow="Monitor detail" title="No monitors yet" description="`monitra monitor add` to create the first one." />;
  }

  const latencies = history.map((r) => r.latency_ms);
  const successes = history.filter((r) => r.success).length;
  const uptimePct = history.length ? (successes / history.length) * 100 : null;
  const recent = [...history].reverse().slice(0, 6);
  const latest = history.at(-1);

  return <>
    <PageHeader eyebrow="Monitor detail" title={monitor.name} description={monitor.target} actions={<select value={monitor.id} onChange={(event) => onSelect(Number(event.target.value))}>{snapshot.monitors.map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}</select>} />
    <div className="monitor-hero">
      <div><StatusPill status={toSignal(monitor.status)} /><h2>{statusMeta[toSignal(monitor.status)].label}</h2><p>{latest ? `last checked ${ago(latest.checked_at)}` : "no checks recorded yet"}</p></div>
      <div className="monitor-meta"><span>Kind<strong>{kindLabel(monitor.kind)}</strong></span><span>Interval<strong>{monitor.interval_secs}s</strong></span><span>Collector<strong>{monitor.agent_id !== null ? `agent #${monitor.agent_id}` : "local"}</strong></span><span>Samples retained<strong>{history.length}</strong></span></div>
    </div>
    <div className="content-grid detail-grid">
      <Panel title="Latency" subtitle={`${history.length} retained samples · milliseconds`}><LargeChart values={latencies} /></Panel>
      <Panel title="Confirmed uptime" subtitle="Over retained samples only"><div className="uptime-ring"><strong>{uptimePct === null ? "—" : uptimePct.toFixed(2)}{uptimePct !== null && <small>%</small>}</strong><span>{history.length} samples</span></div><dl className="key-values"><div><dt>Successes</dt><dd>{successes} / {history.length}</dd></div><div><dt>Failures</dt><dd className={history.length - successes > 0 ? "amber-text" : ""}>{history.length - successes}</dd></div></dl></Panel>
    </div>
    <Panel title="Recent check results" subtitle="Raw outcomes from the selected collector"><div className="event-list">{recent.length === 0 && <p className="truth-note">No checks recorded yet.</p>}{recent.map((result) => <div key={result.checked_at} className={result.success ? "event green" : "event red"}><span>{result.success ? "✓" : "✕"}</span><p>{formatTime(result.checked_at)} · {result.success ? "ok" : "failed"} · {result.latency_ms}ms{result.message ? ` · ${result.message}` : ""}</p></div>)}</div></Panel>
  </>;
}

function AgentsView({ snapshot }: { snapshot: DashboardSnapshot }) {
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const agent = snapshot.agents.find((item) => item.id === selectedId) ?? snapshot.agents[0];
  const feedCount = (agentId: number) => snapshot.monitors.filter((m) => m.agent_id === agentId).length;

  return <>
    <PageHeader eyebrow="Remote collection" title="Agents" description="Liveness is its own signal; a silent agent never turns its targets red." />
    <div className="content-grid agents-grid">
      <Panel title="Registered agents" subtitle={`${snapshot.agents.length} total`}>
        <div className="card-list">
          {snapshot.agents.length === 0 && <p className="truth-note" style={{ padding: "10px 4px" }}>No agents registered — `monitra agent register`.</p>}
          {snapshot.agents.map((item) => <button key={item.id} onClick={() => setSelectedId(item.id)} className={agent?.id === item.id ? "agent-card selected" : "agent-card"}><StatusPill status={item.liveness} compact /><div><strong>{item.name}</strong><span>{item.scope}</span></div><div><strong>{formatDuration(item.heartbeatAgeSecs)} ago</strong><span>{feedCount(item.id)} monitor(s)</span></div><b>›</b></button>)}
        </div>
      </Panel>
      {agent && <Panel title="Blast radius" subtitle={`Selected · ${agent.name}`} className="agent-detail">
        <div className="signal-orbit"><div className={`orbit-core status-bg-${agent.liveness}`}>{statusMeta[agent.liveness].glyph}</div><i /><i /><i /></div>
        <h3>{agent.liveness === "unknown" ? "Signal lost" : "Alive"}</h3>
        <p>{agent.name} last reported {formatDuration(agent.heartbeatAgeSecs)} ago. Its dependent monitors remain explicitly unknown until the source returns.</p>
        <dl className="key-values"><div><dt>Scope</dt><dd>{agent.scope}</dd></div><div><dt>Feeds</dt><dd>{feedCount(agent.id)} monitor(s)</dd></div><div><dt>Alert policy</dt><dd>agent-silent only</dd></div></dl>
      </Panel>}
    </div>
  </>;
}

function KubernetesView({ snapshot }: { snapshot: DashboardSnapshot }) {
  const resources = snapshot.monitors.filter((monitor) => monitor.kind.startsWith("K8s"));
  const clusters = snapshot.health.k8s_clusters;
  return <>
    <PageHeader eyebrow="Orchestrators" title="Kubernetes" description="Cluster health and watched workloads — pod-level breakdown isn't built yet (nothing in the API surface asks for it, DESIGN.md §4 `collector-kubernetes`)." />
    <div className="cluster-strip">
      {clusters.length === 0 && <article className="cluster"><span className="unknown-dot">?</span><div><strong>none attached</strong><small>`monitra k8s attach`</small></div></article>}
      {clusters.map((cluster) => <article key={cluster} className="cluster active"><span className="live-dot" /><div><strong>{cluster}</strong><small>attached</small></div><em>Attached</em></article>)}
    </div>
    <Panel title="Watched resources" subtitle={`${resources.length} K8s-kind monitors`}>
      <div className="resource-list">
        {resources.length === 0 && <p className="truth-note" style={{ padding: "10px 4px" }}>No Kubernetes-kind monitors — `monitra monitor add --kind k8s-deployment ...`.</p>}
        {resources.map((resource) => <div key={resource.id}><StatusPill status={toSignal(resource.status)} /><div><strong>{resource.name}</strong><span>{resource.target}</span></div><em>{kindLabel(resource.kind)}</em></div>)}
      </div>
    </Panel>
  </>;
}

function AlertsView({ snapshot }: { snapshot: DashboardSnapshot }) {
  const [query, setQuery] = useState("");
  const monitorName = (id: number) => snapshot.monitors.find((m) => m.id === id)?.name ?? `monitor #${id}`;
  const filtered = snapshot.alerts.filter((alert) => `${monitorName(alert.monitor_id)} ${alert.sinks_attempted} ${alert.delivery_outcome}`.toLowerCase().includes(query.toLowerCase()));

  return <>
    <PageHeader eyebrow="Delivery ledger" title="Alert history" description="Every transition and every sink outcome remains queryable." />
    <label className="query-bar"><span>⌕</span><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Filter by monitor or sink…" /><small>{filtered.length} events</small></label>
    <Panel title="Alert events" subtitle="Persisted before notification delivery">
      <div className="table-wrap"><table><thead><tr><th>Occurred</th><th>Monitor</th><th>New status</th><th>Sinks</th><th>Delivery</th></tr></thead><tbody>
        {filtered.map((alert) => <AlertRow key={alert.id} alert={alert} monitorName={monitorName(alert.monitor_id)} />)}
      </tbody></table></div>
    </Panel>
  </>;
}

function AlertRow({ alert, monitorName }: { alert: AlertEventDto; monitorName: string }) {
  const risky = alert.delivery_outcome.includes("429") || alert.delivery_outcome.toLowerCase().includes("retry");
  return <tr><td>{formatTime(alert.occurred_at)}</td><td><strong>{monitorName}</strong></td><td><StatusPill status={toSignal(alert.transitioned_to)} /></td><td>{alert.sinks_attempted}</td><td className={risky ? "amber-text" : "green-text"}>{alert.delivery_outcome}</td></tr>;
}

function HealthView({ snapshot }: { snapshot: DashboardSnapshot }) {
  const health = snapshot.health;
  const degradedCount = [!health.store.reachable, health.cache.degraded, health.notifier.queue_len > 0].filter(Boolean).length;
  return <>
    <PageHeader eyebrow="Daemon internals" title="System health" description="Internal failures live here, never disguised as failed probes." actions={degradedCount > 0 ? <span className="health-banner">◐ Degraded · {degradedCount} provider(s)</span> : <span className="health-banner" style={{ color: "var(--green)", borderColor: "rgba(92,224,160,.3)", background: "rgba(92,224,160,.06)" }}>● All providers nominal</span>} />
    <div className="content-grid health-grid">
      <Panel title="Provider health" subtitle="Explicit degradation policy (§4.1)"><ProviderRows providers={snapshot.providers} /></Panel>
      <Panel title="Daemon"><dl className="key-values"><div><dt>Version</dt><dd>{health.version}</dd></div><div><dt>Status</dt><dd>{health.status}</dd></div><div><dt>Notifier queue</dt><dd>{health.notifier.queue_len}</dd></div><div><dt>K8s clusters attached</dt><dd>{health.k8s_clusters.length}</dd></div></dl></Panel>
    </div>
  </>;
}

function ServicesView({ providers }: { providers: Provider[] }) {
  return <><PageHeader eyebrow="Provider topology" title="Attached services" description="The visual equivalent of `monitra service list`, with runtime state." /><Panel title="Provider registry" subtitle="From /health — configuration lives in monitra.toml"><ProviderRows providers={providers} detailed /></Panel></>;
}

function ProviderRows({ providers, detailed = false }: { providers: Provider[]; detailed?: boolean }) {
  return <div className="provider-list">{providers.map((provider, index) => <div key={`${provider.slot}-${index}`}><span className={`provider-state ${provider.state}`}>{provider.state === "ok" ? "●" : "◐"}</span><div><small>{provider.slot}</small><strong>{provider.implementation}</strong></div>{detailed && <code>{provider.endpoint}</code>}<p>{provider.detail}</p><em className={provider.state}>{provider.state}</em></div>)}</div>;
}

function GlobeView({ regions }: { regions: Region[] }) {
  const [metric, setMetric] = useState<GlobeMetric>("failure");
  const [selected, setSelected] = useState("lhr");
  const region = regions.find((item) => item.code === selected) ?? regions[0];
  return <>
    <PageHeader eyebrow="Probe geography" title="Regional signal" description="Separate path health from target health across every source region." actions={<div className="segment-control">{(["failure", "latency", "volume"] as GlobeMetric[]).map((item) => <button key={item} onClick={() => setMetric(item)} className={metric === item ? "active" : ""}>{item === "failure" ? "Failure rate" : item === "latency" ? "P95 latency" : "Probe volume"}</button>)}</div>} />
    <div className="globe-preview-banner">◐ PREVIEW — design data, not live. Real regional probing lands with Phase 11 (ADR-011: <code>Agent.region</code> + per-region aggregation).</div>
    <div className="globe-layout">
      <Panel title="Source regions" subtitle="Circle size = volume · fill = selected metric · drag to rotate" className="map-panel">
        <SciFiGlobe regions={regions} metric={metric} selected={selected} onSelect={setSelected} />
      </Panel>
      <div className="globe-sidebar">
        <Panel title={`Selected · ${region.code.toUpperCase()}`} subtitle={region.site}>
          <div className="selected-region">
            <StatusPill status={region.status} /><strong>{region.status === "unknown" ? "Signal lost" : statusMeta[region.status].label}</strong>
            <p>{region.status === "unknown" ? "Source stopped heartbeating. Its monitors are unknown; no target outage was inferred." : `${region.monitors} monitors are reporting from this location.`}</p>
            <dl className="key-values"><div><dt>P95 latency</dt><dd>{region.p95 ? `${region.p95}ms` : "—"}</dd></div><div><dt>Failure · 24h</dt><dd>{region.failure === null ? "—" : `${region.failure.toFixed(1)}%`}</dd></div><div><dt>Probe volume</dt><dd>{region.volume?.toLocaleString() ?? "—"}</dd></div></dl>
          </div>
        </Panel>
        <Panel title="Reading the globe"><ul className="map-legend"><li><i className="heat-ramp" /><span><strong>Marker fill</strong>Selected metric, normalized across reporting regions</span></li><li><i className="empty-key" /><span><strong>Dim marker</strong>No probe signal; never rendered as a cold success</span></li></ul></Panel>
      </div>
    </div>
    <Panel title="Region × hour" subtitle={`${metric === "failure" ? "Probe failure rate" : metric === "latency" ? "P95 latency" : "Probe volume"} · last 24 hours`}><RegionHeatmap regions={regions} metric={metric} /></Panel>
  </>;
}

function RegionHeatmap({ regions, metric }: { regions: Region[]; metric: GlobeMetric }) {
  const max = Math.max(...regions.flatMap((region) => region.hourly), 1);
  return <div className="heatmap"><div className="heat-hours"><span>00:00</span><span>06:00</span><span>12:00</span><span>18:00</span><span>23:00</span></div>{regions.map((region) => <div className="heat-row" key={region.code}><strong className={`status-text-${region.status}`}>{region.code}</strong><div>{region.hourly.map((value, hour) => { const unavailable = region.status === "unknown" || region.status === "pending"; const stale = region.status === "stale" && hour > 21; const metricScale = metric === "failure" ? 1 : metric === "latency" ? (region.p95 ?? 0) / 120 : (region.volume ?? 0) / 1200; return <i key={hour} className={unavailable ? "no-signal" : stale ? "stale-cell" : ""} style={!unavailable && !stale ? { background: heatColor(Math.min(1, (value * metricScale) / max)) } : undefined} title={`${hour}:00 · ${unavailable ? "no signal" : formatMetric(value * metricScale, metric)}`} />; })}</div></div>)}<div className="heat-legend"><span>Low</span><i /><i /><i /><i /><i /><span>High</span><b>░ No signal</b></div></div>;
}

function LargeChart({ values }: { values: number[] }) {
  if (values.length === 0) {
    return <div className="large-chart"><p className="truth-note" style={{ padding: "16px 20px" }}>No samples yet.</p></div>;
  }
  const max = Math.max(...values, 1);
  const points = values.map((value, index) => `${(index / Math.max(1, values.length - 1)) * 800},${220 - (value / max) * 180}`).join(" ");
  return <div className="large-chart"><svg viewBox="0 0 800 250" preserveAspectRatio="none"><defs><linearGradient id="chart-fill" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stopColor="#55d7e8" stopOpacity=".28" /><stop offset="1" stopColor="#55d7e8" stopOpacity="0" /></linearGradient></defs><g className="chart-grid"><path d="M0 40H800M0 100H800M0 160H800M0 220H800" /></g><polygon points={`0,240 ${points} 800,240`} fill="url(#chart-fill)" /><polyline points={points} fill="none" stroke="#55d7e8" strokeWidth="3" vectorEffect="non-scaling-stroke" /></svg></div>;
}

function formatMetric(value: number, metric: GlobeMetric): string {
  if (metric === "failure") return `${value.toFixed(1)}%`;
  if (metric === "latency") return `${Math.round(value)}ms`;
  return `${Math.round(value).toLocaleString()} probes`;
}

function heatColor(value: number): string {
  const ramp = ["#17323c", "#1d5960", "#2e887c", "#6db269", "#c9b24a", "#e88f52", "#f0708a"];
  return ramp[Math.min(ramp.length - 1, Math.max(0, Math.floor(value * ramp.length)))];
}
