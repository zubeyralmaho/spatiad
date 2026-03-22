use std::{collections::{HashMap, VecDeque}, sync::Arc};

use axum::{
    extract::ws::Message,
    extract::Request,
    extract::{Path, Query, State, WebSocketUpgrade},
    http::{HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use axum::extract::ws::WebSocket;
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use spatiad_core::{EngineStats, JobDispatchState, JobEventFilterKind, JobEventKind, JobEventRecord, JobEventsCursor};
use spatiad_dispatch::DispatchService;
use spatiad_types::{Coordinates, DriverStatus, JobRequest, MatchResult, OfferStatus};
use spatiad_ws::{DriverInbound, DriverOutbound};
use tokio::sync::{mpsc, Mutex};
use tokio::time::{interval, sleep, Duration};
use tracing::info;
use uuid::Uuid;

mod validation;
use validation::{
    validate_category, validate_coordinates, validate_radius,
    validate_timeout_seconds,
};

#[derive(Clone)]
pub struct ApiState {
    pub dispatch: Arc<Mutex<DispatchService>>,
    pub webhook_url: Option<String>,
    pub webhook_secret: Option<String>,
    pub webhook_timeout_ms: u64,
    pub driver_token: Option<String>,
    pub dispatcher_token: Option<String>,
    pub driver_ttl_secs: Option<u64>,
    pub dispatch_rate_limiter: Arc<Mutex<SlidingWindowRateLimiter>>,
    pub ws_reconnect_guard: Arc<Mutex<WsReconnectGuard>>,
    pub sessions: Arc<Mutex<HashMap<Uuid, mpsc::UnboundedSender<DriverOutbound>>>>,
}

#[derive(Debug)]
pub struct SlidingWindowRateLimiter {
    limit_per_window: usize,
    window_seconds: i64,
    entries: HashMap<String, VecDeque<chrono::DateTime<Utc>>>,
}

#[derive(Debug, Clone, Copy)]
struct RateLimitSnapshot {
    allowed: bool,
    limit: usize,
    remaining: usize,
    retry_after_seconds: u64,
    reset_seconds: u64,
}

impl SlidingWindowRateLimiter {
    pub fn new(limit_per_window: usize, window_seconds: i64) -> Self {
        Self {
            limit_per_window: limit_per_window.max(1),
            window_seconds: window_seconds.max(1),
            entries: HashMap::new(),
        }
    }

    fn check(&mut self, key: String) -> RateLimitSnapshot {
        let now = Utc::now();
        let cutoff = now - chrono::Duration::seconds(self.window_seconds);
        let queue = self.entries.entry(key).or_default();

        while queue.front().map(|ts| *ts <= cutoff).unwrap_or(false) {
            queue.pop_front();
        }

        if queue.len() >= self.limit_per_window {
            let reset_seconds = queue
                .front()
                .map(|ts| ((*ts + chrono::Duration::seconds(self.window_seconds)) - now).num_seconds())
                .map(|seconds| seconds.max(1) as u64)
                .unwrap_or(self.window_seconds as u64);

            return RateLimitSnapshot {
                allowed: false,
                limit: self.limit_per_window,
                remaining: 0,
                retry_after_seconds: reset_seconds,
                reset_seconds,
            };
        }

        queue.push_back(now);
        let remaining = self.limit_per_window.saturating_sub(queue.len());
        let reset_seconds = queue
            .front()
            .map(|ts| ((*ts + chrono::Duration::seconds(self.window_seconds)) - now).num_seconds())
            .map(|seconds| seconds.max(1) as u64)
            .unwrap_or(self.window_seconds as u64);

        RateLimitSnapshot {
            allowed: true,
            limit: self.limit_per_window,
            remaining,
            retry_after_seconds: 0,
            reset_seconds,
        }
    }
}

#[derive(Debug)]
pub struct WsReconnectGuard {
    max_reconnects_per_window: usize,
    window_seconds: i64,
    attempts: HashMap<Uuid, VecDeque<chrono::DateTime<Utc>>>,
}

impl WsReconnectGuard {
    pub fn new(max_reconnects_per_window: usize, window_seconds: i64) -> Self {
        Self {
            max_reconnects_per_window: max_reconnects_per_window.max(1),
            window_seconds: window_seconds.max(1),
            attempts: HashMap::new(),
        }
    }

    fn allow(&mut self, driver_id: Uuid) -> bool {
        let now = Utc::now();
        let cutoff = now - chrono::Duration::seconds(self.window_seconds);
        let queue = self.attempts.entry(driver_id).or_default();

        while queue.front().map(|ts| *ts <= cutoff).unwrap_or(false) {
            queue.pop_front();
        }

        if queue.len() >= self.max_reconnects_per_window {
            return false;
        }

        queue.push_back(now);
        true
    }
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
}

#[derive(Debug, Serialize)]
struct ReadyResponse {
    status: &'static str,
    service: &'static str,
    active_sessions: usize,
    engine: EngineStats,
}

#[derive(Debug, Deserialize)]
pub struct OfferRequest {
    pub job_id: Uuid,
    pub category: String,
    pub pickup: Coordinates,
    pub dropoff: Option<Coordinates>,
    pub initial_radius_km: f64,
    pub max_radius_km: f64,
    pub timeout_seconds: u64,
}

#[derive(Debug, Serialize)]
pub struct OfferAccepted {
    pub offer_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct OfferCancelRequest {
    pub offer_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct JobCancelRequest {
    pub job_id: Uuid,
}

#[derive(Debug, Serialize)]
pub struct JobStatusResponse {
    pub job_id: Uuid,
    pub state: &'static str,
    pub matched_driver_id: Option<Uuid>,
    pub matched_offer_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct JobEventsQuery {
    pub limit: Option<usize>,
    pub before: Option<String>,
    pub cursor: Option<String>,
    pub kinds: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct JobEventsResponse {
    pub job_id: Uuid,
    pub events: Vec<JobEventResponse>,
    pub next_before_cursor: Option<String>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ApiErrorResponse {
    pub error: &'static str,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct JobEventResponse {
    pub at: chrono::DateTime<Utc>,
    pub kind: &'static str,
    pub offer_id: Option<Uuid>,
    pub driver_id: Option<Uuid>,
    pub status: Option<&'static str>,
}

fn default_driver_rating() -> f32 {
    5.0
}

#[derive(Debug, Deserialize)]
pub struct DriverUpsertRequest {
    pub driver_id: Uuid,
    pub category: String,
    pub status: DriverStatus,
    pub position: Coordinates,
    /// Driver rating on a 1.0–5.0 scale. Defaults to 5.0 if not supplied.
    #[serde(default = "default_driver_rating")]
    pub rating: f32,
}

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/api/v1/driver/upsert", post(upsert_driver))
        .route("/api/v1/dispatch/offer", post(dispatch_offer))
        .route("/api/v1/dispatch/cancel", post(cancel_offer))
    .route("/api/v1/dispatch/job/cancel", post(cancel_job))
        .route("/api/v1/dispatch/job/:job_id", get(dispatch_job_status))
        .route("/api/v1/dispatch/job/:job_id/events", get(dispatch_job_events))
        .route("/api/v1/stream/driver/:driver_id", get(driver_ws))
        .layer(middleware::from_fn(request_context_middleware))
        .with_state(state)
}

async fn request_context_middleware(mut request: Request, next: Next) -> Response {
    let started = std::time::Instant::now();
    let method = request.method().clone();
    let path = request.uri().path().to_string();

    let request_id = request
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    if let Ok(header) = HeaderValue::from_str(&request_id) {
        request
            .headers_mut()
            .insert("x-request-id", header);
    }

    let mut response = next.run(request).await;
    let status = response.status();
    let elapsed_ms = started.elapsed().as_millis();

    if let Ok(header) = HeaderValue::from_str(&request_id) {
        response
            .headers_mut()
            .insert("x-request-id", header);
    }

    info!(
        request_id = %request_id,
        method = %method,
        path = %path,
        status = status.as_u16(),
        duration_ms = elapsed_ms,
        "http request",
    );

    response
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "spatiad",
    })
}

async fn ready(State(state): State<ApiState>) -> impl IntoResponse {
    let dispatch_guard = match state.dispatch.try_lock() {
        Ok(guard) => guard,
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ApiErrorResponse {
                    error: "not_ready",
                    message: "dispatch state is temporarily busy".to_string(),
                }),
            )
                .into_response();
        }
    };

    let sessions_guard = match state.sessions.try_lock() {
        Ok(guard) => guard,
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ApiErrorResponse {
                    error: "not_ready",
                    message: "session state is temporarily busy".to_string(),
                }),
            )
                .into_response();
        }
    };

    let engine_stats = dispatch_guard.engine.stats();

    (
        StatusCode::OK,
        Json(ReadyResponse {
            status: "ready",
            service: "spatiad",
            active_sessions: sessions_guard.len(),
            engine: engine_stats,
        }),
    )
        .into_response()
}

