# spatiad

Spatiad is an open-source spatial dispatch engine for real-time fleets.

## What this repository contains

- A Rust single-binary engine foundation
- H3-oriented spatial indexing crate scaffold
- HTTP + WebSocket API scaffold for dispatcher/driver flows
- pnpm workspace with SDK, Express integration example, and docs package

## Current status

Initial implementation scaffold is in place. The next milestone is wiring the end-to-end dispatch loop:

1. driver location updates
2. nearest candidate lookup
3. offer timeout/radius expansion
4. match lock-in and webhook callback

## Quick start (scaffold)

### Rust

```bash
cd rust
cargo run -p spatiad-bin
```

### TypeScript workspace

```bash
cd typescript
pnpm install
pnpm -r build
```

## Documentation

High-level architecture and MVP documentation plan:

- docs/IMPLEMENTATION_PLAN.md
- ARCHITECTURE.md
