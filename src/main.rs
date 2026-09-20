//! Entry point. `setup`, `service`, `k8s` (Phase 3) write config only.
//! `start` (Phase 5) boots the backend against a real store; `monitor`/
//! `agent` (register/list/remove, Phase 5) run one-shot against the same
//! store, no daemon required (§3.4). `agent run` (Phase 8) needs no store
//! at all — see `run_agent`. `tui` (Phase 9) shares `start`'s daemon-boot
//! logic (`boot_daemon`) for its embedded, no-daemon session.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use monitra_cli::{AgentCommand, Cli, Commands, K8sCommand, MonitorCommand, ServiceCommand};
use monitra_engine::EngineHandle;
use monitra_provider::{
    ConfigFile, ConfigSources, DegradingCache, FlagOverrides, InProcessCache, K8sClusterConfig,
    ProviderCategory, ResolvedConfig, Store, category_for_scheme, gather_env, load_file,
    project_config_path, resolve, xdg_config_path,
};
use monitra_storage::SqliteStore;

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Version => {
            println!("monitra {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Commands::Start { config, bind } => run_start(config, bind),
        Commands::Setup => run_setup(),
        Commands::Service { command } => run_service(command),
        Commands::K8s { command } => run_k8s(command),
        Commands::Monitor { command } => run_monitor(command),
        Commands::Agent { command } => run_agent(command),
        Commands::Alert { command } => run_alert(command),
        Commands::Tui { url, token } => run_tui(url, token),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("monitra: {message}");
            ExitCode::FAILURE
        }
    }
}

/// The one config file `setup`/`service`/`k8s`/`start` (token generation)
/// ever write to. `./monitra.toml` and `MONITRA_*`/flags remain read-only
/// override layers — a CLI command silently creating a file in whatever
/// directory it happens to be run from would be a surprise, not a
/// convenience.
fn writable_config_path() -> Result<PathBuf, String> {
    xdg_config_path().ok_or_else(|| {
        "config: neither $XDG_CONFIG_HOME nor $HOME is set — cannot determine a config path"
            .to_string()
    })
}

fn load_writable_config() -> Result<ConfigFile, String> {
    let path = writable_config_path()?;
    load_file(&path)
        .map(Option::unwrap_or_default)
        .map_err(|e| e.to_string())
}

fn save_writable_config(config: &ConfigFile) -> Result<(), String> {
    let path = writable_config_path()?;
    monitra_provider::write_file(&path, config).map_err(|e| e.to_string())
}

fn prompt(label: &str) -> Result<Option<String>, String> {
    print!("{label}: ");
    io::stdout()
        .flush()
        .map_err(|e| format!("setup: failed to write prompt: {e}"))?;
    let mut line = String::new();
    io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| format!("setup: failed to read input: {e}"))?;
    let trimmed = line.trim();
    Ok(if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    })
}

/// Interactive first-run wizard. Never required (§11.7) — every field left
/// blank keeps the embedded default, and `monitor add` on a machine that
/// never ran this must work identically.
fn run_setup() -> Result<(), String> {
    println!("monitra setup — press enter on any prompt to keep the embedded default.\n");

    let mut config = load_writable_config()?;

    if let Some(url) = prompt("Store URL (e.g. postgres://…, blank = embedded SQLite")? {
        config.store = Some(url);
    }
    if let Some(url) = prompt("Cache URL (e.g. redis://…, blank = embedded in-process")? {
        config.cache = Some(url);
    }
    if let Some(url) = prompt("Notifier URL (e.g. slack://…, webhook://…, blank = log-only")? {
        config.notifier = Some(url);
    }

    save_writable_config(&config)?;
    let path = writable_config_path()?;
    println!(
        "\nWrote {}. Takes effect on the next `monitra start`.",
        path.display()
    );
    Ok(())
}

