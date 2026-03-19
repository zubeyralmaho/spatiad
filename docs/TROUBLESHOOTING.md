# Troubleshooting Guide

This guide covers common issues and debugging techniques for Spatiad deployments.

## Startup Issues

### Service fails to start on port 3000

**Symptom:** `Error: address already in use`

**Diagnosis:**
```bash
# Check what's using port 3000
lsof -i :3000
# Or
netstat -tulpn | grep 3000
```

**Resolution:**
- Kill existing process: `kill -9 <PID>`
- Or bind to a different port: `SPATIAD_BIND_ADDR=0.0.0.0:3001 cargo run -p spatiad-bin`

### Service starts but crashes immediately

**Symptom:** Process exits after startup logs

**Diagnosis:**
1. Check log level for error traces: `SPATIAD_LOG_LEVEL=debug cargo run -p spatiad-bin`
2. Verify H3 resolution is valid (0-15): `echo $SPATIAD_H3_RESOLUTION`
3. Confirm all environment variables are parseable

**Common causes:**
- Invalid `SPATIAD_H3_RESOLUTION` (must be 0-15)
- Malformed `SPATIAD_BIND_ADDR`
- Missing dependencies (tokio runtime issue - check Rust version)

**Resolution:**
```bash
# Use defaults (safe starting point)
unset SPATIAD_H3_RESOLUTION
unset SPATIAD_BIND_ADDR
cargo run -p spatiad-bin
```

### Health check fails

**Symptom:** `curl http://localhost:3000/health` returns error or timeout

**Diagnosis:**
```bash
# Test with verbose output
curl -v http://localhost:3000/health

# Check if listener is bound
netstat -tulpn | grep 3000

# Check logs for panic/error
SPATIAD_LOG_LEVEL=debug cargo run -p spatiad-bin
```

**Resolution:**
- Ensure service is fully started (wait 2-3 seconds after cargo run)
- Check CPU/memory not exhausted
- Verify firewall allows port 3000

---

## Offer & Dispatch Issues

### No candidates returned (404 response)

**Symptom:**
```
POST /api/v1/dispatch/offer returns 404
{
  "offer_id": "00000000-0000-0000-0000-000000000000"
}
```

**Possible causes:**
1. **No drivers registered** - No drivers exist in the system
2. **Driver not in search radius** - Pickup location too far from any driver
3. **Driver category mismatch** - Job category doesn't match any driver's category
4. **Driver status not Available** - Drivers are Offline or Busy
5. **H3 resolution too high** - Spatial index cells too small, no matches found

**Diagnosis:**

1. Check if any drivers exist:
```bash
# Manually register a test driver
curl -X POST http://localhost:3000/api/v1/driver/upsert \
  -H "content-type: application/json" \
  -d '{
    "driver_id": "22222222-2222-2222-2222-222222222222",
    "category": "tow_truck",
    "status": "Available",
    "position": {"latitude": 38.433, "longitude": 26.768}
  }'
```

2. Verify driver location and job pickup are close:
```bash
# Use known seeded driver (lat/lng: 38.433, 26.768)
# Try job within 1km
curl -X POST http://localhost:3000/api/v1/dispatch/offer \
  -H "content-type: application/json" \
  -d '{
    "job_id": "11111111-1111-1111-1111-111111111111",
    "category": "tow_truck",
    "pickup": {"latitude": 38.433, "longitude": 26.768},
    "initial_radius_km": 1,
    "max_radius_km": 5,
    "timeout_seconds": 20
  }'
```

3. Check H3 resolution value:
```bash
echo "Current H3_RESOLUTION: $SPATIAD_H3_RESOLUTION (default: 8)"
# If using resolution 14-15, try resolution 8:
SPATIAD_H3_RESOLUTION=8 cargo run -p spatiad-bin
```

**Resolution:**
- Ensure at least one driver is registered with category matching job
- Verify driver location is `initial_radius_km` meters or less from pickup
- Use default H3 resolution (8) for broader cell coverage

### Offer expires before driver sees it

**Symptom:** Driver connects to WebSocket and immediately sees `offer_expired` instead of `offer`

**Analysis:**
- Offer was created but timeout elapsed before driver accepted
- Default timeout is in request (example: `timeout_seconds: 20`)
- If driver reconnects after 20 seconds, offer is already expired

**Diagnosis:**
```bash
# Enable debug logging to see offer lifecycle
SPATIAD_LOG_LEVEL=debug cargo run -p spatiad-bin

# Then trigger offer and watch logs for "expires_at" timestamp
```

**Resolution:**
- Increase `timeout_seconds` in offer request (default: 20)
- Ensure driver WebSocket connects within timeout window
- Check driver session logs for connection delays

