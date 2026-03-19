# Spatiad Architecture (MVP)

## Runtime shape

Spatiad runs as a single Rust process.

- In-memory state for active drivers and pending jobs
- Spatial lookups through an H3-oriented index layer
- HTTP API for dispatcher commands
- WebSocket channel for driver sessions
- Webhook callback for final match events

## Primary components

- spatiad-types: shared domain contracts
- spatiad-h3: spatial key/index layer
- spatiad-core: mutable in-memory runtime state
- spatiad-dispatch: offer lifecycle orchestration
- spatiad-ws: protocol contracts for stream messages
- spatiad-api: axum router and handlers
- spatiad-bin: executable entrypoint

## Request flow (target)

1. Driver app or backend upserts live driver location/state to `/api/v1/driver/upsert`.
2. Dispatcher posts a job request.
3. Dispatch service queries nearest eligible drivers within `initial_radius_km`.
4. If no candidate is found, radius expands in +2 km steps up to `max_radius_km`.
5. Offers are sent over WebSocket.
6. First valid acceptance wins the lock.
7. Match webhook is sent back to main backend.

## Data durability (MVP)

- Active state: in-memory
- Optional snapshots and event persistence: post-MVP
