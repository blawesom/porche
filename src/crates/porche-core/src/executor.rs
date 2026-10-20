//! The host-side operations the core orchestrates. The trait is the reconciliation
//! seam (I-12): the PoC CLI drives a direct executor (porche-host); unit tests drive
//! a fake; the P0 agent drives the same trait from desired-state deltas.

use crate::spec::WorkloadSpec;

/// Probe material for the wipe scan: markers the workload wrote before teardown —
/// after wipe, the region must contain no *cryptographically recoverable* marker
/// material (E-6; doc 07 §7.2 wipe row).
pub type Markers = Vec<Vec<u8>>;

#[derive(Debug, thiserror::Error)]
pub enum ExecuteError {
    #[error("netns setup failed: {0}")]
    Netns(String),
    #[error("volume failed: {0}")]
    Volume(String),
    #[error("vm failed: {0}")]
    Vm(String),
    #[error("teardown failed: {0}")]
    Teardown(String),
}

/// The wipe report — the §7.2 probe fields. E-6 semantics: crypto-erase only; ciphertext
/// bytes may legitimately remain on the medium — the claims are about keys and trim.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct WipeReport {
    /// the LUKS2 mapper device is absent after unmap.
    pub mapper_absent: bool,
    /// the mock-KMS keyfile no longer holds this workload's DEK.
    pub keyfile_entry_removed: bool,
    /// LUKS2 header + key slots erased — the DEK is cryptographically gone.
    pub keyslots_erased: bool,
    /// the region was discarded (hole-punch → TRIM to the backing device).
    pub region_trimmed: bool,
    /// region scan (as root) found no marker material (§7.2 probe — the gate).
    pub region_clean: bool,
}

impl WipeReport {
    pub fn all_pass(&self) -> bool {
        self.mapper_absent
            && self.keyfile_entry_removed
            && self.keyslots_erased
            && self.region_trimmed
            && self.region_clean
    }
}

/// Host-side executor — one implementation per trust context, one contract.
pub trait Executor {
    /// Per-workload netns + bridge port + anti-spoof (doc 14 §2, single-host v6
    /// subset: netns, isolated port, one allowed source).
    fn setup_netns(&mut self, spec: &WorkloadSpec) -> Result<(), ExecuteError>;
    /// Tier-E volume: LUKS2 file on the NVMe region, `cryptsetup open`, key from the
    /// mock KMS (one root-only JSON file).
    fn create_volume(&mut self, spec: &WorkloadSpec) -> Result<(), ExecuteError>;
    /// Boot the microVM via the CLH API — the VMM runs jailed inside the netns.
    fn start_vm(&mut self, spec: &WorkloadSpec) -> Result<(), ExecuteError>;
    /// Stop the VM (volume and netns kept — `stop`, not `wipe`).
    fn stop_vm(&mut self, id: &str) -> Result<(), ExecuteError>;
    /// Deterministic teardown, in this order (FR-3): kill VM → unmap → wipe (E-6) →
    /// netns removal. Returns the probe report.
    fn teardown(&mut self, id: &str, markers: &Markers) -> Result<WipeReport, ExecuteError>;
}
