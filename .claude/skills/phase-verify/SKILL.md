---
name: phase-verify
description: Run the Monitra phase gate — fmt, clippy -D warnings, workspace tests, dependency-DAG check, plus the phase-specific acceptance assertions from the DESIGN.md roadmap. Reports a pass/fail table with real output. Use before declaring any phase complete.
---

# Phase gate

Report what actually happened. A gate that reports success on a skipped step is worse than
no gate — it is the same failure mode as a monitoring tool that lies (P1).

## Universal checks — every phase

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python3 scripts/dep-check.py
```

From Phase 3 onward, also verify the feature matrix does not rot (§11.9):

```sh
cargo build --workspace --no-default-features
cargo build --workspace --all-features
```

## Phase-specific acceptance

Take these from the **Gate** column of the §10 roadmap table — read that table directly
rather than trusting the summary below without checking: §10's own intro paragraph records
each ADR that has resequenced the phase numbers (v0.3 by ADR-008/009, v0.4 by ADR-011), and a
future ADR that resequences again will make this table stale in the same way it already was
once (this row itself is the fix for that staleness, found during Phase 9). Current as of v0.4:

| Phase | Must prove |
|---|---|
| 1 | Workspace builds; dep-DAG passes; `monitra version` prints build info |
| 2 | Every documented command form parses; `--help` snapshot; `cli` performs **no** I/O |
| 3 | All three §4.1 policies exercised with fake providers: store fails fast, cache degrades, notify queues. **Zero-config path works on a pristine `$HOME`** (§11.7) |
| 4 | Migrations apply to a fresh temp DB; insert/list round-trip; prune over synthetic rows; WAL actually on |
| 5 | Integration tests on an ephemeral port against a real store; `/health` reports internal state, not bare 200 |
| 6 | §6.4 harness at N=100/500/1000 — p99 drift < 2s, RSS < 512 MB, zero missed checks, DB write p50/p99 reported; flap damping N=2 down / N=1 up; monotonic vs wall clock (§11.5); hard timeout against a hanging target |
| 7 | Slow WebSocket client dropped without back-pressuring the engine; notifier retry/backoff bounded |
| 8 Agent binary | Local checks produce correct payloads standalone (no backend needed); push loop delivers to a real backend; survives the backend being unreachable without crashing or blocking local checks |
| 9 TUI dashboard | Panic restores the terminal (subprocess test); widget snapshots; local-embedded and remote modes exercise the same client code path |
| 10 Web dashboard | Embedded server serves index; SPA exercises the same auth and full API surface TUI does |
| 11 Multi-region latency probing | `monitra-probe` produces identical `ProbeOutcome`s in both `monitra-engine` and `monitra-agent`; a target monitored from N region-tagged agents aggregates correctly; an agent with no declared region is excluded from regional views, never defaulted |
| 12 Bundling | Default build size re-verified against the added surface (§11.12) rather than assumed at the original 25 MB figure; `ldd` static; feature combos build in CI; §11.6 resolved |

## Reporting

One table: check, result, evidence. Paste real output for anything that fails — never
paraphrase a failure. Then state plainly whether the gate passed.

If a check could not be run, say so and say why. "Skipped" is an honest result; a silent
omission is not.

## Numbers go in the repo

Phase 6's benchmark numbers are the honest scaling limit (§6.4). Record them where they can
be compared against next time, and put them in the README as a number — not an adjective.
