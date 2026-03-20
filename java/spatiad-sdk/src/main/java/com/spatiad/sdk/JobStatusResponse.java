package com.spatiad.sdk;

import com.fasterxml.jackson.annotation.JsonProperty;

public class JobStatusResponse {

    @JsonProperty("job_id")
    private String jobId;

    @JsonProperty("state")
    private String state;

    @JsonProperty("matched_driver_id")
    private String matchedDriverId;

    @JsonProperty("matched_offer_id")
    private String matchedOfferId;

    public JobStatusResponse() {
    }

    public String getJobId() {
        return jobId;
    }

    public void setJobId(String jobId) {
        this.jobId = jobId;
    }

    public String getState() {
        return state;
    }

    public void setState(String state) {
        this.state = state;
    }

    public String getMatchedDriverId() {
        return matchedDriverId;
    }

    public void setMatchedDriverId(String matchedDriverId) {
        this.matchedDriverId = matchedDriverId;
    }

    public String getMatchedOfferId() {
        return matchedOfferId;
    }

    public void setMatchedOfferId(String matchedOfferId) {
        this.matchedOfferId = matchedOfferId;
    }
}
