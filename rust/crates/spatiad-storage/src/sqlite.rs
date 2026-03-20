use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection};
use uuid::Uuid;

use crate::{Command, EngineSnapshot, EventCursor, StorageBackend, StorageError, StoredJobEvent};

pub struct SqliteBackend {
    conn: Mutex<Connection>,
}

impl SqliteBackend {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, StorageError> {
        let conn =
            Connection::open(path).map_err(|e| StorageError::WalAppend(e.to_string()))?;

        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA foreign_keys=ON;",
        )
        .map_err(|e| StorageError::WalAppend(e.to_string()))?;

        let backend = Self { conn: Mutex::new(conn) };
        backend.migrate()?;
        Ok(backend)
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().expect("sqlite mutex poisoned")
    }

    fn migrate(&self) -> Result<(), StorageError> {
        self.conn()
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS schema_version (
                    version INTEGER PRIMARY KEY
                );

                CREATE TABLE IF NOT EXISTS wal (
                    seq       INTEGER PRIMARY KEY,
                    command   BLOB NOT NULL,
                    written_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f', 'now'))
                );

                CREATE TABLE IF NOT EXISTS snapshots (
                    seq        INTEGER PRIMARY KEY,
                    data       BLOB NOT NULL,
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f', 'now'))
                );

                CREATE TABLE IF NOT EXISTS job_events (
                    job_id      TEXT NOT NULL,
                    sequence    INTEGER NOT NULL,
                    occurred_at TEXT NOT NULL,
                    kind_json   TEXT NOT NULL,
                    PRIMARY KEY (job_id, sequence)
                );

                CREATE INDEX IF NOT EXISTS idx_job_events_cursor
                    ON job_events (job_id, occurred_at DESC, sequence DESC);

                INSERT OR IGNORE INTO schema_version (version) VALUES (1);",
            )
            .map_err(|e| StorageError::WalAppend(e.to_string()))?;

        Ok(())
    }
}

impl StorageBackend for SqliteBackend {
    fn append_wal(&self, seq: u64, cmd: &Command) -> Result<(), StorageError> {
        let blob =
            bincode::serialize(cmd).map_err(|e| StorageError::WalAppend(e.to_string()))?;

        self.conn()
            .execute("INSERT INTO wal (seq, command) VALUES (?1, ?2)", params![seq as i64, blob])
            .map_err(|e| StorageError::WalAppend(e.to_string()))?;

        Ok(())
    }

    fn write_snapshot(&self, snapshot: &EngineSnapshot) -> Result<(), StorageError> {
        let blob = bincode::serialize(snapshot)
            .map_err(|e| StorageError::SnapshotWrite(e.to_string()))?;

        self.conn()
            .execute(
                "INSERT OR REPLACE INTO snapshots (seq, data) VALUES (?1, ?2)",
                params![snapshot.wal_sequence as i64, blob],
            )
            .map_err(|e| StorageError::SnapshotWrite(e.to_string()))?;

        // Keep only the latest 2 snapshots
        self.conn()
            .execute(
                "DELETE FROM snapshots WHERE seq NOT IN (
                    SELECT seq FROM snapshots ORDER BY seq DESC LIMIT 2
                )",
                [],
            )
            .map_err(|e| StorageError::SnapshotWrite(e.to_string()))?;

