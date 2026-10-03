<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/social/monitra-readme-banner-dark.svg">
    <img alt="Monitra" src="assets/brand/social/monitra-readme-banner-light.svg">
  </picture>
</p>

<p align="center">
  <strong>Single-binary, CLI-first uptime monitoring, written in Rust.</strong><br>
  It reports <em>unknown</em> when it lacks evidence, and never calls a failure on its own side "down".
</p>

<p align="center">
  <a href="https://github.com/rustiqz/monitra/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/rustiqz/monitra/actions/workflows/ci.yml/badge.svg"></a>
  <a href="#license"><img alt="License: MIT OR Apache-2.0" src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue"></a>
  <img alt="Rust edition 2024" src="https://img.shields.io/badge/rust-edition%202024-orange">
  <a href="https://rustiqz.github.io/monitra/docs/"><img alt="Documentation" src="https://img.shields.io/badge/docs-mdBook-informational"></a>
</p>

<p align="center">
  <a href="#quickstart">Quickstart</a> ·
  <a href="#features">Features</a> ·
  <a href="#architecture">Architecture</a> ·
  <a href="#performance">Performance</a> ·
  <a href="CONTRIBUTING.md">Contributing</a> ·
  <a href="docs/DESIGN.md">Design</a>
</p>

---

## Why Monitra

Most uptime tools need a database, a cache and a queue before they show a single green dot.
Monitra is one binary. SQLite, an in-process cache and a logging notifier are built in, so a
pristine machine works with no configuration. Slack and Kubernetes are optional providers.
PostgreSQL and Redis are not supported yet.

## Features

- **Honest status.** Permission errors, internal panics and scheduler gaps are distinct states
  from "target is down". A missing agent or broken collector yields *unknown*, never a guess.
- **CLI first.** Every capability is reachable from the command line. The terminal and web
  dashboards are pure API clients of the same backend.
- **Zero-setup defaults.** `monitra monitor add` works on a machine with no config file and no
  `monitra setup` run.
- **Provider contracts.** Store, Cache, Notifier and Collector categories have documented
  failure behaviour. SQLite and the in-process cache are the currently supported defaults.
- **Distributed agents.** Host agents push checks to the backend. Agents can also probe from
  their own region, so you can compare latency across vantage points.
- **Kubernetes aware.** Optional collector monitors Deployments, StatefulSets and Services.
- **Bounded by design.** Every channel, queue and buffer has an explicit limit. Overflow is
  dropped and logged loudly, never silent.
- **Small.** About 7 MB stripped, and a static musl build is supported.

## Quickstart

