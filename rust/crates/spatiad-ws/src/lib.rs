use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use spatiad_types::{Coordinates, DriverStatus};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DriverInbound {
    Location {
        category: String,
        status: DriverStatus,
        latitude: f64,
        longitude: f64,
        timestamp: i64,
    },
    OfferResponse {
        offer_id: Uuid,
        accepted: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DriverOutbound {
    Offer {
        offer_id: Uuid,
        job_id: Uuid,
        pickup: Coordinates,
        dropoff: Option<Coordinates>,
        expires_at: DateTime<Utc>,
    },
    OfferExpired {
        offer_id: Uuid,
    },
    OfferCancelled {
        offer_id: Uuid,
        job_id: Uuid,
    },
    Matched {
        offer_id: Uuid,
        job_id: Uuid,
    },
}
