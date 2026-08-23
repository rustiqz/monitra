# Monitra — working agreement

Single-binary, CLI-first uptime monitoring platform in Rust.

**`docs/DESIGN.md` is the contract.** It describes what we are building, not what exists.
Read the relevant section before writing code in that area. When code and DESIGN.md
disagree, that is a bug in one of them — resolve it, never leave it.

## Workflow (agreed with the user — do not skip)

1. **Brief before building.** Before any phase or non-trivial change, state in brief what
   will be implemented and get confirmation. No code until the decision is finalised.
2. **Surface choices as choices.** Where a fork exists, present the options with trade-offs
   rather than silently picking. Use the `phase-start` skill.
3. **Report after building.** Deliver: what was built *in each file*, any control/data flow
   introduced, and the decision points actually implemented.
4. **Never run `git commit`.** Provide the commit message; the user commits.
5. **Gate every phase.** Run `phase-verify` before declaring a phase done.

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
models ← provider ← {storage, store-*, cache-*, notify-*}
                  ← engine ← backend
                  ← tui
models ← cli
```

`engine`, `backend`, and `tui` hold `Arc<dyn Store>` — they must **never** import `storage`
or any concrete provider. Only `main.rs` knows which implementations exist.

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

## Provider model (ADR-007)

Three categories, each with an embedded default that needs nothing external. Attaching an
external service is opt-in, by URL, via `monitra setup` or config. Availability failures are
handled **per category** (DESIGN.md 4.1):

| Category | Default | Unreachable when configured |
|---|---|---|
| `Store` | SQLite | **fail fast** — a silent fallback splits history and makes uptime lie |
| `Cache` | in-process | degrade to default, WARN, show in `/health` |
| `Notifier` | log sink | bounded queue + retry, log loudly, never block a probe |

Providers register at compile time behind cargo features. Default build stays under 25 MB.

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
target exists and there is no `musl-gcc`. Phase 10's static musl build needs `rustup` or `cross`
installed first — flag this before starting Phase 10, do not silently skip the static build.

## Style

Match surrounding code. Comments explain *why*, not *what* — the DESIGN.md rationale is the
valuable part and does not survive in a diff.
