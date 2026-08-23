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

Take these from the **Gate** column of the §10 roadmap table. Summarised:

| Phase | Must prove |
|---|---|
| 1 | Workspace builds; dep-DAG passes; `monitra version` prints build info |
| 2 | Every documented command form parses; `--help` snapshot; `cli` performs **no** I/O |
| 3 | All three §4.1 policies exercised with fake providers: store fails fast, cache degrades, notify queues. **Zero-config path works on a pristine `$HOME`** (§11.7) |
| 4 | Migrations apply to a fresh temp DB; insert/list round-trip; prune over synthetic rows; WAL actually on |
| 5 | Integration tests on an ephemeral port against a real store; `/health` reports internal state, not bare 200 |
| 6 | §6.4 harness at N=100/500/1000 — p99 drift < 2s, RSS < 512 MB, zero missed checks, DB write p50/p99 reported; flap damping N=2 down / N=1 up; monotonic vs wall clock (§11.5); hard timeout against a hanging target |
| 7 | Slow WebSocket client dropped without back-pressuring the engine; notifier retry/backoff bounded |
| 8 | Panic restores the terminal — verified by killing a subprocess mid-render; widget snapshots; both remote-mode impls |
| 9 | Embedded server serves index; SPA calls only the public API |
| 10 | Default build < 25 MB stripped; `ldd` reports not-a-dynamic-executable; feature combos build; §11.6 resolved |

## Reporting

One table: check, result, evidence. Paste real output for anything that fails — never
paraphrase a failure. Then state plainly whether the gate passed.

If a check could not be run, say so and say why. "Skipped" is an honest result; a silent
omission is not.

## Numbers go in the repo

Phase 6's benchmark numbers are the honest scaling limit (§6.4). Record them where they can
be compared against next time, and put them in the README as a number — not an adjective.