        Ok(())
    }

    fn load_snapshot(&self) -> Result<Option<EngineSnapshot>, StorageError> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT data FROM snapshots ORDER BY seq DESC LIMIT 1")
            .map_err(|e| StorageError::SnapshotLoad(e.to_string()))?;

        let result: Option<Vec<u8>> = stmt
            .query_row([], |row| row.get(0))
            .ok();

        match result {
            Some(blob) => {
                let snapshot: EngineSnapshot = bincode::deserialize(&blob)
                    .map_err(|e| StorageError::SnapshotLoad(e.to_string()))?;
                Ok(Some(snapshot))
            }
            None => Ok(None),
        }
    }

    fn load_wal_after(&self, after_seq: u64) -> Result<Vec<(u64, Command)>, StorageError> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT seq, command FROM wal WHERE seq > ?1 ORDER BY seq ASC")
            .map_err(|e| StorageError::WalLoad(e.to_string()))?;

        let rows = stmt
            .query_map(params![after_seq as i64], |row| {
                let seq: i64 = row.get(0)?;
                let blob: Vec<u8> = row.get(1)?;
                Ok((seq as u64, blob))
            })
            .map_err(|e| StorageError::WalLoad(e.to_string()))?;

        let mut entries = Vec::new();
        for row in rows {
            let (seq, blob) = row.map_err(|e| StorageError::WalLoad(e.to_string()))?;
            let cmd: Command = bincode::deserialize(&blob)
                .map_err(|e| StorageError::WalLoad(e.to_string()))?;
            entries.push((seq, cmd));
        }

        Ok(entries)
    }

    fn compact_wal(&self, up_to_seq: u64) -> Result<(), StorageError> {
        self.conn()
            .execute(
                "DELETE FROM wal WHERE seq <= ?1",
                params![up_to_seq as i64],
            )
            .map_err(|e| StorageError::WalAppend(e.to_string()))?;
        Ok(())
    }

    fn append_job_event(
        &self,
        job_id: Uuid,
        event: &StoredJobEvent,
    ) -> Result<(), StorageError> {
        self.conn()
            .execute(
                "INSERT INTO job_events (job_id, sequence, occurred_at, kind_json)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    job_id.to_string(),
                    event.sequence as i64,
                    event.occurred_at.to_rfc3339(),
                    event.kind_json,
                ],
            )
            .map_err(|e| StorageError::EventStore(e.to_string()))?;
        Ok(())
    }

    fn query_job_events(
        &self,
        job_id: Uuid,
        limit: usize,
        cursor: Option<EventCursor>,
        _kind_filter: Option<&[String]>,
    ) -> Result<Vec<StoredJobEvent>, StorageError> {
        let conn = self.conn();
        let job_id_str = job_id.to_string();
        let max_items = if limit == 0 { 50 } else { limit };

        // Kind filtering is done in-memory after fetch to keep SQL simple.
        // The volume per job is bounded so this is acceptable.
        let rows = if let Some(c) = cursor {
            let cursor_ts = c.occurred_at.to_rfc3339();
            let cursor_seq = c.sequence as i64;
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT sequence, occurred_at, kind_json FROM job_events
                     WHERE job_id = ?1
                       AND (occurred_at < ?2 OR (occurred_at = ?2 AND sequence < ?3))
                     ORDER BY occurred_at DESC, sequence DESC
                     LIMIT {}",
                    max_items
                ))
                .map_err(|e| StorageError::EventStore(e.to_string()))?;
            Self::collect_event_rows(&mut stmt, params![job_id_str, cursor_ts, cursor_seq])?
        } else {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT sequence, occurred_at, kind_json FROM job_events
                     WHERE job_id = ?1
                     ORDER BY occurred_at DESC, sequence DESC
                     LIMIT {}",
                    max_items
                ))
                .map_err(|e| StorageError::EventStore(e.to_string()))?;
            Self::collect_event_rows(&mut stmt, params![job_id_str])?
        };

        Ok(rows)
    }

    fn is_persistent(&self) -> bool {
        true
    }
}

impl SqliteBackend {
    fn collect_event_rows(
        stmt: &mut rusqlite::Statement,
        params: &[&dyn rusqlite::types::ToSql],
    ) -> Result<Vec<StoredJobEvent>, StorageError> {
        let rows = stmt
            .query_map(params, |row| {
                let seq: i64 = row.get(0)?;
                let ts_str: String = row.get(1)?;
                let kind_json: String = row.get(2)?;
                Ok((seq, ts_str, kind_json))
            })
            .map_err(|e| StorageError::EventStore(e.to_string()))?;

        let mut events = Vec::new();
        for row in rows {
            let (seq, ts_str, kind_json) =
                row.map_err(|e| StorageError::EventStore(e.to_string()))?;
            let occurred_at = chrono::DateTime::parse_from_rfc3339(&ts_str)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .unwrap_or_else(|_| chrono::Utc::now());
            events.push(StoredJobEvent {
                sequence: seq as u64,
                occurred_at,
                kind_json,
            });
        }
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use spatiad_types::{Coordinates, DriverStatus, JobRequest};
    use std::collections::{HashMap, HashSet};

    fn temp_backend() -> SqliteBackend {
        SqliteBackend::open(":memory:").expect("in-memory sqlite should open")
    }

    #[test]
    fn wal_append_and_load() {
        let backend = temp_backend();
        let cmd = Command::RegisterJob {
            job: JobRequest {
                job_id: Uuid::new_v4(),
                category: "tow_truck".to_string(),
                pickup: Coordinates { latitude: 38.0, longitude: 26.0 },
                dropoff: None,
                initial_radius_km: 1.0,
                max_radius_km: 5.0,
                timeout_seconds: 30,
                created_at: Utc::now(),
            },
        };

        backend.append_wal(1, &cmd).unwrap();
        backend.append_wal(2, &cmd).unwrap();

        let entries = backend.load_wal_after(0).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, 1);
        assert_eq!(entries[1].0, 2);

        let after_first = backend.load_wal_after(1).unwrap();
        assert_eq!(after_first.len(), 1);
        assert_eq!(after_first[0].0, 2);
    }

