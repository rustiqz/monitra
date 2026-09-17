# Contributing to Monitra

`docs/DESIGN.md` is the architecture contract — read the relevant section before writing
code in an area it covers. This document is about *process*: how changes get from a branch
to a release.

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
