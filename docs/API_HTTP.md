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
