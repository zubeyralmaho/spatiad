use std::collections::HashSet;

use spatiad_core::Engine;
use spatiad_core::CancelledDriverOffer;
use spatiad_core::ExpiredOffer;
use spatiad_core::JobDispatchState;
use spatiad_core::JobEventsCursor;
use spatiad_core::JobEventFilterKind;
use spatiad_core::JobEventRecord;
use spatiad_core::PendingDriverOffer;
use spatiad_types::{JobRequest, MatchResult, OfferRecord};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error("no available driver in search radius")]
    NoAvailableDriver,
    #[error("invalid offer response")]
    InvalidOfferResponse,
}

#[derive(Debug)]
pub struct DispatchService {
    pub engine: Engine,
}

#[derive(Debug, Default)]
pub struct OfferResponseUpdate {
    pub matched: Option<MatchResult>,
    pub new_offers: Vec<OfferRecord>,
}

#[derive(Debug, Default)]
pub struct ExpirationUpdate {
    pub expired: Vec<ExpiredOffer>,
    pub new_offers: Vec<OfferRecord>,
}

impl DispatchService {
    pub fn new(engine: Engine) -> Self {
        Self { engine }
    }

    pub fn submit_job(&mut self, job: JobRequest) -> Result<OfferRecord, DispatchError> {
        self.engine.register_job(job.clone());

        let mut radius_km = job.initial_radius_km.max(0.1);
        let max_radius_km = job.max_radius_km.max(radius_km);

        while radius_km <= max_radius_km + f64::EPSILON {
            let candidates = self
                .engine
                .nearest_candidates_in_radius(job.pickup, &job.category, radius_km, 3);

            if let Some(driver_id) = candidates.into_iter().next() {
                return Ok(self
                    .engine
                    .create_offer(job.job_id, driver_id, job.timeout_seconds));
            }

            radius_km = expand_radius_km(radius_km, max_radius_km);
            if radius_km > max_radius_km {
                break;
            }
        }

        Err(DispatchError::NoAvailableDriver)
    }

    pub fn cancel_offer(&mut self, offer_id: Uuid) {
        let _ = self
            .engine
            .mark_offer_status(offer_id, spatiad_types::OfferStatus::Cancelled);
    }

    pub fn pending_offers_for_driver(&self, driver_id: Uuid) -> Vec<PendingDriverOffer> {
        self.engine.pending_offers_for_driver(driver_id)
    }

    pub fn expire_pending_offers_for_driver(&mut self, driver_id: Uuid) -> ExpirationUpdate {
        let expired = self.engine.expire_pending_offers_for_driver(driver_id);
        let mut seen_jobs = HashSet::new();
        let mut new_offers = Vec::new();

        for item in &expired {
            if seen_jobs.insert(item.job_id) {
                if let Some(offer) = self.engine.create_next_offer_for_job(item.job_id) {
                    new_offers.push(offer);
                }
            }
        }

        ExpirationUpdate { expired, new_offers }
    }

    pub fn expire_pending_offers_global(&mut self) -> ExpirationUpdate {
        let expired = self.engine.expire_pending_offers_global();
        let mut seen_jobs = HashSet::new();
        let mut new_offers = Vec::new();

        for item in &expired {
            if seen_jobs.insert(item.job_id) {
                if let Some(offer) = self.engine.create_next_offer_for_job(item.job_id) {
                    new_offers.push(offer);
                }
            }
        }

        ExpirationUpdate { expired, new_offers }
    }

    pub fn cancelled_offers_for_job(&self, job_id: Uuid) -> Vec<CancelledDriverOffer> {
        self.engine.cancelled_offers_for_job(job_id)
    }

    pub fn job_dispatch_state(&self, job_id: Uuid) -> JobDispatchState {
        self.engine.job_dispatch_state(job_id)
    }

    pub fn job_events(&self, job_id: Uuid, limit: usize) -> Vec<JobEventRecord> {
        self.engine.job_events(job_id, limit)
    }

    pub fn job_events_before(
        &self,
        job_id: Uuid,
        limit: usize,
        before: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Vec<JobEventRecord> {
        self.engine.job_events_before(job_id, limit, before)
    }

    pub fn job_events_before_filtered(
        &self,
        job_id: Uuid,
        limit: usize,
        before: Option<chrono::DateTime<chrono::Utc>>,
        kinds: Option<&[JobEventFilterKind]>,
    ) -> Vec<JobEventRecord> {
        self.engine
            .job_events_before_filtered(job_id, limit, before, kinds)
    }

    pub fn job_events_cursor_filtered(
        &self,
        job_id: Uuid,
        limit: usize,
        cursor: Option<JobEventsCursor>,
        kinds: Option<&[JobEventFilterKind]>,
    ) -> Vec<JobEventRecord> {
        self.engine
            .job_events_cursor_filtered(job_id, limit, cursor, kinds)
    }

    pub fn handle_offer_response(
        &mut self,
        offer_id: Uuid,
        accepted: bool,
    ) -> Result<OfferResponseUpdate, DispatchError> {
        let job_id = self
            .engine
            .offer_job_id(offer_id)
            .ok_or(DispatchError::InvalidOfferResponse)?;

        let matched = self
            .engine
            .handle_offer_response(offer_id, accepted)
            .map_err(|_| DispatchError::InvalidOfferResponse)?;

        let mut new_offers = Vec::new();
        if matched.is_none() {
            if let Some(offer) = self.engine.create_next_offer_for_job(job_id) {
                new_offers.push(offer);
            }
        }

        Ok(OfferResponseUpdate {
            matched,
            new_offers,
        })
    }
}

fn expand_radius_km(current_radius_km: f64, max_radius_km: f64) -> f64 {
    let next = current_radius_km + 2.0;
    next.min(max_radius_km + 1.0)
}

#[cfg(test)]
mod tests {
    use std::{thread, time::Duration as StdDuration};

