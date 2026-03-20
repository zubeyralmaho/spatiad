package com.spatiad.sdk

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

@Serializable
data class Coordinates(
    val latitude: Double,
    val longitude: Double,
)

// ---------------------------------------------------------------------------
// Dispatch – Create Offer
// ---------------------------------------------------------------------------

@Serializable
data class CreateOfferRequest(
    @SerialName("job_id") val jobId: String,
    val category: String,
    val pickup: Coordinates,
    val dropoff: Coordinates? = null,
    @SerialName("initial_radius_km") val initialRadiusKm: Double = 3.0,
    @SerialName("max_radius_km") val maxRadiusKm: Double = 10.0,
    @SerialName("timeout_seconds") val timeoutSeconds: Int = 30,
)

@Serializable
data class CreateOfferResponse(
    @SerialName("offer_id") val offerId: String,
)

// ---------------------------------------------------------------------------
// Driver – Upsert
// ---------------------------------------------------------------------------

@Serializable
data class UpsertDriverRequest(
    @SerialName("driver_id") val driverId: String,
    val category: String,
    val status: String,
    val position: Coordinates,
)

// ---------------------------------------------------------------------------
// Job Status
// ---------------------------------------------------------------------------

@Serializable
data class JobStatusResponse(
    @SerialName("job_id") val jobId: String,
    val state: String,
    @SerialName("matched_driver_id") val matchedDriverId: String? = null,
    @SerialName("matched_offer_id") val matchedOfferId: String? = null,
)

// ---------------------------------------------------------------------------
// Job Events
// ---------------------------------------------------------------------------

@Serializable
data class JobEvent(
    val at: String,
    val kind: String,
    @SerialName("offer_id") val offerId: String? = null,
    @SerialName("driver_id") val driverId: String? = null,
    val status: String? = null,
)

@Serializable
data class JobEventsResponse(
    @SerialName("job_id") val jobId: String,
    val events: List<JobEvent>,
    @SerialName("next_cursor") val nextCursor: String? = null,
    @SerialName("next_before_cursor") val nextBeforeCursor: String? = null,
)

/**
 * Parameters for [SpatiadClient.getJobEvents]. Not serialized directly;
 * fields are mapped to query-string parameters.
 */
data class GetJobEventsRequest(
    val jobId: String,
    val limit: Int? = null,
    val cursor: String? = null,
    val kinds: List<String>? = null,
)

// ---------------------------------------------------------------------------
// Internal request bodies for cancel endpoints
// ---------------------------------------------------------------------------

@Serializable
internal data class CancelOfferBody(
    @SerialName("offer_id") val offerId: String,
)

@Serializable
internal data class CancelJobBody(
    @SerialName("job_id") val jobId: String,
)

// ---------------------------------------------------------------------------
// Generic error body returned by the Spatiad API
// ---------------------------------------------------------------------------

@Serializable
internal data class ApiErrorBody(
    val error: String? = null,
    val message: String? = null,
)
