# Security Model (MVP target)

## Authentication

- Dispatcher API: bearer token or signed internal token
- Driver stream: short-lived JWT with driver_id claim

## Integrity

- Webhook payload signing via HMAC-SHA256
- Replay prevention with timestamp and nonce checks

## Hardening backlog

- Rate limit on dispatch endpoints
- Abuse controls on WS reconnect storms
- Structured audit event stream
