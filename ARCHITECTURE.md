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

1. Dispatcher posts a job request.
2. Dispatch service asks core + spatial index for nearest eligible drivers.
3. Offers are sent over WebSocket.
4. If timeout expires, radius expands and next candidates are notified.
5. First valid acceptance wins the lock.
6. Match webhook is sent back to main backend.

## Data durability (MVP)

- Active state: in-memory
- Optional snapshots and event persistence: post-MVP
