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

/// Admission quota for one tenant (FR-5 skeleton, MS-2.M3): vCPU/RAM/count
/// ceilings. Enforced **atomically with the row insert** — the create path's
/// check-then-act window is closed here, not in the caller (doc 15, 2026-10-23
/// code finding: duplicate + quota checks outside the store lock raced).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotaLimits {
    pub workloads: u32,
    pub vcpu: u32,
    pub ram_mib: u32,
}

/// The outcome of a [`DesiredStateStore::admit_create`] attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmitOutcome {
    /// Row inserted with the given state.
    Admitted,
    /// A row with this id exists (tenant-global: ids are host-global device
    /// names — two tenants cannot both claim netns w-foo). The existing row
    /// is untouched — admission never replaces.
    DuplicateId,
    /// The tenant's quota would be exceeded; the reason names the ceiling
    /// (same wording as the MS-2.M3 web gate).
    OverQuota(String),
}

/// The desired-state table. Everything the verbs do is a transition on this table
/// plus convergence through the [`crate::Executor`] — the shape P0 keeps.
pub trait DesiredStateStore: Send {
    fn upsert(&self, spec: &WorkloadSpec, state: &WorkloadState) -> Result<(), StoreError>;
    fn get(&self, id: &str) -> Result<Option<Record>, StoreError>;
    fn list(&self) -> Result<Vec<Record>, StoreError>;
    fn set_state(&self, id: &str, state: &WorkloadState) -> Result<(), StoreError>;
    fn delete(&self, id: &str) -> Result<(), StoreError>;

    /// Atomic create admission: insert `spec`/`state` only if the id is
    /// absent and the tenant is within `quota` (None = unquota'd tenant —
    /// the operator/CLI path). Duplicate check, quota accounting and the
    /// insert share **one critical section**, so two concurrent creates can
    /// never both pass. A cross-process writer racing us fails the INSERT's
    /// primary-key constraint — an honest rejection, never a silent replace.
    fn admit_create(
        &self,
        spec: &WorkloadSpec,
        state: &WorkloadState,
        quota: Option<&QuotaLimits>,
    ) -> Result<AdmitOutcome, StoreError>;

    // --- MS-2 tenancy scoping (E-31.2) -----------------------------------
    /// Tenant-filtered list — the user surface's only view (view isolation:
    /// a tenant sees exactly its own rows).
    fn list_for_tenant(&self, tenant_id: &str) -> Result<Vec<Record>, StoreError> {
        Ok(self
            .list()?
            .into_iter()
            .filter(|r| r.spec.tenant_id == tenant_id)
            .collect())
    }
    /// Tenant-scoped get — cross-tenant ids must look absent (404, not a
    /// 403 leak that the object exists).
    fn get_for_tenant(
        &self,
        tenant_id: &str,
        id: &str,
    ) -> Result<Option<Record>, StoreError> {
        Ok(self.get(id)?.filter(|r| r.spec.tenant_id == tenant_id))
    }
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

