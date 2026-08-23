---
name: dep-check
description: Verify the Monitra crate dependency graph matches the DAG declared in DESIGN.md 3.2 (P4 / ADR-005 / ADR-007). Use after adding a crate or a dependency, and as part of every phase gate.
---

# Dependency DAG check

```sh
python3 scripts/dep-check.py
```

Boundaries that exist only as a convention get violated. This makes the §3.2 table a build
error rather than a docs paragraph.

## The rule being enforced

`models` depends on nothing internal. `provider` depends only on `models`. Every provider
implementation (`storage`, `store-*`, `cache-*`, `notify-*`) depends on `models` + `provider`
and **never on a sibling provider**. `engine`, `backend`, and `tui` depend on `provider`
traits — never on a concrete implementation. Only the root binary sees everything.

The most likely violation in practice is `engine` or `backend` importing `storage` for
convenience. That is the thing this check exists to catch: it compiles fine, it works fine,
and it quietly welds the whole product to SQLite, undoing ADR-007.

## Adding a crate

The policy map in `scripts/dep-check.py` mirrors the DESIGN.md 3.2 table. A crate absent from
the policy is a **failure**, not a pass — this is deliberate, so a new crate cannot slip in
without its boundaries being stated.

Update, in this order:

1. `docs/DESIGN.md` §3.2 table and graph — the contract.
2. `docs/DESIGN.md` Appendix repository layout.
3. `ALLOWED` in `scripts/dep-check.py`.
4. `CLAUDE.md` if the shape of the graph changed, not just its size.

If the new crate does not fit the existing shape, that is an ADR, not a table edit.
