use std::collections::HashMap;

use chrono::Utc;
use spatiad_h3::SpatialIndex;
use spatiad_types::{
    Coordinates, DriverSnapshot, DriverStatus, JobRequest, MatchResult, OfferRecord, OfferStatus,
};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("offer not found")]
    OfferNotFound,
    #[error("offer is not pending")]
    OfferNotPending,
    #[error("offer is expired")]
    OfferExpired,
}

#[derive(Debug, Clone)]
pub struct PendingDriverOffer {
    pub offer_id: Uuid,
    pub job_id: Uuid,
    pub pickup: Coordinates,
    pub dropoff: Option<Coordinates>,
    pub expires_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CancelledDriverOffer {
    pub driver_id: Uuid,
    pub offer_id: Uuid,
}

#[derive(Debug, Clone)]
pub enum JobDispatchState {
    UnknownJob,
    Pending,
    Searching,
    Matched {
        driver_id: Uuid,
        offer_id: Uuid,
    },
    Exhausted,
}

#[derive(Debug, Clone)]
pub enum JobEventKind {
    JobRegistered,
    OfferCreated { offer_id: Uuid, driver_id: Uuid },
    OfferExpired { offer_id: Uuid, driver_id: Uuid },
    OfferCancelled { offer_id: Uuid, driver_id: Uuid },
    OfferRejected { offer_id: Uuid, driver_id: Uuid },
    OfferAccepted { offer_id: Uuid, driver_id: Uuid },
    MatchConfirmed { offer_id: Uuid, driver_id: Uuid },
    OfferStatusUpdated { offer_id: Uuid, status: OfferStatus },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JobEventFilterKind {
    JobRegistered,
    OfferCreated,
    OfferExpired,
    OfferCancelled,
    OfferRejected,
    OfferAccepted,
    MatchConfirmed,
    OfferStatusUpdated,
}

#[derive(Debug, Clone)]
pub struct JobEventRecord {
    pub occurred_at: chrono::DateTime<Utc>,
    pub kind: JobEventKind,
}

#[derive(Debug)]
pub struct Engine {
    spatial: SpatialIndex,
    drivers: HashMap<Uuid, DriverSnapshot>,
    jobs: HashMap<Uuid, JobRequest>,
    offers: HashMap<Uuid, OfferRecord>,
    job_events: HashMap<Uuid, Vec<JobEventRecord>>,
}

const MAX_JOB_EVENTS: usize = 200;

impl Engine {
    pub fn new(h3_resolution: u8) -> Self {
        Self {
            spatial: SpatialIndex::new(h3_resolution),
            drivers: HashMap::new(),
            jobs: HashMap::new(),
            offers: HashMap::new(),
            job_events: HashMap::new(),
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
        let job_id = job.job_id;
        self.jobs.insert(job_id, job);
        self.push_job_event(job_id, JobEventKind::JobRegistered);
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
        self.push_job_event(
            job_id,
            JobEventKind::OfferCreated {
                offer_id: offer.offer_id,
                driver_id,
            },
        );
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
        let job_id = offer.job_id;
        let status_for_event = status.clone();
        offer.status = status;
        self.push_job_event(
            job_id,
            JobEventKind::OfferStatusUpdated {
                offer_id,
                status: status_for_event,
            },
        );
        Ok(())
    }

    pub fn pending_offers_for_driver(&self, driver_id: Uuid) -> Vec<PendingDriverOffer> {
        self.offers
            .values()
            .filter(|offer| {
                offer.driver_id == driver_id
                    && offer.status == OfferStatus::Pending
                    && offer.expires_at > Utc::now()
            })
            .filter_map(|offer| {
                self.jobs.get(&offer.job_id).map(|job| PendingDriverOffer {
                    offer_id: offer.offer_id,
                    job_id: offer.job_id,
                    pickup: job.pickup,
                    dropoff: job.dropoff,
                    expires_at: offer.expires_at,
                })
            })
            .collect()
    }

    pub fn expire_pending_offers_for_driver(&mut self, driver_id: Uuid) -> Vec<Uuid> {
        let now = Utc::now();
        let mut expired = Vec::new();
        let mut expired_events: Vec<(Uuid, Uuid)> = Vec::new();

        for offer in self.offers.values_mut() {
            if offer.driver_id == driver_id
                && offer.status == OfferStatus::Pending
                && offer.expires_at <= now
            {
                offer.status = OfferStatus::Expired;
                expired.push(offer.offer_id);
                expired_events.push((offer.job_id, offer.offer_id));
            }
        }

        for (job_id, offer_id) in expired_events {
            self.push_job_event(
                job_id,
                JobEventKind::OfferExpired {
                    offer_id,
                    driver_id,
                },
            );
        }

        expired
    }

    pub fn cancelled_offers_for_job(&self, job_id: Uuid) -> Vec<CancelledDriverOffer> {
        self.offers
            .values()
            .filter(|offer| offer.job_id == job_id && offer.status == OfferStatus::Cancelled)
            .map(|offer| CancelledDriverOffer {
                driver_id: offer.driver_id,
                offer_id: offer.offer_id,
            })
            .collect()
    }

    pub fn job_dispatch_state(&self, job_id: Uuid) -> JobDispatchState {
        if !self.jobs.contains_key(&job_id) {
            return JobDispatchState::UnknownJob;
        }

        let offers: Vec<&OfferRecord> = self
            .offers
            .values()
            .filter(|offer| offer.job_id == job_id)
            .collect();

        if offers.is_empty() {
            return JobDispatchState::Pending;
        }

        if let Some(accepted) = offers.iter().find(|offer| offer.status == OfferStatus::Accepted) {
            return JobDispatchState::Matched {
                driver_id: accepted.driver_id,
                offer_id: accepted.offer_id,
            };
        }

        if offers.iter().any(|offer| offer.status == OfferStatus::Pending) {
            return JobDispatchState::Searching;
        }

        JobDispatchState::Exhausted
    }

    pub fn job_events(&self, job_id: Uuid, limit: usize) -> Vec<JobEventRecord> {
        self.job_events_before(job_id, limit, None)
    }

    pub fn job_events_before(
        &self,
        job_id: Uuid,
        limit: usize,
        before: Option<chrono::DateTime<Utc>>,
    ) -> Vec<JobEventRecord> {
        self.job_events_before_filtered(job_id, limit, before, None)
    }

    pub fn job_events_before_filtered(
        &self,
        job_id: Uuid,
        limit: usize,
        before: Option<chrono::DateTime<Utc>>,
        kinds: Option<&[JobEventFilterKind]>,
    ) -> Vec<JobEventRecord> {
        let max_items = if limit == 0 { 50 } else { limit };
        self.job_events
            .get(&job_id)
            .map(|events| {
                events
                    .iter()
                    .rev()
                    .filter(|event| before.map(|cursor| event.occurred_at < cursor).unwrap_or(true))
                    .filter(|event| {
                        kinds
                            .map(|requested| {
                                requested.is_empty()
                                    || requested.contains(&event_filter_kind(&event.kind))
                            })
                            .unwrap_or(true)
                    })
                    .take(max_items)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn handle_offer_response(
        &mut self,
        offer_id: Uuid,
        accepted: bool,
    ) -> Result<Option<MatchResult>, CoreError> {
        let mut deferred_events: Vec<(Uuid, JobEventKind)> = Vec::new();

        let (job_id, driver_id, selected_offer_id) = {
            let offer = self.offers.get_mut(&offer_id).ok_or(CoreError::OfferNotFound)?;
            if offer.status != OfferStatus::Pending {
                return Err(CoreError::OfferNotPending);
            }
            if offer.expires_at <= Utc::now() {
                offer.status = OfferStatus::Expired;
                return Err(CoreError::OfferExpired);
            }

            if accepted {
                offer.status = OfferStatus::Accepted;
            } else {
                offer.status = OfferStatus::Rejected;
                deferred_events.push((
                    offer.job_id,
                    JobEventKind::OfferRejected {
                        offer_id: offer.offer_id,
                        driver_id: offer.driver_id,
                    },
                ));

                for (event_job_id, event_kind) in deferred_events {
                    self.push_job_event(event_job_id, event_kind);
                }
                return Ok(None);
            }

            (offer.job_id, offer.driver_id, offer.offer_id)
        };

        // Ensure there is only one winner for a job by cancelling competing pending offers.
        for other_offer in self.offers.values_mut() {
            if other_offer.job_id == job_id
                && other_offer.offer_id != selected_offer_id
                && other_offer.status == OfferStatus::Pending
            {
                other_offer.status = OfferStatus::Cancelled;
                deferred_events.push((
                    job_id,
                    JobEventKind::OfferCancelled {
                        offer_id: other_offer.offer_id,
                        driver_id: other_offer.driver_id,
                    },
                ));
            }
        }

        if let Some(driver) = self.drivers.get_mut(&driver_id) {
            driver.status = DriverStatus::Busy;
        }

        deferred_events.push((
            job_id,
            JobEventKind::OfferAccepted {
                offer_id: selected_offer_id,
                driver_id,
            },
        ));
        deferred_events.push((
            job_id,
            JobEventKind::MatchConfirmed {
                offer_id: selected_offer_id,
                driver_id,
            },
        ));

        for (event_job_id, event_kind) in deferred_events {
            self.push_job_event(event_job_id, event_kind);
        }

        Ok(Some(MatchResult {
            job_id,
            driver_id,
            offer_id: selected_offer_id,
            matched_at: Utc::now(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use spatiad_types::{Coordinates, DriverStatus};
    use std::{thread, time::Duration as StdDuration};

    use super::*;

    #[test]
    fn pending_offer_can_expire_for_driver() {
        let mut engine = Engine::new(8);
        let driver_id = Uuid::new_v4();
        let job_id = Uuid::new_v4();

        engine.upsert_driver_location(
            driver_id,
            "tow_truck".to_string(),
            Coordinates {
                latitude: 38.433,
                longitude: 26.768,
            },
            DriverStatus::Available,
        );

        engine.register_job(JobRequest {
            job_id,
            category: "tow_truck".to_string(),
            pickup: Coordinates {
                latitude: 38.433,
                longitude: 26.768,
            },
            dropoff: None,
            initial_radius_km: 1.0,
            max_radius_km: 5.0,
            timeout_seconds: 30,
            created_at: Utc::now(),
        });

        let offer = engine.create_offer(job_id, driver_id, 1);
        if let Some(stored) = engine.offers.get_mut(&offer.offer_id) {
            stored.expires_at = Utc::now() - Duration::seconds(1);
        }

        let expired = engine.expire_pending_offers_for_driver(driver_id);
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0], offer.offer_id);
    }

    #[test]
    fn accepted_offer_cancels_other_pending_offers_for_same_job() {
        let mut engine = Engine::new(8);
        let driver_a = Uuid::new_v4();
        let driver_b = Uuid::new_v4();
        let job_id = Uuid::new_v4();

        for driver_id in [driver_a, driver_b] {
            engine.upsert_driver_location(
                driver_id,
                "tow_truck".to_string(),
                Coordinates {
                    latitude: 38.433,
                    longitude: 26.768,
                },
                DriverStatus::Available,
            );
        }

        engine.register_job(JobRequest {
            job_id,
            category: "tow_truck".to_string(),
            pickup: Coordinates {
                latitude: 38.433,
                longitude: 26.768,
            },
            dropoff: None,
            initial_radius_km: 1.0,
            max_radius_km: 5.0,
            timeout_seconds: 30,
            created_at: Utc::now(),
        });

        let accepted_offer = engine.create_offer(job_id, driver_a, 30);
        let cancelled_offer = engine.create_offer(job_id, driver_b, 30);

        let result = engine
            .handle_offer_response(accepted_offer.offer_id, true)
            .expect("offer response should succeed")
            .expect("accepted offer should produce match result");

        assert_eq!(result.job_id, job_id);
        assert_eq!(result.driver_id, driver_a);

        let accepted_status = engine
            .offers
            .get(&accepted_offer.offer_id)
            .map(|offer| offer.status.clone())
            .expect("accepted offer exists");
        let cancelled_status = engine
            .offers
            .get(&cancelled_offer.offer_id)
            .map(|offer| offer.status.clone())
            .expect("competing offer exists");

        assert_eq!(accepted_status, OfferStatus::Accepted);
        assert_eq!(cancelled_status, OfferStatus::Cancelled);
    }

    #[test]
    fn job_dispatch_state_transitions_to_matched_after_accept() {
        let mut engine = Engine::new(8);
        let driver_id = Uuid::new_v4();
        let job_id = Uuid::new_v4();

        engine.upsert_driver_location(
            driver_id,
            "tow_truck".to_string(),
            Coordinates {
                latitude: 38.433,
                longitude: 26.768,
            },
            DriverStatus::Available,
        );

        engine.register_job(JobRequest {
            job_id,
            category: "tow_truck".to_string(),
            pickup: Coordinates {
                latitude: 38.433,
                longitude: 26.768,
            },
            dropoff: None,
            initial_radius_km: 1.0,
            max_radius_km: 5.0,
            timeout_seconds: 30,
            created_at: Utc::now(),
        });

        let offer = engine.create_offer(job_id, driver_id, 30);
        assert!(matches!(engine.job_dispatch_state(job_id), JobDispatchState::Searching));

        engine
            .handle_offer_response(offer.offer_id, true)
            .expect("response should be accepted")
            .expect("match result should exist");

        assert!(matches!(
            engine.job_dispatch_state(job_id),
            JobDispatchState::Matched { .. }
        ));
    }

    #[test]
    fn job_event_history_contains_offer_and_match_events() {
        let mut engine = Engine::new(8);
        let driver_id = Uuid::new_v4();
        let job_id = Uuid::new_v4();

        engine.upsert_driver_location(
            driver_id,
            "tow_truck".to_string(),
            Coordinates {
                latitude: 38.433,
                longitude: 26.768,
            },
            DriverStatus::Available,
        );

        engine.register_job(JobRequest {
            job_id,
            category: "tow_truck".to_string(),
            pickup: Coordinates {
                latitude: 38.433,
                longitude: 26.768,
            },
            dropoff: None,
            initial_radius_km: 1.0,
            max_radius_km: 5.0,
            timeout_seconds: 30,
            created_at: Utc::now(),
        });

        let offer = engine.create_offer(job_id, driver_id, 30);
        engine
            .handle_offer_response(offer.offer_id, true)
            .expect("offer response should succeed")
            .expect("match result expected");

        let events = engine.job_events(job_id, 20);
        assert!(events.iter().any(|event| matches!(
            event.kind,
            JobEventKind::OfferCreated { .. }
        )));
        assert!(events.iter().any(|event| matches!(
            event.kind,
            JobEventKind::MatchConfirmed { .. }
        )));
    }

    #[test]
    fn job_events_before_cursor_filters_recent_events() {
        let mut engine = Engine::new(8);
        let driver_id = Uuid::new_v4();
        let job_id = Uuid::new_v4();

        engine.upsert_driver_location(
            driver_id,
            "tow_truck".to_string(),
            Coordinates {
                latitude: 38.433,
                longitude: 26.768,
            },
            DriverStatus::Available,
        );

        engine.register_job(JobRequest {
            job_id,
            category: "tow_truck".to_string(),
            pickup: Coordinates {
                latitude: 38.433,
                longitude: 26.768,
            },
            dropoff: None,
            initial_radius_km: 1.0,
            max_radius_km: 5.0,
            timeout_seconds: 30,
            created_at: Utc::now(),
        });

        let offer = engine.create_offer(job_id, driver_id, 30);
        thread::sleep(StdDuration::from_millis(1));
        let _ = engine.handle_offer_response(offer.offer_id, false);

        let page_one = engine.job_events_before(job_id, 2, None);
        assert_eq!(page_one.len(), 2);
        let cursor = page_one
            .last()
            .map(|event| event.occurred_at)
            .expect("expected second event timestamp");

        let page_two = engine.job_events_before(job_id, 10, Some(cursor));
        assert!(page_two.iter().all(|event| event.occurred_at < cursor));
    }

    #[test]
    fn job_events_before_can_filter_by_event_kind() {
        let mut engine = Engine::new(8);
        let driver_id = Uuid::new_v4();
        let job_id = Uuid::new_v4();

        engine.upsert_driver_location(
            driver_id,
            "tow_truck".to_string(),
            Coordinates {
                latitude: 38.433,
                longitude: 26.768,
            },
            DriverStatus::Available,
        );

        engine.register_job(JobRequest {
            job_id,
            category: "tow_truck".to_string(),
            pickup: Coordinates {
                latitude: 38.433,
                longitude: 26.768,
            },
            dropoff: None,
            initial_radius_km: 1.0,
            max_radius_km: 5.0,
            timeout_seconds: 30,
            created_at: Utc::now(),
        });

        let offer = engine.create_offer(job_id, driver_id, 30);
        let _ = engine.handle_offer_response(offer.offer_id, false);

        let filtered = engine.job_events_before_filtered(
            job_id,
            20,
            None,
            Some(&[JobEventFilterKind::OfferRejected]),
        );

        assert!(!filtered.is_empty());
        assert!(filtered
            .iter()
            .all(|event| matches!(event.kind, JobEventKind::OfferRejected { .. })));
    }
}

impl Engine {
    fn push_job_event(&mut self, job_id: Uuid, kind: JobEventKind) {
        let entries = self.job_events.entry(job_id).or_default();
        entries.push(JobEventRecord {
            occurred_at: Utc::now(),
            kind,
        });

        if entries.len() > MAX_JOB_EVENTS {
            let overflow = entries.len() - MAX_JOB_EVENTS;
            entries.drain(0..overflow);
        }
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

fn event_filter_kind(kind: &JobEventKind) -> JobEventFilterKind {
    match kind {
        JobEventKind::JobRegistered => JobEventFilterKind::JobRegistered,
        JobEventKind::OfferCreated { .. } => JobEventFilterKind::OfferCreated,
        JobEventKind::OfferExpired { .. } => JobEventFilterKind::OfferExpired,
        JobEventKind::OfferCancelled { .. } => JobEventFilterKind::OfferCancelled,
        JobEventKind::OfferRejected { .. } => JobEventFilterKind::OfferRejected,
        JobEventKind::OfferAccepted { .. } => JobEventFilterKind::OfferAccepted,
        JobEventKind::MatchConfirmed { .. } => JobEventFilterKind::MatchConfirmed,
        JobEventKind::OfferStatusUpdated { .. } => JobEventFilterKind::OfferStatusUpdated,
    }
}
