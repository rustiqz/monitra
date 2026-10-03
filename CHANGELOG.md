# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.2](https://github.com/rustiqz/monitra/releases/tag/v0.1.2) - 2026-10-03

### Fixed

- *(reliability)* quarantine invalid intervals and schedule result pruning ([#69](https://github.com/rustiqz/monitra/pull/69))

**Full changelog**: https://github.com/rustiqz/monitra/compare/v0.1.1...v0.1.2

## [0.1.1](https://github.com/rustiqz/monitra/releases/tag/v0.1.1) - 2026-10-03

### Fixed

- reject unsupported Postgres and Redis providers ([#51](https://github.com/rustiqz/monitra/pull/51))

**Full changelog**: https://github.com/rustiqz/monitra/compare/v0.1.0...v0.1.1

## [0.1.0](https://github.com/rustiqz/monitra/releases/tag/v0.1.0) - 2026-10-03

### Added

- apply the Monitra brand across README, dashboard and TUI ([#47](https://github.com/rustiqz/monitra/pull/47))
- Phase 12 bundling — release profile, static musl, size gate ([#43](https://github.com/rustiqz/monitra/pull/43))

### Fixed

- six daemon bugs from the 2026-09-30 manual test pass — logging, shutdown, health, agent push ([#45](https://github.com/rustiqz/monitra/pull/45))

**Full changelog**: https://github.com/rustiqz/monitra/compare/monitra-v0.0.4...v0.1.0

## [0.0.4](https://github.com/rustiqz/monitra/releases/tag/monitra-v0.0.4) - 2026-10-02

### Added

- apply the Monitra brand to the README, dashboard and TUI
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

- add secret scanning with gitleaks
- add license, security, conduct and contribution files
- add Monitra logo, favicons and social preview to the landing page and docs site
- replace redirect stub with the Monitra landing page
- add mdBook documentation site published to GitHub Pages
- fix six daemon bugs from the 2026-09-30 manual test pass
- Document deferred/incomplete features found during a Phase 12 audit
- Add Phase 12 bundling: release profile, static musl build, size gate
- bump monitra to 0.0.4 (cascaded from component release)
- release
- Add multi-region latency probing (Phase 11, ADR-011)
- release
- build the embedded dashboard as a real API client (Phase 10)
- build the terminal dashboard as a real API client (Phase 9)
- fix stale phase numbering in phase-start/phase-verify skills
- promote multi-region latency probing to v1 scope (ADR-011)
- Phase 8: agent binary, host checks, push loop, tri-state ingest protocol
- Phase 7: WebSocket fan-out, notifier sinks, agent-ingest, AlertEvent emission
- Merge branch 'main' into phase-6-monitoring-engine
- release
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
