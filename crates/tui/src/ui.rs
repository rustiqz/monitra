//! Widget rendering (DESIGN.md §4 `tui`). Layout/panel styling carried over
//! from the `feat/tui-design` visual reference; every screen now renders
//! from `App`'s real `DashboardSnapshot` instead of static fixtures, and
//! the Globe screen is dropped (its data pipeline is Phase 11, ADR-011).

use std::time::{SystemTime, UNIX_EPOCH};

use monitra_models::{Monitor, MonitorKind, MonitorStatus};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Sparkline, Table, Wrap};

use crate::app::{App, Screen};
use crate::theme;

pub fn render(frame: &mut Frame<'_>, app: &App) {
    let area = frame.area();
    frame.render_widget(Block::default().style(Style::default().bg(theme::BG)), area);
    if area.width < 58 || area.height < 16 {
        frame.render_widget(
            Paragraph::new("MONITRA needs at least 58×16\nresize terminal · q quit")
                .alignment(Alignment::Center)
                .style(theme::title()),
            area,
        );
        return;
    }

    let shell = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Min(8),
        Constraint::Length(1),
    ])
    .split(area);
    render_header(frame, shell[0], app);
    render_tabs(frame, shell[1], app);

    let content = shell[2].inner(Margin::new(1, 1));
    match app.screen {
        Screen::Fleet => render_fleet(frame, content, app),
        Screen::Monitor => render_monitor(frame, content, app),
        Screen::Agents => render_agents(frame, content, app),
        Screen::Kubernetes => render_kubernetes(frame, content, app),
        Screen::Alerts => render_alerts(frame, content, app),
        Screen::Health => render_health(frame, content, app),
        Screen::Services => render_services(frame, content, app),
    }
    render_footer(frame, shell[3], app.screen);

    if app.show_help {
        render_help(frame, centered(64, 20, area));
    } else if app.show_add {
        render_add_monitor(frame, centered(72, 22, area));
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn fmt_ago(now: u64, then: u64) -> String {
    let delta = now.saturating_sub(then);
    match delta {
        0..=90 => format!("{delta}s ago"),
        91..=5399 => format!("{}m ago", delta / 60),
        5400..=169199 => format!("{}h ago", delta / 3_600),
        _ => format!("{}d ago", delta / 86_400),
    }
}

fn status_label(status: MonitorStatus) -> &'static str {
    match status {
        MonitorStatus::Up => "● UP",
        MonitorStatus::Down => "✕ DOWN",
        MonitorStatus::Pending => "◌ PENDING",
        MonitorStatus::Paused => "‖ PAUSED",
        MonitorStatus::Stale => "? STALE",
    }
}

fn kind_label(kind: MonitorKind) -> &'static str {
    match kind {
        MonitorKind::Http => "HTTP",
        MonitorKind::Tcp => "TCP",
        MonitorKind::Icmp => "ICMP",
        MonitorKind::K8sDeployment => "K8S:DEP",
        MonitorKind::K8sStatefulSet => "K8S:STS",
        MonitorKind::K8sService => "K8S:SVC",
        MonitorKind::HostAgentCheck => "AGENT",
    }
}

fn render_header(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let status = match &app.snapshot.last_error {
        Some(error) => Span::styled(format!("△ {error}"), Style::default().fg(theme::AMBER)),
        None => Span::styled(
            format!("backend  {}", app.endpoint),
            Style::default().fg(theme::MUTED),
        ),
    };
    let line = Line::from(vec![
        Span::styled(" MONITRA ", theme::title()),
        Span::styled("│ ", theme::dim()),
        Span::styled(app.screen.title(), Style::default().fg(theme::MUTED)),
        Span::raw("  "),
        status,
    ]);
    frame.render_widget(
        Paragraph::new(line)
            .style(Style::default().bg(theme::BAR).fg(theme::TEXT))
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(theme::dim()),
            ),
        area,
    );
}

fn render_tabs(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let mut spans = Vec::new();
    for (index, screen) in Screen::ALL.iter().enumerate() {
        let label = format!(" {} {} ", index + 1, screen.title());
        let style = if *screen == app.screen {
            theme::selected()
        } else {
            theme::dim()
        };
        spans.push(Span::styled(label, style));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(Color::Rgb(9, 13, 18))),
        area,
    );
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, screen: Screen) {
    let keys = match screen {
        Screen::Fleet => " ↑↓ move   ⏎ detail   a add   ? keys   q quit",
        _ => " ↑↓ move   ←→/tab section   ? keys   q quit",
    };
    frame.render_widget(
        Paragraph::new(keys).style(Style::default().fg(theme::CYAN).bg(theme::BAR)),
        area,
    );
}

