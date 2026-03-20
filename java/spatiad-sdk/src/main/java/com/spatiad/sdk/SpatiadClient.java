package com.spatiad.sdk;

import com.fasterxml.jackson.annotation.JsonInclude;
import com.fasterxml.jackson.annotation.JsonProperty;
import com.fasterxml.jackson.databind.DeserializationFeature;
import com.fasterxml.jackson.databind.ObjectMapper;

import java.io.IOException;
import java.net.URI;
import java.net.URLEncoder;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.List;
import java.util.Map;
import java.util.StringJoiner;

/**
 * Lightweight HTTP client for the Spatiad spatial dispatch engine REST API.
 *
 * <p>Uses Java 11+ {@link HttpClient} with Jackson for JSON serialization.
 * Retries failed requests with exponential backoff (3 attempts, 150ms base).
 */
public class SpatiadClient {

    private static final int MAX_ATTEMPTS = 3;
    private static final long BASE_BACKOFF_MS = 150;

    private final String baseUrl;
    private final String dispatcherToken;
    private final HttpClient httpClient;
    private final ObjectMapper objectMapper;

    /**
     * Create a client without authentication.
     *
     * @param baseUrl the Spatiad server base URL (e.g. "http://localhost:8080")
     */
    public SpatiadClient(String baseUrl) {
        this(baseUrl, null);
    }

    /**
     * Create a client with a dispatcher bearer token.
     *
     * @param baseUrl          the Spatiad server base URL
     * @param dispatcherToken  optional bearer token (may be null)
     */
    public SpatiadClient(String baseUrl, String dispatcherToken) {
        this.baseUrl = baseUrl.endsWith("/") ? baseUrl.substring(0, baseUrl.length() - 1) : baseUrl;
        this.dispatcherToken = dispatcherToken;
        this.httpClient = HttpClient.newBuilder()
                .connectTimeout(Duration.ofSeconds(10))
                .build();
        this.objectMapper = new ObjectMapper()
                .configure(DeserializationFeature.FAIL_ON_UNKNOWN_PROPERTIES, false)
                .setSerializationInclusion(JsonInclude.Include.NON_NULL);
    }

    /**
     * Create a dispatch offer.
     */
    public CreateOfferResponse createOffer(CreateOfferRequest request) {
        return post("/api/v1/dispatch/offer", request, CreateOfferResponse.class);
    }

    /**
     * Insert or update a driver's location and status.
     */
    public void upsertDriver(UpsertDriverRequest request) {
        post("/api/v1/driver/upsert", request, Void.class);
    }

    /**
     * Cancel an active offer.
     */
    public void cancelOffer(String offerId) {
        post("/api/v1/dispatch/cancel", Map.of("offer_id", offerId), Void.class);
    }

    /**
     * Cancel a job.
     */
    public void cancelJob(String jobId) {
        post("/api/v1/dispatch/job/cancel", Map.of("job_id", jobId), Void.class);
    }

    /**
     * Get the current status of a job.
     */
    public JobStatusResponse getJobStatus(String jobId) {
        return get("/api/v1/dispatch/job/" + encode(jobId), JobStatusResponse.class);
    }

    /**
     * Get events for a job with optional pagination and filtering.
     */
    public JobEventsResponse getJobEvents(GetJobEventsRequest request) {
        StringJoiner query = new StringJoiner("&");
        if (request.getLimit() != null) {
            query.add("limit=" + request.getLimit());
        }
        if (request.getCursor() != null) {
            query.add("cursor=" + encode(request.getCursor()));
        }
        List<String> kinds = request.getKinds();
        if (kinds != null && !kinds.isEmpty()) {
            for (String kind : kinds) {
                query.add("kinds=" + encode(kind));
            }
        }

        String path = "/api/v1/dispatch/job/" + encode(request.getJobId()) + "/events";
        if (query.length() > 0) {
            path += "?" + query;
        }
        return get(path, JobEventsResponse.class);
    }

    // ---- internal helpers ----

    private <T> T post(String path, Object body, Class<T> responseType) {
        try {
            byte[] jsonBody = objectMapper.writeValueAsBytes(body);
            HttpRequest.Builder builder = HttpRequest.newBuilder()
                    .uri(URI.create(baseUrl + path))
                    .header("Content-Type", "application/json")
                    .POST(HttpRequest.BodyPublishers.ofByteArray(jsonBody));
            applyAuth(builder);
            return executeWithRetry(builder.build(), responseType);
        } catch (SpatiadApiError e) {
            throw e;
        } catch (Exception e) {
            throw new RuntimeException("Failed to execute POST " + path, e);
        }
    }

    private <T> T get(String path, Class<T> responseType) {
        try {
            HttpRequest.Builder builder = HttpRequest.newBuilder()
                    .uri(URI.create(baseUrl + path))
                    .GET();
            applyAuth(builder);
            return executeWithRetry(builder.build(), responseType);
        } catch (SpatiadApiError e) {
            throw e;
        } catch (Exception e) {
            throw new RuntimeException("Failed to execute GET " + path, e);
        }
    }

    private void applyAuth(HttpRequest.Builder builder) {
        if (dispatcherToken != null && !dispatcherToken.isEmpty()) {
            builder.header("Authorization", "Bearer " + dispatcherToken);
        }
    }

    private <T> T executeWithRetry(HttpRequest request, Class<T> responseType)
            throws IOException, InterruptedException {
        IOException lastException = null;

        for (int attempt = 0; attempt < MAX_ATTEMPTS; attempt++) {
            if (attempt > 0) {
                long backoff = BASE_BACKOFF_MS * (1L << (attempt - 1));
                Thread.sleep(backoff);
            }
            try {
                HttpResponse<byte[]> response = httpClient.send(request,
                        HttpResponse.BodyHandlers.ofByteArray());

                int status = response.statusCode();
                if (status >= 200 && status < 300) {
                    if (responseType == Void.class || response.body().length == 0) {
                        return null;
                    }
                    return objectMapper.readValue(response.body(), responseType);
                }

                if (status >= 500 || status == 429) {
                    lastException = new IOException("HTTP " + status);
                    continue;
                }

                // Client error -- do not retry
                String code = null;
                String message = null;
                try {
                    ErrorBody err = objectMapper.readValue(response.body(), ErrorBody.class);
                    code = err.code;
                    message = err.message;
                } catch (Exception ignored) {
                    message = response.body().length > 0
                            ? new String(response.body(), StandardCharsets.UTF_8)
                            : null;
                }
                throw new SpatiadApiError(status, code, message);

            } catch (IOException e) {
                lastException = e;
            }
        }

        throw lastException != null ? lastException : new IOException("Request failed after retries");
    }

    private static String encode(String value) {
        return URLEncoder.encode(value, StandardCharsets.UTF_8);
    }

    /** Internal DTO for parsing error response bodies. */
    private static class ErrorBody {
        @JsonProperty("code")
        public String code;
        @JsonProperty("message")
        public String message;
    }
}