    fn admit_create(
        &self,
        spec: &WorkloadSpec,
        state: &WorkloadState,
        quota: Option<&QuotaLimits>,
    ) -> Result<AdmitOutcome, StoreError> {
        let spec_json = serde_json::to_string(spec)?;
        let state_json = serde_json::to_string(state)?;
        // one critical section: duplicate check → quota accounting → insert
        let conn = self.conn.lock().unwrap();

        // duplicate first (tenant-global: ids are host-global device names)
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM workloads WHERE id = ?1);",
                params![spec.id],
                |r| r.get(0),
            )
            .unwrap_or(false);
        if exists {
            return Ok(AdmitOutcome::DuplicateId);
        }

        if let Some(q) = quota {
            // tenant usage inside the same critical section — same accounting
            // as the MS-2.M3 web gate: every row of the tenant, any state
            let mut stmt =
                conn.prepare("SELECT spec_json, state_json, created_at, updated_at FROM workloads;")?;
            let rows = stmt.query_map([], Self::row_to_record)?;
            let mut count: u32 = 0;
            let mut cpu: u32 = 0;
            let mut ram: u32 = 0;
            for row in rows {
                let r = row?;
                if r.spec.tenant_id == spec.tenant_id {
                    count += 1;
                    cpu += r.spec.vcpu;
                    ram += r.spec.ram_mib;
                }
            }
            if count + 1 > q.workloads {
                return Ok(AdmitOutcome::OverQuota(format!(
                    "quota: tenant {} workload count {} would exceed {}",
                    spec.tenant_id,
                    count + 1,
                    q.workloads
                )));
            }
            if cpu + spec.vcpu > q.vcpu {
                return Ok(AdmitOutcome::OverQuota(format!(
                    "quota: tenant {} vcpu {}+{} would exceed {}",
                    spec.tenant_id, cpu, spec.vcpu, q.vcpu
                )));
            }
            if ram + spec.ram_mib > q.ram_mib {
                return Ok(AdmitOutcome::OverQuota(format!(
                    "quota: tenant {} ram {}+{} MiB would exceed {} MiB",
                    spec.tenant_id, ram, spec.ram_mib, q.ram_mib
                )));
            }
        }

        conn.execute(
            "INSERT INTO workloads (id, spec_json, state_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4);",
            params![spec.id, spec_json, state_json, now()],
        )?;
        Ok(AdmitOutcome::Admitted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::VolumeSpec;

    fn spec(id: &str) -> WorkloadSpec {
        WorkloadSpec {
            tenant_id: crate::spec::default_tenant(),
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

    fn spec_tenant(tenant: &str, id: &str) -> WorkloadSpec {
        WorkloadSpec {
            tenant_id: tenant.into(),
            ..spec(id)
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

    // MS-2: view isolation — a tenant lists only its rows; a cross-tenant
    // get looks absent (never a leak that the object exists).
    #[test]
    fn tenant_scoping_isolates_views() {
        let store = SqliteStore::in_memory().unwrap();
        store
            .upsert(&spec_tenant("t0", "w1"), &WorkloadState::Running)
            .unwrap();
        store
            .upsert(&spec_tenant("t1", "w2"), &WorkloadState::Running)
            .unwrap();

        let t0 = store.list_for_tenant("t0").unwrap();
        assert_eq!(t0.len(), 1);
        assert_eq!(t0[0].spec.id, "w1");

        // t0 asking for t1's workload → None (404, not 403)
        assert!(store.get_for_tenant("t0", "w2").unwrap().is_none());
        // own row still resolves
        assert!(store.get_for_tenant("t0", "w1").unwrap().is_some());
    }

    // pre-MS-2 rows (no tenant_id in spec_json) default to t0 on read
    #[test]
    fn legacy_rows_default_to_t0() {
        let store = SqliteStore::in_memory().unwrap();
        let _ = spec("w1");
        let legacy_json = serde_json::to_string(&serde_json::json!({
            "id": "legacy1",
            "image_digest": format!("sha256:{}", "b".repeat(64)),
            "command": "true",
            "vcpu": 1,
            "ram_mib": 64,
            "volume": { "size_mib": 8 },
            "ip6": "fd00::1",
            "gw6": "fd00::ff"
        }))
        .unwrap();
        let state_json = serde_json::to_string(&WorkloadState::Running).unwrap();
        let conn = store.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO workloads (id, spec_json, state_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, 1, 1)",
            params!["legacy1", legacy_json, state_json],
        )
        .unwrap();
        drop(conn);
        let rec = store.get("legacy1").unwrap().unwrap();
        assert_eq!(rec.spec.tenant_id, "t0");
    }

    #[test]
    fn delete_removes() {
        let store = SqliteStore::in_memory().unwrap();
        store.upsert(&spec("w1"), &WorkloadState::Running).unwrap();
        store.delete("w1").unwrap();
        assert!(store.get("w1").unwrap().is_none());
    }

    // --- admit_create: atomic admission (doc 15 create-path race, closed) ---

    #[test]
    fn admit_create_inserts_when_absent() {
        let store = SqliteStore::in_memory().unwrap();
        let out = store
            .admit_create(&spec("w1"), &WorkloadState::Creating, None)
            .unwrap();
        assert_eq!(out, AdmitOutcome::Admitted);
        let rec = store.get("w1").unwrap().unwrap();
        assert_eq!(rec.state, WorkloadState::Creating);
    }

    #[test]
    fn admit_create_duplicate_preserves_the_existing_row() {
        let store = SqliteStore::in_memory().unwrap();
        store
            .admit_create(&spec("w1"), &WorkloadState::Creating, None)
            .unwrap();
        // a second, DIFFERENT spec with the same id must not replace the row
        // (the old pre-admission upsert did — the bug the check raced around)
        let mut challenger = spec("w1");
        challenger.image_digest = format!("sha256:{}", "a".repeat(64));
        let out = store
            .admit_create(&challenger, &WorkloadState::Creating, None)
            .unwrap();
        assert_eq!(out, AdmitOutcome::DuplicateId);
        let rec = store.get("w1").unwrap().unwrap();
        assert_eq!(
            rec.spec.image_digest,
            format!("sha256:{}", "b".repeat(64)),
            "the existing row is untouched"
        );
    }

    #[test]
    fn admit_create_enforces_count_vcpu_and_ram_ceilings() {
        let store = SqliteStore::in_memory().unwrap();
        let q = QuotaLimits {
            workloads: 1,
            vcpu: 2,
            ram_mib: 128,
        };
        assert_eq!(
            store
                .admit_create(&spec("w1"), &WorkloadState::Creating, Some(&q))
                .unwrap(),
            AdmitOutcome::Admitted
        );
        // count ceiling (different id, same tenant)
        match store
            .admit_create(&spec("w2"), &WorkloadState::Creating, Some(&q))
            .unwrap()
        {
            AdmitOutcome::OverQuota(r) => assert!(r.contains("workload count"), "{r}"),
            other => panic!("expected over-quota, got {other:?}"),
        }
        // vcpu ceiling (fresh store: one 1-vcpu row, quota 1)
        let store = SqliteStore::in_memory().unwrap();
        let q = QuotaLimits {
            workloads: 10,
            vcpu: 1,
            ram_mib: 4096,
        };
        store
            .admit_create(&spec("w1"), &WorkloadState::Creating, Some(&q))
            .unwrap();
        match store
            .admit_create(&spec("w2"), &WorkloadState::Creating, Some(&q))
            .unwrap()
        {
            AdmitOutcome::OverQuota(r) => assert!(r.contains("vcpu"), "{r}"),
            other => panic!("expected over-quota, got {other:?}"),
        }
        // ram ceiling (fresh store: one 64-MiB row, quota 64)
        let store = SqliteStore::in_memory().unwrap();
        let q = QuotaLimits {
            workloads: 10,
            vcpu: 16,
            ram_mib: 64,
        };
        store
            .admit_create(&spec("w1"), &WorkloadState::Creating, Some(&q))
            .unwrap();
        match store
            .admit_create(&spec("w2"), &WorkloadState::Creating, Some(&q))
            .unwrap()
        {
            AdmitOutcome::OverQuota(r) => assert!(r.contains("ram"), "{r}"),
            other => panic!("expected over-quota, got {other:?}"),
        }
    }

    #[test]
    fn admit_create_without_quota_admits_any_tenant() {
        // None = unquota'd (operator/CLI path) — admission is id-only
        let store = SqliteStore::in_memory().unwrap();
        for i in 0..5 {
            assert_eq!(
                store
                    .admit_create(&spec_tenant("t0", &format!("w{i}")), &WorkloadState::Creating, None)
                    .unwrap(),
                AdmitOutcome::Admitted
            );
        }
    }

    // THE race regression (doc 15, 2026-10-23): concurrent creates with the
    // same id — exactly one admission wins, the rest see DuplicateId, and
    // the store holds exactly one row.
    #[test]
    fn admit_create_same_id_concurrent_exactly_one_wins() {
        let store = std::sync::Arc::new(SqliteStore::in_memory().unwrap());
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let s = std::sync::Arc::clone(&store);
                std::thread::spawn(move || {
                    let mut sp = spec(&format!("w{i}"));
                    sp.id = "race".into();
                    s.admit_create(&sp, &WorkloadState::Creating, None).unwrap()
                })
            })
            .collect();
        let mut admitted = 0;
        let mut duplicates = 0;
        for h in handles {
            match h.join().unwrap() {
                AdmitOutcome::Admitted => admitted += 1,
                AdmitOutcome::DuplicateId => duplicates += 1,
                AdmitOutcome::OverQuota(_) => panic!("no quota set — impossible"),
            }
        }
        assert_eq!(admitted, 1, "exactly one create may win the id");
        assert_eq!(duplicates, 7);
        assert_eq!(store.list().unwrap().len(), 1);
    }

    // quota accounting is scoped per tenant — t1's usage never counts
    // against t0's ceiling
    #[test]
    fn admit_create_quota_is_tenant_scoped() {
        let store = SqliteStore::in_memory().unwrap();
        let q = QuotaLimits {
            workloads: 1,
            vcpu: 1,
            ram_mib: 64,
        };
        store
            .admit_create(&spec_tenant("t0", "w1"), &WorkloadState::Creating, Some(&q))
            .unwrap();
        assert_eq!(
            store
                .admit_create(&spec_tenant("t1", "w2"), &WorkloadState::Creating, Some(&q))
                .unwrap(),
            AdmitOutcome::Admitted
        );
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
