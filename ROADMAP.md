# Roadmap

This document describes the long-term vision for Spatiad. It is a living document and will evolve as the project matures and the community grows.

Spatiad's mission is to become the **go-to open-source spatial dispatch engine** — a production-grade, extensible foundation that any team can deploy to power real-time fleet operations without building from scratch.

## Where we are today (v0.1)

The MVP baseline is complete. Spatiad ships as a single-process, fully in-memory runtime that covers the core dispatch lifecycle:

- H3-based nearest-neighbor matching with radius expansion
- Real-time WebSocket sessions for drivers
- Job offer lifecycle (create, expire, reject, re-offer, accept, match)
- Cursor-paginated job event timeline
- HMAC-SHA256 webhook delivery with retry
- Token authentication, sliding-window rate limiting
- Readiness/liveness probes, request ID propagation, access logs
- TypeScript SDK (`@spatiad/sdk`) and Express webhook plugin
- Docker, Kubernetes, and systemd deployment

This is a solid starting point — but there is a long road ahead.

---

## Phase 1 — Durability & Recovery

**Goal**: Spatiad must not lose state on restart.

| Area | Description |
|------|-------------|
| Pluggable storage backend | Introduce a `StorageBackend` trait so the engine can persist state to SQLite (embedded) or PostgreSQL (networked). The in-memory backend remains the default for development and testing. |
| Write-ahead log (WAL) | Every state mutation is appended to a durable log before being applied. On crash recovery the WAL is replayed to reconstruct state. |
| Event store | Replace the capped in-memory event timeline (currently 200 entries per job) with an append-only persistent event store. This also enables post-hoc analytics and audit. |
| Snapshot & compaction | Periodic snapshots reduce WAL replay time. Old WAL segments are compacted after a snapshot is confirmed. |

**Why this is first**: Without durability, Spatiad cannot be trusted in any environment where uptime matters. Every feature that follows assumes state survives restarts.

---

## Phase 2 — Intelligent Matching

**Goal**: Move from "nearest available driver" to "best available driver."

| Area | Description |
|------|-------------|
| ETA-based matching | Integrate with a routing engine (OSRM or Valhalla) to rank candidates by estimated time of arrival instead of straight-line distance. |
| Multi-factor scoring | Combine distance, ETA, driver rating, current workload, and category affinity into a configurable scoring function. |
| Batch dispatch | When multiple jobs arrive in a short window, solve the assignment as a batch optimization problem rather than sequential greedy search. |
| Geofencing & zones | Define geographic zones (service areas, surge zones, restricted regions) and enforce rules per zone. |

---

## Phase 3 — Horizontal Scaling

**Goal**: Spatiad runs as a cluster, not a single process.

| Area | Description |
|------|-------------|
| H3 cell partitioning | Shard the spatial index by H3 cell ranges. Each node owns a set of cells and handles drivers within them. |
| Message broker integration | Adopt NATS or Redis Streams for inter-node communication, event fan-out, and webhook delivery decoupling. |
| Consistent hashing & rebalancing | When nodes join or leave, cell ownership is rebalanced with minimal disruption. |
| Stateless API tier | Separate the API/WebSocket gateway from the dispatch workers so they can scale independently. |

---

## Phase 4 — Operational Maturity

**Goal**: First-class observability and operator tooling.

| Area | Description |
|------|-------------|
| Prometheus metrics | Export dispatch latency histograms, match rates, offer funnel metrics, active session gauges via a `/metrics` endpoint. |
| OpenTelemetry tracing | Distributed traces across API → dispatch → webhook delivery with Jaeger/Tempo export. |
| Admin dashboard | A lightweight web UI showing a live map, driver/job state, and key metrics. |
| Immutable audit log | A tamper-evident, queryable log of all dispatch decisions for compliance and debugging. |

---

## Phase 5 — Multi-Tenancy

**Goal**: A single Spatiad deployment serves multiple independent fleets.

| Area | Description |
|------|-------------|
| Tenant-scoped API keys | Each tenant gets isolated credentials and namespaced data. |
| Resource quotas | Per-tenant rate limits, driver caps, and job throughput limits. |
| Tenant routing | Requests are routed to the correct shard or namespace based on tenant identity. |

---

## Phase 6 — Ecosystem & Extensibility

**Goal**: Spatiad is a platform, not just a binary.

| Area | Description |
|------|-------------|
| Plugin system | A `DispatchStrategy` trait (and eventually WASM plugin host) lets teams inject custom matching logic without forking. |
| Additional SDKs | Python, Go, and Java clients with generated types from the OpenAPI spec. |
| Fleet simulator | A built-in load testing and simulation framework for benchmarking dispatch strategies against synthetic fleets. |
| OpenAPI spec | A machine-readable API specification that enables code generation and third-party tooling. |

---

## How to contribute

We welcome contributions at every phase. If a topic interests you:

1. Check the [Discussions](https://github.com/zubeyralmaho/spatiad/discussions) tab for the matching thread.
2. Comment with your thoughts, questions, or a proposal.
3. Once the approach is agreed upon, open a PR against `main` following [CONTRIBUTING.md](./CONTRIBUTING.md).

Not sure where to start? Look for issues labeled `good first issue` or drop a comment in the vision discussion — we are happy to help you find a task that matches your skills.

---

## Non-goals

To keep focus, the following are explicitly **not** in scope:

- **Rider-facing mobile app** — Spatiad is a backend engine, not a consumer product.
- **Payment processing** — Billing and payment belong in the integrating application.
- **Map rendering** — Spatiad produces coordinates and metadata; visualization is the client's responsibility.
- **Proprietary lock-in** — All features ship under MIT. There is no "enterprise edition" gate.
