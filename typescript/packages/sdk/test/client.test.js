import test from "node:test";
import assert from "node:assert/strict";

import { SpatiadApiError, SpatiadClient } from "../dist/index.js";

function makeJsonResponse(status, payload) {
  return {
    ok: status >= 200 && status < 300,
    status,
    json: async () => payload
  };
}

test("getJobEventsAllPages follows cursor pagination", async () => {
  const originalFetch = globalThis.fetch;
  const calls = [];

  globalThis.fetch = async (url) => {
    calls.push(String(url));

    if (calls.length === 1) {
      return makeJsonResponse(200, {
        job_id: "job-1",
        events: [{ at: "2026-03-20T10:00:00Z", kind: "offer_created", offer_id: "o1", driver_id: "d1", status: "pending" }],
        next_cursor: "2026-03-20T09:59:59Z|10",
        next_before_cursor: "2026-03-20T09:59:59Z"
      });
    }

    return makeJsonResponse(200, {
      job_id: "job-1",
      events: [{ at: "2026-03-20T09:59:00Z", kind: "match_confirmed", offer_id: "o1", driver_id: "d1", status: "matched" }],
      next_cursor: null,
      next_before_cursor: null
    });
  };

  try {
    const client = new SpatiadClient("http://localhost:3000");
    const events = await client.getJobEventsAllPages({
      jobId: "job-1",
      limit: 1,
      kinds: ["offer_created", "match_confirmed"]
    });

    assert.equal(calls.length, 2);
    assert.match(calls[0], /limit=1/);
    assert.match(calls[0], /kinds=offer_created%2Cmatch_confirmed/);
    assert.match(calls[1], /cursor=2026-03-20T09%3A59%3A59Z%7C10/);
    assert.equal(events.length, 2);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("getJobEventsAllPages respects maxEvents", async () => {
  const originalFetch = globalThis.fetch;
  let calls = 0;

  globalThis.fetch = async () => {
    calls += 1;
    return makeJsonResponse(200, {
      job_id: "job-2",
      events: [
        { at: "2026-03-20T10:00:00Z", kind: "offer_created", offer_id: "o1", driver_id: "d1", status: "pending" },
        { at: "2026-03-20T09:59:00Z", kind: "offer_created", offer_id: "o2", driver_id: "d2", status: "pending" }
      ],
      next_cursor: "2026-03-20T09:58:00Z|20",
      next_before_cursor: "2026-03-20T09:58:00Z"
    });
  };

  try {
    const client = new SpatiadClient("http://localhost:3000");
    const events = await client.getJobEventsAllPages({
      jobId: "job-2",
      maxPages: 10,
      maxEvents: 3
    });

    assert.equal(calls, 2);
    assert.equal(events.length, 3);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("getJobEvents rejects before and cursor together", async () => {
  const client = new SpatiadClient("http://localhost:3000");

  await assert.rejects(
    () =>
      client.getJobEvents({
        jobId: "job-2",
        before: "2026-03-20T09:58:00Z",
        cursor: "2026-03-20T09:58:00Z|20"
      }),
    /either 'before' or 'cursor'/
  );
});

test("getJobEventsAllPages falls back to next_before_cursor", async () => {
  const originalFetch = globalThis.fetch;
  const calls = [];

  globalThis.fetch = async (url) => {
    calls.push(String(url));

    if (calls.length === 1) {
      return makeJsonResponse(200, {
        job_id: "job-fallback",
        events: [{ at: "2026-03-20T10:00:00Z", kind: "offer_created", offer_id: "o1", driver_id: "d1", status: "pending" }],
        next_cursor: null,
        next_before_cursor: "2026-03-20T09:59:59Z"
      });
    }

    return makeJsonResponse(200, {
      job_id: "job-fallback",
      events: [{ at: "2026-03-20T09:59:00Z", kind: "offer_rejected", offer_id: "o1", driver_id: "d1", status: "rejected" }],
      next_cursor: null,
      next_before_cursor: null
    });
  };

  try {
    const client = new SpatiadClient("http://localhost:3000");
    const events = await client.getJobEventsAllPages({ jobId: "job-fallback", limit: 1 });

    assert.equal(calls.length, 2);
    assert.match(calls[1], /cursor=2026-03-20T09%3A59%3A59Z/);
    assert.equal(events.length, 2);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("createOffer retries on configured status", async () => {
  const originalFetch = globalThis.fetch;
  let calls = 0;

  globalThis.fetch = async () => {
    calls += 1;
    if (calls === 1) {
      return makeJsonResponse(503, { error: "temporary" });
    }

    return makeJsonResponse(202, { offer_id: "offer-123" });
  };

  try {
    const client = new SpatiadClient("http://localhost:3000");
    const result = await client.createOffer({
      jobId: "job-3",
      category: "tow_truck",
      pickup: { latitude: 38.433, longitude: 26.768 },
      dropoff: { latitude: 38.44, longitude: 26.78 },
      initialRadiusKm: 1,
      maxRadiusKm: 5,
      timeoutSeconds: 20,
      retry: { maxAttempts: 2, backoffMs: 1, retryOnStatuses: [503] }
    });

    assert.equal(calls, 2);
    assert.equal(result.offer_id, "offer-123");
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("getJobEventsAllPages aborts during backoff wait", async () => {
  const originalFetch = globalThis.fetch;
  const controller = new AbortController();
  let calls = 0;

  globalThis.fetch = async () => {
    calls += 1;
    return makeJsonResponse(503, { error: "temporary" });
  };

  try {
    const client = new SpatiadClient("http://localhost:3000");
    const promise = client.getJobEventsAllPages({
      jobId: "job-4",
      signal: controller.signal,
      retry: { maxAttempts: 3, backoffMs: 200, retryOnStatuses: [503] }
    });

    setTimeout(() => controller.abort(), 20);

    await assert.rejects(promise, /aborted/);
    assert.equal(calls, 1);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("getJobEvents exposes API error details as SpatiadApiError", async () => {
  const originalFetch = globalThis.fetch;

  globalThis.fetch = async () =>
    makeJsonResponse(400, {
      error: "invalid_query",
      message: "invalid 'before' cursor; expected RFC3339 timestamp",
      hint: "use RFC3339"
    });

  try {
    const client = new SpatiadClient("http://localhost:3000");
    await assert.rejects(
      () => client.getJobEvents({ jobId: "job-5", before: "bad" }),
      (error) => {
        assert.ok(error instanceof SpatiadApiError);
        assert.equal(error.status, 400);
        assert.equal(error.code, "invalid_query");
        assert.equal(error.retryable, false);
        assert.match(error.message, /invalid 'before' cursor/);
        assert.equal(error.details?.error, "invalid_query");
        assert.equal(error.details?.hint, "use RFC3339");
        return true;
      }
    );
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("getJobEvents marks transient statuses as retryable", async () => {
  const originalFetch = globalThis.fetch;

  globalThis.fetch = async () =>
    makeJsonResponse(503, {
      error: "service_unavailable",
      message: "temporary outage"
    });

  try {
    const client = new SpatiadClient("http://localhost:3000");
    await assert.rejects(
      () => client.getJobEvents({ jobId: "job-6", retry: { maxAttempts: 1 } }),
      (error) => {
        assert.ok(error instanceof SpatiadApiError);
        assert.equal(error.status, 503);
        assert.equal(error.retryable, true);
        return true;
      }
    );
  } finally {
    globalThis.fetch = originalFetch;
  }
});
