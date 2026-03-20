package com.spatiad.sdk;

import com.fasterxml.jackson.annotation.JsonProperty;

import java.util.List;

public class JobEventsResponse {

    @JsonProperty("job_id")
    private String jobId;

    @JsonProperty("events")
    private List<JobEvent> events;

    @JsonProperty("next_cursor")
    private String nextCursor;

    @JsonProperty("next_before_cursor")
    private String nextBeforeCursor;

    public JobEventsResponse() {
    }

    public String getJobId() {
        return jobId;
    }

    public void setJobId(String jobId) {
        this.jobId = jobId;
    }

    public List<JobEvent> getEvents() {
        return events;
    }

    public void setEvents(List<JobEvent> events) {
        this.events = events;
    }

    public String getNextCursor() {
        return nextCursor;
    }

    public void setNextCursor(String nextCursor) {
        this.nextCursor = nextCursor;
    }

    public String getNextBeforeCursor() {
        return nextBeforeCursor;
    }

    public void setNextBeforeCursor(String nextBeforeCursor) {
        this.nextBeforeCursor = nextBeforeCursor;
    }
}
