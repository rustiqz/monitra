//! Parse-shape coverage for every command form (DESIGN.md §4 `cli`, Phase 2
//! gate). These assert only that argv maps to the right typed shape — no
//! execution exists yet.

use clap::{CommandFactory, Parser};
use monitra_cli::{
    AgentCommand, Cli, Commands, K8sCommand, MonitorCommand, MonitorKindArg, ServiceCommand,
};

fn parse(args: &[&str]) -> Cli {
    let mut argv = vec!["monitra"];
    argv.extend_from_slice(args);
    Cli::try_parse_from(argv).unwrap_or_else(|e| panic!("expected {args:?} to parse: {e}"))
}

fn fails(args: &[&str]) {
    let mut argv = vec!["monitra"];
    argv.extend_from_slice(args);
    assert!(
        Cli::try_parse_from(argv).is_err(),
        "expected {args:?} to fail to parse"
    );
}

#[test]
fn version() {
    assert!(matches!(parse(&["version"]).command, Commands::Version));
}

#[test]
fn start_bare_and_with_flags() {
    assert!(matches!(
        parse(&["start"]).command,
        Commands::Start {
            config: None,
            bind: None
        }
    ));
    match parse(&[
        "start",
        "--config",
        "monitra.toml",
        "--bind",
        "127.0.0.1:8080",
    ])
    .command
    {
        Commands::Start { config, bind } => {
            assert_eq!(config.as_deref(), Some("monitra.toml"));
            assert_eq!(bind.as_deref(), Some("127.0.0.1:8080"));
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn tui_bare_and_with_url() {
    assert!(matches!(
        parse(&["tui"]).command,
        Commands::Tui { url: None }
    ));
    match parse(&["tui", "--url", "https://host:9000"]).command {
        Commands::Tui { url } => assert_eq!(url.as_deref(), Some("https://host:9000")),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn setup_takes_no_args() {
    assert!(matches!(parse(&["setup"]).command, Commands::Setup));
    fails(&["setup", "--anything"]);
}

#[test]
fn monitor_add() {
    match parse(&[
        "monitor",
        "add",
        "--name",
        "api",
        "--target",
        "https://api.example.com/health",
        "--kind",
        "http",
        "--interval",
        "30",
    ])
    .command
    {
        Commands::Monitor {
            command:
                MonitorCommand::Add {
                    name,
                    target,
                    kind,
                    interval,
                    agent_id,
                },
        } => {
            assert_eq!(name, "api");
            assert_eq!(target, "https://api.example.com/health");
            assert!(matches!(kind, MonitorKindArg::Http));
            assert_eq!(interval, 30);
            assert_eq!(agent_id, None);
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn monitor_add_every_kind_value() {
    for (flag, expect_k8s_or_agent) in [
        ("http", false),
        ("tcp", false),
        ("icmp", false),
        ("k8s-deployment", true),
        ("k8s-stateful-set", true),
        ("k8s-service", true),
        ("host-agent-check", true),
    ] {
        let _ = expect_k8s_or_agent;
        parse(&[
            "monitor",
            "add",
            "--name",
            "n",
            "--target",
            "t",
            "--kind",
            flag,
            "--interval",
            "5",
        ]);
    }
}

#[test]
fn monitor_add_accepts_agent_id() {
    match parse(&[
        "monitor",
        "add",
        "--name",
        "edge-disk",
        "--target",
        "disk-root",
        "--kind",
        "host-agent-check",
        "--interval",
        "30",
        "--agent-id",
        "7",
    ])
    .command
    {
        Commands::Monitor {
            command: MonitorCommand::Add { agent_id, .. },
        } => assert_eq!(agent_id, Some(7)),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn monitor_edit_accepts_agent_id() {
    match parse(&["monitor", "edit", "3", "--agent-id", "9"]).command {
        Commands::Monitor {
            command: MonitorCommand::Edit { agent_id, .. },
        } => assert_eq!(agent_id, Some(9)),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn monitor_add_requires_all_flags() {
    fails(&["monitor", "add", "--name", "api"]);
}

#[test]
fn monitor_add_rejects_unknown_kind() {
    fails(&[
        "monitor",
        "add",
        "--name",
        "n",
        "--target",
        "t",
        "--kind",
        "bogus",
        "--interval",
        "5",
    ]);
}

#[test]
fn monitor_list() {
    assert!(matches!(
        parse(&["monitor", "list"]).command,
        Commands::Monitor {
            command: MonitorCommand::List
        }
    ));
}

#[test]
fn monitor_show() {
    match parse(&["monitor", "show", "7"]).command {
        Commands::Monitor {
            command: MonitorCommand::Show { id },
        } => assert_eq!(id, 7),
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn monitor_edit_all_optional() {
    assert!(matches!(
        parse(&["monitor", "edit", "3"]).command,
        Commands::Monitor {
            command: MonitorCommand::Edit {
                id: 3,
                name: None,
                target: None,
                interval: None,
                agent_id: None
            }
        }
    ));
    match parse(&["monitor", "edit", "3", "--interval", "60"]).command {
        Commands::Monitor {
            command: MonitorCommand::Edit { id, interval, .. },
        } => {
            assert_eq!(id, 3);
            assert_eq!(interval, Some(60));
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn monitor_remove_pause_resume() {
    assert!(matches!(
        parse(&["monitor", "remove", "1"]).command,
        Commands::Monitor {
            command: MonitorCommand::Remove { id: 1 }
        }
    ));
    assert!(matches!(
        parse(&["monitor", "pause", "1"]).command,
        Commands::Monitor {
            command: MonitorCommand::Pause { id: 1 }
        }
    ));
    assert!(matches!(
        parse(&["monitor", "resume", "1"]).command,
        Commands::Monitor {
            command: MonitorCommand::Resume { id: 1 }
        }
    ));
}

#[test]
fn agent_register_list_remove() {
    match parse(&[
        "agent",
        "register",
        "--name",
        "host-1",
        "--scope",
        "web-server",
    ])
    .command
    {
        Commands::Agent {
            command: AgentCommand::Register { name, scope },
        } => {
            assert_eq!(name, "host-1");
            assert_eq!(scope, "web-server");
        }
        other => panic!("unexpected: {other:?}"),
    }
    assert!(matches!(
        parse(&["agent", "list"]).command,
        Commands::Agent {
            command: AgentCommand::List
        }
    ));
    assert!(matches!(
        parse(&["agent", "remove", "2"]).command,
        Commands::Agent {
            command: AgentCommand::Remove { id: 2 }
        }
    ));
}

#[test]
fn agent_run() {
    match parse(&[
        "agent",
        "run",
        "--name",
        "host-1",
        "--scope",
        "web-server",
        "--backend-url",
        "https://backend:8443",
        "--agent-id",
        "7",
    ])
    .command
    {
        Commands::Agent {
            command:
                AgentCommand::Run {
                    name,
                    scope,
                    backend_url,
                    agent_id,
                    token,
                    token_file,
                    config,
                },
        } => {
            assert_eq!(name, "host-1");
            assert_eq!(scope, "web-server");
            assert_eq!(backend_url, "https://backend:8443");
            assert_eq!(agent_id, 7);
            assert_eq!(token, None);
            assert_eq!(token_file, None);
            assert_eq!(config, None);
        }
        other => panic!("unexpected: {other:?}"),
    }
}

#[test]
fn agent_run_requires_backend_url() {
    fails(&[
        "agent",
        "run",
        "--name",
        "host-1",
        "--scope",
        "web-server",
        "--agent-id",
        "7",
    ]);
}

#[test]
fn agent_run_requires_agent_id() {
    fails(&[
        "agent",
        "run",
        "--name",
        "host-1",
        "--scope",
        "web-server",
        "--backend-url",
        "https://backend:8443",
    ]);
}

#[test]
fn agent_run_token_and_token_file_conflict() {
    fails(&[
        "agent",
        "run",
        "--name",
        "h",
        "--scope",
        "s",
        "--backend-url",
        "u",
        "--token",
        "abc",
        "--token-file",
        "/path",
    ]);
}

#[test]
fn k8s_attach_list_detach() {
    match parse(&[
        "k8s",
        "attach",
        "--name",
        "prod",
        "--kubeconfig",
        "/home/u/.kube/config",
        "--context",
        "prod-ctx",
        "--namespace",
        "default",
    ])
    .command
    {
        Commands::K8s {
            command:
                K8sCommand::Attach {
                    name,
                    kubeconfig,
                    context,
                    namespace,
                },
        } => {
            assert_eq!(name, "prod");
            assert_eq!(kubeconfig, "/home/u/.kube/config");
            assert_eq!(context.as_deref(), Some("prod-ctx"));
            assert_eq!(namespace.as_deref(), Some("default"));
        }
        other => panic!("unexpected: {other:?}"),
    }
    assert!(matches!(
        parse(&["k8s", "list"]).command,
        Commands::K8s {
            command: K8sCommand::List
        }
    ));
    assert!(matches!(
        parse(&["k8s", "detach", "prod"]).command,
        Commands::K8s { command: K8sCommand::Detach { name } } if name == "prod"
    ));
}

#[test]
fn k8s_attach_requires_kubeconfig() {
    fails(&["k8s", "attach", "--name", "prod"]);
}

#[test]
fn service_attach_detach_list() {
    match parse(&["service", "attach", "postgres://user@host/db"]).command {
        Commands::Service {
            command: ServiceCommand::Attach { url },
        } => {
            assert_eq!(url, "postgres://user@host/db");
        }
        other => panic!("unexpected: {other:?}"),
    }
    assert!(matches!(
        parse(&["service", "detach", "my-store"]).command,
        Commands::Service { command: ServiceCommand::Detach { name } } if name == "my-store"
    ));
    assert!(matches!(
        parse(&["service", "list"]).command,
        Commands::Service {
            command: ServiceCommand::List
        }
    ));
}

#[test]
fn no_command_fails() {
    fails(&[]);
}

#[test]
fn unknown_command_fails() {
    fails(&["frobnicate"]);
}

#[test]
fn help_snapshot_matches_fixture() {
    let rendered = Cli::command().render_help().to_string();
    let fixture = include_str!("fixtures/help.txt");
    assert_eq!(
        rendered.trim_end(),
        fixture.trim_end(),
        "top-level --help text changed — update crates/cli/tests/fixtures/help.txt if intentional"
    );
}
