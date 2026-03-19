# HTTP API Reference

Base URL: http://localhost:3000

## GET /health

Response:

```json
{
  "status": "ok",
  "service": "spatiad"
}
```

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
