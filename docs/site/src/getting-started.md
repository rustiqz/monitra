# Getting started

Build Monitra from this repository with Rust and Node/npm on `PATH`. The backend build script runs `npm ci` and builds the embedded web UI.

```sh
cargo build --release
./target/release/monitra version
```

Configuration is optional. On a pristine machine, add a monitor directly; this creates the default SQLite database under your XDG data directory.

```sh
./target/release/monitra monitor add --name example --target https://example.com --kind http --interval 30
./target/release/monitra monitor list
./target/release/monitra start
```

On first daemon start, Monitra generates an API token, writes it to its XDG config, and prints it once. Save it. The daemon binds `127.0.0.1:8080` by default. In another terminal, use `monitra tui` for a terminal dashboard or `monitra web` for the embedded browser dashboard. Both can start their own local backend when no `--url` is supplied.

For optional configuration, run `monitra setup` or read [Configuration](configuration.md). See [Deployment](deployment.md) before exposing the daemon remotely.
