use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use spatiad_types::{
    Coordinates, DriverSnapshot, DriverStatus, JobRequest, OfferRecord,
};
use thiserror::Error;
use uuid::Uuid;

#[cfg(feature = "sqlite")]
mod sqlite;
#[cfg(feature = "sqlite")]
pub use sqlite::SqliteBackend;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("WAL append failed: {0}")]
    WalAppend(String),
    #[error("snapshot write failed: {0}")]
    SnapshotWrite(String),
    #[error("snapshot load failed: {0}")]
    SnapshotLoad(String),
    #[error("WAL load failed: {0}")]
    WalLoad(String),
    #[error("event store error: {0}")]
    EventStore(String),
}

/// A command that mutates engine state. Each variant captures all arguments
/// needed to replay the mutation deterministically — no `Utc::now()` or
/// `Uuid::new_v4()` during replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    UpsertDriverLocation {
        driver_id: Uuid,
        category: String,
        position: Coordinates,
        status: DriverStatus,
        timestamp: DateTime<Utc>,
    },
    RegisterJob {
        job: JobRequest,
    },
    CreateOffer {
        offer: OfferRecord,
    },
    AcceptOffer {
        offer_id: Uuid,
        responded_at: DateTime<Utc>,
    },
    RejectOffer {
        offer_id: Uuid,
        responded_at: DateTime<Utc>,
    },
    MarkOfferExpired {
        offer_id: Uuid,
    },
    MarkOfferCancelled {
        offer_id: Uuid,
    },
    CancelJob {
        job_id: Uuid,
    },
    RecordWebhookDeliveryFailed {
        job_id: Uuid,
        offer_id: Uuid,
    },
}

/// Serializable snapshot of the full engine state for crash recovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineSnapshot {
    pub h3_resolution: u8,
    pub drivers: HashMap<Uuid, DriverSnapshot>,
    pub jobs: HashMap<Uuid, JobRequest>,
    pub offers: HashMap<Uuid, OfferRecord>,
    pub cancelled_jobs: HashSet<Uuid>,
    pub job_event_sequences: HashMap<Uuid, u64>,
    pub wal_sequence: u64,
}

/// Serializable job event record for the persistent event store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredJobEvent {
    pub sequence: u64,
    pub occurred_at: DateTime<Utc>,
    pub kind_json: String,
}

/// Cursor for paginated event queries against the persistent store.
#[derive(Debug, Clone, Copy)]
pub struct EventCursor {
    pub occurred_at: DateTime<Utc>,
    pub sequence: u64,
}

pub trait StorageBackend: Send + Sync {
    /// Append a command to the write-ahead log. Called BEFORE applying to engine.
    fn append_wal(&self, seq: u64, cmd: &Command) -> Result<(), StorageError>;

    /// Persist a full snapshot of engine state.
    fn write_snapshot(&self, snapshot: &EngineSnapshot) -> Result<(), StorageError>;

    /// Load the latest snapshot, if any.
    fn load_snapshot(&self) -> Result<Option<EngineSnapshot>, StorageError>;

    /// Load WAL entries after the given sequence number, ordered ascending.
    fn load_wal_after(&self, after_seq: u64) -> Result<Vec<(u64, Command)>, StorageError>;

    /// Delete WAL entries up to and including the given sequence number.
    fn compact_wal(&self, up_to_seq: u64) -> Result<(), StorageError>;

    /// Append a job event to the persistent event store.
    fn append_job_event(
        &self,
        job_id: Uuid,
        event: &StoredJobEvent,
    ) -> Result<(), StorageError>;

    /// Query job events with cursor-based pagination (newest first).
    fn query_job_events(
        &self,
        job_id: Uuid,
        limit: usize,
        cursor: Option<EventCursor>,
        kind_filter: Option<&[String]>,
    ) -> Result<Vec<StoredJobEvent>, StorageError>;

    /// Returns true if this backend actually persists data (false for in-memory).
    fn is_persistent(&self) -> bool;
}

/// No-op storage backend that preserves the existing fully in-memory behavior.
pub struct InMemoryBackend;

impl StorageBackend for InMemoryBackend {
    fn append_wal(&self, _seq: u64, _cmd: &Command) -> Result<(), StorageError> {
        Ok(())
    }

    fn write_snapshot(&self, _snapshot: &EngineSnapshot) -> Result<(), StorageError> {
        Ok(())
    }

    fn load_snapshot(&self) -> Result<Option<EngineSnapshot>, StorageError> {
        Ok(None)
    }

    fn load_wal_after(&self, _after_seq: u64) -> Result<Vec<(u64, Command)>, StorageError> {
        Ok(vec![])
    }

    fn compact_wal(&self, _up_to_seq: u64) -> Result<(), StorageError> {
        Ok(())
    }

    fn append_job_event(
        &self,
        _job_id: Uuid,
        _event: &StoredJobEvent,
    ) -> Result<(), StorageError> {
        Ok(())
    }

    fn query_job_events(
        &self,
        _job_id: Uuid,
        _limit: usize,
        _cursor: Option<EventCursor>,
        _kind_filter: Option<&[String]>,
    ) -> Result<Vec<StoredJobEvent>, StorageError> {
        Ok(vec![])
    }

    fn is_persistent(&self) -> bool {
        false
    }
}
