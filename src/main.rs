//! Entry point. `setup`, `service`, `k8s` (Phase 3) write config only.
//! `start` (Phase 5) boots the backend against a real store; `monitor`/
//! `agent` (register/list/remove, Phase 5) run one-shot against the same
//! store, no daemon required (§3.4). `tui` and `agent run` stay unwired
//! until Phase 9/8.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use monitra_cli::{AgentCommand, Cli, Commands, K8sCommand, MonitorCommand, ServiceCommand};
use monitra_provider::{
    ConfigFile, ConfigSources, FlagOverrides, K8sClusterConfig, ProviderCategory, Store,
    category_for_scheme, gather_env, load_file, project_config_path, resolve, xdg_config_path,
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
        Commands::Tui { url } => {
            println!("tui: not implemented until Phase 9 (url = {url:?})");
            Ok(())
        }
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

/// Runs the daemon: opens storage, resolves or generates the API token,
/// binds, and serves until killed. Engine (scheduling/probing) is still a
/// Phase 6 stub, so `start` today serves CRUD and `/health` only.
fn run_start(config: Option<String>, bind: Option<String>) -> Result<(), String> {
    let store = open_store(config.as_deref())?;
    let resolved = resolve(&gather_sources(config.as_deref())?);

    let token = match resolved.api_token.value {
        Some(token) => token,
        None => {
            let generated = monitra_provider::generate_api_token();
            let mut file_config = load_writable_config()?;
            file_config.api_token = Some(generated.clone());
            save_writable_config(&file_config)?;
            println!(
                "Generated API token (save this — it will not be shown again):\n  {generated}"
            );
            println!("Written to {}.", writable_config_path()?.display());
            generated
        }
    };

    let bind_addr = bind.unwrap_or_else(|| "127.0.0.1:8080".to_string());

    block_on(async {
        monitra_provider::resolve_store(Some(&store as &dyn Store))
            .await
            .map_err(|e| e.to_string())?;

        let version = env!("CARGO_PKG_VERSION").to_string();
        let router = monitra_backend::router(Arc::new(store) as Arc<dyn Store>, token, version);
        monitra_backend::serve(&bind_addr, router)
            .await
            .map_err(|e| e.to_string())
    })
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
            } => {
                let monitor = monitra_backend::service::add_monitor(
                    &store,
                    name,
                    target,
                    kind.into(),
                    interval,
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
            } => {
                monitra_backend::service::edit_monitor(&store, id, name, target, interval)
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
        }
    })
}

/// One-shot agent register/list/remove, same direct-to-storage shape as
/// `run_monitor`. `agent run` (the actual agent process, ADR-008) stays
/// unimplemented until Phase 8.
fn run_agent(command: AgentCommand) -> Result<(), String> {
    // `Run` needs no store — it's the future agent process, not a one-shot
    // management command — so it's handled before `open_store` runs at all.
    if matches!(command, AgentCommand::Run { .. }) {
        return Err("agent run: not implemented until Phase 8 (agent binary)".to_string());
    }

    let store = open_store(None)?;

    block_on(async {
        match command {
            AgentCommand::Register { name, scope } => {
                let agent = monitra_backend::service::register_agent(&store, name, scope)
                    .await
                    .map_err(|e| e.to_string())?;
                println!("Registered agent {} ({}).", agent.id, agent.name);
                Ok(())
            }
            AgentCommand::List => {
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
            }
            AgentCommand::Remove { id } => {
                monitra_backend::service::remove_agent(&store, id)
                    .await
                    .map_err(|e| e.to_string())?;
                println!("Removed agent {id}.");
                Ok(())
            }
            AgentCommand::Run { .. } => {
                Err("agent run: not implemented until Phase 8 (agent binary)".to_string())
            }
        }
    })
}