fn panel(title: impl Into<String>) -> Block<'static> {
    Block::default()
        .title(Span::styled(format!(" {} ", title.into()), theme::title()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER))
        .style(Style::default().bg(theme::BG).fg(theme::TEXT))
}

fn table_header<const N: usize>(labels: [&'static str; N]) -> Row<'static> {
    Row::new(labels.map(|label| Cell::from(label).style(theme::dim()))).height(1)
}

fn metric<'a>(value: String, label: &'a str, color: Color) -> Span<'a> {
    Span::styled(
        format!(" {value:>3} {label:<7} "),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

fn render_fleet(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let chunks = Layout::vertical([Constraint::Length(3), Constraint::Min(8)]).split(area);

    let monitors = &app.snapshot.monitors;
    let count = |status: MonitorStatus| monitors.iter().filter(|m| m.status == status).count();
    let rollup = Line::from(vec![
        metric(count(MonitorStatus::Up).to_string(), "UP", theme::GREEN),
        metric(count(MonitorStatus::Down).to_string(), "DOWN", theme::RED),
        metric(
            count(MonitorStatus::Pending).to_string(),
            "PENDING",
            theme::PENDING,
        ),
        metric(
            count(MonitorStatus::Stale).to_string(),
            "STALE",
            theme::VIOLET,
        ),
        metric(
            count(MonitorStatus::Paused).to_string(),
            "PAUSED",
            theme::MUTED,
        ),
    ]);
    frame.render_widget(
        Paragraph::new(rollup)
            .alignment(Alignment::Center)
            .block(panel(format!("ROLL-UP · {} MONITORS", monitors.len()))),
        chunks[0],
    );

    if monitors.is_empty() {
        frame.render_widget(
            Paragraph::new("no monitors configured — press 'a' to add one, or `monitra monitor add` from another shell")
                .block(panel("FLEET")),
            chunks[1],
        );
        return;
    }

    let rows = monitors.iter().enumerate().map(|(index, monitor)| {
        let color = theme::status_color(monitor.status);
        Row::new(vec![
            Cell::from(status_label(monitor.status)).style(Style::default().fg(color)),
            Cell::from(monitor.name.clone()).style(Style::default().fg(theme::BRIGHT)),
            Cell::from(kind_label(monitor.kind)).style(theme::dim()),
            Cell::from(monitor.target.clone()),
            Cell::from(
                monitor
                    .agent_id
                    .map(|id| format!("agent #{id}"))
                    .unwrap_or_else(|| "direct".to_string()),
            )
            .style(theme::dim()),
        ])
        .style(if index == app.selected {
            Style::default().bg(Color::Rgb(15, 31, 39))
        } else {
            Style::default()
        })
    });
    let widths = [
        Constraint::Length(11),
        Constraint::Length(22),
        Constraint::Length(9),
        Constraint::Min(24),
        Constraint::Length(12),
    ];
    let table = Table::new(rows, widths)
        .header(table_header(["STATUS", "NAME", "KIND", "TARGET", "FED BY"]))
        .column_spacing(1)
        .block(panel("FLEET"))
        .row_highlight_style(Style::default().add_modifier(Modifier::BOLD));
    frame.render_widget(table, chunks[1]);
}

fn render_monitor(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let Some(monitor) = app.snapshot.monitors.get(app.selected) else {
        frame.render_widget(
            Paragraph::new("no monitor selected — pick one on the FLEET screen")
                .block(panel("MONITOR")),
            area,
        );
        return;
    };

    let chunks = Layout::vertical([
        Constraint::Length(8),
        Constraint::Length(9),
        Constraint::Min(8),
    ])
    .split(area);
    let top = Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(chunks[0]);

    let color = theme::status_color(monitor.status);
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(
                status_label(monitor.status),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Line::raw(""),
            Line::from(monitor.name.clone()),
            Line::from(monitor.target.clone()),
        ])
        .block(panel("CURRENT STATE")),
        top[0],
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(format!("kind      {}", kind_label(monitor.kind))),
            Line::from(format!("interval  {}s", monitor.interval_secs)),
            Line::from(format!(
                "agent     {}",
                monitor
                    .agent_id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "— (direct probe)".to_string())
            )),
        ])
        .block(panel("MONITOR CONFIG")),
        top[1],
    );

    let history = app
        .snapshot
        .selected_history
        .as_ref()
        .filter(|(id, _)| *id == monitor.id)
        .map(|(_, history)| history.as_slice())
        .unwrap_or(&[]);

    let latencies: Vec<u64> = history.iter().map(|r| r.latency_ms).collect();
    let (p50, max) = percentile_and_max(&latencies);
    frame.render_widget(
        Sparkline::default()
            .block(panel(format!(
                "LATENCY · {} SAMPLES · ms  p50 {p50} · max {max}",
                history.len()
            )))
            .data(&latencies)
            .style(Style::default().fg(theme::CYAN)),
        chunks[1],
    );

    let bottom = Layout::horizontal([Constraint::Percentage(64), Constraint::Percentage(36)])
        .split(chunks[2]);
    let rows = history.iter().rev().take(20).map(|result| {
        let (ok, color) = if result.success {
            ("✓ yes", theme::GREEN)
        } else {
            ("✕ no", theme::RED)
        };
        Row::new([
            fmt_ago(now_unix(), result.checked_at),
            ok.to_string(),
            format!("{}ms", result.latency_ms),
            result.message.clone().unwrap_or_default(),
        ])
        .style(Style::default().fg(color))
    });
    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(10),
                Constraint::Length(8),
                Constraint::Length(10),
                Constraint::Min(18),
            ],
        )
        .header(table_header(["WHEN", "OK", "LATENCY", "MESSAGE"]))
        .block(panel("CHECK RESULTS")),
        bottom[0],
    );

    let successes = history.iter().filter(|r| r.success).count();
    let uptime = if history.is_empty() {
        "n/a — no confirmed samples yet".to_string()
    } else {
        format!(
            "{:.2}% · {successes}/{}",
            (successes as f64 / history.len() as f64) * 100.0,
            history.len()
        )
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(format!("retained samples   {uptime}")),
            Line::raw(""),
            Line::styled(
                "computed over whatever history retention (§5.4) has kept — not a fixed window",
                theme::dim(),
            ),
        ])
        .wrap(Wrap { trim: true })
        .block(panel("UPTIME · CONFIRMED SAMPLES ONLY")),
        bottom[1],
    );
}

