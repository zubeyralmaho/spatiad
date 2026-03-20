import Foundation

/// Error returned when the Spatiad API responds with a non-success HTTP status.
public struct SpatiadApiError: Error, LocalizedError, Sendable {
    /// The HTTP status code of the failed response.
    public let statusCode: Int

    /// Machine-readable error code from the response body, if present.
    public let code: String?

    /// Human-readable error message from the response body.
    public let message: String

    public var errorDescription: String? {
        if !message.isEmpty {
            return "spatiad: \(message) (status \(statusCode))"
        }
        return "spatiad: request failed with status \(statusCode)"
    }

    public init(statusCode: Int, code: String?, message: String) {
        self.statusCode = statusCode
        self.code = code
        self.message = message
    }
}
