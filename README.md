# spatiad

Spatiad is an open-source spatial dispatch engine for real-time fleets.

## What this repository contains

- Rust multi-crate backend (dispatch core, API, WS protocol, binary runtime)
- HTTP + WebSocket dispatch API
- H3-oriented geospatial matching foundation
- TypeScript monorepo (SDK, Express plugin, example app)
- Deployment assets (Docker, docker-compose, Kubernetes, systemd)

## Current status

Production-ready baseline is implemented with:

1. End-to-end dispatch lifecycle (offer creation, expiration, re-offer, match lock-in)
2. Job status and job event timeline APIs with cursor pagination
3. Dispatcher and driver token-based authentication
4. Dispatch rate limiting and WS reconnect guard
5. Webhook signing, retry, and failure event recording
6. Readiness and liveness split for orchestration
7. Request ID propagation and access logs

## Quick start

### Run API server

```bash
cd rust
cargo run -p spatiad-bin
```

Health check:

```bash
curl http://localhost:3000/health
curl http://localhost:3000/ready
```

### TypeScript workspace

```bash
cd typescript
pnpm install
pnpm -r build
```

### Run core tests

```bash
# Rust API unit + integration tests
cd rust
cargo test -p spatiad-api

# SDK tests
cd ../typescript
pnpm --filter @spatiad/sdk test
```

## Key runtime configuration

- SPATIAD_BIND_ADDR (default: 0.0.0.0:3000)
- SPATIAD_LOG_LEVEL (default: info)
- SPATIAD_H3_RESOLUTION (default: 8)
- SPATIAD_DRIVER_TOKEN (optional)
- SPATIAD_DISPATCHER_TOKEN (optional)
- SPATIAD_DISPATCH_RATE_LIMIT_PER_MIN (default: 240)
- SPATIAD_WS_RECONNECT_MAX_PER_MIN (default: 30)
- SPATIAD_WEBHOOK_URL (optional)
- SPATIAD_WEBHOOK_SECRET (optional)
- SPATIAD_WEBHOOK_TIMEOUT_MS (default: 3000)

See full matrix in docs/CONFIG.md.

## Deployment options

- Docker: Dockerfile
- Compose: docker-compose.yml
- Kubernetes: k8s/deployment.yaml
- systemd: dist/spatiad.service

Detailed deployment steps are in docs/DEPLOYMENT.md.

## Documentation

Primary references:

- docs/GETTING_STARTED.md
- docs/API_HTTP.md
- docs/WS_PROTOCOL.md
- docs/SECURITY.md
- docs/CONFIG.md
- docs/DEPLOYMENT.md
- docs/OPERATIONS_RUNBOOK.md
- docs/TROUBLESHOOTING.md
- ARCHITECTURE.md
