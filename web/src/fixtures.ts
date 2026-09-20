// Fixture data for the Globe view only (`api/client.ts`'s `regions` field) —
// every other view now reads real data from `monitra-backend` (ADR-009).
// Real per-region metrics need `Agent.region` and per-region aggregation,
// both Phase 11 / ADR-011 work; see `Region`'s doc comment in
// `./api/types`. Kept as a design/demo reference, not read anywhere else.
import type { Region, Signal } from "./api/types";

const series = (seed: number, center: number, spread: number): number[] =>
  Array.from({ length: 24 }, (_, index) => {
    const wave = Math.sin((index + seed) * 0.73) * 0.54 + Math.cos((index + seed) * 0.29) * 0.31;
    return Math.max(0, Number((center + wave * spread).toFixed(2)));
  });

const region = (
  code: string,
  site: string,
  longitude: number,
  latitude: number,
  status: Signal,
  monitors: number,
  p95: number | null,
  failure: number | null,
  volume: number | null,
  seed: number,
): Region => ({
  code,
  site,
  longitude,
  latitude,
  status,
  monitors,
  p95,
  failure,
  volume,
  hourly: failure === null ? Array(24).fill(0) : series(seed, failure, Math.max(0.12, failure * 0.65)),
});

export const fixtureRegions: Region[] = [
  region("iad", "Ashburn, US", -77.54, 39.02, "up", 9, 74, 0.4, 2880, 2),
  region("sfo", "San Jose, US", -121.89, 37.34, "up", 6, 96, 0.2, 1920, 5),
  region("fra", "Frankfurt, DE", 8.68, 50.11, "up", 3, 61, 0, 960, 8),
  region("dub", "Dublin, IE", -6.26, 53.35, "up", 14, 48, 2.1, 4032, 11),
  region("lhr", "London, UK", -0.13, 51.51, "unknown", 2, 54, null, null, 14),
  region("ams", "Amsterdam, NL", 4.9, 52.37, "up", 4, 58, 0.1, 1152, 17),
  region("syd", "Sydney, AU", 151.21, -33.87, "stale", 3, 241, 1.4, 864, 20),
  region("nrt", "Tokyo, JP", 139.69, 35.68, "up", 4, 188, 0.3, 1152, 23),
  region("sin", "Singapore, SG", 103.82, 1.35, "pending", 5, null, null, null, 26),
  region("gru", "São Paulo, BR", -46.63, -23.55, "down", 3, 212, 6.8, 864, 29),
  region("jnb", "Johannesburg, ZA", 28.05, -26.2, "up", 2, 284, 1.9, 576, 32),
];

// Kept for `client.ts`'s import shape parity with the pre-Phase-10 design
// branch's `fixtureSnapshot.regions`.
export const fixtureSnapshot = { regions: fixtureRegions };
