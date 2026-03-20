import Foundation

/// Lightweight HTTP client for the Spatiad spatial dispatch engine.
public final class SpatiadClient: Sendable {

    private let baseURL: String
    private let dispatcherToken: String?
    private let session: URLSession
    private let encoder: JSONEncoder
    private let decoder: JSONDecoder

    private static let maxAttempts = 3
    private static let backoffBase: TimeInterval = 0.150
    private static let backoffMax: TimeInterval = 2.0
    private static let retryableStatuses: Set<Int> = [408, 429, 500, 502, 503, 504]

    /// Creates a new client for the Spatiad API.
    ///
    /// - Parameters:
    ///   - baseURL: Root URL of the Spatiad server (e.g. `"https://dispatch.example.com"`).
    ///   - dispatcherToken: Optional Bearer token for dispatcher authentication.
    ///   - session: Custom `URLSession` instance. Defaults to `.shared`.
    public init(baseURL: String, dispatcherToken: String? = nil, session: URLSession = .shared) {
        self.baseURL = baseURL.hasSuffix("/") ? String(baseURL.dropLast()) : baseURL
        self.dispatcherToken = dispatcherToken
        self.session = session

        let enc = JSONEncoder()
        enc.keyEncodingStrategy = .convertToSnakeCase
        self.encoder = enc

        let dec = JSONDecoder()
        dec.keyDecodingStrategy = .convertFromSnakeCase
        self.decoder = dec
    }

    // MARK: - Public API

    /// Starts a dispatch offer and returns the generated offer ID.
    public func createOffer(_ request: CreateOfferRequest) async throws -> CreateOfferResponse {
        try await postJSON("/api/v1/dispatch/offer", body: request)
    }

    /// Registers or updates a driver in the dispatch engine.
    public func upsertDriver(_ request: UpsertDriverRequest) async throws {
        let _: EmptyBody? = try await postJSON("/api/v1/driver/upsert", body: request)
    }

    /// Cancels a pending dispatch offer.
    public func cancelOffer(offerId: String) async throws {
        let body = CancelOfferBody(offerId: offerId)
        let _: EmptyBody? = try await postJSON("/api/v1/dispatch/cancel", body: body)
    }

    /// Cancels all dispatch activity for a job.
    public func cancelJob(jobId: String) async throws {
        let body = CancelJobBody(jobId: jobId)
        let _: EmptyBody? = try await postJSON("/api/v1/dispatch/job/cancel", body: body)
    }

    /// Returns the current dispatch state for a job.
    public func getJobStatus(jobId: String) async throws -> JobStatusResponse {
        let path = "/api/v1/dispatch/job/\(jobId.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? jobId)"
        return try await get(path)
    }

    /// Returns paginated dispatch events for a job.
    public func getJobEvents(_ request: GetJobEventsRequest) async throws -> JobEventsResponse {
        let escapedId = request.jobId.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? request.jobId
        var path = "/api/v1/dispatch/job/\(escapedId)/events"

        var queryItems: [URLQueryItem] = []
        if let limit = request.limit {
            queryItems.append(URLQueryItem(name: "limit", value: String(limit)))
        }
        if let cursor = request.cursor {
            queryItems.append(URLQueryItem(name: "cursor", value: cursor))
        }
        if let kinds = request.kinds, !kinds.isEmpty {
            queryItems.append(URLQueryItem(name: "kinds", value: kinds.joined(separator: ",")))
        }

        if !queryItems.isEmpty {
            var components = URLComponents()
            components.queryItems = queryItems
            if let query = components.percentEncodedQuery {
                path += "?\(query)"
            }
        }

        return try await get(path)
    }

    // MARK: - Internal helpers

    private func postJSON<Body: Encodable, Response: Decodable>(
        _ path: String,
        body: Body
    ) async throws -> Response {
        let url = try buildURL(path)
        var urlRequest = URLRequest(url: url)
        urlRequest.httpMethod = "POST"
        urlRequest.setValue("application/json", forHTTPHeaderField: "Content-Type")
        applyAuth(&urlRequest)
        urlRequest.httpBody = try encoder.encode(body)
        return try await execute(urlRequest)
    }

    private func get<Response: Decodable>(_ path: String) async throws -> Response {
        let url = try buildURL(path)
        var urlRequest = URLRequest(url: url)
        urlRequest.httpMethod = "GET"
        applyAuth(&urlRequest)
        return try await execute(urlRequest)
    }

    private func execute<Response: Decodable>(_ request: URLRequest) async throws -> Response {
        var lastError: Error?

        for attempt in 1...Self.maxAttempts {
            let data: Data
            let httpResponse: HTTPURLResponse

            do {
                let (responseData, response) = try await session.data(for: request)
                guard let hr = response as? HTTPURLResponse else {
                    throw URLError(.badServerResponse)
                }
                data = responseData
                httpResponse = hr
            } catch {
                lastError = error

                if (error as? URLError)?.code == .cancelled {
                    throw error
                }

                if attempt < Self.maxAttempts {
                    try await backoff(attempt: attempt)
                    continue
                }
                throw error
            }

            if (200..<300).contains(httpResponse.statusCode) {
                if data.isEmpty || Response.self == EmptyBody?.self {
                    // For void-returning endpoints, return a synthesized empty value.
                    // This works because the caller discards the result.
                    if let empty = Optional<EmptyBody>.none as? Response {
                        return empty
                    }
                }
                return try decoder.decode(Response.self, from: data)
            }

            let apiError = parseApiError(statusCode: httpResponse.statusCode, data: data)

            if attempt < Self.maxAttempts && Self.retryableStatuses.contains(httpResponse.statusCode) {
                lastError = apiError
                try await backoff(attempt: attempt)
                continue
            }

            throw apiError
        }

        throw lastError ?? URLError(.unknown)
    }

    private func buildURL(_ path: String) throws -> URL {
        guard let url = URL(string: baseURL + path) else {
            throw URLError(.badURL)
        }
        return url
    }

    private func applyAuth(_ request: inout URLRequest) {
        if let token = dispatcherToken {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
    }

    private func parseApiError(statusCode: Int, data: Data) -> SpatiadApiError {
        struct ErrorBody: Decodable {
            let error: String?
            let message: String?
        }

        let parsed = try? JSONDecoder().decode(ErrorBody.self, from: data)
        return SpatiadApiError(
            statusCode: statusCode,
            code: parsed?.error,
            message: parsed?.message ?? ""
        )
    }

    private func backoff(attempt: Int) async throws {
        let wait = min(Self.backoffMax, Self.backoffBase * pow(2.0, Double(attempt - 1)))
        try await Task.sleep(nanoseconds: UInt64(wait * 1_000_000_000))
    }
}

// MARK: - Internal request bodies

private struct CancelOfferBody: Encodable {
    let offerId: String
}

private struct CancelJobBody: Encodable {
    let jobId: String
}

private struct EmptyBody: Decodable {}
