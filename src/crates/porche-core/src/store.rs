//! The local desired-state table (I-12): the verbs are writers on this table; the P0
//! control plane replaces the CLI writer, not this store (doc 07 §2 host-agent row;
//! store decision = SQLite, doc 12 §8 / E-27).

use crate::spec::{WorkloadSpec, WorkloadState};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("spec json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("workload not found: {0}")]
    NotFound(String),
}

/// One row: the spec plus its current state.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub spec: WorkloadSpec,
    pub state: WorkloadState,
    pub created_at: i64,
    pub updated_at: i64,
}

/// The desired-state table. Everything the verbs do is a transition on this table
/// plus convergence through the [`crate::Executor`] — the shape P0 keeps.
pub trait DesiredStateStore: Send {
    fn upsert(&self, spec: &WorkloadSpec, state: &WorkloadState) -> Result<(), StoreError>;
    fn get(&self, id: &str) -> Result<Option<Record>, StoreError>;
    fn list(&self) -> Result<Vec<Record>, StoreError>;
    fn set_state(&self, id: &str, state: &WorkloadState) -> Result<(), StoreError>;
    fn delete(&self, id: &str) -> Result<(), StoreError>;
}

pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    /// In-memory table — unit tests.
    pub fn in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> Result<Self, StoreError> {
        // timeout FIRST: two console surfaces (or CLI + console) may init the
        // same store simultaneously — the WAL pragma below is a write and
        // would otherwise race with "database is locked" (observed on dev)
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS workloads (
                 id         TEXT PRIMARY KEY,
                 spec_json  TEXT NOT NULL,
                 state_json TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL
             );",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<Record> {
        let spec_json: String = row.get(0)?;
        let state_json: String = row.get(1)?;
        let json_err = |e: serde_json::Error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Null,
                Box::new(e),
            )
        };
        Ok(Record {
            spec: serde_json::from_str(&spec_json).map_err(json_err)?,
            state: serde_json::from_str(&state_json).map_err(json_err)?,
            created_at: row.get(2)?,
            updated_at: row.get(3)?,
        })
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64
}

impl DesiredStateStore for SqliteStore {
    fn upsert(&self, spec: &WorkloadSpec, state: &WorkloadState) -> Result<(), StoreError> {
        let spec_json = serde_json::to_string(spec)?;
        let state_json = serde_json::to_string(state)?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO workloads (id, spec_json, state_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(id) DO UPDATE SET
                 spec_json  = excluded.spec_json,
                 state_json = excluded.state_json,
                 updated_at = excluded.updated_at;",
            params![spec.id, spec_json, state_json, now()],
        )?;
        Ok(())
    }

    fn get(&self, id: &str) -> Result<Option<Record>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let row = conn
            .query_row(
                "SELECT spec_json, state_json, created_at, updated_at
                 FROM workloads WHERE id = ?1;",
                params![id],
                Self::row_to_record,
            )
            .optional()?;
        Ok(row)
    }

    fn list(&self) -> Result<Vec<Record>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT spec_json, state_json, created_at, updated_at
             FROM workloads ORDER BY id;",
        )?;
        let rows = stmt.query_map([], Self::row_to_record)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    fn set_state(&self, id: &str, state: &WorkloadState) -> Result<(), StoreError> {
        let state_json = serde_json::to_string(state)?;
        let conn = self.conn.lock().unwrap();
        let changed = conn.execute(
            "UPDATE workloads SET state_json = ?2, updated_at = ?3 WHERE id = ?1;",
            params![id, state_json, now()],
        )?;
        if changed == 0 {
            return Err(StoreError::NotFound(id.into()));
        }
        Ok(())
    }

    fn delete(&self, id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let changed = conn.execute("DELETE FROM workloads WHERE id = ?1;", params![id])?;
        if changed == 0 {
            return Err(StoreError::NotFound(id.into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::VolumeSpec;

    fn spec(id: &str) -> WorkloadSpec {
        WorkloadSpec {
            id: id.into(),
            image_digest: format!("sha256:{}", "b".repeat(64)),
            command: "true".into(),
            vcpu: 1,
            ram_mib: 64,
            volume: Some(VolumeSpec { size_mib: 8 }),
            ip6: "fd00::1".into(),
            gw6: "fd00::ff".into(),
        }
    }

    #[test]
    fn upsert_get_roundtrip() {
        let store = SqliteStore::in_memory().unwrap();
        let s = spec("w1");
        store.upsert(&s, &WorkloadState::Creating).unwrap();
        let rec = store.get("w1").unwrap().unwrap();
        assert_eq!(rec.spec, s);
        assert_eq!(rec.state, WorkloadState::Creating);
    }

    #[test]
    fn upsert_is_idempotent_on_id() {
        let store = SqliteStore::in_memory().unwrap();
        store.upsert(&spec("w1"), &WorkloadState::Creating).unwrap();
        store
            .upsert(&spec("w1"), &WorkloadState::Running)
            .unwrap();
        let all = store.list().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].state, WorkloadState::Running);
    }

    #[test]
    fn set_state_persists() {
        let store = SqliteStore::in_memory().unwrap();
        store.upsert(&spec("w1"), &WorkloadState::Creating).unwrap();
        store.set_state("w1", &WorkloadState::Running).unwrap();
        assert_eq!(store.get("w1").unwrap().unwrap().state, WorkloadState::Running);
    }

    #[test]
    fn set_state_missing_is_not_found() {
        let store = SqliteStore::in_memory().unwrap();
        assert!(matches!(
            store.set_state("nope", &WorkloadState::Running),
            Err(StoreError::NotFound(_))
        ));
    }

    #[test]
    fn delete_removes() {
        let store = SqliteStore::in_memory().unwrap();
        store.upsert(&spec("w1"), &WorkloadState::Running).unwrap();
        store.delete("w1").unwrap();
        assert!(store.get("w1").unwrap().is_none());
    }

    #[test]
    fn file_backed_store_survives_reopen() {
        let path = std::env::temp_dir().join(format!(
            "porche-store-test-{}.sqlite3",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        {
            let store = SqliteStore::open(&path).unwrap();
            store.upsert(&spec("w1"), &WorkloadState::Running).unwrap();
        }
        let store = SqliteStore::open(&path).unwrap();
        assert_eq!(
            store.get("w1").unwrap().unwrap().state,
            WorkloadState::Running
        );
        let _ = std::fs::remove_file(&path);
    }
}
