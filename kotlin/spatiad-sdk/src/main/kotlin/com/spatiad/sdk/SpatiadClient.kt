package com.spatiad.sdk

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import okhttp3.HttpUrl
import okhttp3.HttpUrl.Companion.toHttpUrl
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response
import java.io.IOException
import java.util.concurrent.TimeUnit

/**
 * Lightweight HTTP client for the Spatiad spatial dispatch REST API.
 *
 * All public methods are `suspend` functions safe to call from any coroutine
 * context; network I/O is dispatched on [Dispatchers.IO].
 *
 * ```kotlin
 * val client = SpatiadClient(baseUrl = "https://api.spatiad.example.com", token = "sk-...")
 * val response = client.createOffer(CreateOfferRequest(...))
 * client.close()
 * ```
 */
class SpatiadClient(
    private val baseUrl: String,
    private val token: String? = null,
    private val maxRetries: Int = 3,
    private val retryBaseDelayMs: Long = 150L,
    private val retryableStatuses: Set<Int> = setOf(408, 429, 500, 502, 503, 504),
    httpClient: OkHttpClient? = null,
) {
    private val json = Json {
        ignoreUnknownKeys = true
        encodeDefaults = false
    }

    private val client: OkHttpClient = httpClient ?: OkHttpClient.Builder()
        .connectTimeout(30, TimeUnit.SECONDS)
        .readTimeout(30, TimeUnit.SECONDS)
        .writeTimeout(30, TimeUnit.SECONDS)
        .build()

    private val normalizedBaseUrl: String = baseUrl.trimEnd('/')

    private val jsonMediaType = "application/json; charset=utf-8".toMediaType()

    // -- public API ----------------------------------------------------------

    /**
     * Creates a dispatch offer.
     *
     * POST /api/v1/dispatch/offer
     */
    suspend fun createOffer(request: CreateOfferRequest): CreateOfferResponse {
        val body = json.encodeToString(request)
        val response = executeWithRetry(
            buildPost("/api/v1/dispatch/offer", body),
        )
        return json.decodeFromString(response)
    }

    /**
     * Creates or updates a driver's status and position.
     *
     * POST /api/v1/driver/upsert
     */
    suspend fun upsertDriver(request: UpsertDriverRequest) {
        val body = json.encodeToString(request)
        executeWithRetry(buildPost("/api/v1/driver/upsert", body))
    }

    /**
     * Cancels an active offer.
     *
     * POST /api/v1/dispatch/cancel
     */
    suspend fun cancelOffer(offerId: String) {
        val body = json.encodeToString(CancelOfferBody(offerId))
        executeWithRetry(buildPost("/api/v1/dispatch/cancel", body))
    }

    /**
     * Cancels an active job.
     *
     * POST /api/v1/dispatch/job/cancel
     */
    suspend fun cancelJob(jobId: String) {
        val body = json.encodeToString(CancelJobBody(jobId))
        executeWithRetry(buildPost("/api/v1/dispatch/job/cancel", body))
    }

    /**
     * Returns the current status of a job.
     *
     * GET /api/v1/dispatch/job/{job_id}
     */
    suspend fun getJobStatus(jobId: String): JobStatusResponse {
        val response = executeWithRetry(buildGet("/api/v1/dispatch/job/$jobId"))
        return json.decodeFromString(response)
    }

    /**
     * Returns events for a job with optional pagination and filtering.
     *
     * GET /api/v1/dispatch/job/{job_id}/events
     */
    suspend fun getJobEvents(request: GetJobEventsRequest): JobEventsResponse {
        val urlBuilder = "$normalizedBaseUrl/api/v1/dispatch/job/${request.jobId}/events"
            .toHttpUrl()
            .newBuilder()

        request.limit?.let { urlBuilder.addQueryParameter("limit", it.toString()) }
        request.cursor?.let { urlBuilder.addQueryParameter("cursor", it) }
        request.kinds?.takeIf { it.isNotEmpty() }?.let {
            urlBuilder.addQueryParameter("kinds", it.joinToString(","))
        }

        val httpRequest = newRequestBuilder()
            .url(urlBuilder.build())
            .get()
            .build()

        val response = executeWithRetry(httpRequest)
        return json.decodeFromString(response)
    }

    /**
     * Shuts down the underlying OkHttp connection pool and dispatcher.
     */
    fun close() {
        client.dispatcher.executorService.shutdown()
        client.connectionPool.evictAll()
    }

    // -- internals -----------------------------------------------------------

    private fun newRequestBuilder(): Request.Builder {
        val builder = Request.Builder()
        token?.let { builder.addHeader("Authorization", "Bearer $it") }
        return builder
    }

    private fun buildPost(path: String, jsonBody: String): Request =
        newRequestBuilder()
            .url("$normalizedBaseUrl$path")
            .post(jsonBody.toRequestBody(jsonMediaType))
            .build()

    private fun buildGet(path: String): Request =
        newRequestBuilder()
            .url("$normalizedBaseUrl$path")
            .get()
            .build()

    /**
     * Executes [request] with exponential-backoff retries on transient errors.
     *
     * Returns the response body as a [String]. Throws [SpatiadApiError] on
     * non-success responses that are not retryable (or after all retries are
     * exhausted), and re-throws [IOException] for transport-level failures.
     */
    private suspend fun executeWithRetry(request: Request): String {
        var lastException: Exception? = null

        for (attempt in 0 until maxRetries) {
            try {
                val response = execute(request)
                val responseBody = response.body?.string().orEmpty()

                if (response.isSuccessful) {
                    return responseBody
                }

                val apiError = parseError(response.code, responseBody, request)

                if (response.code !in retryableStatuses || attempt == maxRetries - 1) {
                    throw apiError
                }

                lastException = apiError
            } catch (e: IOException) {
                lastException = e
                if (attempt == maxRetries - 1) throw e
            }

            delay(retryBaseDelayMs * (1L shl attempt))
        }

        throw lastException ?: SpatiadApiError(
            statusCode = 0,
            message = "Request failed after $maxRetries retries",
        )
    }

    private suspend fun execute(request: Request): Response =
        withContext(Dispatchers.IO) {
            client.newCall(request).execute()
        }

    private fun parseError(
        statusCode: Int,
        body: String,
        request: Request,
    ): SpatiadApiError {
        val parsed = try {
            json.decodeFromString<ApiErrorBody>(body)
        } catch (_: Exception) {
            null
        }

        return SpatiadApiError(
            statusCode = statusCode,
            code = parsed?.error,
            message = parsed?.message
                ?: "${request.method} ${request.url.encodedPath} failed with status $statusCode",
        )
    }
}