fn run_service(command: ServiceCommand) -> Result<(), String> {
    match command {
        ServiceCommand::Attach { url } => {
            let category = category_for_scheme(&url).map_err(|e| e.to_string())?;
            let mut config = load_writable_config()?;
            match category {
                ProviderCategory::Store => config.store = Some(url.clone()),
                ProviderCategory::Cache => config.cache = Some(url.clone()),
                ProviderCategory::Notifier => config.notifier = Some(url.clone()),
                ProviderCategory::Collector => {
                    return Err("service attach: Kubernetes clusters use `monitra k8s attach`, not `service attach` (§11.8)".to_string());
                }
            }
            save_writable_config(&config)?;
            println!(
                "Attached {category} provider {url}. Takes effect on the next `monitra start`."
            );
            Ok(())
        }
        ServiceCommand::Detach { name } => {
            let mut config = load_writable_config()?;
            let cleared = match name.as_str() {
                "store" => std::mem::take(&mut config.store).is_some(),
                "cache" => std::mem::take(&mut config.cache).is_some(),
                "notifier" => std::mem::take(&mut config.notifier).is_some(),
                other => {
                    return Err(format!(
                        "service detach: unknown category '{other}' (expected store, cache, or notifier)"
                    ));
                }
            };
            if !cleared {
                println!("{name} has no provider attached; nothing to do.");
                return Ok(());
            }
            save_writable_config(&config)?;
            println!(
                "Detached {name}, reverted to its embedded default. Takes effect on the next `monitra start`."
            );
            Ok(())
        }
        ServiceCommand::List => {
            let resolved = resolve(&gather_sources(None)?);
            println!("store:    {}", describe_field(&resolved.store));
            println!("cache:    {}", describe_field(&resolved.cache));
            println!("notifier: {}", describe_field(&resolved.notifier));
            Ok(())
        }
    }
}

fn describe_field(field: &monitra_provider::ResolvedField) -> String {
    match &field.value {
        Some(value) => format!("{value} (from {:?})", field.source),
        None => "(embedded default)".to_string(),
    }
}

fn run_k8s(command: K8sCommand) -> Result<(), String> {
    match command {
        K8sCommand::Attach {
            name,
            kubeconfig,
            context,
            namespace,
        } => {
            let mut config = load_writable_config()?;
            config.k8s.retain(|cluster| cluster.name != name);
            config.k8s.push(K8sClusterConfig {
                name: name.clone(),
                kubeconfig,
                context,
                namespace,
            });
            save_writable_config(&config)?;
            println!(
                "Attached Kubernetes cluster '{name}'. Takes effect on the next `monitra start`."
            );
            Ok(())
        }
        K8sCommand::List => {
            let resolved = resolve(&gather_sources(None)?);
            if resolved.k8s.is_empty() {
                println!("no Kubernetes clusters attached");
            }
            for cluster in resolved.k8s {
                println!(
                    "{}  kubeconfig={}  context={}  namespace={}",
                    cluster.name,
                    cluster.kubeconfig,
                    cluster.context.as_deref().unwrap_or("(default)"),
                    cluster.namespace.as_deref().unwrap_or("(default)"),
                );
            }
            Ok(())
        }
        K8sCommand::Detach { name } => {
            let mut config = load_writable_config()?;
            let before = config.k8s.len();
            config.k8s.retain(|cluster| cluster.name != name);
            if config.k8s.len() == before {
                println!("no Kubernetes cluster named '{name}' is attached; nothing to do.");
                return Ok(());
            }
            save_writable_config(&config)?;
            println!(
                "Detached Kubernetes cluster '{name}'. Takes effect on the next `monitra start`."
            );
            Ok(())
        }
    }
}

/// `config_override` is `Start`'s `--config <path>` flag — when given, it
/// replaces `./monitra.toml` in the "project" precedence slot (§11.7);
/// every other command passes `None`.
fn gather_sources(config_override: Option<&str>) -> Result<ConfigSources, String> {
    let xdg = match xdg_config_path() {
        Some(path) => load_file(&path).map_err(|e| e.to_string())?,
        None => None,
    };
    let project_path = config_override
        .map(PathBuf::from)
        .unwrap_or_else(project_config_path);
    let project = load_file(&project_path).map_err(|e| e.to_string())?;
    Ok(ConfigSources {
        xdg,
        project,
        env: gather_env(),
        flags: FlagOverrides::default(),
    })
}

