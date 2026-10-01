# Scale sweep results (DESIGN.md §6.4)

Authoritative results are the **capped, corrected-metric** runs in the first section. The
uncapped runs further down are kept for history and are **superseded**: they used a drift metric
with known flaws (see *Why the earlier figures are superseded*) on hardware well above the
§1.5 target.

## Authoritative: 2 vCPU / 512 MB, corrected drift metric

Run 2026-10-01 on the commit under test (`tests/scale.rs` with the corrected drift metric,
based on `ab943fa`). The test binary was built first, then run inside a cgroup:

```sh
cargo test --release --test scale --no-run
systemd-run --user --scope -p CPUQuota=200% -p MemoryMax=512M \
  target/release/deps/scale-<hash> --ignored --nocapture --test-threads=1 scale_n1000 scale_n5000
# N=2500 was run the same way in a separate invocation
```

Limits verified from the cgroup: `cpu.max = 200000 100000` (2 CPUs), `memory.max = 536870912` (512 MB).
Host: Intel i5-10210U (4 cores / 8 threads), 7.8 GB RAM, Linux 7.2.4-arch1-2.

| N monitors | Duration | p99 drift | Max drift | Min drift | Lagged | Missed | RSS (start → end) | DB write p50 / p99 | Result |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| 1000 | 600 s | 631.8 ms | 717.1 ms | 10.9 ms | 0 | 0 | 7.1 → 23.8 MB | 5.24 / 10.75 ms | pass |
| 2500 | 600 s | **1978.9 ms** | 2258.4 ms | 36.7 ms | 0 | 0 | 9.5 → 30.9 MB | 6.09 / 15.73 ms | pass (21 ms under bound) |
| 5000 | 600 s | **3150.1 ms** | 3760.9 ms | 56.2 ms | 0 | 0 | 13.7 → 41.1 MB | 6.37 / 34.10 ms | **FAIL** (bound 2000 ms) |

Each run: 30 s interval, 20 expected checks per monitor. N=100 and N=500 were not re-run under the cap.

### Against the §6.4 targets (2 vCPU / 512 MB)

| Target | Measured | Verdict |
|---|---|---|
| 1,000 monitors at 30 s on 2 vCPU / 512 MB (§1.5) | p99 drift 631.8 ms at N=1000 | **met**, ~3x under the 2 s bound |
| p99 schedule drift < 2 s | 1978.9 ms at N=2500; 3150.1 ms at N=5000 | holds at 2500 (barely), **fails at 5000** |
| RSS < 512 MB | 41.1 MB at N=5000 | met, not close |
| Zero missed checks | 0 at every N | met |
| No dropped results on the measurement path | `lagged = 0` at every N | met |

### Scaling limit

**On this setup the p99 < 2 s bound is crossed between N=2500 and N=5000, and N=2500 passes by
only ~1%.** Read the limit as "about 2,500 monitors at a 30 s interval on 2 vCPUs", not as a
guarantee: a repeat run at N=2500 could plausibly fail. The limit is CPU-bound — memory stays
under 45 MB and nothing lagged or was missed, the scheduler simply dispatches late. Drift grows
faster than linearly in N through this range (N=1000 to N=2500 is 2.5x the monitors for ~3.1x the
drift).

This is the first honest scaling limit the harness has produced, in the sense DESIGN.md §6.4 asks
for. It does not trigger the §11.2 timer-wheel decision on its own (that needs a profile of
*where* the time goes), but it is the evidence that decision would be made against.

### Caveats on the authoritative runs

- **Mock targets share the CPUs.** The harness runs N mock HTTP servers and the results
  subscriber in the same process and cgroup as the engine, so they consume part of the 2 CPUs.
  This overstates drift; real deployments probe remote targets. The true limit is probably higher
  than measured, by an unknown amount.
- **Startup counts as drift.** The epoch is taken just before engine start (a lower bound on the
  scheduler's real start), so reported drift can overstate lateness, never understate it.
- **Single run per N.** No variance estimate. This matters most for N=2500.
- **No TLS, no real network latency.** Other real costs named in §6.4 are not exercised.
- **Not re-run:** N=100, N=500 under the cap; N between 2500 and 5000.

## Why the earlier figures are superseded

The first harness measured drift against each monitor's *first observed result* instead of the
scheduler's deadline, clamped early results to zero, skipped each monitor's first sample, silently
`continue`d past `broadcast::Lagged` events, and measured at the subscriber. A monitor that was
consistently late therefore read as on time, which is why p99 drift was often exactly 0.0 ms and
did not vary sensibly with N. The corrected metric anchors to a common epoch (all monitors are
registered at the same instant, so deadlines are `epoch + k * interval`), subtracts each probe's
own latency, assigns each result to its nearest scheduled slot, reports signed drift with a
`min_drift` column, and counts and fails on any lagged events.

## Superseded: uncapped runs with the original drift metric

All on the same host with no cgroup limit (4 cores / 8 threads, 7.8 GB). All passed, but drift
figures are not trustworthy for the reason above.

2026-10-01 (commit `ab943fa`, 3061 s total):

| N | p99 drift | Max drift | Missed | RSS (start → end) | DB write p50 / p99 |
|---:|---:|---:|---:|---:|---:|
| 100 | 0.0 ms | 0.0 ms | 0 | 9.4 → 10.0 MB | 2.72 / 4.15 ms |
| 500 | 96.5 ms | 116.1 ms | 0 | 12.2 → 25.0 MB | 3.60 / 6.22 ms |
| 1000 | 107.6 ms | 133.0 ms | 0 | 9.6 → 25.6 MB | 4.80 / 11.25 ms |
| 2500 | 0.0 ms | 27.5 ms | 0 | 11.0 → 31.6 MB | 6.48 / 17.50 ms |
| 5000 | 68.5 ms | 290.3 ms | 0 | 17.4 → 44.4 MB | 7.08 / 31.02 ms |

2026-09-30 (recorded in the README): p99 drift 0.0 ms at every N, max drift
0.0 / 0.0 / 0.0 / 35.6 / 178.2 ms, RSS ending near 80-110 MB, DB write p99
10.03 / 8.08 / 12.83 / 20.59 / 54.49 ms for N = 100 / 500 / 1000 / 2500 / 5000.

A 60 s `preliminary_n500` check is also non-authoritative.
