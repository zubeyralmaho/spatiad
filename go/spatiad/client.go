package spatiad

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"math"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"time"
)

// ApiError is returned when the Spatiad API responds with a non-success status.
type ApiError struct {
	StatusCode int
	Code       string
	Message    string
}

func (e *ApiError) Error() string {
	if e.Message != "" {
		return fmt.Sprintf("spatiad: %s (status %d)", e.Message, e.StatusCode)
	}
	return fmt.Sprintf("spatiad: request failed with status %d", e.StatusCode)
}

// RetryOptions controls exponential-backoff retry behaviour.
type RetryOptions struct {
	MaxAttempts    int
	BackoffBase    time.Duration
	BackoffMax     time.Duration
	RetryOnStatuses []int
}

var defaultRetryStatuses = []int{408, 429, 500, 502, 503, 504}

// Option configures a Client.
type Option func(*Client)

// WithDispatcherToken sets the Bearer token for dispatcher authentication.
func WithDispatcherToken(token string) Option {
	return func(c *Client) { c.DispatcherToken = token }
}

// WithHTTPClient overrides the default http.Client.
func WithHTTPClient(hc *http.Client) Option {
	return func(c *Client) { c.HTTPClient = hc }
}

// WithRetry configures retry behaviour with exponential backoff.
func WithRetry(opts RetryOptions) Option {
	return func(c *Client) { c.retry = opts }
}

// Client is an HTTP client for the Spatiad spatial dispatch engine.
type Client struct {
	BaseURL         string
	DispatcherToken string
	HTTPClient      *http.Client
	retry           RetryOptions
}

// NewClient creates a Client for the given base URL.
func NewClient(baseURL string, opts ...Option) *Client {
	c := &Client{
		BaseURL:    strings.TrimRight(baseURL, "/"),
		HTTPClient: http.DefaultClient,
		retry: RetryOptions{
			MaxAttempts:     1,
			BackoffBase:     150 * time.Millisecond,
			BackoffMax:      2 * time.Second,
			RetryOnStatuses: defaultRetryStatuses,
		},
	}
	for _, o := range opts {
		o(c)
	}
	return c
}

// CreateOffer starts a dispatch offer and returns the generated offer ID.
func (c *Client) CreateOffer(ctx context.Context, req CreateOfferRequest) (CreateOfferResponse, error) {
	var resp CreateOfferResponse
	if err := c.doJSON(ctx, http.MethodPost, "/api/v1/dispatch/offer", req, &resp); err != nil {
		return CreateOfferResponse{}, err
	}
	return resp, nil
}

// UpsertDriver registers or updates a driver in the dispatch engine.
func (c *Client) UpsertDriver(ctx context.Context, req UpsertDriverRequest) error {
	return c.doJSON(ctx, http.MethodPost, "/api/v1/driver/upsert", req, nil)
}

// CancelOffer cancels a pending dispatch offer.
func (c *Client) CancelOffer(ctx context.Context, offerID string) error {
	body := struct {
		OfferID string `json:"offer_id"`
	}{OfferID: offerID}
	return c.doJSON(ctx, http.MethodPost, "/api/v1/dispatch/cancel", body, nil)
}

// CancelJob cancels all dispatch activity for a job.
func (c *Client) CancelJob(ctx context.Context, jobID string) error {
	body := struct {
		JobID string `json:"job_id"`
	}{JobID: jobID}
	return c.doJSON(ctx, http.MethodPost, "/api/v1/dispatch/job/cancel", body, nil)
}

// GetJobStatus returns the current dispatch state for a job.
func (c *Client) GetJobStatus(ctx context.Context, jobID string) (JobStatusResponse, error) {
	var resp JobStatusResponse
	path := "/api/v1/dispatch/job/" + url.PathEscape(jobID)
	if err := c.doRequest(ctx, http.MethodGet, path, nil, &resp); err != nil {
		return JobStatusResponse{}, err
	}
	return resp, nil
}

