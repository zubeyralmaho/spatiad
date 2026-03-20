use std::sync::Mutex;

use postgres::{Client, NoTls};
use uuid::Uuid;

use crate::{Command, EngineSnapshot, EventCursor, StorageBackend, StorageError, StoredJobEvent};

pub struct PostgresBackend {
    conn: Mutex<Client>,
}

impl PostgresBackend {
    /// Connect to a PostgreSQL database and run migrations.
    ///
    /// `url` is a standard connection string, e.g.
    /// `host=localhost user=spatiad dbname=spatiad` or
    /// `postgresql://spatiad:pass@localhost/spatiad`.
    pub fn open(url: &str) -> Result<Self, StorageError> {
        let client =
            Client::connect(url, NoTls).map_err(|e| StorageError::WalAppend(e.to_string()))?;

        let backend = Self {
            conn: Mutex::new(client),
        };
        backend.migrate()?;
        Ok(backend)
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Client> {
        self.conn.lock().expect("postgres mutex poisoned")
    }

    fn migrate(&self) -> Result<(), StorageError> {
        let mut conn = self.conn();
        conn.batch_execute(
            "CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER PRIMARY KEY
            );

            CREATE TABLE IF NOT EXISTS wal (
                seq       BIGINT PRIMARY KEY,
                command   BYTEA NOT NULL,
                written_at TIMESTAMPTZ NOT NULL DEFAULT now()
            );

            CREATE TABLE IF NOT EXISTS snapshots (
                seq        BIGINT PRIMARY KEY,
                data       BYTEA NOT NULL,
                created_at TIMESTAMPTZ NOT NULL DEFAULT now()
            );

            CREATE TABLE IF NOT EXISTS job_events (
                job_id      TEXT NOT NULL,
                sequence    BIGINT NOT NULL,
                occurred_at TIMESTAMPTZ NOT NULL,
                kind_json   TEXT NOT NULL,
                PRIMARY KEY (job_id, sequence)
            );

            CREATE INDEX IF NOT EXISTS idx_job_events_cursor
                ON job_events (job_id, occurred_at DESC, sequence DESC);

            INSERT INTO schema_version (version) VALUES (1)
                ON CONFLICT (version) DO NOTHING;",
        )
        .map_err(|e| StorageError::WalAppend(e.to_string()))?;

        Ok(())
    }
}

impl StorageBackend for PostgresBackend {
    fn append_wal(&self, seq: u64, cmd: &Command) -> Result<(), StorageError> {
        let blob =
            bincode::serialize(cmd).map_err(|e| StorageError::WalAppend(e.to_string()))?;

        let mut conn = self.conn();
        conn.execute(
            "INSERT INTO wal (seq, command) VALUES ($1, $2)",
            &[&(seq as i64), &blob],
        )
        .map_err(|e| StorageError::WalAppend(e.to_string()))?;

        Ok(())
    }

    fn write_snapshot(&self, snapshot: &EngineSnapshot) -> Result<(), StorageError> {
        let blob = bincode::serialize(snapshot)
            .map_err(|e| StorageError::SnapshotWrite(e.to_string()))?;

        let mut conn = self.conn();

        conn.execute(
            "INSERT INTO snapshots (seq, data) VALUES ($1, $2)
             ON CONFLICT (seq) DO UPDATE SET data = EXCLUDED.data, created_at = now()",
            &[&(snapshot.wal_sequence as i64), &blob],
        )
        .map_err(|e| StorageError::SnapshotWrite(e.to_string()))?;

        // Keep only the latest 2 snapshots.
        conn.execute(
            "DELETE FROM snapshots WHERE seq NOT IN (
                SELECT seq FROM snapshots ORDER BY seq DESC LIMIT 2
            )",
            &[],
        )
        .map_err(|e| StorageError::SnapshotWrite(e.to_string()))?;

        Ok(())
    }

    fn load_snapshot(&self) -> Result<Option<EngineSnapshot>, StorageError> {
        let mut conn = self.conn();
        let rows = conn
            .query(
                "SELECT data FROM snapshots ORDER BY seq DESC LIMIT 1",
                &[],
            )
            .map_err(|e| StorageError::SnapshotLoad(e.to_string()))?;

        match rows.first() {
            Some(row) => {
                let blob: Vec<u8> = row.get(0);
                let snapshot: EngineSnapshot = bincode::deserialize(&blob)
                    .map_err(|e| StorageError::SnapshotLoad(e.to_string()))?;
                Ok(Some(snapshot))
            }
            None => Ok(None),
        }
    }

    fn load_wal_after(&self, after_seq: u64) -> Result<Vec<(u64, Command)>, StorageError> {
        let mut conn = self.conn();
        let rows = conn
            .query(
                "SELECT seq, command FROM wal WHERE seq > $1 ORDER BY seq ASC",
                &[&(after_seq as i64)],
            )
            .map_err(|e| StorageError::WalLoad(e.to_string()))?;

        let mut entries = Vec::new();
        for row in &rows {
            let seq: i64 = row.get(0);
            let blob: Vec<u8> = row.get(1);
            let cmd: Command = bincode::deserialize(&blob)
                .map_err(|e| StorageError::WalLoad(e.to_string()))?;
            entries.push((seq as u64, cmd));
        }

        Ok(entries)
    }

    fn compact_wal(&self, up_to_seq: u64) -> Result<(), StorageError> {
        let mut conn = self.conn();
        conn.execute(
            "DELETE FROM wal WHERE seq <= $1",
            &[&(up_to_seq as i64)],
        )
        .map_err(|e| StorageError::WalAppend(e.to_string()))?;
        Ok(())
    }

    fn append_job_event(
        &self,
        job_id: Uuid,
        event: &StoredJobEvent,
    ) -> Result<(), StorageError> {
        let mut conn = self.conn();
        conn.execute(
            "INSERT INTO job_events (job_id, sequence, occurred_at, kind_json)
             VALUES ($1, $2, $3, $4)",
            &[
                &job_id.to_string(),
                &(event.sequence as i64),
                &event.occurred_at,
                &event.kind_json,
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
        let mut conn = self.conn();
        let job_id_str = job_id.to_string();
        let max_items = if limit == 0 { 50 } else { limit } as i64;

        let rows = if let Some(c) = cursor {
            let cursor_seq = c.sequence as i64;
            conn.query(
                "SELECT sequence, occurred_at, kind_json FROM job_events
                 WHERE job_id = $1
                   AND (occurred_at < $2 OR (occurred_at = $2 AND sequence < $3))
                 ORDER BY occurred_at DESC, sequence DESC
                 LIMIT $4",
                &[&job_id_str, &c.occurred_at, &cursor_seq, &max_items],
            )
            .map_err(|e| StorageError::EventStore(e.to_string()))?
        } else {
            conn.query(
                "SELECT sequence, occurred_at, kind_json FROM job_events
                 WHERE job_id = $1
                 ORDER BY occurred_at DESC, sequence DESC
                 LIMIT $2",
                &[&job_id_str, &max_items],
            )
            .map_err(|e| StorageError::EventStore(e.to_string()))?
        };

        let mut events = Vec::new();
        for row in &rows {
            let seq: i64 = row.get(0);
            let occurred_at: chrono::DateTime<chrono::Utc> = row.get(1);
            let kind_json: String = row.get(2);
            events.push(StoredJobEvent {
                sequence: seq as u64,
                occurred_at,
                kind_json,
            });
        }

        Ok(events)
    }

    fn is_persistent(&self) -> bool {
        true
    }
}
