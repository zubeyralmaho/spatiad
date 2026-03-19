use std::sync::Arc;

use axum::{
    extract::{Path, State, WebSocketUpgrade},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use axum::extract::ws::WebSocket;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use spatiad_dispatch::DispatchService;
use spatiad_types::{Coordinates, JobRequest};
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Clone)]
pub struct ApiState {
    pub dispatch: Arc<Mutex<DispatchService>>,
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

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/health", get(health))
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

async fn cancel_offer(
    State(state): State<ApiState>,
    Json(payload): Json<OfferCancelRequest>,
) -> impl IntoResponse {
    let mut dispatch = state.dispatch.lock().await;
    dispatch.cancel_offer(payload.offer_id);
    axum::http::StatusCode::OK
}

async fn driver_ws(
    Path(_driver_id): Path<Uuid>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(|_socket: WebSocket| async move {
        // Placeholder for resilient WS session manager.
    })
}
