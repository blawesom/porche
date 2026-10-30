//! ConsoleApi — the adapter seam (doc 12 §9, E-28; doc 15 P0 gate).
//!
//! The trait's method names **are** the frozen doc-09 map (E-21/E-26): every
//! user-surface page speaks only these methods. The PoC adapter
//! ([`crate::adapter_poc::PocConsole`]) serves them from the desired-state
//! store + host-agent ops in-process; the P1 adapter swaps in tonic clients
//! to the two planes — **the UIs survive the swap** (until this trait existed,
//! that promise was unearned: handlers called `store`/`ops` directly, and
//! every page built that way was a P0 rewire — doc 15's first
//! implementation debt).
//!
//! Scope: the methods the ephemeral-workload slice implements today (doc 15's
//! "exists" rows). The rest of the map (`StartWorkload`, `AllocateHost`,
//! `ImportImage`, tokens, VPC/rules, tier R/S volumes, …) lands with P0/P1 —
//! a new method lands **here first**, then its page.
//!
//! Design rules carried by the seam:
//! - **Pure projection** (doc 12): no console-side scheduling, policy or
//!   state — the adapter answers, the pages render.
//! - **Visibility enforced at the adapter** (FR-14/SEC-16): every method is
//!   tenant-scoped by [`CallCtx`]; a cross-tenant id looks absent, never
//!   leaks that the object exists. Templates never filter.
//! - **Sync by PoC design**: the in-process adapter is synchronous;
//!   executor-touching mutations are wrapped in `spawn_blocking` by the
//!   calling handler (the same single-writer executor model as the CLI —
//!   I-12 reconciliation removes the constraint in P1).
//! - **Display derivation stays console-side** (E-29): the console derives
//!   `done (rc=N)` etc. from observable signals; the seam returns domain
//!   objects, the pages derive display state.

use crate::state::EventLine;
use porche_core::{Record, WipeReport, WorkloadSpec};

/// The caller of one console API method: the tenant scope fixed by the
/// matched token (E-8/MS-2 view isolation) and the audit actor.
#[derive(Clone, Debug)]
pub struct CallCtx {
    pub tenant_id: String,
    pub actor: String,
}

impl CallCtx {
    /// A read call — no audit actor needed (reads are never audited).
    pub fn read(tenant_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            actor: String::new(),
        }
    }
    /// A mutation call — the actor rides the settle audit entry.
    pub fn mutate(tenant_id: &str, actor: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            actor: actor.to_string(),
        }
    }
}

/// Why a console call failed. The page layer maps: [`ApiError::NotFound`] →
/// the 404 page (cross-tenant ids look absent — never a 403 leak);
/// [`ApiError::Rejected`] → the form re-rendered with the reason (nothing
/// changed); [`ApiError::Failed`] → the error page (the row carries state).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    NotFound(String),
    Rejected(String),
    Failed(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (ApiError::NotFound(s) | ApiError::Rejected(s) | ApiError::Failed(s)) = self;
        write!(f, "{s}")
    }
}

/// One platform image (the `ListImages` read): the digest is the object id;
/// the manifest lines are the §7.1 fixture record of that image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageInfo {
    pub digest: String,
    pub manifest: String,
}

/// The console's view of the API. Implementations: the PoC in-process
/// adapter (this crate) today, the plane clients in P1.
pub trait ConsoleApi: Send + Sync {
    /// `CreateWorkload` (FR-1): atomically admit the spec (duplicate-id +
    /// tenant quota + insert are one store operation — no check-then-act),
    /// then converge in the background (netns → volume → VM). Returns when
    /// the row is `Creating` — the detail page's poll observes the rest.
    fn create_workload(&self, ctx: &CallCtx, spec: WorkloadSpec) -> Result<(), ApiError>;

    /// `StopWorkload`: Running → Stopping → VM down → Stopped (volume and
    /// netns kept).
    fn stop_workload(&self, ctx: &CallCtx, id: &str) -> Result<(), ApiError>;

    /// `DeleteWorkload` — an operation, not a row delete (FR-3): kill →
    /// unmap → wipe, and the returned report IS the result the page renders.
    fn delete_workload(&self, ctx: &CallCtx, id: &str) -> Result<WipeReport, ApiError>;

    /// Workload `List` read — the caller's rows only.
    fn list_workloads(&self, ctx: &CallCtx) -> Result<Vec<Record>, ApiError>;

    /// Workload `Get` read — `None` when the id is not the caller's.
    fn get_workload(&self, ctx: &CallCtx, id: &str) -> Result<Option<Record>, ApiError>;

    /// `GetConsole` — the recorded serial-log text. A cross-tenant id
    /// returns the empty string (never another tenant's serial log; the
    /// MS-1 rule the UAT pins).
    fn get_console(&self, ctx: &CallCtx, id: &str) -> Result<String, ApiError>;

    /// `GetEvents` — the derived timeline (store transitions + console
    /// markers, E-29 display rules).
    fn get_events(&self, ctx: &CallCtx, id: &str) -> Result<Vec<EventLine>, ApiError>;

    /// Volume `List` read — the caller's records that carry a tier-E volume
    /// (volumes live with their workload in the PoC; no `CreateVolume`).
    fn list_volumes(&self, ctx: &CallCtx) -> Result<Vec<Record>, ApiError>;

    /// `DeleteVolume` — the workload's wipe path (the volume file, its
    /// mapper and its DEK die together — never the volume alone).
    fn delete_volume(&self, ctx: &CallCtx, id: &str) -> Result<WipeReport, ApiError>;

    /// `ListImages` — the operator-placed platform images (digest-pinned
    /// dirs; `ImportImage` is P0).
    fn list_images(&self, ctx: &CallCtx) -> Result<Vec<ImageInfo>, ApiError>;
}
