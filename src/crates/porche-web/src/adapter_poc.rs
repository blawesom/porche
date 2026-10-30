//! The PoC `ConsoleApi` adapter (doc 12 §9 Stage 1): serves the frozen doc-09
//! method names from the desired-state store + host-agent ops **in-process**.
//! The P1 adapter swaps in tonic clients to the two planes; the UIs do not
//! change — that swap replaces this file and the one wiring point in
//! `AppState::api`.
//!
//! What lives here vs the pages: every mutation and every object read goes
//! through this adapter (visibility is enforced HERE — FR-14/SEC-16, a
//! cross-tenant id looks absent); the E-29 display derivation
//! (`done (rc=N)` from console markers + socket absence) stays console-side
//! — the seam returns domain objects, the pages derive display state.

use crate::audit::AuditLog;
use crate::console_api::{ApiError, CallCtx, ConsoleApi, ImageInfo};
use crate::state::{console_path, derive_events, read_console, EventLine};
use crate::TenantEntry;
use porche_core::{ops, AdmitOutcome, DesiredStateStore, QuotaLimits, Record, WipeReport, WorkloadSpec, WorkloadState};
use porche_host::exec::DirectExecutor;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub struct PocConsole {
    store: Arc<porche_core::SqliteStore>,
    exec: Arc<Mutex<DirectExecutor>>,
    data_dir: PathBuf,
    tenants: Vec<(String, TenantEntry)>,
    audit: Arc<Mutex<AuditLog>>,
}

impl PocConsole {
    pub fn new(
        store: Arc<porche_core::SqliteStore>,
        exec: Arc<Mutex<DirectExecutor>>,
        data_dir: PathBuf,
        tenants: Vec<(String, TenantEntry)>,
        audit: Arc<Mutex<AuditLog>>,
    ) -> Self {
        Self {
            store,
            exec,
            data_dir,
            tenants,
            audit,
        }
    }

    /// The tenant gate every object-touching method passes: a row that is
    /// not the caller's looks absent (404, never a 403 leak). Store errors
    /// read as absence — the MS-1/MS-2 behavior the UAT pins.
    fn tenant_gate(&self, ctx: &CallCtx, id: &str) -> Result<(), ApiError> {
        match self.store.get_for_tenant(&ctx.tenant_id, id) {
            Ok(Some(_)) => Ok(()),
            _ => Err(ApiError::NotFound(format!("workload not found: {id}"))),
        }
    }
}

impl ConsoleApi for PocConsole {
    fn create_workload(&self, ctx: &CallCtx, spec: WorkloadSpec) -> Result<(), ApiError> {
        // quota resolution stays with the tenant config (E-31.3); the
        // enforcement is INSIDE the store admission (doc-15 race fix):
        // duplicate-id + quota + insert share one critical section.
        let quota = self
            .tenants
            .iter()
            .find(|(tid, _)| *tid == ctx.tenant_id)
            .map(|(_, e)| QuotaLimits {
                vcpu: e.quota.vcpu,
                ram_mib: e.quota.ram_mib,
                workloads: e.quota.workloads,
            });
        match self
            .store
            .admit_create(&spec, &WorkloadState::Creating, quota.as_ref())
        {
            Ok(AdmitOutcome::Admitted) => {}
            Ok(AdmitOutcome::DuplicateId) => {
                return Err(ApiError::Rejected(format!(
                    "workload {:?} already exists",
                    spec.id
                )))
            }
            Ok(AdmitOutcome::OverQuota(reason)) => return Err(ApiError::Rejected(reason)),
            Err(e) => return Err(ApiError::Failed(format!("store: {e}"))),
        }
        // convergence in the background (MS-1 u03: CreateWorkload returns on
        // the `Creating` row; the executor is the single writer). The settle
        // entry is the audit's, same stream the planes will write in P1.
        let store = Arc::clone(&self.store);
        let exec = Arc::clone(&self.exec);
        let audit = Arc::clone(&self.audit);
        let actor = ctx.actor.clone();
        let id = spec.id.clone();
        tokio::task::spawn_blocking(move || {
            let mut ex = exec.lock().unwrap();
            let outcome = match ops::create(store.as_ref(), &mut *ex, spec) {
                Ok(()) => "ok (converged)".to_string(),
                Err(e) => format!("failed: {e}"),
            };
            audit.lock().unwrap().record(&actor, "CreateWorkload", &id, &outcome);
        });
        Ok(())
    }

