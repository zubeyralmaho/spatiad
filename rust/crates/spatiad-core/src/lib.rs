use std::collections::HashMap;

use chrono::Utc;
use spatiad_h3::SpatialIndex;
use spatiad_types::{Coordinates, DriverSnapshot, DriverStatus, JobRequest, OfferRecord, OfferStatus};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("offer not found")]
    OfferNotFound,
}

#[derive(Debug)]
pub struct Engine {
    spatial: SpatialIndex,
    drivers: HashMap<Uuid, DriverSnapshot>,
    jobs: HashMap<Uuid, JobRequest>,
    offers: HashMap<Uuid, OfferRecord>,
}

impl Engine {
    pub fn new(h3_resolution: u8) -> Self {
        Self {
            spatial: SpatialIndex::new(h3_resolution),
            drivers: HashMap::new(),
            jobs: HashMap::new(),
            offers: HashMap::new(),
        }
    }

    pub fn upsert_driver_location(
        &mut self,
        driver_id: Uuid,
        category: String,
        position: Coordinates,
        status: DriverStatus,
    ) {
        let snapshot = DriverSnapshot {
            driver_id,
            category,
            status,
            position,
            last_seen_at: Utc::now(),
        };

        self.spatial.upsert_driver(driver_id, position);
        self.drivers.insert(driver_id, snapshot);
    }

    pub fn register_job(&mut self, job: JobRequest) {
        self.jobs.insert(job.job_id, job);
    }

    pub fn create_offer(&mut self, job_id: Uuid, driver_id: Uuid, timeout_seconds: u64) -> OfferRecord {
        let expires_at = Utc::now() + chrono::Duration::seconds(timeout_seconds as i64);
        let offer = OfferRecord {
            offer_id: Uuid::new_v4(),
            job_id,
            driver_id,
            status: OfferStatus::Pending,
            expires_at,
        };

        self.offers.insert(offer.offer_id, offer.clone());
        offer
    }

    pub fn nearest_candidates_in_radius(
        &self,
        pickup: Coordinates,
        category: &str,
        radius_km: f64,
        limit: usize,
    ) -> Vec<Uuid> {
        let mut candidates: Vec<(Uuid, f64)> = self
            .drivers
            .iter()
            .filter_map(|(driver_id, driver)| {
                if driver.status != DriverStatus::Available || !driver.category.eq_ignore_ascii_case(category) {
                    return None;
                }

                let distance_km = haversine_km(pickup, driver.position);
                if distance_km <= radius_km {
                    Some((*driver_id, distance_km))
                } else {
                    None
                }
            })
            .collect();

        candidates.sort_by(|a, b| a.1.total_cmp(&b.1));
        candidates.into_iter().take(limit).map(|(id, _)| id).collect()
    }

    pub fn mark_offer_status(&mut self, offer_id: Uuid, status: OfferStatus) -> Result<(), CoreError> {
        let offer = self.offers.get_mut(&offer_id).ok_or(CoreError::OfferNotFound)?;
        offer.status = status;
        Ok(())
    }
}

fn haversine_km(a: Coordinates, b: Coordinates) -> f64 {
    let earth_radius_km = 6371.0_f64;

    let a_lat = a.latitude.to_radians();
    let a_lon = a.longitude.to_radians();
    let b_lat = b.latitude.to_radians();
    let b_lon = b.longitude.to_radians();

    let d_lat = b_lat - a_lat;
    let d_lon = b_lon - a_lon;

    let s = (d_lat / 2.0).sin().powi(2)
        + a_lat.cos() * b_lat.cos() * (d_lon / 2.0).sin().powi(2);

    2.0 * earth_radius_km * s.sqrt().asin()
}