---

## WebSocket & Driver Issues

### Driver cannot connect to WebSocket

**Symptom:** WebSocket upgrade fails with 401/403 or connection refused

**Diagnosis:**
1. Check if driver token is configured:
```bash
echo "SPATIAD_DRIVER_TOKEN: $SPATIAD_DRIVER_TOKEN"
```

2. Test WebSocket with proper token (if configured):
```bash
# If SPATIAD_DRIVER_TOKEN is set:
wscat -c "ws://localhost:3000/api/v1/stream/driver/11111111-1111-1111-1111-111111111111" \
  --header "x-spatiad-driver-token: <your-token>"

# If token not configured:
wscat -c "ws://localhost:3000/api/v1/stream/driver/11111111-1111-1111-1111-111111111111"
```

3. Verify server is listening:
```bash
curl -v http://localhost:3000/health
```

**Common issues:**
- Missing `x-spatiad-driver-token` header when `SPATIAD_DRIVER_TOKEN` is set
- Token value mismatch
- WebSocket upgrade blocked by proxy/firewall

**Resolution:**
- If using token auth, always send header: `x-spatiad-driver-token: <configured-token>`
- Disable token temporarily for debugging: `unset SPATIAD_DRIVER_TOKEN && cargo run -p spatiad-bin`
- Check firewall rules allow WebSocket (port 3000)

### Driver reconnects but offers not replayed

**Symptom:** Driver disconnects and reconnects, but pending offers are not resent

**Diagnosis:**
1. Check driver_id consistency (must be same UUID on reconnect):
```js
// JavaScript example
const driverId = "11111111-1111-1111-1111-111111111111";
// ALWAYS use same driverId on reconnect
```

2. Verify driver was marked as Available when offers were created:
```bash
# Check by polling job status
curl "http://localhost:3000/api/v1/dispatch/job/<job_id>"
```

3. Check logs for offer replay events:
```bash
SPATIAD_LOG_LEVEL=debug cargo run -p spatiad-bin
# Connection logs will show "replaying N pending offers"
```

**Causes:**
- Driver uses different UUID on reconnect
- Driver status was Offline/Busy when offers sent
- Offers already expired (see "Offer expires before driver sees it")

**Resolution:**
- Ensure driver_id is constant across reconnections
- Upsert driver with status=Available before creating offers
- Check offer timestamp to confirm not expired

### Driver offer acceptance not confirmed

**Symptom:** Driver sends `offer_response` with `accepted: true`, but no `matched` message received

**Diagnosis:**
1. Check request format is correct:
```json
{
  "type": "offer_response",
  "offer_id": "uuid",
  "accepted": true
}
```

2. Verify offer_id exists and is pending:
```bash
curl "http://localhost:3000/api/v1/dispatch/job/<job_id>"
```

3. Check server logs:
```bash
SPATIAD_LOG_LEVEL=debug cargo run -p spatiad-bin
# Should see "offer accepted" or "offer already matched" logs
```

**Causes:**
- Offer already accepted by another driver (first acceptance wins)
- Offer expired before acceptance processed
- JSON format invalid

**Resolution:**
- Send message as valid JSON (no trailing commas)
- Ensure offer has not expired
- Check that no other driver accepted first (check job status)

---

## Webhook Issues

### Webhook callback never arrives

**Symptom:**
- `SPATIAD_WEBHOOK_URL` is set
- Offer is accepted (job status shows `matched`)
- Webhook receiver shows no incoming request

**Diagnosis:**

1. Verify webhook URL is reachable from Spatiad:
```bash
# From Spatiad container/host
curl -s "http://<webhook-url>/healthz" || echo "unreachable"
```

2. Check webhook configuration:
```bash
echo "SPATIAD_WEBHOOK_URL: $SPATIAD_WEBHOOK_URL"
echo "SPATIAD_WEBHOOK_SECRET: $SPATIAD_WEBHOOK_SECRET"
```

3. Enable debug logging to see webhook attempts:
```bash
SPATIAD_LOG_LEVEL=debug cargo run -p spatiad-bin
# Watch for "webhook delivery attempt" or error logs
```

**Common causes:**
- Webhook URL is localhost (unreachable from container)
- Webhook receiver is down/crashed
- Webhook URL requires auth (not yet implemented in MVP)
- Network firewall blocks outbound requests

**Resolution:**
- Use external webhook URL (not localhost): `SPATIAD_WEBHOOK_URL=https://example.com/webhooks/spatiad`
- Test webhook receiver is accessible: `curl -I https://example.com/webhooks/spatiad`
- Check webhook receiver logs for 5xx errors
- Enable retry logging to see delivery attempts