/// Opens the resolved `Store`. Only the embedded SQLite default is
/// implemented as of Phase 5 — a configured alternative (`postgres://…`)
/// gets a named "not yet implemented" error rather than a silent fallback
/// or a panic (P1: never guess).
fn open_store(config_override: Option<&str>) -> Result<SqliteStore, String> {
    let resolved = resolve(&gather_sources(config_override)?);
    if let Some(url) = &resolved.store.value {
        return Err(format!(
            "store: '{url}' is configured but not yet implemented — only the embedded SQLite default exists as of Phase 5"
        ));
    }
    let db_path = monitra_provider::default_db_path().ok_or_else(|| {
        "store: neither $XDG_DATA_HOME nor $HOME is set — cannot determine a database path"
            .to_string()
    })?;
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            format!(
                "store: failed to create data directory {}: {e}",
                parent.display()
            )
        })?;
    }
    SqliteStore::open(&db_path).map_err(|e| e.to_string())
}

/// Builds a fresh single-use runtime and drives `fut` to completion — every
/// CLI command here does exactly one async unit of work (or, for `start`,
/// one that runs until killed), so there's no reactor to share across calls.
fn block_on<F>(fut: F) -> Result<(), String>
where
    F: std::future::Future<Output = Result<(), String>>,
{
    tokio::runtime::Runtime::new()
        .map_err(|e| format!("failed to build async runtime: {e}"))?
        .block_on(fut)
}

/// Resolves the human API token, generating and persisting one on first use
/// (§11.11) — shared by `start` and `tui`'s embedded-mode bootstrap so
/// neither daemon-boot path has to guess the other's behavior.
fn resolve_or_generate_token(resolved: &ResolvedConfig) -> Result<String, String> {
    match &resolved.api_token.value {
        Some(token) => Ok(token.clone()),
        None => {
            let generated = monitra_provider::generate_api_token();
            let mut file_config = load_writable_config()?;
            file_config.api_token = Some(generated.clone());
            save_writable_config(&file_config)?;
            println!(
                "Generated API token (save this — it will not be shown again):\n  {generated}"
            );
            println!("Written to {}.", writable_config_path()?.display());
            Ok(generated)
        }
    }
}

/// The `Cache` `main.rs` holds purely for `/health` reporting (Phase 9) —
/// nothing in the daemon calls `Cache::get`/`set` anywhere yet (§4
/// `provider`). No alternative `Cache` implementation exists to attach:
/// `cache-redis` is an unimplemented stub (Phase 1). A configured
/// `redis://` cache therefore always degrades to the in-process default at
/// boot, WARN logged — the same "never fails the daemon" policy an
/// unreachable-at-runtime cache would get (§4.1), just triggered earlier.
fn build_cache(cache_url: &Option<String>) -> Arc<DegradingCache> {
    if let Some(url) = cache_url {
        tracing::warn!(
            cache = %url,
            "cache: no alternative Cache implementation exists yet (cache-redis is unimplemented) — running on the in-process default"
        );
    }
    Arc::new(DegradingCache::new(None, Arc::new(InProcessCache::new())))
}

/// Everything a running daemon holds: the store (for one-shot CLI reuse if
/// a caller wants it), the engine handle (for graceful shutdown), and the
/// fully-wired router. Shared by `start` (fixed bind address) and `tui`'s
/// embedded-mode bootstrap (ephemeral loopback port, Phase 9) — one place
/// builds a daemon, so the two never drift.
struct Daemon {
    engine: EngineHandle,
    router: axum::Router,
    token: String,
}

async fn boot_daemon(config: Option<&str>) -> Result<Daemon, String> {
    let store = open_store(config)?;
    let resolved = resolve(&gather_sources(config)?);
    let token = resolve_or_generate_token(&resolved)?;
    let k8s_factory = build_k8s_factory(&resolved.k8s);
    let notifier = build_notifier(&resolved.notifier.value)?;
    let cache = build_cache(&resolved.cache.value);
    let k8s_cluster_names: Vec<String> = resolved.k8s.iter().map(|c| c.name.clone()).collect();

    monitra_provider::resolve_store(Some(&store as &dyn Store))
        .await
        .map_err(|e| e.to_string())?;

    let store: Arc<dyn Store> = Arc::new(store);
    let engine = EngineHandle::start(
        monitra_engine::EngineDeps {
            store: Arc::clone(&store),
            k8s_factory,
            notifier: Arc::clone(&notifier),
        },
        monitra_engine::EngineConfig::default(),
    );

    let version = env!("CARGO_PKG_VERSION").to_string();
    let router = monitra_backend::router(
        Arc::clone(&store),
        token.clone(),
        version,
        engine.results_sender(),
        engine.ingest_handle(),
        notifier,
        cache,
        k8s_cluster_names,
    );

    Ok(Daemon {
        engine,
        router,
        token,
    })
}

