package spatiad

// Coordinates represents a geographic point.
type Coordinates struct {
	Latitude  float64 `json:"latitude"`
	Longitude float64 `json:"longitude"`
}

// DriverStatus represents the availability state of a driver.
type DriverStatus string

const (
	DriverStatusAvailable DriverStatus = "Available"
	DriverStatusOffline   DriverStatus = "Offline"
	DriverStatusBusy      DriverStatus = "Busy"
)

// CreateOfferRequest is the payload for creating a dispatch offer.
type CreateOfferRequest struct {
	JobID          string       `json:"job_id"`
	Category       string       `json:"category"`
	Pickup         Coordinates  `json:"pickup"`
	Dropoff        *Coordinates `json:"dropoff,omitempty"`
	InitialRadiusKm float64    `json:"initial_radius_km"`
	MaxRadiusKm    float64      `json:"max_radius_km"`
	TimeoutSeconds int          `json:"timeout_seconds"`
}

// CreateOfferResponse is the response after successfully creating an offer.
type CreateOfferResponse struct {
	OfferID string `json:"offer_id"`
}

// UpsertDriverRequest is the payload for registering or updating a driver.
type UpsertDriverRequest struct {
	DriverID string       `json:"driver_id"`
	Category string       `json:"category"`
	Status   DriverStatus `json:"status"`
	Position Coordinates  `json:"position"`
}

// JobStatusResponse represents the current dispatch state of a job.
type JobStatusResponse struct {
	JobID          string  `json:"job_id"`
	State          string  `json:"state"`
	MatchedDriverID *string `json:"matched_driver_id"`
	MatchedOfferID  *string `json:"matched_offer_id"`
}

// JobEvent represents a single dispatch event for a job.
type JobEvent struct {
	At       string  `json:"at"`
	Kind     string  `json:"kind"`
	OfferID  *string `json:"offer_id"`
	DriverID *string `json:"driver_id"`
	Status   *string `json:"status"`
}

// JobEventsResponse is the paginated response for job events.
type JobEventsResponse struct {
	JobID            string    `json:"job_id"`
	Events           []JobEvent `json:"events"`
	NextCursor       *string   `json:"next_cursor"`
	NextBeforeCursor *string   `json:"next_before_cursor"`
}

// GetJobEventsRequest specifies query parameters for fetching job events.
type GetJobEventsRequest struct {
	JobID  string
	Limit  int
	Cursor string
	Kinds  []string
}

// apiErrorBody is the JSON error shape returned by the Spatiad API.
type apiErrorBody struct {
	Error   string `json:"error"`
	Message string `json:"message"`
}