fn percentile_and_max(latencies: &[u64]) -> (u64, u64) {
    if latencies.is_empty() {
        return (0, 0);
    }
    let mut sorted = latencies.to_vec();
    sorted.sort_unstable();
    let p50 = sorted[sorted.len() / 2];
    let max = *sorted.last().unwrap_or(&0);
    (p50, max)
}

fn render_agents(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let agents = &app.snapshot.agents;
    if agents.is_empty() {
        frame.render_widget(
            Paragraph::new("no agents registered — `monitra agent register` from another shell")
                .block(panel("AGENTS")),
            area,
        );
        return;
    }
    let now = now_unix();
    let rows = agents.iter().enumerate().map(|(index, agent)| {
        Row::new([
            agent.name.clone(),
            agent.scope.clone(),
            fmt_ago(now, agent.last_heartbeat_at),
        ])
        .style(if index == app.selected {
            Style::default().bg(Color::Rgb(25, 21, 42))
        } else {
            Style::default()
        })
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(20),
            Constraint::Min(24),
            Constraint::Length(14),
        ],
    )
    .header(table_header(["NAME", "SCOPE", "LAST HEARTBEAT"]))
    .block(panel("AGENTS"))
    .row_highlight_style(Style::default().add_modifier(Modifier::BOLD));
    frame.render_widget(table, area);
}

fn render_kubernetes(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let resources: Vec<&Monitor> = app
        .snapshot
        .monitors
        .iter()
        .filter(|m| {
            matches!(
                m.kind,
                MonitorKind::K8sDeployment | MonitorKind::K8sStatefulSet | MonitorKind::K8sService
            )
        })
        .collect();

    let clusters = app
        .snapshot
        .health
        .as_ref()
        .map(|h| h.k8s_clusters.join(", "))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "none attached — `monitra k8s attach`".to_string());

    let chunks = Layout::vertical([Constraint::Length(3), Constraint::Min(8)]).split(area);
    frame.render_widget(
        Paragraph::new(format!("attached clusters: {clusters}")).block(panel("CLUSTERS")),
        chunks[0],
    );

    if resources.is_empty() {
        frame.render_widget(
            Paragraph::new(
                "no Kubernetes-kind monitors — `monitra monitor add --kind k8s-deployment ...`",
            )
            .block(panel("RESOURCES")),
            chunks[1],
        );
        return;
    }

    let rows = resources.iter().map(|monitor| {
        let color = theme::status_color(monitor.status);
        Row::new([
            status_label(monitor.status).to_string(),
            monitor.name.clone(),
            kind_label(monitor.kind).to_string(),
            monitor.target.clone(),
        ])
        .style(Style::default().fg(color))
    });
    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(11),
                Constraint::Length(24),
                Constraint::Length(10),
                Constraint::Min(24),
            ],
        )
        .header(table_header(["STATUS", "NAME", "KIND", "RESOURCE"]))
        .block(panel("RESOURCES")),
        chunks[1],
    );
}