async fn dispatch_offer(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(payload): Json<OfferRequest>,
) -> impl IntoResponse {
    if !is_dispatcher_authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    // Validate inputs
    if let Err(e) = validate_category(&payload.category) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResponse {
                error: "invalid_category",
                message: e.message(),
            }),
        )
            .into_response();
    }

    if let Err(e) = validate_radius(payload.initial_radius_km, payload.max_radius_km) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResponse {
                error: "invalid_radius",
                message: e.message(),
            }),
        )
            .into_response();
    }

    if let Err(e) = validate_coordinates(&payload.pickup) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResponse {
                error: "invalid_coordinates",
                message: e.message(),
            }),
        )
            .into_response();
    }

    if let Some(dropoff) = &payload.dropoff {
        if let Err(e) = validate_coordinates(dropoff) {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResponse {
                    error: "invalid_coordinates",
                    message: e.message(),
                }),
            )
                .into_response();
        }
    }

    if let Err(e) = validate_timeout_seconds(payload.timeout_seconds) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResponse {
                error: "invalid_timeout",
                message: e.message(),
            }),
        )
            .into_response();
    }

    let rate_limit = check_dispatch_request(&state, &headers, "dispatch_offer").await;
    if !rate_limit.allowed {
        return rate_limited_json_response(rate_limit).into_response();
    }

    let mut dispatch = state.dispatch.lock().await;
    let request = JobRequest {
        job_id: payload.job_id,
        category: payload.category,
        pickup: payload.pickup,
        dropoff: payload.dropoff,
        initial_radius_km: payload.initial_radius_km,
        max_radius_km: payload.max_radius_km,
        timeout_seconds: payload.timeout_seconds,
        created_at: Utc::now(),
    };

    match dispatch.submit_job(request) {
        Ok(offer) => (axum::http::StatusCode::ACCEPTED, Json(OfferAccepted { offer_id: offer.offer_id })).into_response(),
        Err(_) => (axum::http::StatusCode::NOT_FOUND, Json(OfferAccepted { offer_id: Uuid::nil() })).into_response(),
    }
}

async fn upsert_driver(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(payload): Json<DriverUpsertRequest>,
) -> impl IntoResponse {
    if !is_dispatcher_authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    // Validate inputs
    if let Err(e) = validate_category(&payload.category) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResponse {
                error: "invalid_category",
                message: e.message(),
            }),
        )
            .into_response();
    }

    if let Err(e) = validate_coordinates(&payload.position) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResponse {
                error: "invalid_coordinates",
                message: e.message(),
            }),
        )
            .into_response();
    }

    let rate_limit = check_dispatch_request(&state, &headers, "driver_upsert").await;
    if !rate_limit.allowed {
        return rate_limited_status_response(rate_limit);
    }

    let mut dispatch = state.dispatch.lock().await;
    dispatch.engine.upsert_driver_location(
        payload.driver_id,
        payload.category,
        payload.position,
        payload.status,
        payload.rating,
    );

    StatusCode::OK.into_response()
}

