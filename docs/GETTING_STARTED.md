# Getting Started

## Requirements

- Rust stable toolchain
- pnpm 9+
- Node.js 20+

## Run engine

```bash
cd rust
cargo run -p spatiad-bin
```

Server starts on port 3000.

## Health check

```bash
curl http://localhost:3000/health
```

## Example offer request

```bash
curl -X POST http://localhost:3000/api/v1/dispatch/offer \
  -H "content-type: application/json" \
  -d '{
    "job_id": "22222222-2222-2222-2222-222222222222",
    "category": "tow_truck",
    "pickup": {"latitude": 38.433, "longitude": 26.768},
    "dropoff": {"latitude": 38.440, "longitude": 26.780},
    "initial_radius_km": 1,
    "max_radius_km": 5,
    "timeout_seconds": 20
  }'
```

## Build TypeScript workspace

```bash
cd typescript
pnpm install
pnpm -r build
```

## SDK job events pagination

```ts
import { SpatiadClient } from "@spatiad/sdk";

const client = new SpatiadClient("http://localhost:3000");
const controller = new AbortController();
setTimeout(() => controller.abort(), 5000);

const events = await client.getJobEventsAllPages({
  jobId: "22222222-2222-2222-2222-222222222222",
  limit: 25,
  maxPages: 10,
  maxEvents: 200,
  kinds: ["offer_created", "match_confirmed"],
  signal: controller.signal,
  onPage: (page, index) => {
    console.log("fetched page", index, page.events.length);
  }
});

console.log(events.length);
```
