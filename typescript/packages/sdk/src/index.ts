export type Coordinates = {
  latitude: number;
  longitude: number;
};

export type RetryOptions = {
  maxAttempts?: number;
  backoffMs?: number;
  maxBackoffMs?: number;
  retryOnStatuses?: number[];
};

export type DispatcherAuthMode = "bearer" | "header";

export type SpatiadClientOptions = {
  dispatcherToken?: string;
  dispatcherAuthMode?: DispatcherAuthMode;
};

export type DispatcherAuthOverride = {
  dispatcherToken?: string;
  dispatcherAuthMode?: DispatcherAuthMode;
};

export type ApiErrorBody = {
  error?: string;
  message?: string;
  [key: string]: unknown;
};

export type DispatchOfferRequest = {
  jobId: string;
  category: string;
  pickup: Coordinates;
  dropoff?: Coordinates;
  initialRadiusKm: number;
  maxRadiusKm: number;
  timeoutSeconds: number;
  signal?: AbortSignal;
  retry?: RetryOptions;
} & DispatcherAuthOverride;

export type DriverStatus = "Offline" | "Available" | "Busy";

export type UpsertDriverRequest = {
  driverId: string;
  category: string;
  status: DriverStatus;
  position: Coordinates;
  signal?: AbortSignal;
  retry?: RetryOptions;
} & DispatcherAuthOverride;

export type CancelOfferRequest = {
  offerId: string;
  signal?: AbortSignal;
  retry?: RetryOptions;
} & DispatcherAuthOverride;

export type GetJobStatusRequest = {
  jobId: string;
  signal?: AbortSignal;
  retry?: RetryOptions;
} & DispatcherAuthOverride;

export type JobDispatchState = "unknown" | "pending" | "searching" | "matched" | "exhausted";

export type JobStatusResponse = {
  job_id: string;
  state: JobDispatchState;
  matched_driver_id: string | null;
  matched_offer_id: string | null;
};

export type JobEventKind =
  | "job_registered"
  | "job_cancelled"
  | "offer_created"
  | "offer_expired"
  | "offer_cancelled"
  | "offer_rejected"
  | "offer_accepted"
  | "match_confirmed"
  | "offer_status_updated";

export type GetJobEventsRequest = {
  jobId: string;
  limit?: number;
  cursor?: string;
  before?: string;
  kinds?: JobEventKind[];
  signal?: AbortSignal;
  retry?: RetryOptions;
} & DispatcherAuthOverride;

export type GetJobEventsAllPagesRequest = Omit<GetJobEventsRequest, "before"> & {
  maxPages?: number;
  maxEvents?: number;
  onPage?: (page: JobEventsResponse, pageIndex: number) => void;
};

export type IterateJobEventsRequest = GetJobEventsAllPagesRequest & {
  resumeOnTransientError?: boolean;
  maxResumeAttempts?: number;
};

export type JobEvent = {
  at: string;
  kind: JobEventKind;
  offer_id: string | null;
  driver_id: string | null;
  status: string | null;
};

export type JobEventsResponse = {
  job_id: string;
  events: JobEvent[];
  next_cursor: string | null;
  next_before_cursor: string | null;
};

export class SpatiadApiError extends Error {
  public readonly status: number;
  public readonly code?: string;
  public readonly retryable: boolean;
  public readonly details?: ApiErrorBody;

  constructor(message: string, status: number, code?: string, retryable?: boolean, details?: ApiErrorBody) {
    super(message);
    this.name = "SpatiadApiError";
    this.status = status;
    this.code = code;
    this.retryable = retryable ?? isRetryableStatus(status);
    this.details = details;
  }
}

export class SpatiadClient {
  constructor(
    private readonly baseUrl: string,
    private readonly options: SpatiadClientOptions = {}
  ) {}

  async createOffer(request: DispatchOfferRequest): Promise<{ offer_id: string }> {
    const response = await this.fetchWithRetry(`${this.baseUrl}/api/v1/dispatch/offer`, {
      method: "POST",
      headers: this.withDispatcherHeaders(
        { "content-type": "application/json" },
        request
      ),
      signal: request.signal,
      body: JSON.stringify({
        job_id: request.jobId,
        category: request.category,
        pickup: request.pickup,
        dropoff: request.dropoff,
        initial_radius_km: request.initialRadiusKm,
        max_radius_km: request.maxRadiusKm,
        timeout_seconds: request.timeoutSeconds
      })
    }, request.retry);

    if (!response.ok) {
      throw await this.createApiError("dispatch offer failed", response);
    }

    return response.json() as Promise<{ offer_id: string }>;
  }