async fn cancel_offer(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(payload): Json<OfferCancelRequest>,
) -> impl IntoResponse {
    if !is_dispatcher_authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let rate_limit = check_dispatch_request(&state, &headers, "dispatch_cancel_offer").await;
    if !rate_limit.allowed {
        return rate_limited_status_response(rate_limit);
    }

    let mut dispatch = state.dispatch.lock().await;
    dispatch.cancel_offer(payload.offer_id);
    axum::http::StatusCode::OK.into_response()
}

async fn cancel_job(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(payload): Json<JobCancelRequest>,
) -> impl IntoResponse {
    if !is_dispatcher_authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let rate_limit = check_dispatch_request(&state, &headers, "dispatch_cancel_job").await;
    if !rate_limit.allowed {
        return rate_limited_status_response(rate_limit);
    }

    let mut dispatch = state.dispatch.lock().await;
    if !dispatch.cancel_job(payload.job_id) {
        return StatusCode::NOT_FOUND.into_response();
    }

    StatusCode::OK.into_response()
}

async fn dispatch_job_status(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(job_id): Path<Uuid>,
) -> impl IntoResponse {
    if !is_dispatcher_authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let rate_limit = check_dispatch_request(&state, &headers, "dispatch_job_status").await;
    if !rate_limit.allowed {
        return rate_limited_json_response(rate_limit).into_response();
    }

    let dispatch = state.dispatch.lock().await;
    let job_state = dispatch.job_dispatch_state(job_id);

    let response = match job_state {
        JobDispatchState::UnknownJob => JobStatusResponse {
            job_id,
            state: "unknown",
            matched_driver_id: None,
            matched_offer_id: None,
        },
        JobDispatchState::Pending => JobStatusResponse {
            job_id,
            state: "pending",
            matched_driver_id: None,
            matched_offer_id: None,
        },
        JobDispatchState::Searching => JobStatusResponse {
            job_id,
            state: "searching",
            matched_driver_id: None,
            matched_offer_id: None,
        },
        JobDispatchState::Cancelled => JobStatusResponse {
            job_id,
            state: "cancelled",
            matched_driver_id: None,
            matched_offer_id: None,
        },
        JobDispatchState::Matched {
            driver_id,
            offer_id,
        } => JobStatusResponse {
            job_id,
            state: "matched",
            matched_driver_id: Some(driver_id),
            matched_offer_id: Some(offer_id),
        },
        JobDispatchState::Exhausted => JobStatusResponse {
            job_id,
            state: "exhausted",
            matched_driver_id: None,
            matched_offer_id: None,
        },
    };

    Json(response).into_response()
}

async fn dispatch_job_events(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(job_id): Path<Uuid>,
    Query(query): Query<JobEventsQuery>,
) -> Result<Json<JobEventsResponse>, (StatusCode, HeaderMap, Json<ApiErrorResponse>)> {
    if !is_dispatcher_authorized(&state, &headers) {
        return Err((
            StatusCode::UNAUTHORIZED,
            HeaderMap::new(),
            Json(ApiErrorResponse {
                error: "unauthorized",
                message: "missing or invalid dispatcher auth token".to_string(),
            }),
        ));
    }

    let rate_limit = check_dispatch_request(&state, &headers, "dispatch_job_events").await;
    if !rate_limit.allowed {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            rate_limit_headers(rate_limit),
            Json(ApiErrorResponse {
                error: "rate_limited",
                message: "dispatch request rate limit exceeded".to_string(),
            }),
        ));
    }

    let limit = query.limit.unwrap_or(50);

    if query.before.is_some() && query.cursor.is_some() {
        return Err((
            StatusCode::BAD_REQUEST,
            HeaderMap::new(),
            Json(ApiErrorResponse {
                error: "invalid_query",
                message: "use either 'before' or 'cursor', not both".to_string(),
            }),
        ));
    }

    let before = match query.before.as_deref() {
        Some(raw) => Some(
            chrono::DateTime::parse_from_rfc3339(raw)
                .map(|value| value.with_timezone(&Utc))
                .map_err(|_| {
                    (
                        StatusCode::BAD_REQUEST,
                        HeaderMap::new(),
                        Json(ApiErrorResponse {
                            error: "invalid_query",
                            message: "invalid 'before' cursor; expected RFC3339 timestamp"
                                .to_string(),
                        }),
                    )
                })?,
        ),
        None => None,
    };

    let cursor = match query.cursor.as_deref() {
        Some(raw) => Some(parse_events_cursor(raw).map_err(|message| {
            (
                StatusCode::BAD_REQUEST,
                HeaderMap::new(),
                Json(ApiErrorResponse {
                    error: "invalid_query",
                    message,
                }),
            )
        })?),
        None => None,
    };

    let kinds = parse_job_event_kinds(query.kinds.as_deref()).map_err(|message| {
        (
            StatusCode::BAD_REQUEST,
            HeaderMap::new(),
            Json(ApiErrorResponse {
                error: "invalid_query",
                message,
            }),
        )
    })?;

    let dispatch = state.dispatch.lock().await;

    let event_records = if cursor.is_some() {
        dispatch.job_events_cursor_filtered(job_id, limit, cursor, kinds.as_deref())
    } else {
        dispatch.job_events_before_filtered(job_id, limit, before, kinds.as_deref())
    };

    let next_before_cursor = if event_records.len() >= limit.max(1) {
        event_records.last().map(|event| event.occurred_at.to_rfc3339())
    } else {
        None
    };

    let next_cursor = if event_records.len() >= limit.max(1) {
        event_records.last().map(encode_events_cursor)
    } else {
        None
    };

    let events = event_records
        .into_iter()
        .map(map_job_event)
        .collect::<Vec<_>>();

    Ok(Json(JobEventsResponse {
        job_id,
        events,
        next_before_cursor,
        next_cursor,
    }))
}

