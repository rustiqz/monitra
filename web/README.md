# Monitra web dashboard

The embedded Phase 10 dashboard — a real HTTP/WebSocket client of
`monitra-backend`'s API (ADR-009), behind the same `DashboardSource`
interface the original fixture-backed design kept it behind. `regions`
(the Globe view) is the one field still fixture data: real per-region
metrics need `Agent.region` and per-region aggregation, both Phase 11 /
ADR-011 work — the Globe view is a clearly-labeled preview until then.

```sh
npm install
npm run dev
```

`npm run dev` talks to whatever `monitra-backend` is listening on `/` in
the same origin — run `monitra start` (or `monitra web`) separately and
proxy it, or just build and let `monitra` serve it (below).

Production assets are generated in `web/dist/`:

```sh
npm run build
```

`cargo build` (any profile, any crate in the workspace) runs this
automatically via `crates/backend/build.rs`, which shells out to `npm ci
&& npm run build` whenever this directory's sources change — Node/npm is
therefore a build-time prerequisite for the whole workspace, not just this
directory (see the repo root `CLAUDE.md`'s toolchain note). `crates/backend`
embeds whatever lands in `dist/` via `rust-embed` and serves it at `/`.

Views are directly addressable with hashes such as `#fleet`, `#monitor`,
and `#globe`; the SPA is hash-routed, so the server never needs
client-side-route fallback logic beyond serving `index.html` for `/`.

Auth is a one-time gate, not a login system: the backend's human-facing API
is a single static bearer token (§11.11), so on first load the SPA asks for
it, verifies it against a real endpoint, and stores it in `localStorage`
for next time. `/ws` can't carry that token via an `Authorization` header
(the browser `WebSocket` API doesn't expose one) — it's offered as a
`Sec-WebSocket-Protocol` instead, which `monitra-backend`'s
`auth::require_token_ws` accepts alongside the header path the TUI uses.
