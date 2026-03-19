# Security Model (MVP target)

## Authentication

- Dispatcher API: optional static token via `Authorization: Bearer <token>` or `x-spatiad-dispatcher-token` when `SPATIAD_DISPATCHER_TOKEN` is set
- Driver stream: optional static token via `x-spatiad-driver-token` when `SPATIAD_DRIVER_TOKEN` is set

## Integrity

- Webhook payload signing via HMAC-SHA256 (`x-spatiad-signature`)
- Webhook timestamp header (`x-spatiad-timestamp`) and nonce header (`x-spatiad-nonce`)
- Replay prevention with timestamp and nonce checks

## Consumer verification example

In `@spatiad/express-plugin`:

- `spatiadWebhookJson()` captures raw request body for canonical signature validation.
- `verifySpatiadWebhook({ secret })` validates signature, timestamp skew, and nonce replay.

Recommended receiver order:

1. raw-body json parser
2. signature/timestamp/nonce verifier
3. business handler

## Hardening backlog

- Rate limit on dispatch endpoints
- Abuse controls on WS reconnect storms
- Structured audit event stream
