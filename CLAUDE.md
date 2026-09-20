# Monitra — working agreement

Single-binary, CLI-first uptime monitoring platform in Rust.

**`docs/DESIGN.md` is the contract.** It describes what we are building, not what exists.
Read the relevant section before writing code in that area. When code and DESIGN.md
disagree, that is a bug in one of them — resolve it, never leave it.

**Architecture decisions and their history live in DESIGN.md §9 (ADRs).** This file only
tracks the operational rules that follow from the current decisions — when an ADR changes
one of those rules (crate graph, provider categories, etc.), this file must be updated in
the same change, not left stale. Three recent ones worth knowing before touching the crate
graph or the client architecture: **ADR-008** (distributed agents — Kubernetes direct-poll +
push fallback, host-agent checks, `Agent`/`AlertEvent` entities), **ADR-009** (backend-first
clients — `tui`/web are pure API clients in every mode, ADR-004 superseded), and **ADR-011**
(multi-region latency probing promoted to v1 — `Agent.region`, probe execution extracted into
a new shared `monitra-probe` leaf so `monitra-agent` can run real network probes; Phase 11,
not yet built).

## Workflow (agreed with the user — do not skip)

1. **Brief before building.** Before any phase or non-trivial change, state in brief what
   will be implemented and get confirmation. No code until the decision is finalised.
2. **Surface choices as choices.** Where a fork exists, present the options with trade-offs
   rather than silently picking. Use the `phase-start` skill.
3. **Report after building.** Deliver: what was built *in each file*, any control/data flow
   introduced, and the decision points actually implemented.
4. **Commit, push, and raise PRs when asked.** Show the exact commit message first and wait
   for confirmation (per the global working-process rule), then commit, push, and open the PR.
5. **Gate every phase.** Run `phase-verify` before declaring a phase done.
6. **Check `main` is current before branching.** `git fetch origin main` and fast-forward
   local `main` before creating any branch or worktree — this repo's release-plz jobs and PR
   merges land fast enough that local `main` goes stale within a single session, and branching
   off a stale `main` means untangling it later instead of a 5-second check up front.

## Hard rules

- **No `unwrap()` / `expect()`** outside `#[cfg(test)]` and provably-infallible cases (P1).
  A panic in the engine takes down monitoring for everything.
- **Errors name their component.** `thiserror` enums per crate; `anyhow` + `.context()` at
  boundaries. `"storage: failed to open monitra.db: permission denied"`, never `"error: denied"`.
- **Bounded everything.** Every channel, queue, and buffer has an explicit bound. On overflow:
  drop and log loudly. A silent drop violates P1 (§7.3).
- **No new external runtime dependency.** P2 is load-bearing, not a preference (§2).
- **Every capability reachable from the CLI.** Web-only functionality is incomplete (P3).
- **Never report "down" for a failure on our side.** Permission errors, internal panics, and
  scheduler gaps are distinct states from target-down (P1, §11.3).
- **`clippy -D warnings` is the floor**, not an aspiration.

## Dependency DAG (DESIGN.md 3.2) — enforced by `scripts/dep-check.py`

```
monitra-models ← monitra-provider ← {monitra-storage, store-*, cache-*, notify-*, collector-*}
                                  ← monitra-engine ← monitra-backend
monitra-models ← monitra-probe   ← {monitra-engine, monitra-agent}   (ADR-011 — shared probe execution, not a provider)
monitra-models ← monitra-cli
monitra-models ← monitra-tui     (ADR-009 — API client only; must never import monitra-provider or monitra-storage)
monitra-models ← monitra-agent   (ADR-008/ADR-011 — push client + regional prober; may import monitra-probe only)
```

(The `monitra-` prefix on 9 of the 14 crate names — `models`, `provider`, `storage`, `engine`,
`backend`, `cli`, `agent`, `tui`, `probe` — exists only to avoid colliding with unrelated public
crates of the same short name on crates.io; those collisions were silently feeding release-plz's
per-package version-diff a foreign package to compare against, forcing a spurious release PR
every cycle. `store-postgres`, `cache-redis`, `notify-webhook`, `notify-slack`, and
`collector-kubernetes` had no collision and keep their short names.)

`monitra-engine` and `monitra-backend` hold `Arc<dyn Store>` (and the other provider traits) —
they must **never** import `monitra-storage` or any concrete provider directly. `monitra-tui`
and `monitra-agent` are stricter still: they must never import `monitra-provider` at all, in
any mode — `monitra-tui` only ever speaks the wire protocol over HTTP/WS (even for a local,
no-daemon session, via an embedded backend `main.rs` boots on loopback), and `monitra-agent`
only ever pushes to it. `monitra-probe` (ADR-011) is the one exception to "leaves are providers
or wired-by-main.rs-only": it holds no trait, no registry entry, and is imported directly by
both `monitra-engine` and `monitra-agent` because it's the same probe code running in two
processes, not a swappable implementation. Only `main.rs` knows which provider implementations
exist.

Adding a crate means updating **both** DESIGN.md 3.2 and `scripts/dep-check.py`. dep-check
fails on any crate missing from its policy, by design.

## Design principles (DESIGN.md 2) — earlier wins on conflict

| | |
|---|---|
| P1 | Reliability over features — never guess; say "unknown" |
| P2 | Single binary, always |
| P3 | CLI is the primary interface |
| P4 | Modular crates, acyclic dependencies |
| P5 | Correct before fast — optimise only against a benchmark in the repo |
| P6 | Boring, observable operations — diagnosable at 3 a.m. |

## Provider model (ADR-007, extended by ADR-008)

Four categories. Three have an embedded default that needs nothing external; the fourth
(`Collector`, added by ADR-008) has none by design — it only exists when explicitly configured.
Attaching an external service is opt-in, by URL, via `monitra setup` or config. Availability
failures are handled **per category** (DESIGN.md 4.1):

| Category | Default | Unreachable when configured |
|---|---|---|
| `Store` | SQLite | **fail fast** — a silent fallback splits history and makes uptime lie |
| `Cache` | in-process | degrade to default, WARN, show in `/health` |
| `Notifier` | log sink | bounded queue + retry, log loudly, never block a probe |
| `Collector` | *(none)* | per-resource WARN + unknown status on the affected Monitor; never fails the daemon |

Providers register at compile time behind cargo features. Default build target is under 25 MB,
but that figure predates ADR-008/009/011's added surface and needs re-measuring, not assuming
(DESIGN.md §11.13) — don't quote it as settled until Phase 12 actually checks it.

**`monitra setup` must never become mandatory** (§11.7) — `monitra monitor add` on a pristine
machine with no config must work.

## Commands

```sh
cargo build --workspace
cargo test  --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
python3 scripts/dep-check.py
```

Toolchain note: Rust is installed via pacman, **not rustup**. Only the `x86_64-unknown-linux-gnu`
target exists and there is no `musl-gcc`. Phase 12's static musl build needs `rustup` or `cross`
installed first — flag this before starting Phase 12, do not silently skip the static build.

Toolchain note (Phase 10): `cargo build`/`cargo test` on **any** workspace crate now requires
Node/npm on `PATH` — `crates/backend/build.rs` shells out to `npm ci && npm run build` against
`web/` and embeds the result via `rust-embed`. This machine has it via `mise`. A build
environment without Node/npm fails at `monitra-backend`'s build script with a named error
(`monitra-backend/build.rs: failed to run …`), not a silent skip.

## Style

Match surrounding code. Comments explain *why*, not *what* — the DESIGN.md rationale is the
valuable part and does not survive in a diff.
