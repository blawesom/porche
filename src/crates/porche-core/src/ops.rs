//! Verb orchestration — doc 07 §2: create / status / stop / wipe, shaped for
//! reconciliation (I-12): every verb is a desired-state-table transition plus a
//! convergence step through the [`Executor`].

use crate::executor::{ExecuteError, Executor, Markers, WipeReport};
use crate::spec::{WorkloadSpec, WorkloadState};
use crate::store::{DesiredStateStore, StoreError};

#[derive(Debug, thiserror::Error)]
pub enum OpsError {
    #[error("invalid spec: {0}")]
    InvalidSpec(String),
    #[error("store: {0}")]
    Store(#[from] StoreError),
    #[error("execution: {0}")]
    Execute(#[from] ExecuteError),
    #[error("workload not found: {0}")]
    NotFound(String),
    #[error("workload {0} is {1:?}, expected {2}")]
    WrongState(String, WorkloadState, &'static str),
}

/// create: validate → Creating → converge (netns → volume → VM) → Running.
/// On convergence failure the whole thing is torn down (kill → unmap → wipe) —
/// nothing half-built survives, and volume material gets wipe semantics even on the
/// failure path (E-6).
pub fn create<S: DesiredStateStore, E: Executor>(
    store: &S,
    exec: &mut E,
    spec: WorkloadSpec,
) -> Result<(), OpsError> {
    spec.validate().map_err(OpsError::InvalidSpec)?;
    store.upsert(&spec, &WorkloadState::Creating)?;

    let converged = exec
        .setup_netns(&spec)
        .and_then(|()| exec.create_volume(&spec))
        .and_then(|()| exec.start_vm(&spec));

    match converged {
        Ok(()) => {
            store.set_state(&spec.id, &WorkloadState::Running)?;
            Ok(())
        }
        Err(e) => {
            // Best-effort full teardown: the volume may exist, so the wipe path —
            // not just cleanup — is the correct rollback (E-6).
            let rollback = exec.teardown(&spec.id, &Markers::new());
            let reason = match rollback {
                Ok(report) if !report.all_pass() => {
                    format!("{e}; rollback wipe incomplete: {report:?}")
                }
                Err(re) => format!("{e}; rollback teardown failed: {re}"),
                _ => e.to_string(),
            };
            store.set_state(&spec.id, &WorkloadState::Failed { reason })?;
            Err(e.into())
        }
    }
}

/// stop: Running → Stopping → VM down → Stopped. Volume and netns are kept.
pub fn stop<S: DesiredStateStore, E: Executor>(
    store: &S,
    exec: &mut E,
    id: &str,
) -> Result<(), OpsError> {
    let record = store.get(id)?.ok_or_else(|| OpsError::NotFound(id.into()))?;
    if record.state != WorkloadState::Running {
        return Err(OpsError::WrongState(
            id.into(),
            record.state,
            "running",
        ));
    }
    store.set_state(id, &WorkloadState::Stopping)?;
    exec.stop_vm(id)?;
    store.set_state(id, &WorkloadState::Stopped)?;
    Ok(())
}

/// wipe: Running|Stopped → kill → unmap → wipe (E-6) → row deleted. The report is
/// the §7.2 probe record; a failed teardown leaves the row behind as `failed` —
/// a workload that did not provably wipe is not "gone" (FR-3).
pub fn wipe<S: DesiredStateStore, E: Executor>(
    store: &S,
    exec: &mut E,
    id: &str,
    markers: &Markers,
) -> Result<WipeReport, OpsError> {
    let record = store.get(id)?.ok_or_else(|| OpsError::NotFound(id.into()))?;
    match record.state {
        WorkloadState::Running
        | WorkloadState::Stopped
        | WorkloadState::Creating
        | WorkloadState::Failed { .. } => {}
        WorkloadState::Stopping => {
            return Err(OpsError::WrongState(
                id.into(),
                record.state,
                "stop in flight — wait for stopped, then wipe",
            ))
        }
    }
    match exec.teardown(id, markers) {
        Ok(report) => {
            store.delete(id)?;
            Ok(report)
        }
        Err(e) => {
            store.set_state(
                id,
                &WorkloadState::Failed {
                    reason: format!("wipe failed: {e}"),
                },
            )?;
            Err(e.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{ExecuteError, Executor, Markers, WipeReport};
    use crate::spec::{VolumeSpec, WorkloadState};
    use crate::store::SqliteStore;
    use std::cell::RefCell;
    fn spec(id: &str) -> WorkloadSpec {
        WorkloadSpec {
            tenant_id: crate::spec::default_tenant(),
            id: id.into(),
            image_digest: format!("sha256:{}", "c".repeat(64)),
            command: "true".into(),
            vcpu: 1,
            ram_mib: 64,
            volume: Some(VolumeSpec { size_mib: 8 }),
            ip6: "fd00::1".into(),
            gw6: "fd00::ff".into(),
        }
    }

    /// The test executor: records the call sequence, fails where told to.
    struct Fake {
        calls: RefCell<Vec<&'static str>>,
        fail_at: Option<&'static str>,
    }

    impl Fake {
        fn ok() -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                fail_at: None,
            }
        }
        fn fail_at(stage: &'static str) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                fail_at: Some(stage),
            }
        }
        fn record(&self, name: &'static str) -> Result<(), ExecuteError> {
            self.calls.borrow_mut().push(name);
            if Some(name) == self.fail_at {
                Err(ExecuteError::Vm(format!("induced failure at {name}")))
            } else {
                Ok(())
            }
        }
    }

    impl Executor for Fake {
        fn setup_netns(&mut self, _spec: &WorkloadSpec) -> Result<(), ExecuteError> {
            self.record("netns")
        }
        fn create_volume(&mut self, _spec: &WorkloadSpec) -> Result<(), ExecuteError> {
            self.record("volume")
        }
        fn start_vm(&mut self, _spec: &WorkloadSpec) -> Result<(), ExecuteError> {
            self.record("vm")
        }
        fn stop_vm(&mut self, _id: &str) -> Result<(), ExecuteError> {
            self.record("stop_vm")
        }
        fn teardown(&mut self, id: &str, _m: &Markers) -> Result<WipeReport, ExecuteError> {
            self.calls.borrow_mut().push("teardown");
            assert!(!id.is_empty());
            Ok(WipeReport {
                mapper_absent: true,
                keyfile_entry_removed: true,
                keyslots_erased: true,
                region_trimmed: true,
                region_clean: true,
            })
        }
    }

    #[test]
    fn create_converges_in_order() {
        let store = SqliteStore::in_memory().unwrap();
        let mut ex = Fake::ok();
        create(&store, &mut ex, spec("w1")).unwrap();
        assert_eq!(*ex.calls.borrow(), vec!["netns", "volume", "vm"]);
        assert_eq!(store.get("w1").unwrap().unwrap().state, WorkloadState::Running);
    }

    #[test]
    fn create_failure_tears_down_and_marks_failed() {
        let store = SqliteStore::in_memory().unwrap();
        let mut ex = Fake::fail_at("vm");
        let err = create(&store, &mut ex, spec("w1")).unwrap_err();
        assert!(err.to_string().contains("induced failure"));
        assert_eq!(*ex.calls.borrow(), vec!["netns", "volume", "vm", "teardown"]);
        match store.get("w1").unwrap().unwrap().state {
            WorkloadState::Failed { reason } => assert!(reason.contains("induced failure")),
            other => panic!("expected failed, got {other:?}"),
        }
    }

    #[test]
    fn create_rejects_invalid_spec_before_anything() {
        let store = SqliteStore::in_memory().unwrap();
        let mut ex = Fake::ok();
        let mut s = spec("w1");
        s.ip6 = "not-an-address".into();
        assert!(create(&store, &mut ex, s).is_err());
        assert!(ex.calls.borrow().is_empty());
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn stop_requires_running_and_transitions() {
        let store = SqliteStore::in_memory().unwrap();
        let mut ex = Fake::ok();
        create(&store, &mut ex, spec("w1")).unwrap();
        stop(&store, &mut ex, "w1").unwrap();
        assert_eq!(store.get("w1").unwrap().unwrap().state, WorkloadState::Stopped);
        // stopping again is a wrong-state error, not a silent noop
        assert!(matches!(
            stop(&store, &mut ex, "w1"),
            Err(OpsError::WrongState(..))
        ));
    }

    #[test]
    fn wipe_tears_down_and_deletes_row() {
        let store = SqliteStore::in_memory().unwrap();
        let mut ex = Fake::ok();
        create(&store, &mut ex, spec("w1")).unwrap();
        let report = wipe(&store, &mut ex, "w1", &Markers::new()).unwrap();
        assert!(report.all_pass());
        assert!(store.get("w1").unwrap().is_none());
    }

    #[test]
    fn wipe_missing_workload_is_not_found() {
        let store = SqliteStore::in_memory().unwrap();
        let mut ex = Fake::ok();
        assert!(matches!(
            wipe(&store, &mut ex, "nope", &Markers::new()),
            Err(OpsError::NotFound(_))
        ));
    }
}
