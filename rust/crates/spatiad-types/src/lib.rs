use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Coordinates {
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DriverStatus {
    Offline,
    Available,
    Busy,
}

fn default_driver_rating() -> f32 {
    5.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriverSnapshot {
    pub driver_id: Uuid,
    pub category: String,
    pub status: DriverStatus,
    pub position: Coordinates,
    pub last_seen_at: DateTime<Utc>,
    /// Driver rating on a 1.0–5.0 scale. Defaults to 5.0 when not provided.
    #[serde(default = "default_driver_rating")]
    pub rating: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRequest {
    pub job_id: Uuid,
    pub category: String,
    pub pickup: Coordinates,
    pub dropoff: Option<Coordinates>,
    pub initial_radius_km: f64,
    pub max_radius_km: f64,
    pub timeout_seconds: u64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum OfferStatus {
    Pending,
    Accepted,
    Rejected,
    Expired,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfferRecord {
    pub offer_id: Uuid,
    pub job_id: Uuid,
    pub driver_id: Uuid,
    pub status: OfferStatus,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchResult {
    pub job_id: Uuid,
    pub driver_id: Uuid,
    pub offer_id: Uuid,
    pub matched_at: DateTime<Utc>,
}
