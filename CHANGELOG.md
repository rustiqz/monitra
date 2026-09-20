# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.3](https://github.com/rustiqz/monitra/releases/tag/monitra-v0.0.3) - 2026-09-20

### Added

- Phase 2 — CLI command tree (parsing only)
- scaffold Phase 1 workspace — 13 crates + root bin

### Fixed

- crate-name rename fallout from the Phase 4/5 rebase
- rename 8 crates.io-colliding crates to stop the release-plz loop
- release-plz workflow_run checkout leaves detached HEAD
- *(ci)* disable release-plz semver-check
- *(ci)* skip release-plz until a Cargo.toml exists
- *(ci)* correct mangled release-plz action reference

### Other

- fix stale Phase 0 header in DESIGN.md
- Axum API with bearer-token auth, monitor/agent CRUD (Phase 5)
- implement SQLite Store (Phase 4)
- bump monitra to 0.0.3 (cascaded from component release)
- release
- bump monitra to 0.0.2 (cascaded from component release)
- release
- cascade monitra's version when an internal crate bumps
- release
- Merge branch 'main' into release-plz-2026-09-17T16-04-13Z
- enforce §11.9 feature matrix, gate release-plz on post-merge CI
- Implement Phase 3: provider layer, config resolution, service/k8s attach
- release
- revert git_only — incompatible with an unpublished workspace
- add main-freshness check to the workflow rules
- enable git_only so monitra's version cascades from its deps
- restrict git tags/releases to the monitra package
- release v0.0.1
- add ADR-008 and ADR-009, resequence roadmap to v0.3
- add CI, PR title lint, and release automation
- Phase 0: scaffolding, design contract, and phase workflow

## [0.0.2](https://github.com/rustiqz/monitra/releases/tag/monitra-v0.0.2) - 2026-09-18

### Added

- Phase 2 — CLI command tree (parsing only)
- scaffold Phase 1 workspace — 13 crates + root bin

### Fixed

- release-plz workflow_run checkout leaves detached HEAD
- *(ci)* disable release-plz semver-check
- *(ci)* skip release-plz until a Cargo.toml exists
- *(ci)* correct mangled release-plz action reference

### Other

- bump monitra to 0.0.2 (cascaded from component release)
- release
- cascade monitra's version when an internal crate bumps
- release
- Merge branch 'main' into release-plz-2026-09-17T16-04-13Z
- enforce §11.9 feature matrix, gate release-plz on post-merge CI
- Implement Phase 3: provider layer, config resolution, service/k8s attach
- release
- revert git_only — incompatible with an unpublished workspace
- add main-freshness check to the workflow rules
- enable git_only so monitra's version cascades from its deps
- restrict git tags/releases to the monitra package
- release v0.0.1
- add ADR-008 and ADR-009, resequence roadmap to v0.3
- add CI, PR title lint, and release automation
- Phase 0: scaffolding, design contract, and phase workflow

## [0.0.1](https://github.com/rustiqz/monitra/releases/tag/monitra-v0.0.1) - 2026-09-17

### Added

- scaffold Phase 1 workspace — 13 crates + root bin

### Fixed

- *(ci)* disable release-plz semver-check
- *(ci)* skip release-plz until a Cargo.toml exists
- *(ci)* correct mangled release-plz action reference

### Other

- add ADR-008 and ADR-009, resequence roadmap to v0.3
- add CI, PR title lint, and release automation
- Phase 0: scaffolding, design contract, and phase workflow
