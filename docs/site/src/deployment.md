# Deployment

Build the release binary from source with Rust and Node/npm available. A Linux musl static build also needs the Rust musl target and `musl-gcc` (provided by `musl-tools` on Ubuntu):

```sh
rustup target add x86_64-unknown-linux-musl
CC_x86_64_unknown_linux_musl=musl-gcc cargo build --release --target x86_64-unknown-linux-musl
```

Run `monitra start` to serve the API and embedded web dashboard. It binds `127.0.0.1:8080` by default. `monitra start --bind 0.0.0.0:8080` listens on all interfaces; choose a bind address appropriate to your network. Human API routes require the bearer token saved in XDG config or supplied through `MONITRA_API_TOKEN`; `/health` is public, and agent ingestion uses a separate per-agent token. On first startup without a human token, Monitra generates and prints one once.

For a remote client, pass the daemon URL and token to `monitra tui --url <URL> --token <TOKEN>` or `monitra web --url <URL> --token <TOKEN>`. The latter prints a URL and token to enter in the browser; it does not start a second remote server. The backend serves until SIGINT/SIGTERM and coordinates an engine and HTTP drain on shutdown.

Provider configuration is loaded at startup. `monitra service attach <URL>` configures a provider for the next start; it does not manage an operating system service. See [Configuration](configuration.md).
