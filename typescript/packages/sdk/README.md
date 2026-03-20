# @spatiad/sdk

TypeScript SDK for Spatiad dispatch APIs.

## Install

```bash
npm i @spatiad/sdk
# or
pnpm add @spatiad/sdk
```

## Usage

```ts
import { SpatiadClient } from "@spatiad/sdk";

const client = new SpatiadClient("http://localhost:3000", {
  dispatcherToken: process.env.SPATIAD_DISPATCHER_TOKEN,
  dispatcherAuthMode: "bearer"
});

const status = await client.getJobStatus({
  jobId: "22222222-2222-2222-2222-222222222222"
});

console.log(status.state);
```

See full examples in ../../docs/GETTING_STARTED.md.
