# Introduction

Monitra is a Rust uptime monitor operated from the command line. It checks network targets, can receive checks from host agents, and exposes a terminal and web dashboard.

Its operating principles are straightforward:

- Show an unknown or stale state when Monitra lacks evidence. A broken collector or missing agent does not prove that a target is down.
- Keep the normal installation self-contained: SQLite, an in-process cache, and a logging notifier need no external service.
- Make capabilities available from the CLI, including monitor management and alert history.

Start with [Getting started](getting-started.md), then see [Concepts](concepts.md) and the [CLI reference](cli.md).