    #[test]
    fn snapshot_write_and_load() {
        let backend = temp_backend();

        let snapshot = EngineSnapshot {
            h3_resolution: 8,
            drivers: HashMap::new(),
            jobs: HashMap::new(),
            offers: HashMap::new(),
            cancelled_jobs: HashSet::new(),
            job_event_sequences: HashMap::new(),
            wal_sequence: 42,
        };

        backend.write_snapshot(&snapshot).unwrap();
        let loaded = backend.load_snapshot().unwrap().expect("snapshot should exist");
        assert_eq!(loaded.wal_sequence, 42);
        assert_eq!(loaded.h3_resolution, 8);
    }

    #[test]
    fn wal_compaction() {
        let backend = temp_backend();
        let cmd = Command::CancelJob { job_id: Uuid::new_v4() };

        for i in 1..=5 {
            backend.append_wal(i, &cmd).unwrap();
        }

        backend.compact_wal(3).unwrap();
        let remaining = backend.load_wal_after(0).unwrap();
        assert_eq!(remaining.len(), 2);
        assert_eq!(remaining[0].0, 4);
        assert_eq!(remaining[1].0, 5);
    }

    #[test]
    fn job_events_append_and_query() {
        let backend = temp_backend();
        let job_id = Uuid::new_v4();

        for i in 1..=5 {
            backend
                .append_job_event(
                    job_id,
                    &StoredJobEvent {
                        sequence: i,
                        occurred_at: Utc::now(),
                        kind_json: format!("\"event_{}\"", i),
                    },
                )
                .unwrap();
        }

        let events = backend.query_job_events(job_id, 3, None, None).unwrap();
        assert_eq!(events.len(), 3);
        // newest first
        assert_eq!(events[0].sequence, 5);
        assert_eq!(events[1].sequence, 4);
        assert_eq!(events[2].sequence, 3);
    }

    #[test]
    fn job_events_cursor_pagination() {
        let backend = temp_backend();
        let job_id = Uuid::new_v4();
        let base_time = Utc::now();

        for i in 1..=5u64 {
            backend
                .append_job_event(
                    job_id,
                    &StoredJobEvent {
                        sequence: i,
                        occurred_at: base_time + chrono::Duration::milliseconds(i as i64),
                        kind_json: "\"test\"".to_string(),
                    },
                )
                .unwrap();
        }

        let page1 = backend.query_job_events(job_id, 2, None, None).unwrap();
        assert_eq!(page1.len(), 2);

        let cursor = EventCursor {
            occurred_at: page1.last().unwrap().occurred_at,
            sequence: page1.last().unwrap().sequence,
        };

        let page2 = backend.query_job_events(job_id, 10, Some(cursor), None).unwrap();
        assert_eq!(page2.len(), 3);
    }

    #[test]
    fn is_persistent_returns_true() {
        let backend = temp_backend();
        assert!(backend.is_persistent());
    }

    #[test]
    fn all_command_variants_serialize() {
        let commands = vec![
            Command::UpsertDriverLocation {
                driver_id: Uuid::new_v4(),
                category: "tow_truck".to_string(),
                position: Coordinates { latitude: 38.0, longitude: 26.0 },
                status: DriverStatus::Available,
                timestamp: Utc::now(),
            },
            Command::RegisterJob {
                job: JobRequest {
                    job_id: Uuid::new_v4(),
                    category: "tow_truck".to_string(),
                    pickup: Coordinates { latitude: 38.0, longitude: 26.0 },
                    dropoff: None,
                    initial_radius_km: 1.0,
                    max_radius_km: 5.0,
                    timeout_seconds: 30,
                    created_at: Utc::now(),
                },
            },
            Command::CreateOffer {
                offer: spatiad_types::OfferRecord {
                    offer_id: Uuid::new_v4(),
                    job_id: Uuid::new_v4(),
                    driver_id: Uuid::new_v4(),
                    status: spatiad_types::OfferStatus::Pending,
                    expires_at: Utc::now(),
                },
            },
            Command::AcceptOffer { offer_id: Uuid::new_v4(), responded_at: Utc::now() },
            Command::RejectOffer { offer_id: Uuid::new_v4(), responded_at: Utc::now() },
            Command::MarkOfferExpired { offer_id: Uuid::new_v4() },
            Command::MarkOfferCancelled { offer_id: Uuid::new_v4() },
            Command::CancelJob { job_id: Uuid::new_v4() },
            Command::RemoveDriver { driver_id: Uuid::new_v4() },
            Command::RecordWebhookDeliveryFailed {
                job_id: Uuid::new_v4(),
                offer_id: Uuid::new_v4(),
            },
        ];

        let backend = temp_backend();
        for (i, cmd) in commands.iter().enumerate() {
            backend.append_wal((i + 1) as u64, cmd).unwrap();
        }

        let loaded = backend.load_wal_after(0).unwrap();
        assert_eq!(loaded.len(), commands.len());
    }
}