fn render_alerts(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let alerts = &app.snapshot.alerts;
    if alerts.is_empty() {
        frame.render_widget(
            Paragraph::new("no alerts recorded yet").block(panel("ALERTS")),
            area,
        );
        return;
    }
    let now = now_unix();
    let rows = alerts.iter().map(|event| {
        let color = theme::status_color(event.transitioned_to);
        Row::new([
            fmt_ago(now, event.occurred_at),
            format!("monitor #{}", event.monitor_id),
            format!("{:?}", event.transitioned_to),
            event.sinks_attempted.clone(),
            event.delivery_outcome.clone(),
        ])
        .style(Style::default().fg(color))
    });
    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(10),
                Constraint::Length(14),
                Constraint::Length(10),
                Constraint::Length(20),
                Constraint::Min(20),
            ],
        )
        .header(table_header(["WHEN", "MONITOR", "TO", "SINKS", "DELIVERY"]))
        .block(panel(format!("ALERTS · {} TOTAL", alerts.len()))),
        area,
    );
}

fn render_health(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let Some(health) = &app.snapshot.health else {
        frame.render_widget(
            Paragraph::new("/health unreachable — nothing to show").block(panel("HEALTH")),
            area,
        );
        return;
    };
    let store_color = if health.store.reachable {
        theme::GREEN
    } else {
        theme::RED
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(format!("monitra   {}", health.version)),
            Line::raw(""),
            Line::styled(
                format!(
                    "store     {}  {}",
                    health.store.name,
                    if health.store.reachable {
                        "reachable"
                    } else {
                        "UNREACHABLE"
                    }
                ),
                Style::default().fg(store_color),
            ),
            Line::from(format!(
                "cache     {}  {}",
                health.cache.name,
                if health.cache.degraded {
                    "degraded"
                } else {
                    "active"
                }
            )),
            Line::from(format!(
                "notifier  {}  queue={}",
                health.notifier.name, health.notifier.queue_len
            )),
            Line::from(format!(
                "k8s       {} cluster(s) attached",
                health.k8s_clusters.len()
            )),
        ])
        .block(panel("HEALTH")),
        area,
    );
}

fn render_services(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let Some(health) = &app.snapshot.health else {
        frame.render_widget(
            Paragraph::new("/health unreachable — nothing to show").block(panel("SERVICES")),
            area,
        );
        return;
    };
    let rows = [
        Row::new([
            "store".to_string(),
            health.store.name.clone(),
            if health.store.reachable {
                "reachable".to_string()
            } else {
                "unreachable".to_string()
            },
        ]),
        Row::new([
            "cache".to_string(),
            health.cache.name.clone(),
            if health.cache.degraded {
                "degraded → default".to_string()
            } else {
                "active".to_string()
            },
        ]),
        Row::new([
            "notifier".to_string(),
            health.notifier.name.clone(),
            format!("queue={}", health.notifier.queue_len),
        ]),
    ];
    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(12),
                Constraint::Length(16),
                Constraint::Min(20),
            ],
        )
        .header(table_header(["CATEGORY", "PROVIDER", "STATE"]))
        .block(panel("SERVICES")),
        area,
    );
}

fn render_help(frame: &mut Frame<'_>, area: Rect) {
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled("KEYS", theme::title()),
            Line::raw(""),
            Line::raw("1-7        jump to screen"),
            Line::raw("tab/←→     next/previous screen"),
            Line::raw("↑↓ / j k   move selection"),
            Line::raw("⏎          open selected monitor (Fleet)"),
            Line::raw("a          add monitor (Fleet)"),
            Line::raw("esc        back to Fleet / close this"),
            Line::raw("?          toggle this help"),
            Line::raw("q / ^C     quit"),
        ])
        .block(panel("HELP")),
        area,
    );
}

fn render_add_monitor(frame: &mut Frame<'_>, area: Rect) {
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled("ADD MONITOR", theme::title()),
            Line::raw(""),
            Line::styled(
                "not yet interactive from the TUI — use, from another shell:",
                theme::dim(),
            ),
            Line::raw(""),
            Line::styled(
                "monitra monitor add <name> <target> --kind http --interval 30",
                Style::default().fg(theme::CYAN),
            ),
            Line::raw(""),
            Line::styled("esc/q to close", theme::dim()),
        ])
        .wrap(Wrap { trim: true })
        .block(panel("ADD")),
        area,
    );
}

fn centered(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width.saturating_sub(2));
    let height = height.min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width, height)
}
