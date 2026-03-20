package com.spatiad.sdk;

import java.util.List;

public class GetJobEventsRequest {

    private String jobId;
    private Integer limit;
    private String cursor;
    private List<String> kinds;

    public GetJobEventsRequest() {
    }

    public GetJobEventsRequest(String jobId) {
        this.jobId = jobId;
    }

    public String getJobId() {
        return jobId;
    }

    public void setJobId(String jobId) {
        this.jobId = jobId;
    }

    public Integer getLimit() {
        return limit;
    }

    public void setLimit(Integer limit) {
        this.limit = limit;
    }

    public String getCursor() {
        return cursor;
    }

    public void setCursor(String cursor) {
        this.cursor = cursor;
    }

    public List<String> getKinds() {
        return kinds;
    }

    public void setKinds(List<String> kinds) {
        this.kinds = kinds;
    }
}