    use chrono::Utc;
    use spatiad_types::{Coordinates, DriverStatus};

    use super::*;

    #[test]
    fn submit_job_expands_radius_until_driver_found() {
        let mut engine = Engine::new(8);
        let driver_id = Uuid::new_v4();
        engine.upsert_driver_location(
            driver_id,
            "tow_truck".to_string(),
            Coordinates {
                latitude: 38.443,
                longitude: 26.768,
            },
            DriverStatus::Available,
        );

        let mut dispatch = DispatchService::new(engine);
        let job = JobRequest {
            job_id: Uuid::new_v4(),
            category: "tow_truck".to_string(),
            pickup: Coordinates {
                latitude: 38.433,
                longitude: 26.768,
            },
            dropoff: None,
            initial_radius_km: 0.5,
            max_radius_km: 5.0,
            timeout_seconds: 20,
            created_at: Utc::now(),
        };

        let offer = dispatch.submit_job(job).expect("expected driver to be found after expansion");
        assert_eq!(offer.driver_id, driver_id);
    }

    #[test]
    fn rejected_offer_creates_next_offer_for_same_job() {
        let mut engine = Engine::new(8);
        let driver_a = Uuid::new_v4();
        let driver_b = Uuid::new_v4();

        engine.upsert_driver_location(
            driver_a,
            "tow_truck".to_string(),
            Coordinates {
                latitude: 38.433,
                longitude: 26.768,
            },
            DriverStatus::Available,
        );
        engine.upsert_driver_location(
            driver_b,
            "tow_truck".to_string(),
            Coordinates {
                latitude: 38.434,
                longitude: 26.769,
            },
            DriverStatus::Available,
        );

        let mut dispatch = DispatchService::new(engine);
        let job = JobRequest {
            job_id: Uuid::new_v4(),
            category: "tow_truck".to_string(),
            pickup: Coordinates {
                latitude: 38.433,
                longitude: 26.768,
            },
            dropoff: None,
            initial_radius_km: 1.0,
            max_radius_km: 5.0,
            timeout_seconds: 20,
            created_at: Utc::now(),
        };

        let first_offer = dispatch.submit_job(job).expect("first offer expected");
        let update = dispatch
            .handle_offer_response(first_offer.offer_id, false)
            .expect("reject should be handled");

        assert!(update.matched.is_none());
        assert_eq!(update.new_offers.len(), 1);
        assert_ne!(update.new_offers[0].driver_id, first_offer.driver_id);
    }

    #[test]
    fn global_expiration_advances_job_to_next_driver() {
        let mut engine = Engine::new(8);
        let driver_a = Uuid::new_v4();
        let driver_b = Uuid::new_v4();

        engine.upsert_driver_location(
            driver_a,
            "tow_truck".to_string(),
            Coordinates {
                latitude: 38.433,
                longitude: 26.768,
            },
            DriverStatus::Available,
        );
        engine.upsert_driver_location(
            driver_b,
            "tow_truck".to_string(),
            Coordinates {
                latitude: 38.434,
                longitude: 26.769,
            },
            DriverStatus::Available,
        );

        let mut dispatch = DispatchService::new(engine);
        let job = JobRequest {
            job_id: Uuid::new_v4(),
            category: "tow_truck".to_string(),
            pickup: Coordinates {
                latitude: 38.433,
                longitude: 26.768,
            },
            dropoff: None,
            initial_radius_km: 1.0,
            max_radius_km: 5.0,
            timeout_seconds: 1,
            created_at: Utc::now(),
        };

        let first_offer = dispatch.submit_job(job).expect("first offer expected");
        thread::sleep(StdDuration::from_millis(1200));

        let update = dispatch.expire_pending_offers_global();
        assert_eq!(update.expired.len(), 1);
        assert_eq!(update.expired[0].offer_id, first_offer.offer_id);
        assert_eq!(update.new_offers.len(), 1);
        assert_ne!(update.new_offers[0].driver_id, first_offer.driver_id);
    }
}
