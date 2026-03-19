use std::{collections::HashMap, sync::Arc};

use axum::{
    extract::ws::Message,
    extract::{Path, State, WebSocketUpgrade},
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
use spatiad_core::JobDispatchState;
use spatiad_dispatch::DispatchService;
use spatiad_types::{Coordinates, DriverStatus, JobRequest, MatchResult};
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

#[cfg(test)]
mod tests {
    use super::sign_webhook;

    #[test]
    fn webhook_signature_is_deterministic_for_same_input() {
        let signature_a = sign_webhook("secret", "1710000000", "nonce-1", "{\"k\":1}")
            .expect("signature should be generated");
        let signature_b = sign_webhook("secret", "1710000000", "nonce-1", "{\"k\":1}")
            .expect("signature should be generated");

        assert_eq!(signature_a, signature_b);
        assert_eq!(signature_a.len(), 64);
    }
}