async fn driver_ws(
    State(state): State<ApiState>,
    Path(driver_id): Path<Uuid>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    if !is_driver_authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    if !allow_driver_ws_connect(&state, driver_id).await {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }

    ws.on_upgrade(move |socket: WebSocket| async move {
        handle_driver_session(state, driver_id, socket).await;
    })
    .into_response()
}

async fn check_dispatch_request(
    state: &ApiState,
    headers: &HeaderMap,
    endpoint: &str,
) -> RateLimitSnapshot {
    let actor = dispatch_actor_key(headers);
    let key = format!("{}:{}", endpoint, actor);
    let mut limiter = state.dispatch_rate_limiter.lock().await;
    limiter.check(key)
}

fn rate_limited_json_response(
    snapshot: RateLimitSnapshot,
) -> (StatusCode, HeaderMap, Json<ApiErrorResponse>) {
    (
        StatusCode::TOO_MANY_REQUESTS,
        rate_limit_headers(snapshot),
        Json(ApiErrorResponse {
            error: "rate_limited",
            message: "dispatch request rate limit exceeded".to_string(),
        }),
    )
}

fn rate_limited_status_response(snapshot: RateLimitSnapshot) -> Response {
    let mut response = StatusCode::TOO_MANY_REQUESTS.into_response();
    apply_rate_limit_headers(response.headers_mut(), snapshot);
    response
}

fn rate_limit_headers(snapshot: RateLimitSnapshot) -> HeaderMap {
    let mut headers = HeaderMap::new();
    apply_rate_limit_headers(&mut headers, snapshot);
    headers
}

fn apply_rate_limit_headers(headers: &mut HeaderMap, snapshot: RateLimitSnapshot) {
    if let Ok(value) = HeaderValue::from_str(&snapshot.limit.to_string()) {
        headers.insert("x-ratelimit-limit", value);
    }
    if let Ok(value) = HeaderValue::from_str(&snapshot.remaining.to_string()) {
        headers.insert("x-ratelimit-remaining", value);
    }
    if let Ok(value) = HeaderValue::from_str(&snapshot.reset_seconds.to_string()) {
        headers.insert("x-ratelimit-reset", value);
    }
    if !snapshot.allowed {
        if let Ok(value) = HeaderValue::from_str(&snapshot.retry_after_seconds.to_string()) {
            headers.insert(axum::http::header::RETRY_AFTER, value);
        }
    }
}

fn dispatch_actor_key(headers: &HeaderMap) -> String {
    if let Some(client_id) = headers
        .get("x-spatiad-client-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return format!("client:{}", client_id);
    }

    if headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .is_some()
    {
        return "auth_header".to_string();
    }

    if headers
        .get("x-spatiad-dispatcher-token")
        .and_then(|value| value.to_str().ok())
        .is_some()
    {
        return "dispatcher_header".to_string();
    }

    "anonymous".to_string()
}

async fn allow_driver_ws_connect(state: &ApiState, driver_id: Uuid) -> bool {
    let mut guard = state.ws_reconnect_guard.lock().await;
    guard.allow(driver_id)
}

fn is_driver_authorized(state: &ApiState, headers: &HeaderMap) -> bool {
    let Some(expected) = &state.driver_token else {
        return true;
    };

    headers
        .get("x-spatiad-driver-token")
        .and_then(|value| value.to_str().ok())
        .map(|value| value == expected)
        .unwrap_or(false)
}

fn is_dispatcher_authorized(state: &ApiState, headers: &HeaderMap) -> bool {
    let Some(expected) = &state.dispatcher_token else {
        return true;
    };

    let auth_matches = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(|token| token == expected)
        .unwrap_or(false);

    if auth_matches {
        return true;
    }

    headers
        .get("x-spatiad-dispatcher-token")
        .and_then(|value| value.to_str().ok())
        .map(|token| token == expected)
        .unwrap_or(false)
}

async fn handle_driver_session(state: ApiState, driver_id: Uuid, mut socket: WebSocket) {
    let (session_tx, mut session_rx) = mpsc::unbounded_channel::<DriverOutbound>();
    {
        let mut sessions = state.sessions.lock().await;
        sessions.insert(driver_id, session_tx);
    }

    if replay_pending_offers(&state, driver_id, &mut socket).await.is_err() {
        let mut sessions = state.sessions.lock().await;
        sessions.remove(&driver_id);
        return;
    }

    let mut ticker = interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if flush_expired_offers(&state, driver_id, &mut socket)
                    .await
                    .is_err()
                {
                    break;
                }
            }
            outbound = session_rx.recv() => {
                let Some(outbound) = outbound else {
                    break;
                };
                if send_outbound(&mut socket, &outbound).await.is_err() {
                    break;
                }
            }
            inbound = socket.recv() => {
                let Some(message_result) = inbound else {
                    break;
                };

                let message = match message_result {
                    Ok(value) => value,
                    Err(_) => break,
                };

                if handle_driver_message(&state, driver_id, &mut socket, message)
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }
    }

    let mut sessions = state.sessions.lock().await;
    sessions.remove(&driver_id);
}

async fn replay_pending_offers(
    state: &ApiState,
    driver_id: Uuid,
    socket: &mut WebSocket,
) -> Result<(), ()> {
    flush_expired_offers(state, driver_id, socket).await?;

    let pending = {
        let dispatch = state.dispatch.lock().await;
        dispatch.pending_offers_for_driver(driver_id)
    };

    for offer in pending {
        let payload = DriverOutbound::Offer {
            offer_id: offer.offer_id,
            job_id: offer.job_id,
            pickup: offer.pickup,
            dropoff: offer.dropoff,
            expires_at: offer.expires_at,
        };

        send_outbound(socket, &payload).await?;
    }

    Ok(())
}

