use std::collections::HashMap;

use chrono::Utc;
use spatiad_h3::SpatialIndex;
use spatiad_types::{Coordinates, DriverSnapshot, DriverStatus, JobRequest, OfferRecord, OfferStatus};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("driver not found")]
    DriverNotFound,
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

    pub fn nearest_candidates(&self, pickup: Coordinates, category: &str, limit: usize) -> Vec<Uuid> {
        self.spatial
            .candidates_in_same_cell(pickup)
            .into_iter()
            .filter(|driver_id| {
                self.drivers
                    .get(driver_id)
                    .map(|driver| {
                        driver.status == DriverStatus::Available && driver.category.eq_ignore_ascii_case(category)
                    })
                    .unwrap_or(false)
            })
            .take(limit)
            .collect()
    }

    pub fn mark_offer_status(&mut self, offer_id: Uuid, status: OfferStatus) -> Result<(), CoreError> {
        let offer = self.offers.get_mut(&offer_id).ok_or(CoreError::DriverNotFound)?;
        offer.status = status;
        Ok(())
    }
}