/// Runs the daemon: opens storage, resolves or generates the API token,
/// binds, and serves until killed.
fn run_start(config: Option<String>, bind: Option<String>) -> Result<(), String> {
    let bind_addr = bind.unwrap_or_else(|| "127.0.0.1:8080".to_string());

    block_on(async {
        let daemon = boot_daemon(config.as_deref()).await?;
        let listener = monitra_backend::bind(&bind_addr)
            .await
            .map_err(|e| e.to_string())?;
        let result = monitra_backend::serve(listener, daemon.router)
            .await
            .map_err(|e| e.to_string());
        // §7.4 graceful shutdown. `serve` today only returns on a genuine
        // server error, not a SIGINT/SIGTERM (that coordination is still
        // unwired — a pre-existing gap from Phase 5's `backend::serve`, not
        // new to this phase); this path exists so `EngineHandle::shutdown`
        // is exercised whenever `serve` does return.
        daemon.engine.shutdown().await;
        result
    })
}

/// Runs the terminal dashboard (Phase 9, ADR-009). `--url` connects to a
/// remote daemon directly; with no `--url`, boots an embedded backend on an
/// OS-assigned loopback port via the same `boot_daemon` `start` uses, and
/// points the (otherwise identical) TUI client at it — one client
/// implementation regardless of mode, per ADR-009/§11.4.
fn run_tui(url: Option<String>, token: Option<String>) -> Result<(), String> {
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|e| format!("failed to build async runtime: {e}"))?;

    runtime.block_on(async {
        let (base_url, bearer, mut embedded) = match url {
            Some(base_url) => {
                let resolved = resolve(&gather_sources(None)?);
                let bearer = token.or(resolved.api_token.value).ok_or_else(|| {
                    "tui: no API token available for a remote --url — pass --token, set \
                         MONITRA_API_TOKEN, or run `monitra setup`"
                        .to_string()
                })?;
                (base_url, bearer, None)
            }
            None => {
                let daemon = boot_daemon(None).await?;
                let listener = monitra_backend::bind("127.0.0.1:0")
                    .await
                    .map_err(|e| e.to_string())?;
                let addr = listener.local_addr().map_err(|e| e.to_string())?;
                let base_url = format!("http://{addr}");
                let bearer = daemon.token.clone();
                let engine = daemon.engine;
                let router = daemon.router;
                tokio::spawn(async move {
                    if let Err(error) = monitra_backend::serve(listener, router).await {
                        tracing::error!(%error, "tui: embedded backend stopped unexpectedly");
                    }
                });
                (base_url, bearer, Some(engine))
            }
        };

        let client = monitra_tui::Client::new(base_url, bearer);
        let result = monitra_tui::run(client).await.map_err(|e| e.to_string());

        if let Some(engine) = embedded.take() {
            engine.shutdown().await;
        }
        result
    })
}

/// Builds the `Notifier` `engine` emits `AlertEvent`s through (§4.1, Phase
/// 7), wrapped in `monitra-provider`'s bounded-queue-with-retry
/// `RetryingNotifier` regardless of which sink is underneath — every
/// `Notifier` gets the same "queue with bounded retry and backoff, never
/// block a probe" treatment (§4.1). `webhook://`/no target always resolve
/// (`notify-webhook` is the always-compiled default, logging instead of
/// sending with no target attached); `slack://` requires the `slack`
/// feature, same "named error, never a silent fallback" rule `open_store`
/// already uses for an unimplemented `Store` scheme.
fn build_notifier(
    notifier_url: &Option<String>,
) -> Result<Arc<monitra_provider::RetryingNotifier>, String> {
    let inner: Arc<dyn monitra_provider::Notifier> = match notifier_url.as_deref() {
        None => Arc::new(notify_webhook::WebhookNotifier::new(None)),
        Some(url) if url.starts_with("webhook://") => {
            Arc::new(notify_webhook::WebhookNotifier::new(Some(url.to_string())))
        }
        Some(url) if url.starts_with("slack://") => build_slack_notifier(url)?,
        Some(url) => {
            return Err(format!(
                "notifier: '{url}' is not a recognized notifier URL (expected webhook:// or slack://)"
            ));
        }
    };
    const RETRY_QUEUE_CAPACITY: usize = 256;
    Ok(Arc::new(monitra_provider::RetryingNotifier::new(
        inner,
        RETRY_QUEUE_CAPACITY,
    )))
}

