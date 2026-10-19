//! The VM configuration — the frozen device allow-list is *structural* (SEC-3/SEC-4):
//! the only devices a workload can ever get are the fields of this config; there is no
//! escape hatch that adds a legacy device. Resource ceilings ride the same config
//! (I-9); virtio-net offloads are off for untrusted workloads (SEC-7 / E-7 toggle).

use serde::{Deserialize, Serialize};

/// Kernel + initramfs + rootfs/volume disks + one virtio-net iface on the workload's
/// tap + serial console to a file (the PERF-1 marker channel, doc 07 §7.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VmConfig {
    pub vcpu: u32,
    pub ram_mib: u32,
    /// Path to the guest kernel (from the platform image dir).
    pub kernel_path: String,
    /// Path to the initramfs (platform init — applies `porche.ip6=`).
    pub initramfs_path: String,
    /// Kernel cmdline — built by the host-agent (E-25: `porche.ip6=<addr>,<gw>`,
    /// `porche.cmd=...`, `console=ttyS0`).
    pub cmdline: String,
    /// virtio-block disks: the platform image first, then the tier-E volume mapper.
    pub disks: Vec<String>,
    /// The tap device, already waiting inside the workload netns (doc 14 §2).
    pub tap: String,
    /// The host-assigned MAC — anti-spoof keeps exactly one allowed source.
    pub mac: String,
    /// Virtio-net offloads (checksum/TSO/GSO/GRO) — **off** for untrusted workloads
    /// (SEC-7/I-7); the E-7 baseline-delta run toggles this.
    pub offloads_off: bool,
    /// Serial console log file — `porche.ready` lands here (PERF-1 T1).
    pub console_log: String,
}

impl VmConfig {
    /// Serialize to the CLH API JSON shape (v54 openapi subset).
    ///
    /// NOTE (WS-C verify-on-node): field names and units mirror the v54 openapi as
    /// documented; the first on-node burst pins them against a live socket and this
    /// function is corrected in the same commit if they differ.
    pub fn to_clh_json(&self) -> String {
        let mut obj = serde_json::json!({
            "cpus":   { "boot_vcpus": self.vcpu, "max_vcpus": self.vcpu },
            "memory": { "size": self.ram_mib },
            "kernel":    { "path": self.kernel_path },
            "initramfs": { "path": self.initramfs_path },
            "cmdline":   { "args": self.cmdline },
            "disks": self.disks.iter()
                .map(|p| serde_json::json!({ "path": p }))
                .collect::<Vec<_>>(),
            "net": {
                "ifs": [ {
                    "mac": self.mac,
                    "tap": self.tap,
                } ]
            },
            "serial": { "console": { "file": self.console_log, "mode": "ReadWrite" } },
        });
        if self.offloads_off {
            // SEC-7: the PwnEscape vector closed at the virtio-net end.
            obj["net"]["ifs"][0]["offload_tso"] = serde_json::json!(false);
            obj["net"]["ifs"][0]["offload_gso"] = serde_json::json!(false);
            obj["net"]["ifs"][0]["offload_gro"] = serde_json::json!(false);
            obj["net"]["ifs"][0]["offload_ufo"] = serde_json::json!(false);
        }
        serde_json::to_string(&obj).expect("vm config serializes")
    }
}

/// The kernel cmdline the host-agent builds (E-25): the guest never configures
/// itself — address and gateway arrive on the command line, applied by the platform
/// init; `porche.cmd` is the workload command (single shell string).
pub fn build_cmdline(spec: &porche_core::WorkloadSpec) -> String {
    format!(
        "console=ttyS0 porche.ip6={},{} porche.cmd={}",
        spec.ip6, spec.gw6, spec.command
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use porche_core::{VolumeSpec, WorkloadSpec};

    fn spec() -> WorkloadSpec {
        WorkloadSpec {
            id: "w1".into(),
            image_digest: format!("sha256:{}", "d".repeat(64)),
            command: "echo ok".into(),
            vcpu: 2,
            ram_mib: 128,
            volume: Some(VolumeSpec { size_mib: 64 }),
            ip6: "fd00:abcd::1".into(),
            gw6: "fd00:abcd::ff".into(),
        }
    }

    #[test]
    fn cmdline_carries_the_v6_bootstrap() {
        let c = build_cmdline(&spec());
        assert!(c.contains("console=ttyS0"), "{c}");
        assert!(c.contains("porche.ip6=fd00:abcd::1,fd00:abcd::ff"), "{c}");
        assert!(c.contains("porche.cmd=echo ok"), "{c}");
    }

    #[test]
    fn clh_json_has_the_frozen_surface_and_offloads_off() {
        let cfg = VmConfig {
            vcpu: 1,
            ram_mib: 64,
            kernel_path: "/var/lib/porche/images/<digest>/kernel".into(),
            initramfs_path: "/var/lib/porche/images/<digest>/initramfs".into(),
            cmdline: build_cmdline(&spec()),
            disks: vec!["/dev/mapper/porche-w1".into()],
            tap: "tap-w1".into(),
            mac: "02:00:00:00:00:01".into(),
            offloads_off: true,
            console_log: "/var/lib/porche/consoles/w1.log".into(),
        };
        let json: serde_json::Value =
            serde_json::from_str(&cfg.to_clh_json()).expect("valid json");
        assert_eq!(json["cpus"]["boot_vcpus"], 1);
        assert_eq!(json["memory"]["size"], 64);
        assert_eq!(json["disks"][0]["path"], "/dev/mapper/porche-w1");
        assert_eq!(json["net"]["ifs"][0]["tap"], "tap-w1");
        assert_eq!(json["net"]["ifs"][0]["offload_tso"], false);
        assert_eq!(json["net"]["ifs"][0]["offload_gso"], false);
        assert_eq!(json["serial"]["console"]["file"], "/var/lib/porche/consoles/w1.log");
    }
}
