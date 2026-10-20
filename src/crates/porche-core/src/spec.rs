//! Workload types — doc 07 §2: platform-image digest + command + volume size; that is
//! all (FR-1 post-E-17). `vcpu`/`ram_mib` carry the resource ceilings (I-9); `ip6`/`gw6`
//! carry the single-host v6 /128 model (E-22/E-25 — the guest gets its address via the
//! `porche.ip6=<addr>,<gw>` cmdline key).

use serde::{Deserialize, Serialize};
use std::net::Ipv6Addr;

/// Tier-E volume: LUKS2 file on the local NVMe region, raw sparse inside (I-2 reduced
/// to tier E — doc 07 §2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeSpec {
    pub size_mib: u32,
}

/// The PoC workload spec — exactly doc 07 §2's "workload spec" row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadSpec {
    /// Workload id — also names the netns (`w-<id>`), tap (`tap-<id>`), veth
    /// (`veth-<id>`) and mapper (`porche-<id>`). At most 10 chars keeps every derived
    /// interface name inside the kernel's 15-char limit.
    pub id: String,
    /// Platform image digest (`sha256:<64 hex>`) — pinned by digest; the signing
    /// pipeline is P0 (doc 07 §2). Resolves to the local image dir (kernel +
    /// initramfs) on the host.
    pub image_digest: String,
    /// Workload command — a shell string run by the guest's platform init (the
    /// `porche.cmd=` cmdline key, E-25 pattern: the guest never configures itself).
    pub command: String,
    /// vCPU ceiling (I-9).
    pub vcpu: u32,
    /// RAM ceiling in MiB (I-9).
    pub ram_mib: u32,
    /// Tier-E volume, if any.
    pub volume: Option<VolumeSpec>,
    /// The workload's /128 — bare address, no prefix (the /128 is the model, E-22).
    pub ip6: String,
    /// Gateway address on the netns tap — the `<gw>` of `porche.ip6=<addr>,<gw>`.
    pub gw6: String,
}

/// Host-local lifecycle state. The P0 agent splits desired vs reported state (doc 12
/// §1); at PoC scale one transition covers both.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkloadState {
    /// netns → volume → VM convergence in progress.
    Creating,
    Running,
    /// stop ordered (VM down; volume and netns kept).
    Stopping,
    Stopped,
    Failed { reason: String },
}

impl WorkloadSpec {
    /// Validate the spec. Rejects anything that would leak into device names, the
    /// kernel cmdline, or the volume path un-sanitized.
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty()
            || !self
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(format!(
                "id {:?} must be non-empty [a-z0-9-] (it names netns/tap/veth/mapper devices)",
                self.id
            ));
        }
        if self.id.len() > 10 {
            return Err(format!(
                "id {:?} longer than 10 chars — derived interface names would exceed the 15-char limit",
                self.id
            ));
        }
        let digest = self.image_digest.strip_prefix("sha256:").unwrap_or("");
        if digest.len() != 64 || !digest.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!(
                "image_digest {:?} must be sha256:<64 hex>",
                self.image_digest
            ));
        }
        if self.command.trim().is_empty() {
            return Err("command must be a non-empty shell string".into());
        }
        if self.command.contains(['\0', '\n']) {
            return Err("command must not contain NUL or newline (it rides the kernel cmdline)".into());
        }
        if self.vcpu == 0 || self.vcpu > 64 {
            return Err(format!("vcpu {} out of range 1..=64", self.vcpu));
        }
        if self.ram_mib < 64 || self.ram_mib > 65_536 {
            return Err(format!("ram_mib {} out of range 64..=65536", self.ram_mib));
        }
        if let Some(v) = &self.volume {
            if v.size_mib == 0 || v.size_mib > 102_400 {
                return Err(format!(
                    "volume size_mib {} out of range 1..=102400",
                    v.size_mib
                ));
            }
        }
        if self.ip6.parse::<Ipv6Addr>().is_err() {
            return Err(format!("ip6 {:?} is not a bare IPv6 address", self.ip6));
        }
        if self.gw6.parse::<Ipv6Addr>().is_err() {
            return Err(format!("gw6 {:?} is not a bare IPv6 address", self.gw6));
        }
        if self.ip6.parse::<Ipv6Addr>() == self.gw6.parse::<Ipv6Addr>() {
            return Err("ip6 and gw6 must differ".into());
        }
        Ok(())
    }

    /// The host-assigned MAC — anti-spoof keeps exactly one allowed source MAC and
    /// address per family (doc 14 §3, SEC-43); control-plane-enforced, never
    /// guest-cooperative. Deterministic from the /128: locally-administered bit + low
    /// 40 bits of the address.
    pub fn assigned_mac(&self) -> String {
        let octets = self
            .ip6
            .parse::<Ipv6Addr>()
            .expect("validated spec")
            .octets();
        format!(
            "02:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            octets[11], octets[12], octets[13], octets[14], octets[15]
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> WorkloadSpec {
        WorkloadSpec {
            id: "w1".into(),
            image_digest: format!("sha256:{}", "a".repeat(64)),
            command: "echo hello > /dev/console".into(),
            vcpu: 2,
            ram_mib: 256,
            volume: Some(VolumeSpec { size_mib: 512 }),
            ip6: "fd00:abcd::1".into(),
            gw6: "fd00:abcd::ffff".into(),
        }
    }

    #[test]
    fn valid_spec_passes() {
        assert!(spec().validate().is_ok());
    }

    #[test]
    fn rejects_uppercase_id() {
        let mut s = spec();
        s.id = "W1".into();
        assert!(s.validate().is_err());
    }

    #[test]
    fn rejects_overlong_id() {
        let mut s = spec();
        s.id = "abcdefghijk".into(); // 11 chars
        assert!(s.validate().is_err());
    }

    #[test]
    fn rejects_bad_digest() {
        let mut s = spec();
        s.image_digest = "sha256:short".into();
        assert!(s.validate().is_err());
    }

    #[test]
    fn rejects_prefixed_ip6() {
        let mut s = spec();
        s.ip6 = "fd00:abcd::1/128".into();
        assert!(s.validate().is_err());
    }

    #[test]
    fn rejects_cmdline_unsafe_command() {
        let mut s = spec();
        s.command = "sh -c 'x'\nrm -rf /".into();
        assert!(s.validate().is_err());
    }

    #[test]
    fn rejects_zero_vcpu() {
        let mut s = spec();
        s.vcpu = 0;
        assert!(s.validate().is_err());
    }

    #[test]
    fn rejects_equal_addr_and_gw() {
        let mut s = spec();
        s.gw6 = s.ip6.clone();
        assert!(s.validate().is_err());
    }

    #[test]
    fn mac_is_deterministic_and_locally_administered() {
        let s = spec();
        let mac = s.assigned_mac();
        assert!(mac.starts_with("02:"));
        assert_eq!(mac, s.assigned_mac());
        let other = {
            let mut s = spec();
            s.ip6 = "fd00:abcd::2".into();
            s.assigned_mac()
        };
        assert_ne!(mac, other);
    }
}
