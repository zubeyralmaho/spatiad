package com.spatiad.sdk;

import com.fasterxml.jackson.annotation.JsonInclude;
import com.fasterxml.jackson.annotation.JsonProperty;

@JsonInclude(JsonInclude.Include.NON_NULL)
public class CreateOfferRequest {

    @JsonProperty("job_id")
    private String jobId;

    @JsonProperty("category")
    private String category;

    @JsonProperty("pickup")
    private Coordinates pickup;

    @JsonProperty("dropoff")
    private Coordinates dropoff;

    @JsonProperty("initial_radius_km")
    private double initialRadiusKm;

    @JsonProperty("max_radius_km")
    private double maxRadiusKm;

    @JsonProperty("timeout_seconds")
    private int timeoutSeconds;

    public CreateOfferRequest() {
    }

    public String getJobId() {
        return jobId;
    }

    public void setJobId(String jobId) {
        this.jobId = jobId;
    }

    public String getCategory() {
        return category;
    }

    public void setCategory(String category) {
        this.category = category;
    }

    public Coordinates getPickup() {
        return pickup;
    }

    public void setPickup(Coordinates pickup) {
        this.pickup = pickup;
    }

    public Coordinates getDropoff() {
        return dropoff;
    }

    public void setDropoff(Coordinates dropoff) {
        this.dropoff = dropoff;
    }

    public double getInitialRadiusKm() {
        return initialRadiusKm;
    }

    public void setInitialRadiusKm(double initialRadiusKm) {
        this.initialRadiusKm = initialRadiusKm;
    }

    public double getMaxRadiusKm() {
        return maxRadiusKm;
    }

    public void setMaxRadiusKm(double maxRadiusKm) {
        this.maxRadiusKm = maxRadiusKm;
    }

    public int getTimeoutSeconds() {
        return timeoutSeconds;
    }

    public void setTimeoutSeconds(int timeoutSeconds) {
        this.timeoutSeconds = timeoutSeconds;
    }
}
