# Getting started

Download options for Linux x86_64 and aarch64 are on the [releases page](https://github.com/rustiqz/monitra/releases).

| Method | Command |
|---|---|
| Verified installer | Run the one-liner below. |
| GitHub Release | Download the matching tarball and `SHA256SUMS` from the [latest release](https://github.com/rustiqz/monitra/releases/latest), verify with `sha256sum -c SHA256SUMS`, and install the extracted binary. |
| Docker | `docker compose up -d` with [compose.yaml](../../../compose.yaml). See [Deployment](deployment.md) for the volume. |
| Cargo Binstall | `cargo binstall monitra --manifest-path Cargo.toml` from a checkout. Plain `cargo binstall monitra` awaits crates.io publication. |
| Build from source | `cargo build --release` with Rust and Node/npm on `PATH`. |

```sh
curl -fsSL https://raw.githubusercontent.com/rustiqz/monitra/main/scripts/install.sh | sh
```

Verify a binary install with `monitra --version`. The installer defaults to `~/.local/bin`; use `--dir PATH` to change it. Ensure the chosen directory is on `PATH`.

Configuration is optional. On a pristine machine, add a monitor directly; this creates the default SQLite database under your XDG data directory.

```sh
monitra monitor add --name example --target https://example.com --kind http --interval 30
monitra monitor list
monitra start
```

On first daemon start, Monitra generates an API token, writes it to its XDG config, and prints it once. Save it. The daemon binds `127.0.0.1:8080` by default. In another terminal, use `monitra tui` for a terminal dashboard or `monitra web` for the embedded browser dashboard. Both can start their own local backend when no `--url` is supplied.

For optional configuration, run `monitra setup` or read [Configuration](configuration.md). See [Deployment](deployment.md) before exposing the daemon remotely.
