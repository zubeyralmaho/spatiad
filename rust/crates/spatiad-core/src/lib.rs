use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use spatiad_h3::SpatialIndex;
use spatiad_storage::{
    Command, EngineSnapshot, EventCursor, StorageBackend, StoredJobEvent, InMemoryBackend,
};
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
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CancelledDriverOffer {
    pub driver_id: Uuid,
    pub offer_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct ExpiredOffer {
    pub offer_id: Uuid,
    pub driver_id: Uuid,
    pub job_id: Uuid,
}

#[derive(Debug, Clone)]
pub enum JobDispatchState {
    UnknownJob,
    Pending,
    Searching,
    Cancelled,
    Matched {
        driver_id: Uuid,
        offer_id: Uuid,
    },
    Exhausted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JobEventKind {
    JobRegistered,
    JobCancelled,
    WebhookDeliveryFailed { offer_id: Uuid },
    OfferCreated { offer_id: Uuid, driver_id: Uuid },
    OfferExpired { offer_id: Uuid, driver_id: Uuid },
    OfferCancelled { offer_id: Uuid, driver_id: Uuid },
    OfferRejected { offer_id: Uuid, driver_id: Uuid },
    OfferAccepted { offer_id: Uuid, driver_id: Uuid },
    MatchConfirmed { offer_id: Uuid, driver_id: Uuid },
    OfferStatusUpdated { offer_id: Uuid, status: OfferStatus },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum JobEventFilterKind {
    JobRegistered,
    JobCancelled,
    WebhookDeliveryFailed,
    OfferCreated,
    OfferExpired,
    OfferCancelled,
    OfferRejected,
    OfferAccepted,
    MatchConfirmed,
    OfferStatusUpdated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobEventRecord {
    pub sequence: u64,
    pub occurred_at: DateTime<Utc>,
    pub kind: JobEventKind,
}

#[derive(Debug, Clone, Serialize)]
pub struct EngineStats {
    pub drivers: usize,
    pub jobs: usize,
    pub offers: usize,
    pub pending_offers: usize,
    pub cancelled_jobs: usize,
    pub wal_sequence: u64,
    pub storage_persistent: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct JobEventsCursor {
    pub occurred_at: DateTime<Utc>,
    pub sequence: u64,
}

pub struct Engine {
    h3_resolution: u8,
    spatial: SpatialIndex,
    drivers: HashMap<Uuid, DriverSnapshot>,
    jobs: HashMap<Uuid, JobRequest>,
    offers: HashMap<Uuid, OfferRecord>,
    cancelled_jobs: HashSet<Uuid>,
    job_events: HashMap<Uuid, Vec<JobEventRecord>>,
    job_event_sequences: HashMap<Uuid, u64>,
    storage: Box<dyn StorageBackend>,
    wal_sequence: u64,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("h3_resolution", &self.h3_resolution)
            .field("wal_sequence", &self.wal_sequence)
            .field("drivers", &self.drivers.len())
            .field("jobs", &self.jobs.len())
            .field("offers", &self.offers.len())
            .finish()
    }
}

const MAX_JOB_EVENTS: usize = 200;

impl Engine {
    pub fn new(h3_resolution: u8) -> Self {
        Self::with_storage(h3_resolution, Box::new(InMemoryBackend))
    }

    pub fn with_storage(h3_resolution: u8, storage: Box<dyn StorageBackend>) -> Self {
        Self {
            h3_resolution,
            spatial: SpatialIndex::new(h3_resolution),
            drivers: HashMap::new(),
            jobs: HashMap::new(),
            offers: HashMap::new(),
            cancelled_jobs: HashSet::new(),
            job_events: HashMap::new(),
            job_event_sequences: HashMap::new(),
            storage,
            wal_sequence: 0,
        }
    }

    /// Recover engine state from persistent storage (snapshot + WAL replay).
    pub fn recover(h3_resolution: u8, storage: Box<dyn StorageBackend>) -> Result<Self, String> {
        let snapshot = storage.load_snapshot().map_err(|e| e.to_string())?;
        let mut engine = if let Some(snap) = snapshot {
            let mut e = Self {
                h3_resolution: snap.h3_resolution,
                spatial: SpatialIndex::new(snap.h3_resolution),
                drivers: snap.drivers,
                jobs: snap.jobs,
                offers: snap.offers,
                cancelled_jobs: snap.cancelled_jobs,
                job_events: HashMap::new(),
                job_event_sequences: snap.job_event_sequences,
                storage,
                wal_sequence: snap.wal_sequence,
            };
            // Rebuild spatial index from driver positions
            let positions: Vec<(Uuid, Coordinates)> = e
                .drivers
                .iter()
                .map(|(id, d)| (*id, d.position))
                .collect();
            for (id, pos) in positions {
                e.spatial.upsert_driver(id, pos);
            }
            e
        } else {
            Self::with_storage(h3_resolution, storage)
        };

        // Replay WAL entries after the snapshot
        let wal_entries = engine
            .storage
            .load_wal_after(engine.wal_sequence)
            .map_err(|e| e.to_string())?;

        for (seq, cmd) in wal_entries {
            engine.apply_command(&cmd);
            engine.wal_sequence = seq;
        }

        Ok(engine)
    }

    /// Create a snapshot of the current state for persistence.
    pub fn to_snapshot(&self) -> EngineSnapshot {
        EngineSnapshot {
            h3_resolution: self.h3_resolution,
            drivers: self.drivers.clone(),
            jobs: self.jobs.clone(),
            offers: self.offers.clone(),
            cancelled_jobs: self.cancelled_jobs.clone(),
            job_event_sequences: self.job_event_sequences.clone(),
            wal_sequence: self.wal_sequence,
        }
    }

    /// Write a snapshot and compact the WAL.
    pub fn create_snapshot(&self) -> Result<(), String> {
        let snapshot = self.to_snapshot();
        self.storage
            .write_snapshot(&snapshot)
            .map_err(|e| e.to_string())?;
        self.storage
            .compact_wal(self.wal_sequence)
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Apply a command to engine state without WAL (used for replay and internal use).
    fn apply_command(&mut self, cmd: &Command) {
        match cmd {
            Command::UpsertDriverLocation {
                driver_id,
                category,
                position,
                status,
                timestamp,
                rating,
            } => {
                let snapshot = DriverSnapshot {
                    driver_id: *driver_id,
                    category: category.clone(),
                    status: status.clone(),
                    position: *position,
                    last_seen_at: *timestamp,
                    rating: *rating,
                };
                self.spatial.upsert_driver(*driver_id, *position);
                self.drivers.insert(*driver_id, snapshot);
            }
            Command::RegisterJob { job } => {
                let job_id = job.job_id;
                self.jobs.insert(job_id, job.clone());
                self.cancelled_jobs.remove(&job_id);
                self.push_job_event(job_id, JobEventKind::JobRegistered);
            }
            Command::CreateOffer { offer } => {
                let offer_id = offer.offer_id;
                let job_id = offer.job_id;
                let driver_id = offer.driver_id;
                self.offers.insert(offer_id, offer.clone());
                self.push_job_event(
                    job_id,
                    JobEventKind::OfferCreated { offer_id, driver_id },
                );
            }
            Command::AcceptOffer {
                offer_id,
                responded_at: _,
            } => {
                if let Some(offer) = self.offers.get_mut(offer_id) {
                    let job_id = offer.job_id;
                    let driver_id = offer.driver_id;
                    let selected_offer_id = offer.offer_id;
                    offer.status = OfferStatus::Accepted;

                    // Cancel competing offers
                    let mut cancelled = Vec::new();
                    for other in self.offers.values_mut() {
                        if other.job_id == job_id
                            && other.offer_id != selected_offer_id
                            && other.status == OfferStatus::Pending
                        {
                            other.status = OfferStatus::Cancelled;
                            cancelled.push((other.offer_id, other.driver_id));
                        }
                    }

                    if let Some(driver) = self.drivers.get_mut(&driver_id) {
                        driver.status = DriverStatus::Busy;
                    }

                    for (oid, did) in cancelled {
                        self.push_job_event(
                            job_id,
                            JobEventKind::OfferCancelled {
                                offer_id: oid,
                                driver_id: did,
                            },
                        );
                    }
                    self.push_job_event(
                        job_id,
                        JobEventKind::OfferAccepted {
                            offer_id: selected_offer_id,
                            driver_id,
                        },
                    );
                    self.push_job_event(
                        job_id,
                        JobEventKind::MatchConfirmed {
                            offer_id: selected_offer_id,
                            driver_id,
                        },
                    );
                }
            }
            Command::RejectOffer {
                offer_id,
                responded_at: _,
            } => {
                if let Some(offer) = self.offers.get_mut(offer_id) {
                    let job_id = offer.job_id;
                    let driver_id = offer.driver_id;
                    offer.status = OfferStatus::Rejected;
                    self.push_job_event(
                        job_id,
                        JobEventKind::OfferRejected {
                            offer_id: *offer_id,
                            driver_id,
                        },
                    );
                }
            }
            Command::MarkOfferExpired { offer_id } => {
                if let Some(offer) = self.offers.get_mut(offer_id) {
                    let job_id = offer.job_id;
                    let driver_id = offer.driver_id;
                    offer.status = OfferStatus::Expired;
                    self.push_job_event(
                        job_id,
                        JobEventKind::OfferExpired {
                            offer_id: *offer_id,
                            driver_id,
                        },
                    );
                }
            }
            Command::MarkOfferCancelled { offer_id } => {
                if let Some(offer) = self.offers.get_mut(offer_id) {
                    let job_id = offer.job_id;
                    let driver_id = offer.driver_id;
                    offer.status = OfferStatus::Cancelled;
                    self.push_job_event(
                        job_id,
                        JobEventKind::OfferCancelled {
                            offer_id: *offer_id,
                            driver_id,
                        },
                    );
                }
            }
            Command::CancelJob { job_id } => {
                if self.jobs.contains_key(job_id) {
                    let was_new = self.cancelled_jobs.insert(*job_id);
                    let mut cancelled_offers = Vec::new();
                    for offer in self.offers.values_mut() {
                        if offer.job_id == *job_id && offer.status == OfferStatus::Pending {
                            offer.status = OfferStatus::Cancelled;
                            cancelled_offers.push((offer.offer_id, offer.driver_id));
                        }
                    }
                    for (oid, did) in cancelled_offers {
                        self.push_job_event(
                            *job_id,
                            JobEventKind::OfferCancelled {
                                offer_id: oid,
                                driver_id: did,
                            },
                        );
                    }
                    if was_new {
                        self.push_job_event(*job_id, JobEventKind::JobCancelled);
                    }
                }
            }
            Command::RemoveDriver { driver_id } => {
                self.spatial.remove_driver(*driver_id);
                self.drivers.remove(driver_id);
            }
            Command::RecordWebhookDeliveryFailed { job_id, offer_id } => {
                if self.jobs.contains_key(job_id) {
                    self.push_job_event(
                        *job_id,
                        JobEventKind::WebhookDeliveryFailed {
                            offer_id: *offer_id,
                        },
                    );
                }
            }
        }
    }

    fn append_and_apply(&mut self, cmd: Command) {
        self.wal_sequence += 1;
        // Best-effort WAL append — log but don't fail the operation
        if let Err(e) = self.storage.append_wal(self.wal_sequence, &cmd) {
            tracing::error!(error = %e, "WAL append failed");
        }
        self.apply_command(&cmd);
    }

    // ─── Public mutation methods ─────────────────────────────────────

    pub fn upsert_driver_location(
        &mut self,
        driver_id: Uuid,
        category: String,
        position: Coordinates,
        status: DriverStatus,
        rating: f32,
    ) {
        let now = Utc::now();
        let rating = rating.clamp(1.0, 5.0);
        self.append_and_apply(Command::UpsertDriverLocation {
            driver_id,
            category,
            position,
            status,
            timestamp: now,
            rating,
        });
    }

    /// Return a driver snapshot by ID, or `None` if the driver is unknown.
    pub fn driver_snapshot(&self, driver_id: Uuid) -> Option<&DriverSnapshot> {
        self.drivers.get(&driver_id)
    }

    /// Count how many pending (not-yet-responded) offers a driver currently has.
    pub fn pending_offer_count_for_driver(&self, driver_id: Uuid) -> usize {
        self.offers
            .values()
            .filter(|o| o.driver_id == driver_id && o.status == OfferStatus::Pending)
            .count()
    }

    /// Like `nearest_candidates_in_radius`, but returns `(driver_id, distance_km)` pairs
    /// so callers can use distance in multi-factor scoring without recomputing it.
    pub fn nearest_candidates_with_distance(
        &self,
        pickup: Coordinates,
        category: &str,
        radius_km: f64,
        limit: usize,
    ) -> Vec<(Uuid, f64)> {
        let mut candidates: Vec<(Uuid, f64)> = self
            .drivers
            .iter()
            .filter_map(|(driver_id, driver)| {
                if driver.status != DriverStatus::Available
                    || !driver.category.eq_ignore_ascii_case(category)
                {
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
        candidates.truncate(limit);
        candidates
    }

    pub fn register_job(&mut self, job: JobRequest) {
        self.append_and_apply(Command::RegisterJob { job });
    }

    pub fn cancel_job(&mut self, job_id: Uuid) -> bool {
        if !self.jobs.contains_key(&job_id) {
            return false;
        }
        self.append_and_apply(Command::CancelJob { job_id });
        true
    }

    pub fn record_webhook_delivery_failed(&mut self, job_id: Uuid, offer_id: Uuid) {
        if !self.jobs.contains_key(&job_id) {
            return;
        }
        self.append_and_apply(Command::RecordWebhookDeliveryFailed { job_id, offer_id });
    }

    /// Remove drivers that have not sent a location update within `ttl`.
    /// Returns the list of removed driver IDs.
    pub fn expire_stale_drivers(&mut self, ttl: chrono::Duration) -> Vec<Uuid> {
        let cutoff = Utc::now() - ttl;
        let stale: Vec<Uuid> = self
            .drivers
            .iter()
            .filter(|(_, d)| d.last_seen_at < cutoff)
            .map(|(id, _)| *id)
            .collect();

        for driver_id in &stale {
            self.append_and_apply(Command::RemoveDriver {
                driver_id: *driver_id,
            });
        }
        stale
    }

    /// Return basic engine statistics for health/diagnostics.
    pub fn stats(&self) -> EngineStats {
        let pending_offers = self
            .offers
            .values()
            .filter(|o| o.status == OfferStatus::Pending)
            .count();
        EngineStats {
            drivers: self.drivers.len(),
            jobs: self.jobs.len(),
            offers: self.offers.len(),
            pending_offers,
            cancelled_jobs: self.cancelled_jobs.len(),
            wal_sequence: self.wal_sequence,
            storage_persistent: self.storage.is_persistent(),
        }
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
        self.append_and_apply(Command::CreateOffer {
            offer: offer.clone(),
        });
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
        let offer = self.offers.get(&offer_id).ok_or(CoreError::OfferNotFound)?;
        let _job_id = offer.job_id;
        match status {
            OfferStatus::Cancelled => {
                self.append_and_apply(Command::MarkOfferCancelled { offer_id });
            }
            OfferStatus::Expired => {
                self.append_and_apply(Command::MarkOfferExpired { offer_id });
            }
            _ => {
                // For other status changes, apply directly (offer_status_updated event)
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
            }
        }
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

    pub fn expire_pending_offers_for_driver(&mut self, driver_id: Uuid) -> Vec<ExpiredOffer> {
        let now = Utc::now();
        let to_expire: Vec<Uuid> = self
            .offers
            .values()
            .filter(|offer| {
                offer.driver_id == driver_id
                    && offer.status == OfferStatus::Pending
                    && offer.expires_at <= now
            })
            .map(|offer| offer.offer_id)
            .collect();

        let mut expired = Vec::new();
        for offer_id in to_expire {
            if let Some(offer) = self.offers.get(&offer_id) {
                expired.push(ExpiredOffer {
                    offer_id: offer.offer_id,
                    driver_id: offer.driver_id,
                    job_id: offer.job_id,
                });
            }
            self.append_and_apply(Command::MarkOfferExpired { offer_id });
        }
        expired
    }

    pub fn expire_pending_offers_global(&mut self) -> Vec<ExpiredOffer> {
        let now = Utc::now();
        let to_expire: Vec<Uuid> = self
            .offers
            .values()
            .filter(|offer| offer.status == OfferStatus::Pending && offer.expires_at <= now)
            .map(|offer| offer.offer_id)
            .collect();

        let mut expired = Vec::new();
        for offer_id in to_expire {
            if let Some(offer) = self.offers.get(&offer_id) {
                expired.push(ExpiredOffer {
                    offer_id: offer.offer_id,
                    driver_id: offer.driver_id,
                    job_id: offer.job_id,
                });
            }
            self.append_and_apply(Command::MarkOfferExpired { offer_id });
        }
        expired
    }

    pub fn offer_job_id(&self, offer_id: Uuid) -> Option<Uuid> {
        self.offers.get(&offer_id).map(|offer| offer.job_id)
    }

    pub fn create_next_offer_for_job(&mut self, job_id: Uuid) -> Option<OfferRecord> {
        let job = self.jobs.get(&job_id)?.clone();

        if self.cancelled_jobs.contains(&job_id) {
            return None;
        }

        if self
            .offers
            .values()
            .any(|offer| offer.job_id == job_id && offer.status == OfferStatus::Accepted)
        {
            return None;
        }

        if self
            .offers
            .values()
            .any(|offer| offer.job_id == job_id && offer.status == OfferStatus::Pending)
        {
            return None;
        }

        let already_offered: HashSet<Uuid> = self
            .offers
            .values()
            .filter(|offer| offer.job_id == job_id)
            .map(|offer| offer.driver_id)
            .collect();

        let mut radius_km = job.initial_radius_km.max(0.1);
        let max_radius_km = job.max_radius_km.max(radius_km);

        while radius_km <= max_radius_km + f64::EPSILON {
            let candidates = self.nearest_candidates_in_radius(
                job.pickup,
                &job.category,
                radius_km,
                32,
            );

            if let Some(driver_id) = candidates
                .into_iter()
                .find(|driver_id| !already_offered.contains(driver_id))
            {
                return Some(self.create_offer(job_id, driver_id, job.timeout_seconds));
            }

            radius_km = expand_radius_km(radius_km, max_radius_km);
            if radius_km > max_radius_km {
                break;
            }
        }

        None
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

        if self.cancelled_jobs.contains(&job_id) {
            return JobDispatchState::Cancelled;
        }

        if offers.iter().any(|offer| offer.status == OfferStatus::Pending) {
            return JobDispatchState::Searching;
        }

        JobDispatchState::Exhausted
    }

    pub fn handle_offer_response(
        &mut self,
        offer_id: Uuid,
        accepted: bool,
    ) -> Result<Option<MatchResult>, CoreError> {
        let offer = self.offers.get(&offer_id).ok_or(CoreError::OfferNotFound)?;
        if offer.status != OfferStatus::Pending {
            return Err(CoreError::OfferNotPending);
        }
        if offer.expires_at <= Utc::now() {
            // Mark expired via WAL
            self.append_and_apply(Command::MarkOfferExpired { offer_id });
            return Err(CoreError::OfferExpired);
        }

        let job_id = offer.job_id;
        let driver_id = offer.driver_id;
        let now = Utc::now();

        if accepted {
            self.append_and_apply(Command::AcceptOffer {
                offer_id,
                responded_at: now,
            });
            Ok(Some(MatchResult {
                job_id,
                driver_id,
                offer_id,
                matched_at: now,
            }))
        } else {
            self.append_and_apply(Command::RejectOffer {
                offer_id,
                responded_at: now,
            });
            Ok(None)
        }
    }

    // ─── Event query methods ─────────────────────────────────────────

    pub fn job_events(&self, job_id: Uuid, limit: usize) -> Vec<JobEventRecord> {
        if self.storage.is_persistent() {
            return self.query_persistent_events(job_id, limit, None, None);
        }
        self.job_events_before(job_id, limit, None)
    }

    pub fn job_events_before(
        &self,
        job_id: Uuid,
        limit: usize,
        before: Option<DateTime<Utc>>,
    ) -> Vec<JobEventRecord> {
        self.job_events_before_filtered(job_id, limit, before, None)
    }

    pub fn job_events_before_filtered(
        &self,
        job_id: Uuid,
        limit: usize,
        before: Option<DateTime<Utc>>,
        kinds: Option<&[JobEventFilterKind]>,
    ) -> Vec<JobEventRecord> {
        let cursor = before.map(|occurred_at| JobEventsCursor {
            occurred_at,
            sequence: 0,
        });
        self.job_events_cursor_filtered(job_id, limit, cursor, kinds)
    }

    pub fn job_events_cursor_filtered(
        &self,
        job_id: Uuid,
        limit: usize,
        cursor: Option<JobEventsCursor>,
        kinds: Option<&[JobEventFilterKind]>,
    ) -> Vec<JobEventRecord> {
        if self.storage.is_persistent() {
            let storage_cursor = cursor.map(|c| EventCursor {
                occurred_at: c.occurred_at,
                sequence: c.sequence,
            });
            return self.query_persistent_events(job_id, limit, storage_cursor, kinds);
        }

        // In-memory path (unchanged behavior)
        let max_items = if limit == 0 { 50 } else { limit };
        self.job_events
            .get(&job_id)
            .map(|events| {
                events
                    .iter()
                    .rev()
                    .filter(|event| {
                        cursor
                            .map(|value| {
                                event.occurred_at < value.occurred_at
                                    || (event.occurred_at == value.occurred_at
                                        && event.sequence < value.sequence)
                            })
                            .unwrap_or(true)
                    })
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

    fn query_persistent_events(
        &self,
        job_id: Uuid,
        limit: usize,
        cursor: Option<EventCursor>,
        kinds: Option<&[JobEventFilterKind]>,
    ) -> Vec<JobEventRecord> {
        let kind_filter: Option<Vec<String>> = kinds.map(|ks| {
            ks.iter()
                .map(|k| serde_json::to_string(k).unwrap_or_default())
                .collect()
        });
        let filter_refs: Option<Vec<String>> = kind_filter;

        match self.storage.query_job_events(
            job_id,
            limit,
            cursor,
            filter_refs.as_deref().map(|v| {
                // Convert &[String] to &[String] — already correct type
                v
            }),
        ) {
            Ok(stored) => stored
                .into_iter()
                .filter_map(|se| {
                    let kind: JobEventKind = serde_json::from_str(&se.kind_json).ok()?;
                    Some(JobEventRecord {
                        sequence: se.sequence,
                        occurred_at: se.occurred_at,
                        kind,
                    })
                })
                .collect(),
            Err(_) => vec![],
        }
    }
}

// ─── Private helpers ─────────────────────────────────────────────────

impl Engine {
    fn push_job_event(&mut self, job_id: Uuid, kind: JobEventKind) {
        let next_sequence = {
            let current = self.job_event_sequences.entry(job_id).or_insert(0);
            *current += 1;
            *current
        };

        let now = Utc::now();

        // Persist to storage
        if self.storage.is_persistent() {
            let kind_json = serde_json::to_string(&kind).unwrap_or_default();
            let _ = self.storage.append_job_event(
                job_id,
                &StoredJobEvent {
                    sequence: next_sequence,
                    occurred_at: now,
                    kind_json,
                },
            );
        }

        // Always maintain in-memory for reads (in-memory mode) and fast access
        let entries = self.job_events.entry(job_id).or_default();
        entries.push(JobEventRecord {
            sequence: next_sequence,
            occurred_at: now,
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
        JobEventKind::JobCancelled => JobEventFilterKind::JobCancelled,
        JobEventKind::WebhookDeliveryFailed { .. } => JobEventFilterKind::WebhookDeliveryFailed,
        JobEventKind::OfferCreated { .. } => JobEventFilterKind::OfferCreated,
        JobEventKind::OfferExpired { .. } => JobEventFilterKind::OfferExpired,
        JobEventKind::OfferCancelled { .. } => JobEventFilterKind::OfferCancelled,
        JobEventKind::OfferRejected { .. } => JobEventFilterKind::OfferRejected,
        JobEventKind::OfferAccepted { .. } => JobEventFilterKind::OfferAccepted,
        JobEventKind::MatchConfirmed { .. } => JobEventFilterKind::MatchConfirmed,
        JobEventKind::OfferStatusUpdated { .. } => JobEventFilterKind::OfferStatusUpdated,
    }
}

fn expand_radius_km(current_radius_km: f64, max_radius_km: f64) -> f64 {
    let next = current_radius_km + 2.0;
    next.min(max_radius_km + 1.0)
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
            5.0,
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
        assert_eq!(expired[0].offer_id, offer.offer_id);
        assert_eq!(expired[0].driver_id, driver_id);
        assert_eq!(expired[0].job_id, job_id);
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
                5.0,
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
            5.0,
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
            5.0,
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
            5.0,
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
            5.0,
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

    #[test]
    fn job_events_cursor_uses_sequence_for_stable_pagination() {
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
            5.0,
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

        let first = engine.job_events(job_id, 1);
        let cursor = first.first().map(|event| JobEventsCursor {
            occurred_at: event.occurred_at,
            sequence: event.sequence,
        });

        let second = engine.job_events_cursor_filtered(job_id, 1, cursor, None);
        assert_eq!(second.len(), 1);
        assert!(second[0].sequence < first[0].sequence);
    }

    #[test]
    fn cancelled_job_is_reported_as_cancelled() {
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
            5.0,
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

        let _ = engine.create_offer(job_id, driver_id, 30);
        assert!(engine.cancel_job(job_id));

        assert!(matches!(
            engine.job_dispatch_state(job_id),
            JobDispatchState::Cancelled
        ));
        assert!(engine.create_next_offer_for_job(job_id).is_none());
        assert!(engine.job_events(job_id, 20).iter().any(|event| matches!(
            event.kind,
            JobEventKind::JobCancelled
        )));
    }
}