// GetJobEvents returns paginated dispatch events for a job.
func (c *Client) GetJobEvents(ctx context.Context, req GetJobEventsRequest) (JobEventsResponse, error) {
	var resp JobEventsResponse
	path := "/api/v1/dispatch/job/" + url.PathEscape(req.JobID) + "/events"

	params := url.Values{}
	if req.Limit > 0 {
		params.Set("limit", strconv.Itoa(req.Limit))
	}
	if req.Cursor != "" {
		params.Set("cursor", req.Cursor)
	}
	if len(req.Kinds) > 0 {
		params.Set("kinds", strings.Join(req.Kinds, ","))
	}
	if encoded := params.Encode(); encoded != "" {
		path += "?" + encoded
	}

	if err := c.doRequest(ctx, http.MethodGet, path, nil, &resp); err != nil {
		return JobEventsResponse{}, err
	}
	return resp, nil
}

// doJSON encodes body as JSON, sends the request, and decodes the response into dest (if non-nil).
func (c *Client) doJSON(ctx context.Context, method, path string, body any, dest any) error {
	var buf bytes.Buffer
	if err := json.NewEncoder(&buf).Encode(body); err != nil {
		return fmt.Errorf("spatiad: encode request body: %w", err)
	}
	return c.doRequest(ctx, method, path, &buf, dest)
}

// doRequest performs an HTTP request with retry logic.
func (c *Client) doRequest(ctx context.Context, method, path string, body io.Reader, dest any) error {
	fullURL := c.BaseURL + path

	maxAttempts := c.retry.MaxAttempts
	if maxAttempts < 1 {
		maxAttempts = 1
	}

	retryStatuses := c.retry.RetryOnStatuses
	if retryStatuses == nil {
		retryStatuses = defaultRetryStatuses
	}

	// Buffer the body so it can be replayed across retries.
	var bodyBytes []byte
	if body != nil {
		var err error
		bodyBytes, err = io.ReadAll(body)
		if err != nil {
			return fmt.Errorf("spatiad: read request body: %w", err)
		}
	}

	var lastErr error
	for attempt := 1; attempt <= maxAttempts; attempt++ {
		var reqBody io.Reader
		if bodyBytes != nil {
			reqBody = bytes.NewReader(bodyBytes)
		}

		req, err := http.NewRequestWithContext(ctx, method, fullURL, reqBody)
		if err != nil {
			return fmt.Errorf("spatiad: create request: %w", err)
		}

		if bodyBytes != nil {
			req.Header.Set("Content-Type", "application/json")
		}
		if c.DispatcherToken != "" {
			req.Header.Set("Authorization", "Bearer "+c.DispatcherToken)
		}

		resp, err := c.HTTPClient.Do(req)
		if err != nil {
			lastErr = err
			if ctx.Err() != nil {
				return err
			}
			if attempt < maxAttempts {
				c.backoff(ctx, attempt)
				continue
			}
			return err
		}

		respBody, err := io.ReadAll(resp.Body)
		resp.Body.Close()
		if err != nil {
			return fmt.Errorf("spatiad: read response body: %w", err)
		}

		if resp.StatusCode >= 200 && resp.StatusCode < 300 {
			if dest != nil && len(respBody) > 0 {
				if err := json.Unmarshal(respBody, dest); err != nil {
					return fmt.Errorf("spatiad: decode response: %w", err)
				}
			}
			return nil
		}

		apiErr := c.parseApiError(resp.StatusCode, respBody)

		if attempt < maxAttempts && isRetryable(resp.StatusCode, retryStatuses) {
			lastErr = apiErr
			c.backoff(ctx, attempt)
			continue
		}

		return apiErr
	}

	return lastErr
}

// parseApiError builds an ApiError from a failed response.
func (c *Client) parseApiError(statusCode int, body []byte) *ApiError {
	var parsed apiErrorBody
	_ = json.Unmarshal(body, &parsed)
	return &ApiError{
		StatusCode: statusCode,
		Code:       parsed.Error,
		Message:    parsed.Message,
	}
}

// backoff sleeps with exponential back-off, respecting context cancellation.
func (c *Client) backoff(ctx context.Context, attempt int) {
	base := c.retry.BackoffBase
	if base <= 0 {
		base = 150 * time.Millisecond
	}
	maxB := c.retry.BackoffMax
	if maxB < base {
		maxB = 2 * time.Second
	}

	wait := time.Duration(float64(base) * math.Pow(2, float64(attempt-1)))
	if wait > maxB {
		wait = maxB
	}

	t := time.NewTimer(wait)
	defer t.Stop()

	select {
	case <-ctx.Done():
	case <-t.C:
	}
}

func isRetryable(status int, statuses []int) bool {
	for _, s := range statuses {
		if s == status {
			return true
		}
	}
	return false
}
