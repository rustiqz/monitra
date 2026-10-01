# Contributing

The repository [contribution guide](https://github.com/rustiqz/monitra/blob/main/CONTRIBUTING.md) covers the project workflow. The [design document](https://github.com/rustiqz/monitra/blob/main/docs/DESIGN.md) contains the architecture and ADRs in section 9.

Before proposing a code change, run the repository's development checks:

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
python3 scripts/dep-check.py
```

The backend build script requires Node/npm on `PATH`, including when Cargo builds or tests workspace crates. Documentation pages live in `docs/site/src/` and can be built with `mdbook build docs/site`.