  async upsertDriver(request: UpsertDriverRequest): Promise<void> {
    const response = await this.fetchWithRetry(`${this.baseUrl}/api/v1/driver/upsert`, {
      method: "POST",
      headers: this.withDispatcherHeaders(
        { "content-type": "application/json" },
        request
      ),
      signal: request.signal,
      body: JSON.stringify({
        driver_id: request.driverId,
        category: request.category,
        status: request.status,
        position: request.position
      })
    }, request.retry);

    if (!response.ok) {
      throw await this.createApiError("driver upsert failed", response);
    }
  }

  async cancelOffer(request: CancelOfferRequest): Promise<void> {
    const response = await this.fetchWithRetry(`${this.baseUrl}/api/v1/dispatch/cancel`, {
      method: "POST",
      headers: this.withDispatcherHeaders(
        { "content-type": "application/json" },
        request
      ),
      signal: request.signal,
      body: JSON.stringify({ offer_id: request.offerId })
    }, request.retry);

    if (!response.ok) {
      throw await this.createApiError("cancel offer failed", response);
    }
  }

  async getJobStatus(request: GetJobStatusRequest): Promise<JobStatusResponse> {
    const response = await this.fetchWithRetry(
      `${this.baseUrl}/api/v1/dispatch/job/${request.jobId}`,
      {
        method: "GET",
        headers: this.withDispatcherHeaders({}, request),
        signal: request.signal
      },
      request.retry
    );

    if (!response.ok) {
      throw await this.createApiError("job status request failed", response);
    }

    return response.json() as Promise<JobStatusResponse>;
  }

  async getJobEvents(request: GetJobEventsRequest): Promise<JobEventsResponse> {
    const url = this.buildJobEventsUrl(request);

    const response = await this.fetchWithRetry(
      url,
      {
        method: "GET",
        headers: this.withDispatcherHeaders({}, request),
        signal: request.signal
      },
      request.retry
    );
    if (!response.ok) {
      throw await this.createApiError("job events request failed", response);
    }

    return response.json() as Promise<JobEventsResponse>;
  }

  async getJobEventsAllPages(request: GetJobEventsAllPagesRequest): Promise<JobEvent[]> {
    const maxPages = request.maxPages ?? 10;
    const maxEvents = request.maxEvents;
    if (maxPages < 1) {
      return [];
    }

    const allEvents: JobEvent[] = [];
    let cursor: string | undefined;

    for (let page = 0; page < maxPages; page += 1) {
      const current = await this.getJobEvents({
        jobId: request.jobId,
        limit: request.limit,
        cursor,
        kinds: request.kinds,
        dispatcherToken: request.dispatcherToken,
        dispatcherAuthMode: request.dispatcherAuthMode,
        signal: request.signal,
        retry: request.retry
      });

      request.onPage?.(current, page);

      allEvents.push(...current.events);
      if (typeof maxEvents === "number" && maxEvents >= 0 && allEvents.length >= maxEvents) {
        return allEvents.slice(0, maxEvents);
      }

      const nextCursor = current.next_cursor ?? current.next_before_cursor;
      if (!nextCursor) {
        break;
      }

      cursor = nextCursor;
    }

    return allEvents;
  }

  async *iterateJobEvents(request: IterateJobEventsRequest): AsyncGenerator<JobEvent, void, void> {
    const maxPages = request.maxPages ?? 10;
    const maxEvents = request.maxEvents;
    if (maxPages < 1) {
      return;
    }

    const resumeOnTransientError = request.resumeOnTransientError ?? false;
    const maxResumeAttempts = Math.max(0, request.maxResumeAttempts ?? 3);
    let cursor: string | undefined;
    let yielded = 0;
    let resumeAttempts = 0;

    for (let page = 0; page < maxPages; page += 1) {
      let current: JobEventsResponse;
      try {
        current = await this.getJobEvents({
          jobId: request.jobId,
          limit: request.limit,
          cursor,
          kinds: request.kinds,
          dispatcherToken: request.dispatcherToken,
          dispatcherAuthMode: request.dispatcherAuthMode,
          signal: request.signal,
          retry: request.retry
        });
      } catch (error) {
        if (
          resumeOnTransientError
          && error instanceof SpatiadApiError
          && error.retryable
          && resumeAttempts < maxResumeAttempts
        ) {
          resumeAttempts += 1;
          const backoffBase = Math.max(0, request.retry?.backoffMs ?? 150);
          const backoffMax = Math.max(backoffBase, request.retry?.maxBackoffMs ?? 2000);
          const waitMs = Math.min(backoffMax, backoffBase * (2 ** (resumeAttempts - 1)));
          await waitWithSignal(waitMs, request.signal);
          page -= 1;
          continue;
        }

        throw error;
      }

      resumeAttempts = 0;

      request.onPage?.(current, page);

      for (const event of current.events) {
        yield event;
        yielded += 1;

        if (typeof maxEvents === "number" && maxEvents >= 0 && yielded >= maxEvents) {
          return;
        }
      }

      const nextCursor = current.next_cursor ?? current.next_before_cursor;
      if (!nextCursor) {
        return;
      }

      cursor = nextCursor;
    }
  }

