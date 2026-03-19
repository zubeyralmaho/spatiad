# Security Model (MVP target)

## Authentication

- Dispatcher API: bearer token or signed internal token
- Driver stream: optional static token via `x-spatiad-driver-token` when `SPATIAD_DRIVER_TOKEN` is set

## Integrity

- Webhook payload signing via HMAC-SHA256 (`x-spatiad-signature`)
- Webhook timestamp header (`x-spatiad-timestamp`) and nonce header (`x-spatiad-nonce`)
- Replay prevention with timestamp and nonce checks

## Hardening backlog

- Rate limit on dispatch endpoints
- Abuse controls on WS reconnect storms
- Structured audit event stream
