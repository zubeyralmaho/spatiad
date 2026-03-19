export type Coordinates = {
  latitude: number;
  longitude: number;
};

export type DispatchOfferRequest = {
  jobId: string;
  category: string;
  pickup: Coordinates;
  dropoff?: Coordinates;
  initialRadiusKm: number;
  maxRadiusKm: number;
  timeoutSeconds: number;
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
};

export type GetJobEventsAllPagesRequest = Omit<GetJobEventsRequest, "before"> & {
  maxPages?: number;
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

export class SpatiadClient {
  constructor(private readonly baseUrl: string) {}

  async createOffer(request: DispatchOfferRequest): Promise<{ offer_id: string }> {
    const response = await fetch(`${this.baseUrl}/api/v1/dispatch/offer`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        job_id: request.jobId,
        category: request.category,
        pickup: request.pickup,
        dropoff: request.dropoff,
        initial_radius_km: request.initialRadiusKm,
        max_radius_km: request.maxRadiusKm,
        timeout_seconds: request.timeoutSeconds
      })
    });

    if (!response.ok) {
      throw new Error(`dispatch offer failed with status ${response.status}`);
    }

    return response.json() as Promise<{ offer_id: string }>;
  }

  async getJobEvents(request: GetJobEventsRequest): Promise<JobEventsResponse> {
    const url = this.buildJobEventsUrl(request);

    const response = await fetch(url, { method: "GET" });
    if (!response.ok) {
      throw new Error(`job events request failed with status ${response.status}`);
    }

    return response.json() as Promise<JobEventsResponse>;
  }

  async getJobEventsAllPages(request: GetJobEventsAllPagesRequest): Promise<JobEvent[]> {
    const maxPages = request.maxPages ?? 10;
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
        kinds: request.kinds
      });

      allEvents.push(...current.events);
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
}