#[cfg(feature = "slack")]
fn build_slack_notifier(url: &str) -> Result<Arc<dyn monitra_provider::Notifier>, String> {
    Ok(Arc::new(notify_slack::SlackNotifier::new(url)))
}

#[cfg(not(feature = "slack"))]
fn build_slack_notifier(_url: &str) -> Result<Arc<dyn monitra_provider::Notifier>, String> {
    Err(
        "notifier: 'slack://' is configured but this build does not have the 'slack' feature enabled"
            .to_string(),
    )
}

/// Builds the `Collector` factory `engine` polls K8s-kind monitors through,
/// or `None` when the `kubernetes` feature is off or no clusters are
/// attached — `engine` never depends on `collector-kubernetes` directly
/// (CLAUDE.md's dependency DAG); this is the one place that bridges them.
#[cfg(feature = "kubernetes")]
fn build_k8s_factory(
    clusters: &[monitra_provider::K8sClusterConfig],
) -> Option<Arc<dyn monitra_provider::K8sCollectorFactory>> {
    if clusters.is_empty() {
        return None;
    }
    Some(Arc::new(collector_kubernetes::Factory::new(clusters)))
}

#[cfg(not(feature = "kubernetes"))]
fn build_k8s_factory(
    _clusters: &[monitra_provider::K8sClusterConfig],
) -> Option<Arc<dyn monitra_provider::K8sCollectorFactory>> {
    None
}

/// One-shot monitor CRUD, direct against storage — works with or without a
/// running `monitra start` (§3.4). Calls the same `monitra_backend::service`
/// functions the HTTP handlers do (ADR-009).
fn run_monitor(command: MonitorCommand) -> Result<(), String> {
    let store = open_store(None)?;

    block_on(async {
        match command {
            MonitorCommand::Add {
                name,
                target,
                kind,
                interval,
                agent_id,
            } => {
                let monitor = monitra_backend::service::add_monitor(
                    &store,
                    name,
                    target,
                    kind.into(),
                    interval,
                    agent_id,
                )
                .await
                .map_err(|e| e.to_string())?;
                println!("Created monitor {} ({}).", monitor.id, monitor.name);
                Ok(())
            }
            MonitorCommand::List => {
                let monitors = monitra_backend::service::list_monitors(&store)
                    .await
                    .map_err(|e| e.to_string())?;
                if monitors.is_empty() {
                    println!("no monitors configured");
                }
                for monitor in monitors {
                    println!(
                        "{:>4}  {:<20} {:?}  {:<30}  every {}s  [{:?}]",
                        monitor.id,
                        monitor.name,
                        monitor.kind,
                        monitor.target,
                        monitor.interval_secs,
                        monitor.status
                    );
                }
                Ok(())
            }
            MonitorCommand::Show { id } => {
                match monitra_backend::service::get_monitor(&store, id)
                    .await
                    .map_err(|e| e.to_string())?
                {
                    Some(monitor) => {
                        println!("{monitor:#?}");
                        Ok(())
                    }
                    None => Err(format!("monitor {id} not found")),
                }
            }
            MonitorCommand::Edit {
                id,
                name,
                target,
                interval,
                agent_id,
            } => {
                monitra_backend::service::edit_monitor(
                    &store, id, name, target, interval, agent_id,
                )
                .await
                .map_err(|e| e.to_string())?;
                println!("Updated monitor {id}.");
                Ok(())
            }
            MonitorCommand::Remove { id } => {
                monitra_backend::service::remove_monitor(&store, id)
                    .await
                    .map_err(|e| e.to_string())?;
                println!("Removed monitor {id}.");
                Ok(())
            }
            MonitorCommand::Pause { id } => {
                monitra_backend::service::pause_monitor(&store, id)
                    .await
                    .map_err(|e| e.to_string())?;
                println!("Paused monitor {id}.");
                Ok(())
            }
            MonitorCommand::Resume { id } => {
                monitra_backend::service::resume_monitor(&store, id)
                    .await
                    .map_err(|e| e.to_string())?;
                println!("Resumed monitor {id} (status: pending — not yet re-checked).");
                Ok(())
            }
            MonitorCommand::History { id, since } => {
                let history = monitra_backend::service::monitor_history(&store, id, since)
                    .await
                    .map_err(|e| e.to_string())?;
                if history.is_empty() {
                    println!("no check results recorded for monitor {id}");
                }
                for result in history {
                    println!(
                        "{}  {:<3}  {:>6}ms  {}",
                        result.checked_at,
                        if result.success { "ok" } else { "no" },
                        result.latency_ms,
                        result.message.as_deref().unwrap_or(""),
                    );
                }
                Ok(())
            }
        }
    })
}

