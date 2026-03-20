# Configuration Matrix

## Runtime env vars (planned)

- SPATIAD_BIND_ADDR: default 0.0.0.0:3000
- SPATIAD_LOG_LEVEL: default info
- SPATIAD_H3_RESOLUTION: default 8
- SPATIAD_WEBHOOK_URL: optional in scaffold
- SPATIAD_WEBHOOK_SECRET: optional in scaffold
- SPATIAD_DRIVER_TOKEN: optional WS driver auth token
- SPATIAD_DISPATCHER_TOKEN: optional dispatcher API auth token
- SPATIAD_DISPATCH_RATE_LIMIT_PER_MIN: max dispatch API requests per actor per minute (default 240)
- SPATIAD_WS_RECONNECT_MAX_PER_MIN: max WS reconnect attempts per driver per minute (default 30)
- SPATIAD_WEBHOOK_TIMEOUT_MS: webhook HTTP request timeout in milliseconds (default 3000, min 100, max 60000)

## Current implemented config

- `SPATIAD_WEBHOOK_URL`: when set, spatiad sends `trip_matched` callbacks after offer acceptance.
- `SPATIAD_WEBHOOK_SECRET`: when set, spatiad signs webhook requests with HMAC-SHA256.
- `SPATIAD_DRIVER_TOKEN`: when set, driver WS upgrade requires header `x-spatiad-driver-token`.
- `SPATIAD_DISPATCHER_TOKEN`: when set, dispatcher HTTP endpoints require either `Authorization: Bearer <token>` or header `x-spatiad-dispatcher-token`.
- `SPATIAD_BIND_ADDR`: bind address for HTTP/WS server.
- `SPATIAD_LOG_LEVEL`: tracing log level.
- `SPATIAD_H3_RESOLUTION`: H3 resolution used for indexing.
- `SPATIAD_DISPATCH_RATE_LIMIT_PER_MIN`: per-actor dispatch API sliding-window rate limit.
- `SPATIAD_WS_RECONNECT_MAX_PER_MIN`: per-driver WS reconnect sliding-window guard.
- `SPATIAD_WEBHOOK_TIMEOUT_MS`: per-attempt timeout for webhook delivery requests.

## Current scaffold behavior

- One seeded driver exists for smoke testing
