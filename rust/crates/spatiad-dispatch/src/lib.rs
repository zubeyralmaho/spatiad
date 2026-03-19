use spatiad_core::Engine;
use spatiad_core::CancelledDriverOffer;
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

    pub fn expire_pending_offers_for_driver(&mut self, driver_id: Uuid) -> Vec<Uuid> {
        self.engine.expire_pending_offers_for_driver(driver_id)
    }

    pub fn cancelled_offers_for_job(&self, job_id: Uuid) -> Vec<CancelledDriverOffer> {
        self.engine.cancelled_offers_for_job(job_id)
    }

    pub fn handle_offer_response(
        &mut self,
        offer_id: Uuid,
        accepted: bool,
    ) -> Result<Option<MatchResult>, DispatchError> {
        self.engine
            .handle_offer_response(offer_id, accepted)
            .map_err(|_| DispatchError::InvalidOfferResponse)
    }
}

fn expand_radius_km(current_radius_km: f64, max_radius_km: f64) -> f64 {
    let next = current_radius_km + 2.0;
    next.min(max_radius_km + 1.0)
}

#[cfg(test)]
mod tests {
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
}
