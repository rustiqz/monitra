<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/social/monitra-readme-banner-dark.svg">
    <img alt="Monitra" src="assets/brand/social/monitra-readme-banner-light.svg">
  </picture>
</p>

# Monitra

Single-binary, CLI-first uptime monitoring platform in Rust. See `docs/DESIGN.md` for the
full contract (architecture, data model, ADRs); this file just holds the numbers DESIGN.md
asks to be measured rather than assumed.

```sh
cargo build --release
./target/release/monitra monitor add --name web --target https://example.com --kind http --interval 30
./target/release/monitra start
```

## Build size (§1.5, §8, §11.13)

Measured 2026-09-30, Phase 12 toolchain (`opt-level=z`, LTO, stripped), default features
(no `postgres`/`redis`/`slack`/`kubernetes`):

| Build | Size |
|---|---|
| `cargo build --release` (default features) | 7.0 MB |
| `cargo build --release --features kubernetes` | 7.1 MB |
| `cargo build --release --target x86_64-unknown-linux-musl` (static) | 7.1 MB |

All well under the "<25 MB stripped" figure §1.5/§8 quote — that figure predates
ADR-008/009/011's added surface and had never been re-measured against it until now.

## Scaling limit (§6.4)

> **Superseded.** The table below used a drift metric that could not read late-but-consistent
> monitors as late, on a machine well above the target spec. The authoritative results, with a
> corrected metric under 2 vCPU / 512 MB, are in
> [`docs/SCALE_RESULTS.md`](docs/SCALE_RESULTS.md): N=1000 passes (p99 drift 632 ms), N=2500
> passes barely (1979 ms), N=5000 **fails** the 2 s bound (3150 ms). The limit is about 2,500
> monitors at a 30 s interval.

The `#[ignore]`d full falsification protocol (`cargo test --release --test scale --
--ignored --nocapture`, N = 100/500/1000/2500/5000, 10 minutes per N, ~50 minutes total) had
never been run end-to-end before this pass. Run 2026-09-30 on this development machine:

| N | Duration | p99 drift | max drift | RSS (start → end) | Missed checks | DB write p50 / p99 | Result |
|---:|---:|---:|---:|---:|---:|---:|---|
| 100 | 600.0s | 0.0ms | 0.0ms | 8.2 → 109.9 MB | 0 | 2.81 / 10.03ms | PASS |
| 500 | 600.0s | 0.0ms | 0.0ms | 11.8 → 109.6 MB | 0 | 4.81 / 8.08ms | PASS |
| 1000 | 600.0s | 0.0ms | 0.0ms | 23.6 → 100.7 MB | 0 | 5.37 / 12.83ms | PASS |
| 2500 | 600.0s | 0.0ms | 35.6ms | 46.5 → 94.2 MB | 0 | 8.10 / 20.59ms | PASS |
| 5000 | 600.0s | 0.0ms | 178.2ms | 66.2 → 78.8 MB | 0 | 10.08 / 54.49ms | PASS |

All five levels passed the §6.4 acceptance bounds under the original metric on the
development machine. **That does not stand:** the drift metric was flawed and the machine was
far above the 2 vCPU / 512 MB target. With both fixed, a scaling limit appears at about 2,500
monitors; see the note at the top of this section and `docs/SCALE_RESULTS.md`.

RSS *decreasing* as N grows (109.9MB at N=100 vs 78.8MB at N=5000) reflects measurement
timing relative to allocator/OS memory reclamation between runs, not that more monitors use
less memory — read the numbers as "comfortably bounded," not as a precise trend.
