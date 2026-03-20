import Foundation

// MARK: - Common

public struct Coordinates: Codable, Sendable {
    public let latitude: Double
    public let longitude: Double

    public init(latitude: Double, longitude: Double) {
        self.latitude = latitude
        self.longitude = longitude
    }
}

// MARK: - Create Offer

public struct CreateOfferRequest: Codable, Sendable {
    public let jobId: String
    public let category: String
    public let pickup: Coordinates
    public let dropoff: Coordinates?
    public let initialRadiusKm: Double
    public let maxRadiusKm: Double
    public let timeoutSeconds: Int

    public init(
        jobId: String,
        category: String,
        pickup: Coordinates,
        dropoff: Coordinates? = nil,
        initialRadiusKm: Double,
        maxRadiusKm: Double,
        timeoutSeconds: Int
    ) {
        self.jobId = jobId
        self.category = category
        self.pickup = pickup
        self.dropoff = dropoff
        self.initialRadiusKm = initialRadiusKm
        self.maxRadiusKm = maxRadiusKm
        self.timeoutSeconds = timeoutSeconds
    }
}

public struct CreateOfferResponse: Codable, Sendable {
    public let offerId: String

    public init(offerId: String) {
        self.offerId = offerId
    }
}

// MARK: - Upsert Driver

public struct UpsertDriverRequest: Codable, Sendable {
    public let driverId: String
    public let category: String
    public let status: String
    public let position: Coordinates

    public init(driverId: String, category: String, status: String, position: Coordinates) {
        self.driverId = driverId
        self.category = category
        self.status = status
        self.position = position
    }
}

// MARK: - Job Status

public struct JobStatusResponse: Codable, Sendable {
    public let jobId: String
    public let state: String
    public let matchedDriverId: String?
    public let matchedOfferId: String?

    public init(jobId: String, state: String, matchedDriverId: String? = nil, matchedOfferId: String? = nil) {
        self.jobId = jobId
        self.state = state
        self.matchedDriverId = matchedDriverId
        self.matchedOfferId = matchedOfferId
    }
}

// MARK: - Job Events

public struct GetJobEventsRequest: Sendable {
    public let jobId: String
    public let limit: Int?
    public let cursor: String?
    public let kinds: [String]?

    public init(jobId: String, limit: Int? = nil, cursor: String? = nil, kinds: [String]? = nil) {
        self.jobId = jobId
        self.limit = limit
        self.cursor = cursor
        self.kinds = kinds
    }
}

public struct JobEventsResponse: Codable, Sendable {
    public let jobId: String
    public let events: [JobEvent]
    public let nextCursor: String?
    public let nextBeforeCursor: String?

    public init(jobId: String, events: [JobEvent], nextCursor: String? = nil, nextBeforeCursor: String? = nil) {
        self.jobId = jobId
        self.events = events
        self.nextCursor = nextCursor
        self.nextBeforeCursor = nextBeforeCursor
    }
}

public struct JobEvent: Codable, Sendable {
    public let at: String
    public let kind: String
    public let offerId: String?
    public let driverId: String?
    public let status: String?

    public init(at: String, kind: String, offerId: String? = nil, driverId: String? = nil, status: String? = nil) {
        self.at = at
        self.kind = kind
        self.offerId = offerId
        self.driverId = driverId
        self.status = status
    }
}
