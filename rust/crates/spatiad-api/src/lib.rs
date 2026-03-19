use std::{collections::HashMap, sync::Arc};

use axum::{
    extract::ws::Message,
    extract::{Path, Query, State, WebSocketUpgrade},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use axum::extract::ws::WebSocket;
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use spatiad_core::{JobDispatchState, JobEventFilterKind, JobEventKind, JobEventRecord};
use spatiad_dispatch::DispatchService;
use spatiad_types::{Coordinates, DriverStatus, JobRequest, MatchResult, OfferStatus};
use spatiad_ws::{DriverInbound, DriverOutbound};
use tokio::sync::{mpsc, Mutex};
use tokio::time::{interval, sleep, Duration};
use uuid::Uuid;

#[derive(Clone)]
pub struct ApiState {
    pub dispatch: Arc<Mutex<DispatchService>>,
    pub webhook_url: Option<String>,
    pub webhook_secret: Option<String>,
    pub driver_token: Option<String>,
    pub sessions: Arc<Mutex<HashMap<Uuid, mpsc::UnboundedSender<DriverOutbound>>>>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
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
    pub kinds: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct JobEventsResponse {
    pub job_id: Uuid,
    pub events: Vec<JobEventResponse>,
    pub next_before_cursor: Option<String>,
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

#[derive(Debug, Deserialize)]
pub struct DriverUpsertRequest {
    pub driver_id: Uuid,
    pub category: String,
    pub status: DriverStatus,
    pub position: Coordinates,
}

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/v1/driver/upsert", post(upsert_driver))
        .route("/api/v1/dispatch/offer", post(dispatch_offer))
        .route("/api/v1/dispatch/cancel", post(cancel_offer))
        .route("/api/v1/dispatch/job/:job_id", get(dispatch_job_status))
        .route("/api/v1/dispatch/job/:job_id/events", get(dispatch_job_events))
        .route("/api/v1/stream/driver/:driver_id", get(driver_ws))
        .with_state(state)
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "spatiad",
    })
}

async fn dispatch_offer(
    State(state): State<ApiState>,
    Json(payload): Json<OfferRequest>,
) -> impl IntoResponse {
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
    Json(payload): Json<DriverUpsertRequest>,
) -> impl IntoResponse {
    let mut dispatch = state.dispatch.lock().await;
    dispatch.engine.upsert_driver_location(
        payload.driver_id,
        payload.category,
        payload.position,
        payload.status,
    );

    axum::http::StatusCode::OK
}

async fn cancel_offer(
    State(state): State<ApiState>,
    Json(payload): Json<OfferCancelRequest>,
) -> impl IntoResponse {
    let mut dispatch = state.dispatch.lock().await;
    dispatch.cancel_offer(payload.offer_id);
    axum::http::StatusCode::OK
}

async fn dispatch_job_status(
    State(state): State<ApiState>,
    Path(job_id): Path<Uuid>,
) -> impl IntoResponse {
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

    Json(response)
}

async fn dispatch_job_events(
    State(state): State<ApiState>,
    Path(job_id): Path<Uuid>,
    Query(query): Query<JobEventsQuery>,
) -> Result<Json<JobEventsResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let limit = query.limit.unwrap_or(50);

    let before = match query.before.as_deref() {
        Some(raw) => Some(
            chrono::DateTime::parse_from_rfc3339(raw)
                .map(|value| value.with_timezone(&Utc))
                .map_err(|_| {
                    (
                        StatusCode::BAD_REQUEST,
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

    let kinds = parse_job_event_kinds(query.kinds.as_deref()).map_err(|message| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResponse {
                error: "invalid_query",
                message,
            }),
        )
    })?;

    let dispatch = state.dispatch.lock().await;

    let events = dispatch
        .job_events_before_filtered(job_id, limit, before, kinds.as_deref())
        .into_iter()
        .map(map_job_event)
        .collect::<Vec<_>>();

    let next_before_cursor = if events.len() >= limit.max(1) {
        events.last().map(|event| event.at.to_rfc3339())
    } else {
        None
    };

    Ok(Json(JobEventsResponse {
        job_id,
        events,
        next_before_cursor,
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

    ws.on_upgrade(move |socket: WebSocket| async move {
        handle_driver_session(state, driver_id, socket).await;
    })
    .into_response()
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
    let expired_offer_ids = {
        let mut dispatch = state.dispatch.lock().await;
        dispatch.expire_pending_offers_for_driver(driver_id)
    };

    for offer_id in expired_offer_ids {
        let payload = DriverOutbound::OfferExpired { offer_id };
        send_outbound(socket, &payload).await?;
    }

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
                    dispatch.engine.upsert_driver_location(
                        driver_id,
                        category,
                        Coordinates { latitude, longitude },
                        status,
                    );
                    Ok(())
                }
                DriverInbound::OfferResponse { offer_id, accepted } => {
                    flush_expired_offers(state, driver_id, socket).await?;

                    let match_result = {
                        let mut dispatch = state.dispatch.lock().await;
                        dispatch
                            .handle_offer_response(offer_id, accepted)
                            .map_err(|_| ())?
                    };

                    if let Some(result) = match_result {
                        let outbound = DriverOutbound::Matched {
                            offer_id: result.offer_id,
                            job_id: result.job_id,
                        };
                        send_outbound(socket, &outbound).await?;

                        notify_competing_cancellations(state, result.job_id).await;

                        if let Some(webhook_url) = &state.webhook_url {
                            let _ = send_match_webhook(
                                webhook_url,
                                state.webhook_secret.as_deref(),
                                &result,
                            )
                            .await;
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

    let client = reqwest::Client::new();
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
            Path(job_id),
            Query(JobEventsQuery {
                limit: Some(1),
                before: None,
                kinds: Some("offer_created,offer_rejected".to_string()),
            }),
        )
        .await
        .expect("first page should succeed")
        .0;

        assert_eq!(first_page.events.len(), 1);
        assert_eq!(first_page.events[0].kind, "offer_rejected");

        let before = first_page
            .next_before_cursor
            .expect("first page should expose cursor");

        let second_page = dispatch_job_events(
            State(state),
            Path(job_id),
            Query(JobEventsQuery {
                limit: Some(1),
                before: Some(before),
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
            Path(job_id),
            Query(JobEventsQuery {
                limit: Some(10),
                before: None,
                kinds: Some("offer_created,unknown_event".to_string()),
            }),
        )
        .await
        .expect_err("invalid kind should fail");

        assert_eq!(error.0, StatusCode::BAD_REQUEST);
        assert_eq!(error.1.error, "invalid_query");
        assert!(error
            .1
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
            Path(job_id),
            Query(JobEventsQuery {
                limit: Some(10),
                before: Some("invalid-timestamp".to_string()),
                kinds: None,
            }),
        )
        .await
        .expect_err("invalid cursor should fail");

        assert_eq!(error.0, StatusCode::BAD_REQUEST);
        assert_eq!(error.1.error, "invalid_query");
        assert!(error
            .1
            .message
            .contains("invalid 'before' cursor"));
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
            driver_token: None,
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}
