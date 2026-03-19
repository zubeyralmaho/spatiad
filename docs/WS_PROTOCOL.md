# WebSocket Protocol (Driver Stream)

Endpoint:

- /api/v1/stream/driver/{driver_id}

Current implementation includes a basic session loop:

- On connect/reconnect, pending offers for that driver are replayed.
- `location` messages update in-memory driver position.
- `offer_response` messages mark the offer as accepted/rejected.
- On accepted response, a `matched` message is sent back.

## Inbound messages (driver -> spatiad)

```json
{ "type": "location", "latitude": 38.433, "longitude": 26.768, "timestamp": 1710000000 }
```

```json
{ "type": "offer_response", "offer_id": "uuid", "accepted": true }
```

## Reconnect behavior

If a driver disconnects while offers are still pending, those offers remain in-memory and are flushed on the next successful reconnect of the same `driver_id`.
Any pending offer that already exceeded `expires_at` is marked expired and emitted as `offer_expired` instead of `offer`.
While connected, the session loop runs a periodic 1-second expiration tick and emits `offer_expired` without waiting for a new inbound message.

## Outbound messages (spatiad -> driver)

```json
{
  "type": "offer",
  "offer_id": "uuid",
  "job_id": "uuid",
  "pickup": { "latitude": 38.433, "longitude": 26.768 },
  "dropoff": { "latitude": 38.44, "longitude": 26.78 },
  "expires_at": "2026-03-20T10:00:00Z"
}
```

```json
{ "type": "offer_expired", "offer_id": "uuid" }
```

```json
{ "type": "matched", "offer_id": "uuid", "job_id": "uuid" }
```
