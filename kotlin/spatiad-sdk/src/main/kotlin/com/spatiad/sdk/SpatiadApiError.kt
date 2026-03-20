package com.spatiad.sdk

/**
 * Thrown when the Spatiad API returns a non-success HTTP response.
 */
class SpatiadApiError(
    val statusCode: Int,
    val code: String? = null,
    override val message: String,
) : Exception(message) {

    override fun toString(): String =
        "SpatiadApiError(statusCode=$statusCode, code=$code, message=$message)"
}
