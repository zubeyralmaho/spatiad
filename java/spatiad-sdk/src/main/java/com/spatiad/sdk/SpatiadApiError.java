package com.spatiad.sdk;

public class SpatiadApiError extends RuntimeException {

    private final int statusCode;
    private final String code;
    private final String message;

    public SpatiadApiError(int statusCode, String code, String message) {
        super(message != null ? message : "Spatiad API error (HTTP " + statusCode + ")");
        this.statusCode = statusCode;
        this.code = code;
        this.message = message;
    }

    public int getStatusCode() {
        return statusCode;
    }

    public String getCode() {
        return code;
    }

    @Override
    public String getMessage() {
        return message;
    }
}