  private buildJobEventsUrl(request: GetJobEventsRequest): string {
    if (request.before && request.cursor) {
      throw new Error("use either 'before' or 'cursor', not both");
    }

    const search = new URLSearchParams();
    if (typeof request.limit === "number") {
      search.set("limit", String(request.limit));
    }
    if (request.cursor) {
      search.set("cursor", request.cursor);
    }
    if (request.before) {
      search.set("before", request.before);
    }
    if (request.kinds && request.kinds.length > 0) {
      search.set("kinds", request.kinds.join(","));
    }

    const suffix = search.toString();
    return `${this.baseUrl}/api/v1/dispatch/job/${request.jobId}/events${suffix ? `?${suffix}` : ""}`;
  }

  private withDispatcherHeaders(
    base: Record<string, string> = {},
    override: DispatcherAuthOverride = {}
  ): Record<string, string> {
    const headers = { ...base };
    const token = override.dispatcherToken ?? this.options.dispatcherToken;
    if (!token) {
      return headers;
    }

    const mode = override.dispatcherAuthMode ?? this.options.dispatcherAuthMode ?? "bearer";
    if (mode === "header") {
      headers["x-spatiad-dispatcher-token"] = token;
    } else {
      headers.Authorization = `Bearer ${token}`;
    }

    return headers;
  }

  private async fetchWithRetry(
    url: string,
    init: RequestInit,
    retry?: RetryOptions
  ): Promise<Response> {
    const signal = init.signal ?? undefined;
    const maxAttempts = Math.max(1, retry?.maxAttempts ?? 1);
    const baseBackoffMs = Math.max(0, retry?.backoffMs ?? 150);
    const maxBackoffMs = Math.max(baseBackoffMs, retry?.maxBackoffMs ?? 2000);
    const retryStatuses = retry?.retryOnStatuses ?? [408, 429, 500, 502, 503, 504];

    let lastError: unknown;

    for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
      try {
        const response = await fetch(url, init);
        if (response.ok || !retryStatuses.includes(response.status) || attempt === maxAttempts) {
          return response;
        }
      } catch (error) {
        if (signal?.aborted) {
          throw error;
        }

        lastError = error;
        if (attempt === maxAttempts) {
          throw error;
        }
      }

      const backoff = Math.min(maxBackoffMs, baseBackoffMs * (2 ** (attempt - 1)));
      await waitWithSignal(backoff, signal);
    }

    throw lastError instanceof Error ? lastError : new Error("request failed after retries");
  }

  private async createApiError(prefix: string, response: Response): Promise<SpatiadApiError> {
    let body: ApiErrorBody | undefined;
    try {
      const parsed = await response.json() as unknown;
      body = isApiErrorBody(parsed) ? parsed : undefined;
    } catch {
      body = undefined;
    }

    const details = body?.message ?? `${prefix} with status ${response.status}`;
    return new SpatiadApiError(details, response.status, body?.error, undefined, body);
  }
}

function isRetryableStatus(status: number): boolean {
  return status === 408 || status === 429 || status === 502 || status === 503 || status === 504;
}

function isApiErrorBody(value: unknown): value is ApiErrorBody {
  return typeof value === "object" && value !== null;
}

function waitWithSignal(ms: number, signal?: AbortSignal): Promise<void> {
  if (ms <= 0) {
    return Promise.resolve();
  }

  return new Promise<void>((resolve, reject) => {
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", onAbort);
      resolve();
    }, ms);

    const onAbort = () => {
      clearTimeout(timer);
      signal?.removeEventListener("abort", onAbort);
      reject(new Error("request aborted"));
    };

    signal?.addEventListener("abort", onAbort, { once: true });
  });
}