    fn stop_workload(&self, ctx: &CallCtx, id: &str) -> Result<(), ApiError> {
        self.tenant_gate(ctx, id)?;
        let mut ex = self.exec.lock().unwrap();
        ops::stop(self.store.as_ref(), &mut *ex, id).map_err(|e| ApiError::Failed(e.to_string()))
    }

    fn delete_workload(&self, ctx: &CallCtx, id: &str) -> Result<WipeReport, ApiError> {
        self.tenant_gate(ctx, id)?;
        let mut ex = self.exec.lock().unwrap();
        ops::wipe(self.store.as_ref(), &mut *ex, id, &porche_core::Markers::new())
            .map_err(|e| ApiError::Failed(e.to_string()))
    }

    fn list_workloads(&self, ctx: &CallCtx) -> Result<Vec<Record>, ApiError> {
        self.store
            .list_for_tenant(&ctx.tenant_id)
            .map_err(|e| ApiError::Failed(format!("store: {e}")))
    }

    fn get_workload(&self, ctx: &CallCtx, id: &str) -> Result<Option<Record>, ApiError> {
        self.store
            .get_for_tenant(&ctx.tenant_id, id)
            .map_err(|e| ApiError::Failed(format!("store: {e}")))
    }

    fn get_console(&self, ctx: &CallCtx, id: &str) -> Result<String, ApiError> {
        // cross-tenant → the empty tail (MS-1 rule: never another tenant's
        // serial log; a 404 would leak that the id exists)
        if self
            .store
            .get_for_tenant(&ctx.tenant_id, id)
            .ok()
            .flatten()
            .is_some()
        {
            Ok(read_console(&self.data_dir, id))
        } else {
            Ok(String::new())
        }
    }

    fn get_events(&self, ctx: &CallCtx, id: &str) -> Result<Vec<EventLine>, ApiError> {
        let Some(r) = self
            .store
            .get_for_tenant(&ctx.tenant_id, id)
            .ok()
            .flatten()
        else {
            return Err(ApiError::NotFound(format!("workload not found: {id}")));
        };
        let console = read_console(&self.data_dir, id);
        let log_mtime = std::fs::metadata(console_path(&self.data_dir, id))
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64);
        Ok(derive_events(r.created_at, r.updated_at, &console, log_mtime))
    }

    fn list_volumes(&self, ctx: &CallCtx) -> Result<Vec<Record>, ApiError> {
        Ok(self
            .list_workloads(ctx)?
            .into_iter()
            .filter(|r| r.spec.volume.is_some())
            .collect())
    }

    fn delete_volume(&self, ctx: &CallCtx, id: &str) -> Result<WipeReport, ApiError> {
        // DeleteVolume goes through the workload's wipe path (the volume
        // file, its mapper and its DEK die together — never the volume
        // alone); volumes have no separate CRUD in the PoC (doc 09 §1 bet).
        self.tenant_gate(ctx, id)?;
        let mut ex = self.exec.lock().unwrap();
        ops::wipe(self.store.as_ref(), &mut *ex, id, &porche_core::Markers::new())
            .map_err(|e| ApiError::Failed(e.to_string()))
    }

    fn list_images(&self, _ctx: &CallCtx) -> Result<Vec<ImageInfo>, ApiError> {
        let mut names: Vec<String> = std::fs::read_dir(self.data_dir.join("images"))
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        Ok(names
            .into_iter()
            .map(|digest| ImageInfo {
                manifest: std::fs::read_to_string(self.data_dir.join("images").join(&digest).join("manifest.txt"))
                    .map(|s| s.lines().collect::<Vec<_>>().join(" · "))
                    .unwrap_or_default(),
                digest,
            })
            .collect())
    }
}