async fn flush_expired_offers(
    state: &ApiState,
    driver_id: Uuid,
    socket: &mut WebSocket,
) -> Result<(), ()> {
    let update = {
        let mut dispatch = state.dispatch.lock().await;
        dispatch.expire_pending_offers_for_driver(driver_id)
    };

    for item in update.expired {
        let payload = DriverOutbound::OfferExpired {
            offer_id: item.offer_id,
        };
        send_outbound(socket, &payload).await?;
    }

    notify_new_offers(state, update.new_offers).await;

    Ok(())
}

async fn handle_driver_message(
    state: &ApiState,
    driver_id: Uuid,
    socket: &mut WebSocket,
    message: Message,
) -> Result<(), ()> {
    match message {
        Message::Text(text) => {
            let inbound: DriverInbound = serde_json::from_str(&text).map_err(|_| ())?;
            match inbound {
                DriverInbound::Location {
                    category,
                    status,
                    latitude,
                    longitude,
                    timestamp: _,
                } => {
                    let mut dispatch = state.dispatch.lock().await;
                    // Preserve the driver's existing rating across location updates.
                    let rating = dispatch
                        .engine
                        .driver_snapshot(driver_id)
                        .map(|s| s.rating)
                        .unwrap_or(5.0);
                    dispatch.engine.upsert_driver_location(
                        driver_id,
                        category,
                        Coordinates { latitude, longitude },
                        status,
                        rating,
                    );
                    Ok(())
                }
                DriverInbound::OfferResponse { offer_id, accepted } => {
                    flush_expired_offers(state, driver_id, socket).await?;

                    let update = {
                        let mut dispatch = state.dispatch.lock().await;
                        dispatch
                            .handle_offer_response(offer_id, accepted)
                            .map_err(|_| ())?
                    };

                    notify_new_offers(state, update.new_offers).await;

                    if let Some(result) = update.matched {
                        let outbound = DriverOutbound::Matched {
                            offer_id: result.offer_id,
                            job_id: result.job_id,
                        };
                        send_outbound(socket, &outbound).await?;

                        notify_competing_cancellations(state, result.job_id).await;

                        if let Some(webhook_url) = &state.webhook_url {
                            let webhook_result = send_match_webhook(
                                webhook_url,
                                state.webhook_secret.as_deref(),
                                state.webhook_timeout_ms,
                                &result,
                            )
                            .await;

                            if webhook_result.is_err() {
                                let mut dispatch = state.dispatch.lock().await;
                                dispatch.record_webhook_delivery_failed(result.job_id, result.offer_id);
                            }
                        }
                    }

                    Ok(())
                }
            }
        }
        Message::Close(_) => Err(()),
        Message::Ping(payload) => {
            socket.send(Message::Pong(payload)).await.map_err(|_| ())
        }
        Message::Binary(_) | Message::Pong(_) => Ok(()),
    }
}

async fn send_outbound(socket: &mut WebSocket, payload: &DriverOutbound) -> Result<(), ()> {
    let text = serde_json::to_string(payload).map_err(|_| ())?;
    socket.send(Message::Text(text)).await.map_err(|_| ())
}

async fn notify_competing_cancellations(state: &ApiState, job_id: Uuid) {
    let cancelled = {
        let dispatch = state.dispatch.lock().await;
        dispatch.cancelled_offers_for_job(job_id)
    };

    if cancelled.is_empty() {
        return;
    }

    let sessions = state.sessions.lock().await;
    for item in cancelled {
        if let Some(tx) = sessions.get(&item.driver_id) {
            let _ = tx.send(DriverOutbound::OfferCancelled {
                offer_id: item.offer_id,
                job_id,
            });
        }
    }
}

async fn notify_new_offers(state: &ApiState, offers: Vec<spatiad_types::OfferRecord>) {
    if offers.is_empty() {
        return;
    }

    for offer in offers {
        let outbound = {
            let dispatch = state.dispatch.lock().await;
            dispatch
                .pending_offers_for_driver(offer.driver_id)
                .into_iter()
                .find(|pending| pending.offer_id == offer.offer_id)
                .map(|pending| DriverOutbound::Offer {
                    offer_id: pending.offer_id,
                    job_id: pending.job_id,
                    pickup: pending.pickup,
                    dropoff: pending.dropoff,
                    expires_at: pending.expires_at,
                })
        };

        let Some(outbound) = outbound else {
            continue;
        };

        let sessions = state.sessions.lock().await;
        if let Some(tx) = sessions.get(&offer.driver_id) {
            let _ = tx.send(outbound);
        }
    }
}

pub fn start_background_tasks(state: ApiState) {
    // Offer expiration task (1s interval)
    let bg = state.clone();
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(1));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            ticker.tick().await;
            let update = {
                let mut dispatch = bg.dispatch.lock().await;
                dispatch.expire_pending_offers_global()
            };

            if update.expired.is_empty() && update.new_offers.is_empty() {
                continue;
            }

            let sessions = bg.sessions.lock().await;
            for item in &update.expired {
                if let Some(tx) = sessions.get(&item.driver_id) {
                    let _ = tx.send(DriverOutbound::OfferExpired {
                        offer_id: item.offer_id,
                    });
                }
            }
            drop(sessions);

            notify_new_offers(&bg, update.new_offers).await;
        }
    });

    // Driver TTL expiration task (10s interval)
    if let Some(ttl_secs) = state.driver_ttl_secs {
        let bg = state;
        tokio::spawn(async move {
            let ttl = chrono::Duration::seconds(ttl_secs as i64);
            let mut ticker = interval(Duration::from_secs(10));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                ticker.tick().await;
                let removed = {
                    let mut dispatch = bg.dispatch.lock().await;
                    dispatch.engine.expire_stale_drivers(ttl)
                };
                if !removed.is_empty() {
                    tracing::info!(count = removed.len(), "expired stale drivers");
                }
            }
        });
    }
}

#[derive(Debug, Serialize)]
struct MatchWebhookPayload {
    event: &'static str,
    job_id: Uuid,
    driver_id: Uuid,
    offer_id: Uuid,
    matched_at: chrono::DateTime<Utc>,
}

