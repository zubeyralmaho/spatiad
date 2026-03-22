use std::collections::HashSet;
use std::sync::{Arc, RwLock};

use spatiad_core::Engine;
use spatiad_core::CancelledDriverOffer;
use spatiad_core::ExpiredOffer;
use spatiad_core::JobDispatchState;
use spatiad_core::JobEventsCursor;
use spatiad_core::JobEventFilterKind;
use spatiad_core::JobEventRecord;
use spatiad_core::PendingDriverOffer;
use spatiad_types::{Coordinates, JobRequest, MatchResult, OfferRecord};
use spatiad_zones::{Point, ZoneCheckResult, ZoneRegistry};
use thiserror::Error;
use uuid::Uuid;

pub mod eta;
pub mod scoring;
pub use eta::{EtaProvider, OsrmEtaProvider, StraightLineEtaProvider};
pub use scoring::{ScoringConfig, ScoringWeights};

// Re-export zone types so callers don't need a direct spatiad-zones dependency.
pub use spatiad_zones::{DenialReason as ZoneDenialReason, Zone, ZoneType};

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error("no available driver in search radius")]
    NoAvailableDriver,
    #[error("invalid offer response")]
    InvalidOfferResponse,
    #[error("pickup denied by zone policy: {reason}")]
    ZoneDenied { reason: String },
}

#[derive(Debug)]
pub struct DispatchService {
    pub engine: Engine,
    pub scoring: ScoringConfig,
    /// Zone registry shared with the API layer (behind a read-write lock so
    /// zone updates don't block dispatch operations).
    pub zones: Arc<RwLock<ZoneRegistry>>,
    /// ETA provider used for travel-time estimates during scoring.
    pub eta_provider: Box<dyn EtaProvider>,
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
        Self {
            engine,
            scoring: ScoringConfig::default(),
            zones: Arc::new(RwLock::new(ZoneRegistry::new())),
            eta_provider: Box::new(StraightLineEtaProvider::default()),
        }
    }

    pub fn with_scoring(engine: Engine, scoring: ScoringConfig) -> Self {
        Self {
            engine,
            scoring,
            zones: Arc::new(RwLock::new(ZoneRegistry::new())),
            eta_provider: Box::new(StraightLineEtaProvider::default()),
        }
    }

    /// Replace the ETA provider (e.g. to use OSRM in production).
    pub fn set_eta_provider(&mut self, provider: Box<dyn EtaProvider>) {
        self.eta_provider = provider;
    }

    pub fn submit_job(&mut self, job: JobRequest) -> Result<OfferRecord, DispatchError> {
        // Zone check before registering the job.
        {
            let registry = self.zones.read().expect("zone lock poisoned");
            let pickup = Point {
                latitude: job.pickup.latitude,
                longitude: job.pickup.longitude,
            };
            match registry.check(pickup) {
                ZoneCheckResult::Denied { reason } => {
                    return Err(DispatchError::ZoneDenied {
                        reason: format!("{reason:?}"),
                    });
                }
                ZoneCheckResult::Allowed | ZoneCheckResult::Surge { .. } => {}
            }
        }

        self.engine.register_job(job.clone());

        let mut radius_km = job.initial_radius_km.max(0.1);
        let max_radius_km = job.max_radius_km.max(radius_km);

        while radius_km <= max_radius_km + f64::EPSILON {
            // Fetch up to 10 candidates and apply multi-factor scoring
            let candidates = self
                .engine
                .nearest_candidates_with_distance(job.pickup, &job.category, radius_km, 10);

            if let Some(driver_id) =
                self.best_candidate(candidates, job.pickup, radius_km)
            {
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

    /// Score a set of candidate `(driver_id, distance_km)` pairs and return the
    /// best driver, or `None` if the list is empty.
    ///
    /// ETA is estimated for each candidate using the configured `eta_provider`
    /// and incorporated into the distance component so that road-network travel
    /// time can replace straight-line distance when an OSRM provider is active.
    fn best_candidate(
        &self,
        candidates: Vec<(Uuid, f64)>,
        pickup: Coordinates,
        radius_km: f64,
    ) -> Option<Uuid> {
        // Estimate max possible ETA within this radius for normalisation (using
        // straight-line at the edge — avoids a second ETA call per candidate).
        let max_eta_secs = (radius_km / self.scoring.weights.distance.max(0.1) as f64)
            .max(1.0)
            * 120.0; // generous upper bound

        candidates
            .into_iter()
            .filter_map(|(driver_id, distance_km)| {
                let snapshot = self.engine.driver_snapshot(driver_id)?;
                let workload = self.engine.pending_offer_count_for_driver(driver_id) as f32;

                // Use ETA-based proximity when an ETA provider is available.
                let eta_secs = self.eta_provider.estimate_secs(
                    eta::LatLon {
                        latitude: snapshot.position.latitude,
                        longitude: snapshot.position.longitude,
                    },
                    eta::LatLon {
                        latitude: pickup.latitude,
                        longitude: pickup.longitude,
                    },
                );
                // Normalise ETA to [0,1] then invert so closer (faster) = higher score.
                let eta_km_equiv = (eta_secs / 3600.0) * 30.0; // convert at 30 km/h for normalisation
                let effective_distance_km = eta_km_equiv.min(distance_km * 2.0);
                let normalise_radius = radius_km.max(max_eta_secs / 3600.0 * 30.0);

                let score = scoring::score_candidate(
                    effective_distance_km,
                    normalise_radius,
                    snapshot.rating,
                    workload,
                    &self.scoring.weights,
                );
                Some((driver_id, score))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _)| id)
    }

    pub fn cancel_offer(&mut self, offer_id: Uuid) {
        let _ = self
            .engine
            .mark_offer_status(offer_id, spatiad_types::OfferStatus::Cancelled);
    }

    pub fn cancel_job(&mut self, job_id: Uuid) -> bool {
        self.engine.cancel_job(job_id)
    }

    pub fn record_webhook_delivery_failed(&mut self, job_id: Uuid, offer_id: Uuid) {
        self.engine.record_webhook_delivery_failed(job_id, offer_id);
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
            5.0,
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
            5.0,
        );
        engine.upsert_driver_location(
            driver_b,
            "tow_truck".to_string(),
            Coordinates {
                latitude: 38.434,
                longitude: 26.769,
            },
            DriverStatus::Available,
            5.0,
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
            5.0,
        );
        engine.upsert_driver_location(
            driver_b,
            "tow_truck".to_string(),
            Coordinates {
                latitude: 38.434,
                longitude: 26.769,
            },
            DriverStatus::Available,
            5.0,
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
