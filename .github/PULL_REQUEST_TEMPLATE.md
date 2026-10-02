<!-- The PR title must be a Conventional Commit, e.g. `fix: clamp scheduler interval`. -->

## What and why

<!-- What changes, and what problem does it solve? Link the issue: Closes #123 -->

## How it was verified

<!-- Tests added or run, commands, manual checks. -->

## Checklist

- [ ] `cargo build --workspace`, `cargo test --workspace` pass
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] `cargo fmt --all --check` passes
- [ ] `python3 scripts/dep-check.py` passes
- [ ] New behaviour has tests that would fail if it broke
- [ ] No `unwrap()`/`expect()` outside tests; new channels and buffers are bounded
- [ ] `docs/DESIGN.md` and `docs/site/src/` updated where behaviour or the contract changed
