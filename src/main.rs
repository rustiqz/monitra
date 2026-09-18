//! Entry point. Phase 3 wires real execution for `setup`, `service`, and
//! `k8s` — the provider-attach commands that only need `provider`'s config
//! layer, nothing from `storage`/`engine`/`backend` yet. Every other command
//! still just prints what it parsed; their execution needs crates that don't
//! exist in usable form until later phases.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use monitra_cli::{Cli, Commands, K8sCommand, ServiceCommand};
use monitra_provider::{
    ConfigFile, ConfigSources, FlagOverrides, K8sClusterConfig, ProviderCategory,
    category_for_scheme, gather_env, load_file, project_config_path, resolve, xdg_config_path,
};

fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Version => {
            println!("monitra {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Commands::Setup => run_setup(),
        Commands::Service { command } => run_service(command),
        Commands::K8s { command } => run_k8s(command),
        other => {
            println!("{other:#?}");
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

/// The one config file `setup`/`service`/`k8s` ever write to. `./monitra.toml`
/// and `MONITRA_*`/flags remain read-only override layers — a CLI command
/// silently creating a file in whatever directory it happens to be run from
/// would be a surprise, not a convenience.
fn writable_config_path() -> Result<PathBuf, String> {
    xdg_config_path().ok_or_else(|| {
        "setup: config: neither $XDG_CONFIG_HOME nor $HOME is set — cannot determine a config path".to_string()
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
            let resolved = resolve(&gather_sources()?);
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
            let resolved = resolve(&gather_sources()?);
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

fn gather_sources() -> Result<ConfigSources, String> {
    let xdg = match xdg_config_path() {
        Some(path) => load_file(&path).map_err(|e| e.to_string())?,
        None => None,
    };
    let project = load_file(&project_config_path()).map_err(|e| e.to_string())?;
    Ok(ConfigSources {
        xdg,
        project,
        env: gather_env(),
        flags: FlagOverrides::default(),
    })
}
