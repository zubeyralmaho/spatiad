use std::sync::Arc;

use axum::{
    extract::ws::Message,
    extract::{Path, State, WebSocketUpgrade},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use axum::extract::ws::WebSocket;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use spatiad_dispatch::DispatchService;
use spatiad_types::{Coordinates, DriverStatus, JobRequest, MatchResult};
use spatiad_ws::{DriverInbound, DriverOutbound};
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Clone)]
pub struct ApiState {
    pub dispatch: Arc<Mutex<DispatchService>>,
    pub webhook_url: Option<String>,
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

async fn driver_ws(
    State(state): State<ApiState>,
    Path(_driver_id): Path<Uuid>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket: WebSocket| async move {
        handle_driver_session(state, _driver_id, socket).await;
    })
}

async fn handle_driver_session(state: ApiState, driver_id: Uuid, mut socket: WebSocket) {
    if replay_pending_offers(&state, driver_id, &mut socket).await.is_err() {
        return;
    }

    loop {
        let Some(message_result) = socket.recv().await else {
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

        let text = serde_json::to_string(&payload).map_err(|_| ())?;
        socket.send(Message::Text(text)).await.map_err(|_| ())?;
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
        let text = serde_json::to_string(&payload).map_err(|_| ())?;
        socket.send(Message::Text(text)).await.map_err(|_| ())?;
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
                    latitude,
                    longitude,
                    timestamp: _,
                } => {
                    let mut dispatch = state.dispatch.lock().await;
                    dispatch.engine.upsert_driver_location(
                        driver_id,
                        "tow_truck".to_string(),
                        Coordinates { latitude, longitude },
                        DriverStatus::Available,
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
                        let text = serde_json::to_string(&outbound).map_err(|_| ())?;
                        socket.send(Message::Text(text)).await.map_err(|_| ())?;

                        if let Some(webhook_url) = &state.webhook_url {
                            let _ = send_match_webhook(webhook_url, &result).await;
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

#[derive(Debug, Serialize)]
struct MatchWebhookPayload {
    event: &'static str,
    job_id: Uuid,
    driver_id: Uuid,
    offer_id: Uuid,
    matched_at: chrono::DateTime<Utc>,
}

async fn send_match_webhook(webhook_url: &str, result: &MatchResult) -> Result<(), ()> {
    let payload = MatchWebhookPayload {
        event: "trip_matched",
        job_id: result.job_id,
        driver_id: result.driver_id,
        offer_id: result.offer_id,
        matched_at: result.matched_at,
    };

    let response = reqwest::Client::new()
        .post(webhook_url)
        .json(&payload)
        .send()
        .await
        .map_err(|_| ())?;

    if response.status().is_success() {
        Ok(())
    } else {
        Err(())
    }
}
