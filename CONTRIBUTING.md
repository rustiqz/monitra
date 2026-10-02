# Contributing to Monitra

Thanks for helping make Monitra better. This guide covers how to set up, what a change must
satisfy, and how a change travels from a branch to a release.

By participating you agree to the [Code of Conduct](CODE_OF_CONDUCT.md). Report security
issues privately, as described in [SECURITY.md](SECURITY.md).

## The contract

[`docs/DESIGN.md`](docs/DESIGN.md) describes what Monitra is meant to be, including its
principles (§2), crate graph (§3.2) and architecture decision records (§9). Read the section
for the area you are touching before writing code. If code and DESIGN.md disagree, that is a
bug in one of them. Fix whichever is wrong in the same change.

**For anything beyond a small fix, open an issue first.** Say what you plan to change and why,
and wait for agreement. That avoids work on a direction the design already rules out.

## Development setup

You need:

- A recent stable Rust toolchain (edition 2024), installed via `rustup`.
- Node.js and npm on `PATH`. `crates/backend/build.rs` runs `npm ci && npm run build` for the
  embedded web UI, so **every** `cargo build` or `cargo test` needs them. Without them the
  build fails with a named error rather than skipping the UI.
- Python 3 for `scripts/dep-check.py`.
- Optional: `musl-gcc` and `rustup target add x86_64-unknown-linux-musl` for the static build.

```sh
git clone https://github.com/rustiqz/monitra.git
cd monitra
cargo build --workspace
```

## Before you open a pull request

Run all five checks. CI runs the same ones, and a change is not ready until they pass.

```sh
cargo build --workspace
cargo test  --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
python3 scripts/dep-check.py
```

Clippy warnings are errors. That is the floor, not a stretch goal.

## Engineering rules

These follow from the design principles, where earlier principles win on conflict
(P1 reliability, P2 single binary, P3 CLI first, P4 modular crates, P5 correct before fast,
P6 boring operations).

- **No `unwrap()` or `expect()`** outside `#[cfg(test)]` and provably infallible cases. A
  panic in the engine takes down monitoring for everything.
- **Errors name their component.** Use `thiserror` enums per crate and `anyhow` with
  `.context()` at boundaries: `storage: failed to open monitra.db: permission denied`, never
  `error: denied`.
- **Bound everything.** Every channel, queue and buffer has an explicit limit. On overflow,
  drop and log loudly. A silent drop is a bug.
- **No new external runtime dependency.** The single-binary guarantee (P2) is load-bearing.
- **Every capability is reachable from the CLI.** Web-only functionality is incomplete.
- **Never report "down" for a failure on our side.** Permission errors, internal panics and
  scheduler gaps are distinct states from target-down.
- **Optimise only against a benchmark in the repo** (P5).
- **Every component ships with real tests** that would fail if the behaviour broke.

### Crate dependencies

The crate graph is a DAG enforced by `scripts/dep-check.py`. In short: `monitra-engine` and
`monitra-backend` hold trait objects and must never import `monitra-storage` or a concrete
provider. `monitra-tui` and `monitra-agent` must never import `monitra-provider`. Only
`main.rs` knows which providers exist.

Adding a crate means updating **both** DESIGN.md §3.2 and `scripts/dep-check.py`.

### Decisions

If your change alters a principle, the crate graph or the roadmap, record it as an ADR in
DESIGN.md §9. Update any operational rules that follow from it in the same change.

### Code style

Match the surrounding code. Comments should explain *why*, not *what*. The rationale is the
part that does not survive in a diff.

## Branching and PRs

- **No direct commits to `main`.** Enforced by branch protection, including for the repo
  owner. Every change — code, docs, config — goes through a branch and a pull request.
- Branch from `main`, open a PR into `main`.
- CI must pass before merge (`cargo fmt --check`, `clippy -D warnings`,
  `cargo test --workspace`, `scripts/dep-check.py`).
- **Merge strategy: squash.** The PR title becomes the commit on `main`, so it has to be a
  valid Conventional Commit on its own — see below.
- No required review count currently (solo/small-team project) — CI passing is the gate.

## PR title format (Conventional Commits)

PR titles are linted and must match:

```
type(scope): summary
```

Allowed types: `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `build`, `ci`, `chore`.
Scope is optional but should usually be the crate a change touches (e.g. `feat(engine): ...`,
`fix(storage): ...`) — it makes it obvious at a glance which component moved, even though
version bumps are computed from changed file paths, not the scope string itself.

## Commit and PR body format

Flat and minimal — a changelog entry, not a narrative:

- Title: `type(scope): summary`
- Body: flat bullet points only, one line each — what was added, changed, or removed
- No paragraphs, no rationale, no references to the task, ticket, or how the change was
  built — that belongs in the PR discussion, not the permanent history

## Versioning

Each crate under `crates/` versions independently, starting at `0.0.1`. A crate's version
only moves when a PR actually touches that crate — untouched crates don't bump just because
a release happened elsewhere in the workspace.

The bump type (major/minor/patch) is computed automatically from Conventional Commit types
on the PRs that touched each crate since its last release (`fix` → patch, `feat` → minor,
`BREAKING CHANGE:` footer → major).

The root `monitra` binary crate's version is the **overall release version** shown to users.
It moves by one patch on every release, regardless of which internal component actually
changed — it's an identifier for "which release this is," not a semantic rollup of internal
crate churn.

## Release process

1. A PR merges to `main` (squash merge, CI green).
2. `release-plz` opens or updates a bot-authored `chore: release` PR on `main`, containing
   the computed version bumps and changelog entries for every crate with unreleased changes.
3. That PR goes through the same rules as any other — CI must pass, no direct merge.
4. Merging the release PR tags the affected crates and publishes a GitHub Release. This is
   the only way a release happens — there is no separate manual release step.

Crates are not published to crates.io; `release-plz` is used here purely for independent
per-crate version + changelog + GitHub Release management.

## Documentation

User documentation lives in `docs/site/src/` as an mdBook (`mdbook build docs/site`). Update
it when behaviour a user can see changes. Keep
[Limitations](docs/site/src/limitations.md) honest: if you find a gap, document it rather
than hiding it.

## Reporting bugs and requesting features

Use the [issue templates](https://github.com/rustiqz/monitra/issues/new/choose). For bugs,
include the Monitra version (`monitra version`), your OS, the command you ran and the output.
For anything that might be a security problem, follow [SECURITY.md](SECURITY.md) instead.

## License

Monitra is dual licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE). Unless you
state otherwise, any contribution you intentionally submit for inclusion is licensed the same
way, with no additional terms or conditions.
