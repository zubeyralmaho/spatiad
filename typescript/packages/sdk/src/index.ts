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
};

export type JobEventKind =
  | "job_registered"
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
  before?: string;
  kinds?: JobEventKind[];
  signal?: AbortSignal;
  retry?: RetryOptions;
};

export type GetJobEventsAllPagesRequest = Omit<GetJobEventsRequest, "before"> & {
  maxPages?: number;
  maxEvents?: number;
  onPage?: (page: JobEventsResponse, pageIndex: number) => void;
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
  constructor(private readonly baseUrl: string) {}

  async createOffer(request: DispatchOfferRequest): Promise<{ offer_id: string }> {
    const response = await this.fetchWithRetry(`${this.baseUrl}/api/v1/dispatch/offer`, {
      method: "POST",
      headers: { "content-type": "application/json" },
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

  async getJobEvents(request: GetJobEventsRequest): Promise<JobEventsResponse> {
    const url = this.buildJobEventsUrl(request);

    const response = await this.fetchWithRetry(
      url,
      { method: "GET", signal: request.signal },
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
    let before: string | undefined;

    for (let page = 0; page < maxPages; page += 1) {
      const current = await this.getJobEvents({
        jobId: request.jobId,
        limit: request.limit,
        before,
        kinds: request.kinds,
        signal: request.signal,
        retry: request.retry
      });

      request.onPage?.(current, page);

      allEvents.push(...current.events);
      if (typeof maxEvents === "number" && maxEvents >= 0 && allEvents.length >= maxEvents) {
        return allEvents.slice(0, maxEvents);
      }

      if (!current.next_before_cursor) {
        break;
      }

      before = current.next_before_cursor;
    }

    return allEvents;
  }

  private buildJobEventsUrl(request: GetJobEventsRequest): string {
    const search = new URLSearchParams();
    if (typeof request.limit === "number") {
      search.set("limit", String(request.limit));
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
