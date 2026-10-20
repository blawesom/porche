//! porche-core — the role-neutral core of the one binary (I-13).
//!
//! PoC scope (doc 07 §2): the workload types, the local desired-state table (I-12)
//! and the verb orchestration (create/status/stop/wipe). The PoC CLI is a writer on
//! the table; the P0 control plane replaces the writer, not this core.

pub mod executor;
pub mod ops;
pub mod spec;
pub mod store;

pub use executor::{ExecuteError, Executor, Markers, WipeReport};
pub use ops::OpsError;
pub use spec::{VolumeSpec, WorkloadSpec, WorkloadState};
pub use store::{DesiredStateStore, Record, SqliteStore, StoreError};
