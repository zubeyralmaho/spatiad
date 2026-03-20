package com.spatiad.sdk;

import com.fasterxml.jackson.annotation.JsonProperty;

public class UpsertDriverRequest {

    @JsonProperty("driver_id")
    private String driverId;

    @JsonProperty("category")
    private String category;

    @JsonProperty("status")
    private String status;

    @JsonProperty("position")
    private Coordinates position;

    public UpsertDriverRequest() {
    }

    public String getDriverId() {
        return driverId;
    }

    public void setDriverId(String driverId) {
        this.driverId = driverId;
    }

    public String getCategory() {
        return category;
    }

    public void setCategory(String category) {
        this.category = category;
    }

    public String getStatus() {
        return status;
    }

    public void setStatus(String status) {
        this.status = status;
    }

    public Coordinates getPosition() {
        return position;
    }

    public void setPosition(Coordinates position) {
        this.position = position;
    }
}