### Webhook signature verification fails

**Symptom:** Webhook arrives but `verifySpatiadWebhook` in Express middleware rejects it

**Diagnosis:**

1. Check signature header format:
```bash
# Webhook should include:
# x-spatiad-signature: <hex-encoded-hmac-sha256>
# x-spatiad-timestamp: <unix-seconds>
# x-spatiad-nonce: <random-string>
```

2. Ensure secret matches:
```bash
# Spatiad config
echo "SPATIAD_WEBHOOK_SECRET: $SPATIAD_WEBHOOK_SECRET"

# Express config
const secret = process.env.SPATIAD_WEBHOOK_SECRET;
// MUST match Spatiad's value
```

3. Check raw request body is used for verification:
```javascript
// Middleware order is critical
app.post(
  "/webhooks/spatiad",
  spatiadWebhookJson(),  // MUST capture raw body
  verifySpatiadWebhook({ secret: "..." }),  // THEN verify
  handler
);
```

4. Test signature locally:
```javascript
// Debug: log signature components
app.post("/webhooks/spatiad", spatiadWebhookJson(), (req, res) => {
  console.log("Headers:", req.headers);
  console.log("Raw body:", req.rawBody);  // Set by spatiadWebhookJson()
  res.status(204).end();
});
```

**Common causes:**
- Secret mismatch between Spatiad and webhook receiver
- Middleware order wrong (body parsed before signature check)
- Nonce replay limit too strict (default checks recent 300s window)
- Timestamp skew (server clocks not synced)

**Resolution:**
- Confirm secrets match exactly
- Middleware order: `spatiadWebhookJson()` → `verifySpatiadWebhook()` → handler
- For testing, temporarily disable verification:
```javascript
// Temporarily remove verifySpatiadWebhook() middleware
app.post("/webhooks/spatiad", spatiadWebhookJson(), (req, res) => {
  console.log("Webhook received:", req.body);
  res.status(204).end();
});
```

---

## Rate Limiting & Performance Issues

### Dispatcher requests return 429 (Too Many Requests)

**Symptom:** Legitimate dispatcher requests rejected with 429 status

**Diagnosis:**

1. Check rate limit configuration:
```bash
echo "Dispatch limit per minute: $SPATIAD_DISPATCH_RATE_LIMIT_PER_MIN (default: 240)"
```

2. Calculate actual load:
```
Default: 240 requests per 60 seconds = 4 requests per second
```

3. Enable rate limit headers to see remaining:
```bash
curl -i -X POST http://localhost:3000/api/v1/dispatch/offer \
  -H "content-type: application/json" \
  -d '...' | grep -i "ratelimit"
```

**Causes:**
- Legitimate spike in offer requests exceeds configured limit
- Multiple dispatcher instances hitting same Spatiad instance
- Default limit too conservative for use case

**Resolution:**
- Increase rate limit: `SPATIAD_DISPATCH_RATE_LIMIT_PER_MIN=600 cargo run -p spatiad-bin`
- Implement dispatcher-side request batching
- Add exponential backoff when 429 received

### WebSocket reconnects are rate-limited (429)

**Symptom:** Drivers attempting reconnect receive 429 in WebSocket error

**Diagnosis:**

1. Check WS reconnect limit:
```bash
echo "WS reconnect limit: $SPATIAD_WS_RECONNECT_MAX_PER_MIN (default: 30)"
```

2. Calculate per driver:
```
30 reconnects per 60 seconds across ALL drivers
= 0.5 reconnects/sec total
= For 10 drivers, average 3 seconds between reconnects
```

3. Monitor reconnect frequency:
```bash
SPATIAD_LOG_LEVEL=debug cargo run -p spatiad-bin
# Watch for "ws_reconnect_guard" events
```

**Causes:**
- Driver repeatedly reconnecting (network instability)
- Many drivers reconnecting simultaneously
- Single driver hitting reconnect limit alone

**Resolution:**
- Increase limit: `SPATIAD_WS_RECONNECT_MAX_PER_MIN=60 cargo run -p spatiad-bin`
- Implement driver-side exponential backoff on 429
- Monitor driver network stability

---

## Data & State Issues

### Job status shows "unknown" but offer was created

**Symptom:** 
```json
{
  "state": "unknown"
}
```
But dispatcher has job_id for recent offer.

**Diagnosis:**

1. Verify job lifecycle:
```bash
# Create offer first
curl -X POST http://localhost:3000/api/v1/dispatch/offer \
  -H "content-type: application/json" \
  -d '{"job_id": "<UUID>", ...}'

# Then check status (must use same job_id)
curl "http://localhost:3000/api/v1/dispatch/job/<SAME-UUID>"
```

