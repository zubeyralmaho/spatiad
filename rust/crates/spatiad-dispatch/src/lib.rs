use spatiad_core::Engine;
use spatiad_types::{JobRequest, OfferRecord};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error("no available driver in current cell")]
    NoAvailableDriver,
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

        let candidates = self
            .engine
            .nearest_candidates(job.pickup, &job.category, 3);

        let Some(driver_id) = candidates.into_iter().next() else {
            return Err(DispatchError::NoAvailableDriver);
        };

        Ok(self
            .engine
            .create_offer(job.job_id, driver_id, job.timeout_seconds))
    }

    pub fn cancel_offer(&mut self, offer_id: Uuid) {
        let _ = self
            .engine
            .mark_offer_status(offer_id, spatiad_types::OfferStatus::Cancelled);
    }
}