/// Same one-shot shape as `run_monitor`/`run_agent`'s read-only arms.
fn run_alert(command: monitra_cli::AlertCommand) -> Result<(), String> {
    let store = open_store(None)?;
    block_on(async {
        match command {
            monitra_cli::AlertCommand::List => {
                let events = monitra_backend::service::list_all_alert_events(&store)
                    .await
                    .map_err(|e| e.to_string())?;
                if events.is_empty() {
                    println!("no alerts recorded yet");
                }
                for event in events {
                    println!(
                        "{}  monitor {}  -> {:?}  sinks={}  {}",
                        event.occurred_at,
                        event.monitor_id,
                        event.transitioned_to,
                        event.sinks_attempted,
                        event.delivery_outcome,
                    );
                }
                Ok(())
            }
        }
    })
}

/// `register`/`list`/`remove` run one-shot direct-to-storage, same shape as
/// `run_monitor`. `run` (the actual agent process, ADR-008, Phase 8) needs
/// no store at all — only a backend URL and a push token — so it is its
/// own arm rather than sharing the `open_store` call the other three need.
fn run_agent(command: AgentCommand) -> Result<(), String> {
    match command {
        AgentCommand::Run {
            name,
            scope,
            backend_url,
            agent_id,
            token,
            token_file,
            config,
        } => {
            let run_config = monitra_agent::RunConfig {
                name,
                scope,
                backend_url,
                agent_id,
                token,
                token_file: token_file.map(PathBuf::from),
                config_path: config.map(PathBuf::from),
                buffer_capacity: 256,
            };
            block_on(async move {
                monitra_agent::run(run_config)
                    .await
                    .map_err(|e| e.to_string())
            })
        }
        AgentCommand::Register { name, scope } => {
            let store = open_store(None)?;
            block_on(async move {
                let agent = monitra_backend::service::register_agent(&store, name, scope)
                    .await
                    .map_err(|e| e.to_string())?;
                println!("Registered agent {} ({}).", agent.id, agent.name);
                println!(
                    "Push token (save this — it will not be shown again; re-running \
                     `agent register` for this name issues a new one and revokes this one):\n  {}",
                    agent.token
                );
                Ok(())
            })
        }
        AgentCommand::List => {
            let store = open_store(None)?;
            block_on(async move {
                let agents = monitra_backend::service::list_agents(&store)
                    .await
                    .map_err(|e| e.to_string())?;
                if agents.is_empty() {
                    println!("no agents registered");
                }
                for agent in agents {
                    println!(
                        "{:>4}  {:<20} scope={}  last_heartbeat_at={}",
                        agent.id, agent.name, agent.scope, agent.last_heartbeat_at
                    );
                }
                Ok(())
            })
        }
        AgentCommand::Remove { id } => {
            let store = open_store(None)?;
            block_on(async move {
                monitra_backend::service::remove_agent(&store, id)
                    .await
                    .map_err(|e| e.to_string())?;
                println!("Removed agent {id}.");
                Ok(())
            })
        }
    }
}
