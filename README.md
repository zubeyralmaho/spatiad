<p align="center">
  <strong>spatiad (spatia*-daemon)</strong><br>
  Open-source spatial dispatch engine for real-time fleets
</p>

<p align="center">
  <a href="https://github.com/zubeyralmaho/spatiad/actions"><img src="https://github.com/zubeyralmaho/spatiad/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://www.npmjs.com/package/@spatiad/sdk"><img src="https://img.shields.io/npm/v/@spatiad/sdk" alt="npm"></a>
  <a href="./LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License"></a>
</p>

---

Spatiad matches drivers to jobs in real time using [H3 hierarchical geospatial indexing](https://h3geo.org/). It runs as a single Rust binary with zero external dependencies — no database, no message broker, no Redis. Start one process and you have a working dispatch engine.

## How it works

```
                         ┌─────────────┐
                         │  Dispatcher  │
                         │  (your app)  │
                         └──────┬───────┘
                                │  POST /api/v1/dispatch/offer
                                ▼
┌──────────┐  WebSocket  ┌─────────────┐  Webhook (HMAC-signed)
│  Driver   │◄──────────►│   spatiad    │─────────────────────────►  Your backend
│   app     │  location,  │             │  match result callback
└──────────┘  offers      └─────────────┘
                                │
                          H3 spatial index
                          nearest-neighbor
                          radius expansion
```

1. **Drivers** connect via WebSocket and stream their GPS coordinates.
2. **Your app** submits a job with a pickup location and radius constraints.
3. Spatiad finds the nearest available driver using H3 cell lookups, expanding the search radius in +2 km steps.
4. An **offer** is pushed to the driver over WebSocket with a configurable timeout.
5. If the driver accepts, the match is locked and a **signed webhook** is delivered to your backend.
6. If the driver rejects or the offer expires, the next candidate is tried automatically.

## Features

- **H3 geospatial matching** — nearest-neighbor search with configurable resolution and radius expansion
- **Real-time WebSocket** — driver sessions with offer push, expiration ticks, and reconnect replay
- **Full offer lifecycle** — create, expire, reject, re-offer, accept, cancel, with first-acceptor-wins semantics
- **Cursor-paginated event timeline** — every state transition is recorded per job with filterable event kinds
- **HMAC-SHA256 webhook delivery** — signed payloads with timestamp, nonce, and 3-attempt exponential backoff
- **Token authentication** — separate tokens for dispatcher API and driver WebSocket
- **Sliding-window rate limiting** — per-actor throttling with standard `Retry-After` and `X-RateLimit-*` headers
- **Observability** — request ID propagation, structured access logs, readiness/liveness split probes
- **Zero external dependencies** — no database, no cache, no broker; single binary, single process
- **TypeScript SDK** — `@spatiad/sdk` with retry, pagination, abort support, and `@spatiad/express-plugin` for webhook verification

## Quick start

### Prerequisites

- Rust stable toolchain ([rustup.rs](https://rustup.rs))
- Node.js 20+ and pnpm 9+ (for TypeScript SDK)

### Start the server

```bash
cd rust
cargo run -p spatiad-bin
```

The server starts on `http://localhost:3000` with a seeded test driver.

### Verify it's running

```bash
curl http://localhost:3000/health
# {"status":"ok","service":"spatiad"}
```

### Create a dispatch job

```bash
curl -X POST http://localhost:3000/api/v1/dispatch/offer \
  -H "content-type: application/json" \
  -d '{
    "job_id": "22222222-2222-2222-2222-222222222222",
    "category": "tow_truck",
    "pickup": {"latitude": 38.433, "longitude": 26.768},
    "initial_radius_km": 5,
    "max_radius_km": 20,
    "timeout_seconds": 30
  }'
```

### Check job status

```bash
curl http://localhost:3000/api/v1/dispatch/job/22222222-2222-2222-2222-222222222222
```

### Connect as a driver (WebSocket)

```bash
websocat ws://localhost:3000/api/v1/stream/driver/11111111-1111-1111-1111-111111111111
```

Send a location update:
```json
{"type":"location","category":"tow_truck","status":"Available","latitude":38.433,"longitude":26.768,"timestamp":1710000000}
```

Accept an offer:
```json
{"type":"offer_response","offer_id":"<offer-id-from-push>","accepted":true}
```

## TypeScript SDK

```bash
npm i @spatiad/sdk
```

```typescript
import { SpatiadClient } from "@spatiad/sdk";

const client = new SpatiadClient("http://localhost:3000", {
  dispatcherToken: "your-token",
});

// Create a dispatch job
const offer = await client.createOffer({
  jobId: "22222222-2222-2222-2222-222222222222",
  category: "tow_truck",
  pickup: { latitude: 38.433, longitude: 26.768 },
  initialRadiusKm: 5,
  maxRadiusKm: 20,
  timeoutSeconds: 30,
});

// Check job status
const status = await client.getJobStatus({
  jobId: "22222222-2222-2222-2222-222222222222",
});

// Iterate through job events
for await (const event of client.iterateJobEvents({
  jobId: "22222222-2222-2222-2222-222222222222",
  limit: 25,
  resumeOnTransientError: true,
})) {
  console.log(event.kind, event.at);
}
```

For webhook verification, see [`@spatiad/express-plugin`](./typescript/packages/express-plugin).

## API reference

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/health` | Liveness probe |
| `GET` | `/ready` | Readiness probe with active session count |
| `POST` | `/api/v1/driver/upsert` | Register or update driver location and status |
| `POST` | `/api/v1/dispatch/offer` | Submit a job and start candidate search |
| `POST` | `/api/v1/dispatch/cancel` | Cancel a specific offer |
| `POST` | `/api/v1/dispatch/job/cancel` | Cancel an entire job |
| `GET` | `/api/v1/dispatch/job/{id}` | Get job state (pending/searching/matched/exhausted) |
| `GET` | `/api/v1/dispatch/job/{id}/events` | Cursor-paginated event timeline with kind filtering |
| `GET` | `/api/v1/stream/driver/{id}` | WebSocket upgrade for driver session |

Full details: [docs/API_HTTP.md](./docs/API_HTTP.md) | WebSocket protocol: [docs/WS_PROTOCOL.md](./docs/WS_PROTOCOL.md)

## Configuration

All configuration is via environment variables. No config files needed.

| Variable | Default | Description |
|----------|---------|-------------|
| `SPATIAD_BIND_ADDR` | `0.0.0.0:3000` | Listen address |
| `SPATIAD_LOG_LEVEL` | `info` | Tracing filter level |
| `SPATIAD_H3_RESOLUTION` | `8` | H3 cell resolution (0–15) |
| `SPATIAD_DISPATCHER_TOKEN` | — | API authentication token |
| `SPATIAD_DRIVER_TOKEN` | — | WebSocket authentication token |
| `SPATIAD_DISPATCH_RATE_LIMIT_PER_MIN` | `240` | Per-actor request limit |
| `SPATIAD_WS_RECONNECT_MAX_PER_MIN` | `30` | Per-driver reconnect limit |
| `SPATIAD_WEBHOOK_URL` | — | Match result callback URL |
| `SPATIAD_WEBHOOK_SECRET` | — | HMAC-SHA256 signing key |
| `SPATIAD_WEBHOOK_TIMEOUT_MS` | `3000` | Webhook timeout (100–60000) |

Full matrix: [docs/CONFIG.md](./docs/CONFIG.md)

## Deployment

Spatiad ships with multiple deployment options:

```bash
# Docker
docker build -t spatiad .
docker run -p 3000:3000 spatiad

# Docker Compose (local development)
docker compose -f deploy/docker-compose.yml up

# Kubernetes
kubectl apply -f deploy/kubernetes/deployment.yaml

# systemd (bare metal)
sudo cp deploy/systemd/spatiad.service /etc/systemd/system/
sudo cp deploy/systemd/spatiad.env.example /etc/spatiad/spatiad.env
sudo systemctl enable --now spatiad

# One-command install (downloads pre-built binary)
curl -sSL https://raw.githubusercontent.com/zubeyralmaho/spatiad/main/scripts/install.sh | bash
```

Detailed guide: [docs/DEPLOYMENT.md](./docs/DEPLOYMENT.md)

## Architecture

```
spatiad-bin          Binary entrypoint, config, graceful shutdown
  └─ spatiad-api     Axum HTTP/WS router, middleware, rate limiting
      └─ spatiad-dispatch   Offer lifecycle orchestration
          └─ spatiad-core   In-memory engine (drivers, jobs, offers, events)
              ├─ spatiad-h3      H3 spatial index (nearest-neighbor lookups)
              └─ spatiad-types   Shared domain types (Coordinates, DriverStatus, etc.)
```

Types flow downward. No circular dependencies. Each crate has a single responsibility.

Full architecture doc: [ARCHITECTURE.md](./ARCHITECTURE.md)

## Roadmap

We have a six-phase plan to take Spatiad from MVP to a production-grade fleet platform:

1. **Durability & Recovery** — pluggable storage, WAL, persistent event store
2. **Intelligent Matching** — ETA-based scoring, batch dispatch, geofencing
3. **Horizontal Scaling** — H3 cell sharding, message broker, stateless API tier
4. **Operational Maturity** — Prometheus metrics, OpenTelemetry, admin dashboard
5. **Multi-Tenancy** — tenant-scoped keys, resource quotas, namespace isolation
6. **Ecosystem** — plugin system, additional SDKs, fleet simulator

Full details: [ROADMAP.md](./ROADMAP.md) | Join the [discussion](https://github.com/zubeyralmaho/spatiad/discussions/3)

## Contributing

We welcome contributions of all kinds — code, documentation, bug reports, and design feedback.

1. Fork the repo and create a branch (`feat/my-feature`, `fix/my-fix`)
2. Follow [Conventional Commits](https://www.conventionalcommits.org): `type(scope): message`
3. Ensure CI passes: `cargo test` (Rust) and `pnpm -r build && pnpm -r test` (TypeScript)
4. Open a PR against `main`

See [CONTRIBUTING.md](./CONTRIBUTING.md) for full guidelines.

## Documentation

| Document | Description |
|----------|-------------|
| [Getting Started](./docs/GETTING_STARTED.md) | Setup, first request, SDK examples |
| [HTTP API](./docs/API_HTTP.md) | Complete endpoint reference |
| [WebSocket Protocol](./docs/WS_PROTOCOL.md) | Driver stream message format and reconnect behavior |
| [Security](./docs/SECURITY.md) | Authentication, webhook signing, rate limiting |
| [Configuration](./docs/CONFIG.md) | All environment variables |
| [Deployment](./docs/DEPLOYMENT.md) | Docker, Kubernetes, systemd guides |
| [State Machine](./docs/STATE_MACHINE.md) | Job and offer state transitions |
| [Operations Runbook](./docs/OPERATIONS_RUNBOOK.md) | Operational procedures |
| [Troubleshooting](./docs/TROUBLESHOOTING.md) | Common issues and debugging |
| [Architecture](./ARCHITECTURE.md) | Runtime shape and request flow |

## License

[MIT](./LICENSE)