Linux x86_64 and aarch64 binaries are available on the [releases page](https://github.com/rustiqz/monitra/releases).

| Method | Install or start |
|---|---|
| Verified installer | Run the one-liner below. |
| GitHub Release | Download the matching musl tarball and `SHA256SUMS` from the [latest release](https://github.com/rustiqz/monitra/releases/latest); verify with `sha256sum -c SHA256SUMS`, then install `monitra` from the archive. |
| Docker | `docker compose up -d` using [compose.yaml](compose.yaml); image: `ghcr.io/rustiqz/monitra:latest`. |
| Cargo Binstall | `cargo binstall monitra --manifest-path Cargo.toml` from a checkout with cargo-binstall installed. Plain `cargo binstall monitra` requires publishing the crate manifest to crates.io. |
| Build from source | `cargo build --release` (requires Rust and Node/npm). |

```sh
curl -fsSL https://raw.githubusercontent.com/rustiqz/monitra/main/scripts/install.sh | sh
```

Verify a binary installation with `monitra --version`. The installer puts it in `~/.local/bin`
by default; add that directory to `PATH` if needed. Use `--dir PATH` with the installer to
choose another location. Docker stores config and the default SQLite database in the
`monitra-data` volume; the database path is `/data/monitra/monitra.db`. On first start the
container prints the generated API token once, so save it from `docker compose logs monitra`.
Homebrew and AUR packages are possible follow-ups; neither is published yet.

On the host, add a monitor and start the daemon:

```sh
monitra monitor add \
  --name example --target https://example.com --kind http --interval 30
monitra monitor list
monitra start
```

On first start the daemon generates an API token, writes it to your XDG config and prints it
once, so save it. It binds `127.0.0.1:8080` by default. In another terminal:

```sh
monitra tui    # terminal dashboard
monitra web    # embedded browser dashboard
```

See [Getting started](docs/site/src/getting-started.md) and
[Deployment](docs/site/src/deployment.md) before exposing the daemon beyond localhost.

### Optional providers

Optional providers are compiled in behind Cargo features:

| Feature | Adds |
|---|---|
| `slack` | Slack notifier |
| `kubernetes` | Kubernetes collector |

```sh
cargo build --release --features kubernetes
```

PostgreSQL and Redis have no Cargo features or working providers; see
[Providers](docs/site/src/providers.md) for current support.

## Architecture

Monitra is a workspace of small crates with an acyclic dependency graph, enforced in CI by
`scripts/dep-check.py`.

```
monitra-models ← monitra-provider ← { monitra-storage, notify-*, collector-kubernetes }
                                  ← monitra-engine ← monitra-backend
monitra-models ← monitra-probe    ← { monitra-engine, monitra-agent }
monitra-models ← monitra-cli
monitra-models ← monitra-tui      (API client only)
monitra-models ← monitra-agent    (push client + regional prober)
```

Only `main.rs` knows which provider implementations exist. How each category fails is part
of the contract:

| Category | Default | When unreachable |
|---|---|---|
| Store | SQLite | Fail fast. A silent fallback would split history. |
| Cache | In-process | Degrade to the default, warn, report in `/health`. |
| Notifier | Log sink | Bounded queue with retry. Never blocks a probe. |
| Collector | None | Warn and mark the monitor unknown. Never fails the daemon. |

The full contract, data model and decision records live in [`docs/DESIGN.md`](docs/DESIGN.md).

## Performance

Measured figures, not marketing adjectives.

### Build size

Measured 2026-09-30 (`opt-level=z`, LTO, stripped), default features:

| Build | Size |
|---|---|
| `cargo build --release` | 7.0 MB |
| `cargo build --release --features kubernetes` | 7.1 MB |
| `cargo build --release --target x86_64-unknown-linux-musl` (static) | 7.1 MB |

### Scaling limit

Under a cgroup matching the target spec (2 vCPU / 512 MB), with a 30 s check interval:

| Monitors | p99 scheduling drift | Result against the 2 s bound |
|---:|---:|---|
| 1,000 | 632 ms | Pass |
| 2,500 | 1,979 ms | Pass, barely |
| 5,000 | 3,150 ms | **Fail** |

**The limit is about 2,500 monitors, and it is CPU-bound.** Caveats and the full protocol are
in [`docs/SCALE_RESULTS.md`](docs/SCALE_RESULTS.md). Reproduce with:

```sh
cargo test --release --test scale -- --ignored --nocapture
```

## Documentation

- [Documentation site](https://rustiqz.github.io/monitra/docs/): concepts, CLI reference,
  configuration, agents, Kubernetes, deployment
- [`docs/DESIGN.md`](docs/DESIGN.md): the design contract and architecture decision records
- [`docs/SCALE_RESULTS.md`](docs/SCALE_RESULTS.md): scaling measurements

## Contributing

Contributions are welcome. Please read [CONTRIBUTING.md](CONTRIBUTING.md) first, and the
[Code of Conduct](CODE_OF_CONDUCT.md). To report a vulnerability, follow
[SECURITY.md](SECURITY.md) rather than opening a public issue.

## License

Licensed under either of

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT license](LICENSE-MIT)

at your option. Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in Monitra by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
