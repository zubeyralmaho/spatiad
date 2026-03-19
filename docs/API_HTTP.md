# HTTP API Reference

Base URL: http://localhost:3000

Authentication notes:

- If `SPATIAD_DISPATCHER_TOKEN` is configured, dispatcher endpoints require either:
  - `Authorization: Bearer <token>`
  - `x-spatiad-dispatcher-token: <token>`
- Dispatch endpoints are rate-limited per actor and may return `429 Too Many Requests`.

## GET /health

Response:

```json
{
  "status": "ok",
  "service": "spatiad"
}
```

## POST /api/v1/driver/upsert

Registers or updates a driver snapshot in the in-memory engine.

Request:

```json
{
  "driver_id": "uuid",
  "category": "tow_truck",
  "status": "Available",
  "position": { "latitude": 38.433, "longitude": 26.768 }
}
```

Response: 200 OK

## POST /api/v1/dispatch/offer

Request:

```json
{
  "job_id": "uuid",
  "category": "tow_truck",
  "pickup": { "latitude": 38.433, "longitude": 26.768 },
  "dropoff": { "latitude": 38.44, "longitude": 26.78 },
  "initial_radius_km": 1,
  "max_radius_km": 5,
  "timeout_seconds": 20
}
```

Success response (202):

```json
{ "offer_id": "uuid" }
```

Behavior notes:

- Candidate search starts at `initial_radius_km`.
- If no candidate is found, search radius expands in +2 km steps until `max_radius_km`.
- Engine selects the nearest available driver in the current search radius.
- On first successful acceptance, that job is locked to a single winner and competing pending offers are cancelled.

Webhook notes:

- If `SPATIAD_WEBHOOK_URL` is set, an outbound callback is sent after a successful driver acceptance.
- Delivery retry policy (current): up to 3 attempts with exponential backoff (200ms, 400ms, 800ms).
- Each webhook request includes `x-spatiad-timestamp` and `x-spatiad-nonce` headers.
- If `SPATIAD_WEBHOOK_SECRET` is configured, request includes `x-spatiad-signature` (hex HMAC-SHA256).
- Signature base string format: `<timestamp>.<nonce>.<raw_json_body>`.
- Callback payload:

```json
{
  "event": "trip_matched",
  "job_id": "uuid",
  "driver_id": "uuid",
  "offer_id": "uuid",
  "matched_at": "2026-03-20T10:00:00Z"
}
```

Fallback response (404 in current scaffold):

```json
{ "offer_id": "00000000-0000-0000-0000-000000000000" }
```

## POST /api/v1/dispatch/cancel

Request:

```json
{ "offer_id": "uuid" }
```

Response: 200 OK

## POST /api/v1/dispatch/job/cancel

Cancels dispatch for a job and marks all pending offers for that job as cancelled.

Request:

```json
{ "job_id": "uuid" }
```

Response:

- `200 OK` when job exists and is cancelled
- `404 Not Found` when `job_id` is unknown
- `429 Too Many Requests` when dispatch rate limit is exceeded

## GET /api/v1/dispatch/job/{job_id}

Returns current dispatch state for the given job.

Response:

```json
{
  "job_id": "uuid",
  "state": "searching",
  "matched_driver_id": null,
  "matched_offer_id": null
}
```

Possible `state` values:

- `unknown`
- `pending`
- `searching`
- `cancelled`
- `matched`
- `exhausted`

## GET /api/v1/dispatch/job/{job_id}/events?limit=50&cursor=2026-03-20T10:00:00Z%7C42&kinds=offer_created,match_confirmed

Returns recent dispatch event history for a job (most recent first).

Query params:

- `limit` (optional): max event count, default 50
- `cursor` (optional): opaque pagination cursor in `<rfc3339>|<sequence>` format
- `before` (optional): RFC3339 timestamp cursor (legacy compatibility mode)
- `kinds` (optional): comma-separated event kind filter
  - Supported values: `job_registered`, `job_cancelled`, `offer_created`, `offer_expired`, `offer_cancelled`, `offer_rejected`, `offer_accepted`, `match_confirmed`, `offer_status_updated`

Response:

```json
{
  "job_id": "uuid",
  "next_cursor": "2026-03-20T09:58:11Z|41",
  "next_before_cursor": "2026-03-20T09:58:11Z",
  "events": [
    {
      "at": "2026-03-20T10:00:00Z",
      "kind": "match_confirmed",
      "offer_id": "uuid",
      "driver_id": "uuid",
      "status": "matched"
    }
  ]
}
```

Pagination:

- Preferred: if `next_cursor` is non-null, call the same endpoint with `cursor=<next_cursor>`.
- Legacy compatibility: `next_before_cursor` can still be used with `before=<next_before_cursor>`.

Validation:

- If `kinds` contains an unsupported value, endpoint returns `400 Bad Request` with:
- If `before` is not a valid RFC3339 timestamp, endpoint returns `400 Bad Request` with `error=invalid_query`.
- If `cursor` format is invalid, endpoint returns `400 Bad Request` with `error=invalid_query`.
- `before` and `cursor` cannot be sent together.

```json
{
  "error": "invalid_query",
  "message": "unsupported event kind '...'"
}
```
