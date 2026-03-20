# CLAUDE.md

This file provides context for AI assistants working on the Spatiad codebase.

## Project overview

Spatiad is an open-source spatial dispatch engine for real-time fleets. It matches drivers to jobs using H3 geospatial indexing, communicates over WebSocket, and delivers match results via signed webhooks.

## Repository layout

```
rust/                          Rust workspace (edition 2021)
  crates/
    spatiad-types/             Shared domain types (Coordinates, DriverStatus, OfferRecord, etc.)
    spatiad-h3/                H3 spatial index (SpatialIndex wrapping h3o)
    spatiad-core/              In-memory dispatch engine (Engine struct, state, events)
    spatiad-dispatch/          Offer lifecycle orchestration (DispatchService)
    spatiad-ws/                WebSocket protocol message types
    spatiad-api/               Axum HTTP + WS handlers, middleware, rate limiting
    spatiad-bin/               Binary entrypoint with config and graceful shutdown
typescript/                    TypeScript monorepo (pnpm 9, Node 20+)
  packages/
    sdk/                       @spatiad/sdk — client library (fetch + retry)
    express-plugin/            @spatiad/express-plugin — webhook verification middleware
  examples/
    ride-dispatch/             Example webhook receiver
docs/                          Project documentation
k8s/                           Kubernetes manifests
dist/                          Distribution assets (installer, systemd unit, env template)
```

## Build & test commands

### Rust

```bash
cd rust
cargo check                    # Type check all crates
cargo test                     # Run all tests
cargo test -p spatiad-api      # Run API tests only
cargo test -p spatiad-core     # Run core engine tests only
cargo run -p spatiad-bin       # Start the server (default :3000)
cargo clippy                   # Lint
```

### TypeScript

```bash
cd typescript
pnpm install                   # Install deps (use --frozen-lockfile in CI)
pnpm -r build                  # Build all packages
pnpm -r test                   # Run all tests
pnpm --filter @spatiad/sdk test           # SDK tests only
pnpm --filter @spatiad/express-plugin test # Plugin tests only
```

## Architecture principles

- **Single process, in-memory** — no external database dependency (persistence is a future phase, see ROADMAP.md)
- **Crate boundaries are API boundaries** — types flow downward: `bin → api → dispatch → core → h3 → types`
- **No circular dependencies** between crates
- **Axum 0.7** is the HTTP/WS framework; handlers use `State<ApiState>` extraction
- **DashMap** for concurrent driver state; `tokio::sync::Mutex` for dispatch service access
- **H3 resolution** is configurable at startup (default 8, env `SPATIAD_H3_RESOLUTION`)

## Coding conventions

- **Rust edition 2021**, stable toolchain
- Use `thiserror` for library errors, `anyhow` for binary/application errors
- Prefer `tracing::info!` / `tracing::warn!` over `println!`
- All public API types derive `Serialize, Deserialize` via serde
- UUIDs are `uuid::Uuid` with v4 generation
- Timestamps are `chrono::DateTime<Utc>`
- Conventional Commits enforced by commitlint: `type(scope): message`
- Valid types: `feat`, `fix`, `docs`, `refactor`, `test`, `chore`
- Branch naming: `feat/*`, `fix/*`, `docs/*`, `chore/*`

## Configuration (environment variables)

| Variable | Default | Purpose |
|----------|---------|---------|
| `SPATIAD_BIND_ADDR` | `0.0.0.0:3000` | Listen address |
| `SPATIAD_LOG_LEVEL` | `info` | Tracing filter |
| `SPATIAD_H3_RESOLUTION` | `8` | H3 cell resolution (0-15) |
| `SPATIAD_DISPATCH_RATE_LIMIT_PER_MIN` | `240` | Sliding window rate limit |
| `SPATIAD_WS_RECONNECT_MAX_PER_MIN` | `30` | WebSocket reconnect guard |
| `SPATIAD_WEBHOOK_URL` | *(none)* | Match callback endpoint |
| `SPATIAD_WEBHOOK_SECRET` | *(none)* | HMAC-SHA256 signing key |
| `SPATIAD_WEBHOOK_TIMEOUT_MS` | `3000` | Webhook HTTP timeout (100-60000) |
| `SPATIAD_DRIVER_TOKEN` | *(none)* | WebSocket auth token |
| `SPATIAD_DISPATCHER_TOKEN` | *(none)* | HTTP API auth token |

## Key types and entry points

- `Engine` (`spatiad-core/src/lib.rs`) — central state machine; holds drivers, jobs, offers, events
- `DispatchService` (`spatiad-dispatch/src/lib.rs`) — wraps Engine with offer orchestration logic
- `ApiState` (`spatiad-api/src/lib.rs`) — shared application state passed to Axum handlers
- `router()` (`spatiad-api/src/lib.rs`) — builds the full Axum router with all routes and middleware
- `main()` (`spatiad-bin/src/main.rs`) — reads config, seeds test data, starts server

## API surface

```
GET  /health                           Liveness probe
GET  /ready                            Readiness probe (reports active sessions)
POST /api/v1/driver/upsert             Register or update driver location
POST /api/v1/dispatch/offer            Create a dispatch job and find candidates
POST /api/v1/dispatch/cancel           Cancel a specific offer
POST /api/v1/dispatch/job/cancel       Cancel an entire job
GET  /api/v1/dispatch/job/{id}         Job status (pending/searching/matched/exhausted)
GET  /api/v1/dispatch/job/{id}/events  Cursor-paginated event timeline
GET  /api/v1/stream/driver/{id}        WebSocket upgrade for driver sessions
```

## CI pipeline

GitHub Actions (`.github/workflows/ci.yml`):
- **Rust job**: `cargo check` + `cargo test` on ubuntu-latest with stable toolchain
- **TypeScript job**: `pnpm install --frozen-lockfile` + `pnpm -r build` + `pnpm -r test`

Runs on push to `main`, `feat/**`, `fix/**`, `docs/**`, `chore/**`, and all PRs.

## Common pitfalls

- The dispatch service is behind a `tokio::sync::Mutex` — hold the lock for the shortest time possible
- Offer expiration is driven by a background task (`start_background_tasks`), not lazy evaluation
- WebSocket reconnect replays pending offers — test reconnect scenarios when modifying offer state
- The seeded test driver (`11111111-...`) is hardcoded in `main.rs` for development convenience
- Rate limiter and reconnect guard are per-actor, sliding window — check `SlidingWindowRateLimiter` when modifying throttling
