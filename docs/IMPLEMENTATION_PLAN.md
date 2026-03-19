# Implementation Plan (MVP)

## Mandatory documentation deliverables

- Setup guide
- API reference (HTTP and WebSocket)
- Dispatch state machine lifecycle
- Configuration matrix and defaults
- Security model (auth, webhook signing)
- Deployment and operations runbook
- Troubleshooting guide

## Acceptance gate

MVP is not complete until documentation deliverables are published and validated against running examples.

## Security implementation note

Webhook callbacks now include signature/timestamp/nonce headers.
The TypeScript example receiver demonstrates verification and replay protection middleware usage.
