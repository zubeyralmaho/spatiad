# WebSocket Protocol (Driver Stream)

Endpoint:

- /api/v1/stream/driver/{driver_id}

Current scaffold includes endpoint handshake only. Session manager is pending.

## Inbound messages (driver -> spatiad)

```json
{ "type": "location", "latitude": 38.433, "longitude": 26.768, "timestamp": 1710000000 }
```

```json
{ "type": "offer_response", "offer_id": "uuid", "accepted": true }
```

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