async fn send_match_webhook(
    webhook_url: &str,
    webhook_secret: Option<&str>,
    webhook_timeout_ms: u64,
    result: &MatchResult,
) -> Result<(), ()> {
    let payload = MatchWebhookPayload {
        event: "trip_matched",
        job_id: result.job_id,
        driver_id: result.driver_id,
        offer_id: result.offer_id,
        matched_at: result.matched_at,
    };

    let body = serde_json::to_string(&payload).map_err(|_| ())?;
    let timestamp = Utc::now().timestamp().to_string();
    let nonce = Uuid::new_v4().to_string();

    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(webhook_timeout_ms.max(100)))
        .build()
        .map_err(|_| ())?;
    let mut backoff = Duration::from_millis(200);

    for attempt in 1..=3 {
        let mut request = client
            .post(webhook_url)
            .header("content-type", "application/json")
            .header("x-spatiad-timestamp", &timestamp)
            .header("x-spatiad-nonce", &nonce)
            .body(body.clone());

        if let Some(secret) = webhook_secret {
            let signature = sign_webhook(secret, &timestamp, &nonce, &body).map_err(|_| ())?;
            request = request.header("x-spatiad-signature", signature);
        }

        let response = request.send().await;

        match response {
            Ok(resp) if resp.status().is_success() => return Ok(()),
            _ if attempt < 3 => {
                sleep(backoff).await;
                backoff *= 2;
            }
            _ => return Err(()),
        }
    }

    Err(())
}

type HmacSha256 = Hmac<Sha256>;

fn sign_webhook(secret: &str, timestamp: &str, nonce: &str, body: &str) -> Result<String, ()> {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).map_err(|_| ())?;
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(nonce.as_bytes());
    mac.update(b".");
    mac.update(body.as_bytes());
    let bytes = mac.finalize().into_bytes();
    Ok(hex::encode(bytes))
}

fn map_job_event(event: JobEventRecord) -> JobEventResponse {
    let at = event.occurred_at;

    match event.kind {
        JobEventKind::JobRegistered => JobEventResponse {
            at,
            kind: "job_registered",
            offer_id: None,
            driver_id: None,
            status: None,
        },
        JobEventKind::JobCancelled => JobEventResponse {
            at,
            kind: "job_cancelled",
            offer_id: None,
            driver_id: None,
            status: Some("cancelled"),
        },
        JobEventKind::WebhookDeliveryFailed { offer_id } => JobEventResponse {
            at,
            kind: "webhook_delivery_failed",
            offer_id: Some(offer_id),
            driver_id: None,
            status: Some("webhook_delivery_failed"),
        },
        JobEventKind::OfferCreated { offer_id, driver_id } => JobEventResponse {
            at,
            kind: "offer_created",
            offer_id: Some(offer_id),
            driver_id: Some(driver_id),
            status: Some("pending"),
        },
        JobEventKind::OfferExpired { offer_id, driver_id } => JobEventResponse {
            at,
            kind: "offer_expired",
            offer_id: Some(offer_id),
            driver_id: Some(driver_id),
            status: Some("expired"),
        },
        JobEventKind::OfferCancelled { offer_id, driver_id } => JobEventResponse {
            at,
            kind: "offer_cancelled",
            offer_id: Some(offer_id),
            driver_id: Some(driver_id),
            status: Some("cancelled"),
        },
        JobEventKind::OfferRejected { offer_id, driver_id } => JobEventResponse {
            at,
            kind: "offer_rejected",
            offer_id: Some(offer_id),
            driver_id: Some(driver_id),
            status: Some("rejected"),
        },
        JobEventKind::OfferAccepted { offer_id, driver_id } => JobEventResponse {
            at,
            kind: "offer_accepted",
            offer_id: Some(offer_id),
            driver_id: Some(driver_id),
            status: Some("accepted"),
        },
        JobEventKind::MatchConfirmed { offer_id, driver_id } => JobEventResponse {
            at,
            kind: "match_confirmed",
            offer_id: Some(offer_id),
            driver_id: Some(driver_id),
            status: Some("matched"),
        },
        JobEventKind::OfferStatusUpdated { offer_id, status } => JobEventResponse {
            at,
            kind: "offer_status_updated",
            offer_id: Some(offer_id),
            driver_id: None,
            status: Some(offer_status_label(status)),
        },
    }
}

fn offer_status_label(status: OfferStatus) -> &'static str {
    match status {
        OfferStatus::Pending => "pending",
        OfferStatus::Accepted => "accepted",
        OfferStatus::Rejected => "rejected",
        OfferStatus::Expired => "expired",
        OfferStatus::Cancelled => "cancelled",
    }
}

fn encode_events_cursor(event: &JobEventRecord) -> String {
    format!("{}|{}", event.occurred_at.to_rfc3339(), event.sequence)
}

fn parse_events_cursor(raw: &str) -> Result<JobEventsCursor, String> {
    let (timestamp, sequence) = raw.rsplit_once('|').ok_or_else(|| {
        "invalid 'cursor' format; expected '<rfc3339>|<sequence>'".to_string()
    })?;

    let occurred_at = chrono::DateTime::parse_from_rfc3339(timestamp)
        .map_err(|_| "invalid cursor timestamp; expected RFC3339".to_string())?
        .with_timezone(&Utc);

    let sequence = sequence
        .parse::<u64>()
        .map_err(|_| "invalid cursor sequence; expected unsigned integer".to_string())?;

    Ok(JobEventsCursor {
        occurred_at,
        sequence,
    })
}

