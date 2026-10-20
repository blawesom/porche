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
    /// Serialize to the CLH v54 API JSON shape — aligned with the v54 openapi
    /// (vmm/src/api/openapi/cloud-hypervisor.yaml) and verified on the node
    /// (2026-10-19): payload is plain strings, net is an ARRAY, serial is
    /// {mode,file}, memory.size is BYTES, v54 offload flags are tso/ufo/csum.
    pub fn to_clh_json(&self) -> String {
        let mut net_if = serde_json::json!({
            "mac": self.mac,
            "tap": self.tap,
        });
        if self.offloads_off {
            // SEC-7: the PwnEscape vector closed at the virtio-net end. v54's
            // defaults are already false — the E-7/E-18 delta run flips them ON
            // for the unhardened baseline.
            net_if["offload_tso"] = serde_json::json!(false);
            net_if["offload_ufo"] = serde_json::json!(false);
            net_if["offload_csum"] = serde_json::json!(false);
        }
        let mut payload = serde_json::json!({
            "kernel": self.kernel_path,
            "cmdline": self.cmdline,
        });
        if !self.initramfs_path.is_empty() {
            payload["initramfs"] = serde_json::json!(self.initramfs_path);
        }
        let obj = serde_json::json!({
            "cpus":   { "boot_vcpus": self.vcpu, "max_vcpus": self.vcpu },
            "memory": { "size": (self.ram_mib as u64) << 20 },
            "payload": payload,
            "disks": self.disks.iter()
                .map(|p| serde_json::json!({ "path": p, "image_type": "Raw" }))
                .collect::<Vec<_>>(),
            "net": [ net_if ],
            "serial": { "mode": "File", "file": self.console_log },
        });
        serde_json::to_string(&obj).expect("vm config serializes")
    }
}

/// The kernel cmdline the host-agent builds (E-25): the guest never configures
/// itself — address and gateway arrive on the command line, applied by the platform
/// init. The command is base64-encoded: the kernel splits cmdline on spaces, so the
/// workload command must ride as a single space-free token.
pub fn build_cmdline(spec: &porche_core::WorkloadSpec) -> String {
    format!(
        "console=ttyS0 porche.ip6={},{} porche.cmd={}",
        spec.ip6,
        spec.gw6,
        base64(spec.command.as_bytes())
    )
}

/// Minimal base64 (standard alphabet, padded) — the encoding for cmdline-safe
/// tokens. No dependency on purpose (doc 12 §4: boring).
fn base64(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use porche_core::{VolumeSpec, WorkloadSpec};

    fn spec() -> WorkloadSpec {
        WorkloadSpec {
            tenant_id: porche_core::spec::default_tenant(),
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
        // the command is base64-encoded (space-free token)
        let b64 = c.split_whitespace()
            .find(|w| w.starts_with("porche.cmd="))
            .expect("porche.cmd key");
        assert_eq!(b64, "porche.cmd=ZWNobyBvaw==", "{c}");
        // no raw spaces from the command leak into the cmdline
        assert!(!c.contains("echo ok"), "{c}");
    }

    #[test]
    fn base64_matches_reference_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        // space-free alphabet only
        assert!(!base64(b"echo alive > /dev/console").contains(' '));
    }

    #[test]
    fn clh_json_matches_v54_openapi_shape() {
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
        assert_eq!(json["memory"]["size"], 64u64 << 20); // v54: bytes
        assert_eq!(json["payload"]["kernel"], "/var/lib/porche/images/<digest>/kernel");
        assert_eq!(json["payload"]["cmdline"], build_cmdline(&spec()));
        assert_eq!(json["disks"][0]["path"], "/dev/mapper/porche-w1");
        assert_eq!(json["disks"][0]["image_type"], "Raw"); // v54: required at boot
        assert_eq!(json["net"].as_array().unwrap().len(), 1); // v54: net is a sequence
        assert_eq!(json["net"][0]["tap"], "tap-w1");
        assert_eq!(json["net"][0]["offload_tso"], false);
        assert_eq!(json["net"][0]["offload_csum"], false);
        assert_eq!(json["serial"]["mode"], "File");
        assert_eq!(json["serial"]["file"], "/var/lib/porche/consoles/w1.log");
    }
}
