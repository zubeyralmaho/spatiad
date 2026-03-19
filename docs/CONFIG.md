# Configuration Matrix

## Runtime env vars (planned)

- SPATIAD_BIND_ADDR: default 0.0.0.0:3000
- SPATIAD_LOG_LEVEL: default info
- SPATIAD_H3_RESOLUTION: default 8
- SPATIAD_WEBHOOK_URL: optional in scaffold
- SPATIAD_WEBHOOK_SECRET: optional in scaffold
- SPATIAD_DRIVER_TOKEN: optional WS driver auth token

## Current implemented config

- `SPATIAD_WEBHOOK_URL`: when set, spatiad sends `trip_matched` callbacks after offer acceptance.
- `SPATIAD_DRIVER_TOKEN`: when set, driver WS upgrade requires header `x-spatiad-driver-token`.

## Current scaffold behavior

- Bind address is hardcoded to 0.0.0.0:3000
- One seeded driver exists for smoke testing