2. Check job events to confirm job was registered:
```bash
curl "http://localhost:3000/api/v1/dispatch/job/<job_id>/events?limit=50"
# Should show JobRegistered event
```

**Common causes:**
- Wrong job_id used in status check
- Job expired from memory (no active drivers/offers = eligible for cleanup - future feature)
- Offer request failed silently (check HTTP status code from offer endpoint)

**Resolution:**
- Use exact job_id from offer creation response
- Verify job has at least one pending or active offer
- Check offer response was 202/201, not 404

### Job event pagination cursor invalid

**Symptom:** 
```
GET /api/v1/dispatch/job/{job_id}/events?cursor=<value>
Returns 400: "error":"invalid_query"
```

**Diagnosis:**

1. Check cursor format - must be `<rfc3339>|<sequence>`:
```
Example: 2026-03-20T10:00:00Z|42
Not: 2026-03-20T10:00:00Z (missing sequence)
```

2. Test with known good cursor:
```bash
# First get a page to get next_cursor
curl "http://localhost:3000/api/v1/dispatch/job/<job_id>/events?limit=1"
# Response includes: "next_cursor": "2026-03-20T10:00:00Z|42"

# Then use that exact cursor
curl "http://localhost:3000/api/v1/dispatch/job/<job_id>/events?cursor=2026-03-20T10%3A00%3A00Z%7C42"
```

3. Verify timestamp is RFC3339 format:
```bash
# Valid: 2026-03-20T10:00:00Z or 2026-03-20T10:00:00+00:00
# Invalid: 2026-03-20 (missing time)
```

**Causes:**
- Cursor not URL-encoded (`:` should be `%3A`, `|` should be `%7C`)
- Using `before` parameter instead of `cursor`
- Cursor from old/different job
- Timestamp not in RFC3339 format

**Resolution:**
- Copy cursor directly from `next_cursor` response (SDK handles encoding)
- Use SDK's `getJobEventsAllPages()` for automatic pagination
- Manually URL-encode if using curl: pipe through `jq -r .next_cursor | jq -Rs @uri`

---

## Performance & Debugging

### Checking active drivers and jobs in-memory state

**Current limitation:** No API endpoint to list all drivers/jobs (feature for post-MVP)

**Workaround diagnosis:**
```bash
# Enable debug logging to see state operations
SPATIAD_LOG_LEVEL=debug cargo run -p spatiad-bin

# Monitor logs for:
# - "upsert_driver" events (shows driver registrations)
# - "register_job" events (shows job submissions)
# - "create_offer" events (shows offer creation)
```

### Memory usage grows over time

**Symptom:** Resident memory increases continuously over hours

**Current architecture:**
- All state is in-memory
- No automatic cleanup of completed/failed jobs (designed by MVP - stateless per spec)
- Completed jobs, expired offers remain in memory until process restart

**Diagnosis:**
```bash
# Monitor memory
top -p $(pgrep -f "spatiad-bin") | tail -20

# Or with systemd
systemctl status spatiad
```

**Expected behavior for MVP:**
- Memory grows with active dispatcher load
- No data loss on restart (stateless by design)
- Rest API only exposes recent events (default 50 limit)
- Post-MVP: event persistence + cleanup will be added

**Temporary solutions:**
- Implement supervisor restart on schedule: `systemctl ExecStartPost=...`
- Monitor and alert on memory threshold: setup alerts if RSS > 500MB
- Reduce job event retention in code (future enhancement)

---

## Debugging Checklist

When troubleshooting an issue, verify in order:

- [ ] Service health: `curl http://localhost:3000/health`
- [ ] Port binding: `lsof -i :3000`
- [ ] Environment variables: `env | grep SPATIAD`
- [ ] Log level: `SPATIAD_LOG_LEVEL=debug`
- [ ] Seeded driver exists: manually upsert test driver
- [ ] Request format valid: use `curl -i -X POST ... | head -20` to check response headers
- [ ] WebSocket connection: `wscat -c ws://localhost:3000/api/v1/stream/driver/<uuid>`
- [ ] Webhook receiver accessible: `curl -I $SPATIAD_WEBHOOK_URL`
- [ ] Signature secrets match: verify `SPATIAD_WEBHOOK_SECRET` on both sides

---

## Getting Help

For issues not covered here:

1. Enable debug logging: `SPATIAD_LOG_LEVEL=debug`
2. Reproduce minimal example (single driver, single offer)
3. Collect logs and request/response samples
4. Check [OPERATIONS_RUNBOOK.md](OPERATIONS_RUNBOOK.md) for deployment context
5. Review [API_HTTP.md](API_HTTP.md) to confirm request format