fn parse_job_event_kinds(raw: Option<&str>) -> Result<Option<Vec<JobEventFilterKind>>, String> {
    let Some(raw) = raw else {
        return Ok(None);
    };

    let mut parsed = Vec::new();
    for part in raw.split(',') {
        let token = part.trim();
        if token.is_empty() {
            continue;
        }

        let kind = match token {
            "job_registered" => JobEventFilterKind::JobRegistered,
            "job_cancelled" => JobEventFilterKind::JobCancelled,
            "webhook_delivery_failed" => JobEventFilterKind::WebhookDeliveryFailed,
            "offer_created" => JobEventFilterKind::OfferCreated,
            "offer_expired" => JobEventFilterKind::OfferExpired,
            "offer_cancelled" => JobEventFilterKind::OfferCancelled,
            "offer_rejected" => JobEventFilterKind::OfferRejected,
            "offer_accepted" => JobEventFilterKind::OfferAccepted,
            "match_confirmed" => JobEventFilterKind::MatchConfirmed,
            "offer_status_updated" => JobEventFilterKind::OfferStatusUpdated,
            other => {
                return Err(format!(
                    "unsupported event kind '{}' in 'kinds' query parameter",
                    other
                ));
            }
        };

        if !parsed.contains(&kind) {
            parsed.push(kind);
        }
    }

    if parsed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(parsed))
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc, thread, time::Duration as StdDuration};

    use axum::extract::{Path, Query, State};
    use axum::http::{HeaderMap, HeaderValue};
    use chrono::Utc;
    use spatiad_core::Engine;
    use spatiad_dispatch::DispatchService;
    use spatiad_types::{Coordinates, DriverStatus, JobRequest};
    use tokio::sync::Mutex;
    use uuid::Uuid;

    use super::*;

    #[test]
    fn webhook_signature_is_deterministic_for_same_input() {
        let signature_a = sign_webhook("secret", "1710000000", "nonce-1", "{\"k\":1}")
            .expect("signature should be generated");
        let signature_b = sign_webhook("secret", "1710000000", "nonce-1", "{\"k\":1}")
            .expect("signature should be generated");

        assert_eq!(signature_a, signature_b);
        assert_eq!(signature_a.len(), 64);
    }

    #[tokio::test]
    async fn dispatch_job_events_applies_kinds_and_before_cursor() {
        let job_id = Uuid::new_v4();
        let driver_id = Uuid::new_v4();
        let state = seeded_api_state(job_id, driver_id);

        let first_page = dispatch_job_events(
            State(state.clone()),
            HeaderMap::new(),
            Path(job_id),
            Query(JobEventsQuery {
                limit: Some(1),
                before: None,
                cursor: None,
                kinds: Some("offer_created,offer_rejected".to_string()),
            }),
        )
        .await
        .expect("first page should succeed")
        .0;

        assert_eq!(first_page.events.len(), 1);
        assert_eq!(first_page.events[0].kind, "offer_rejected");
        assert!(first_page.next_cursor.is_some());

        let cursor = first_page.next_cursor.expect("first page should expose cursor");

        let second_page = dispatch_job_events(
            State(state),
            HeaderMap::new(),
            Path(job_id),
            Query(JobEventsQuery {
                limit: Some(1),
                before: None,
                cursor: Some(cursor),
                kinds: Some("offer_created,offer_rejected".to_string()),
            }),
        )
        .await
        .expect("second page should succeed")
        .0;

        assert_eq!(second_page.events.len(), 1);
        assert_eq!(second_page.events[0].kind, "offer_created");
    }

    #[tokio::test]
    async fn dispatch_job_events_rejects_unsupported_kind() {
        let job_id = Uuid::new_v4();
        let driver_id = Uuid::new_v4();
        let state = seeded_api_state(job_id, driver_id);

        let error = dispatch_job_events(
            State(state),
            HeaderMap::new(),
            Path(job_id),
            Query(JobEventsQuery {
                limit: Some(10),
                before: None,
                cursor: None,
                kinds: Some("offer_created,unknown_event".to_string()),
            }),
        )
        .await
        .expect_err("invalid kind should fail");

        assert_eq!(error.0, StatusCode::BAD_REQUEST);
        assert_eq!(error.2.error, "invalid_query");
        assert!(error
            .2
            .message
            .contains("unsupported event kind 'unknown_event'"));
    }

    #[tokio::test]
    async fn dispatch_job_events_rejects_invalid_before_cursor() {
        let job_id = Uuid::new_v4();
        let driver_id = Uuid::new_v4();
        let state = seeded_api_state(job_id, driver_id);

        let error = dispatch_job_events(
            State(state),
            HeaderMap::new(),
            Path(job_id),
            Query(JobEventsQuery {
                limit: Some(10),
                before: Some("invalid-timestamp".to_string()),
                cursor: None,
                kinds: None,
            }),
        )
        .await
        .expect_err("invalid cursor should fail");

        assert_eq!(error.0, StatusCode::BAD_REQUEST);
        assert_eq!(error.2.error, "invalid_query");
        assert!(error
            .2
            .message
            .contains("invalid 'before' cursor"));
    }

    #[tokio::test]
    async fn dispatch_job_events_rejects_invalid_cursor_format() {
        let job_id = Uuid::new_v4();
        let driver_id = Uuid::new_v4();
        let state = seeded_api_state(job_id, driver_id);

        let error = dispatch_job_events(
            State(state),
            HeaderMap::new(),
            Path(job_id),
            Query(JobEventsQuery {
                limit: Some(10),
                before: None,
                cursor: Some("bad-cursor".to_string()),
                kinds: None,
            }),
        )
        .await
        .expect_err("invalid cursor should fail");

        assert_eq!(error.0, StatusCode::BAD_REQUEST);
        assert_eq!(error.2.error, "invalid_query");
        assert!(error.2.message.contains("invalid 'cursor' format"));
    }

    #[tokio::test]
    async fn dispatch_job_events_rejects_before_and_cursor_together() {
        let job_id = Uuid::new_v4();
        let driver_id = Uuid::new_v4();
        let state = seeded_api_state(job_id, driver_id);

        let error = dispatch_job_events(
            State(state),
            HeaderMap::new(),
            Path(job_id),
            Query(JobEventsQuery {
                limit: Some(10),
                before: Some("2026-03-20T10:00:00Z".to_string()),
                cursor: Some("2026-03-20T09:59:59Z|42".to_string()),
                kinds: None,
            }),
        )
        .await
        .expect_err("before+cursor should fail");

        assert_eq!(error.0, StatusCode::BAD_REQUEST);
        assert_eq!(error.2.error, "invalid_query");
        assert!(error
            .2
            .message
            .contains("either 'before' or 'cursor'"));
    }

    #[tokio::test]
    async fn dispatch_job_events_requires_dispatcher_token_when_configured() {
        let job_id = Uuid::new_v4();
        let driver_id = Uuid::new_v4();
        let mut state = seeded_api_state(job_id, driver_id);
        state.dispatcher_token = Some("secret-token".to_string());

        let error = dispatch_job_events(
            State(state),
            HeaderMap::new(),
            Path(job_id),
            Query(JobEventsQuery {
                limit: Some(10),
                before: None,
                cursor: None,
                kinds: None,
            }),
        )
        .await
        .expect_err("missing token should fail");

        assert_eq!(error.0, StatusCode::UNAUTHORIZED);
        assert_eq!(error.2.error, "unauthorized");
    }

    #[tokio::test]
    async fn dispatch_job_events_accepts_valid_bearer_token() {
        let job_id = Uuid::new_v4();
        let driver_id = Uuid::new_v4();
        let mut state = seeded_api_state(job_id, driver_id);
        state.dispatcher_token = Some("secret-token".to_string());

        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer secret-token"),
        );

        let response = dispatch_job_events(
            State(state),
            headers,
            Path(job_id),
            Query(JobEventsQuery {
                limit: Some(10),
                before: None,
                cursor: None,
                kinds: None,
            }),
        )
        .await
        .expect("valid bearer token should pass")
        .0;

        assert!(!response.events.is_empty());
    }

    #[tokio::test]
    async fn cancel_job_returns_not_found_for_unknown_job() {
        let state = seeded_api_state(Uuid::new_v4(), Uuid::new_v4());
        let status = cancel_job(
            State(state),
            HeaderMap::new(),
            Json(JobCancelRequest {
                job_id: Uuid::new_v4(),
            }),
        )
        .await
        .into_response()
        .status();

        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn cancel_job_transitions_status_to_cancelled() {
        let job_id = Uuid::new_v4();
        let driver_id = Uuid::new_v4();
        let state = seeded_api_state(job_id, driver_id);

        let status = cancel_job(
            State(state.clone()),
            HeaderMap::new(),
            Json(JobCancelRequest { job_id }),
        )
        .await
        .into_response()
        .status();

        assert_eq!(status, StatusCode::OK);

        let dispatch = state.dispatch.lock().await;
        assert!(matches!(
            dispatch.job_dispatch_state(job_id),
            JobDispatchState::Cancelled
        ));
    }

    #[tokio::test]
    async fn dispatch_job_events_returns_429_when_rate_limited() {
        let job_id = Uuid::new_v4();
        let driver_id = Uuid::new_v4();
        let mut state = seeded_api_state(job_id, driver_id);
        state.dispatch_rate_limiter = Arc::new(Mutex::new(SlidingWindowRateLimiter::new(1, 60)));

        let first = dispatch_job_events(
            State(state.clone()),
            HeaderMap::new(),
            Path(job_id),
            Query(JobEventsQuery {
                limit: Some(10),
                before: None,
                cursor: None,
                kinds: None,
            }),
        )
        .await;
        assert!(first.is_ok());

        let second = dispatch_job_events(
            State(state),
            HeaderMap::new(),
            Path(job_id),
            Query(JobEventsQuery {
                limit: Some(10),
                before: None,
                cursor: None,
                kinds: None,
            }),
        )
        .await
        .expect_err("second request should be rate limited");

        assert_eq!(second.0, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(second.2.error, "rate_limited");
        assert!(second.1.get("x-ratelimit-limit").is_some());
        assert!(second.1.get("x-ratelimit-remaining").is_some());
        assert!(second.1.get(axum::http::header::RETRY_AFTER).is_some());
    }

    #[tokio::test]
    async fn ws_reconnect_guard_blocks_storm_attempts() {
        let job_id = Uuid::new_v4();
        let driver_id = Uuid::new_v4();
        let mut state = seeded_api_state(job_id, driver_id);
        state.ws_reconnect_guard = Arc::new(Mutex::new(WsReconnectGuard::new(1, 60)));

        assert!(allow_driver_ws_connect(&state, driver_id).await);
        assert!(!allow_driver_ws_connect(&state, driver_id).await);
    }

    fn seeded_api_state(job_id: Uuid, driver_id: Uuid) -> ApiState {
        let mut engine = Engine::new(8);

        engine.upsert_driver_location(
            driver_id,
            "tow_truck".to_string(),
            Coordinates {
                latitude: 38.433,
                longitude: 26.768,
            },
            DriverStatus::Available,
            5.0,
        );

        engine.register_job(JobRequest {
            job_id,
            category: "tow_truck".to_string(),
            pickup: Coordinates {
                latitude: 38.433,
                longitude: 26.768,
            },
            dropoff: None,
            initial_radius_km: 1.0,
            max_radius_km: 5.0,
            timeout_seconds: 30,
            created_at: Utc::now(),
        });

        let offer = engine.create_offer(job_id, driver_id, 30);
        thread::sleep(StdDuration::from_millis(2));
        let _ = engine.handle_offer_response(offer.offer_id, false);

        ApiState {
            dispatch: Arc::new(Mutex::new(DispatchService::new(engine))),
            webhook_url: None,
            webhook_secret: None,
            webhook_timeout_ms: 3_000,
            driver_token: None,
            dispatcher_token: None,
            driver_ttl_secs: None,
            dispatch_rate_limiter: Arc::new(Mutex::new(SlidingWindowRateLimiter::new(1000, 60))),
            ws_reconnect_guard: Arc::new(Mutex::new(WsReconnectGuard::new(1000, 60))),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}
