# Operations Runbook

## Start service

```bash
cd rust
cargo run -p spatiad-bin
```

## Verify service

1. GET /health returns status ok
2. POST /api/v1/dispatch/offer returns 202 for matching category near seeded driver

## Common checks

- If no offers are returned, verify seeded driver category and pickup proximity
- Inspect logs for startup and request traces

## Incident starter checklist

1. Confirm process is running and port 3000 is bound
2. Confirm inbound request volume and status code pattern
3. Validate driver connectivity on WS endpoint
4. Validate webhook destination reachability
