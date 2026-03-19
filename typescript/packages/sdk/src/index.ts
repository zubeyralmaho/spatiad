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
}
