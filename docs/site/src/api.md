# HTTP API

Human API routes require a bearer token. `GET /health` is unauthenticated.

`POST /monitors` and `PATCH /monitors/{id}` accept `interval_secs` from 1 through 86,400. Invalid values return HTTP 400 with a `backend: monitor interval_secs` error.

Monitor responses include `status` and `status_reason`. `status` may be `Pending`, `Up`, `Down`, `Paused`, `Stale`, or `Unknown`. A legacy row with an invalid stored interval is returned as `Unknown` with `status_reason: "invalid stored interval"`. The scheduler does not check that monitor until its interval is repaired. Quarantine never creates a `Down` alert.

`GET /health` includes `quarantined_monitors`, the current count of invalid-interval monitors skipped by the scheduler. This count updates on scheduler resync.

`GET /monitors/{id}/history` and `GET /regions` accept optional `since` Unix seconds. The server applies the later of `since` and the configured raw-result retention cutoff. Uptime percentages in clients use only the returned retained samples. `GET /alerts` reads stored alert events; it does not calculate from raw check results.
