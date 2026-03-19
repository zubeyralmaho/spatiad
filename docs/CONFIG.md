# Configuration Matrix

## Runtime env vars (planned)

- SPATIAD_BIND_ADDR: default 0.0.0.0:3000
- SPATIAD_LOG_LEVEL: default info
- SPATIAD_H3_RESOLUTION: default 8
- SPATIAD_WEBHOOK_URL: optional in scaffold
- SPATIAD_WEBHOOK_SECRET: optional in scaffold
- SPATIAD_DRIVER_TOKEN: optional WS driver auth token
- SPATIAD_DISPATCHER_TOKEN: optional dispatcher API auth token

## Current implemented config

- `SPATIAD_WEBHOOK_URL`: when set, spatiad sends `trip_matched` callbacks after offer acceptance.
- `SPATIAD_WEBHOOK_SECRET`: when set, spatiad signs webhook requests with HMAC-SHA256.
- `SPATIAD_DRIVER_TOKEN`: when set, driver WS upgrade requires header `x-spatiad-driver-token`.
- `SPATIAD_DISPATCHER_TOKEN`: when set, dispatcher HTTP endpoints require either `Authorization: Bearer <token>` or header `x-spatiad-dispatcher-token`.
- `SPATIAD_BIND_ADDR`: bind address for HTTP/WS server.
- `SPATIAD_LOG_LEVEL`: tracing log level.
- `SPATIAD_H3_RESOLUTION`: H3 resolution used for indexing.

## Current scaffold behavior

- One seeded driver exists for smoke testing
