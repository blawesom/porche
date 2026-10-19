//! Per-workload netns composition (doc 14 §2, the single-host v6 subset the PoC
//! measures — E-22/E-25; cross-host fabric, proxies and rules are deferred, doc 07
//! §3):
//!
//! - one netns per workload (`w-<id>`), the tap never leaves it — CLH runs jailed
//!   inside (SEC-12/13: process and network boundary coincide);
//! - veth root side enslaved to `brporche` with **isolated on, hairpin off** — no
//!   workload↔workload L2 at all;
//! - the tap carries the gateway /64; the guest holds the /128 (cmdline key);
//! - nftables anti-spoof: from the tap, exactly one allowed source MAC and one
//!   allowed v6 source (SEC-43 subset) — everything else drops.
//!
//! All argv are built here as data and unit-tested; execution happens on the node.

use crate::cmdspec::CommandSpec;
use porche_core::WorkloadSpec;

pub const BRIDGE: &str = "brporche";

pub fn ns_name(id: &str) -> String {
    format!("w-{id}")
}

pub fn tap_name(id: &str) -> String {
    format!("tap-{id}")
}

pub fn veth_name(id: &str) -> String {
    format!("veth-{id}")
}

/// Idempotent bridge bring-up is owned by the executor (`exec.rs` runs `ip link show`
/// first): WS-A applies it at node setup; the agent re-converges on every create.

/// The netns + port isolation + anti-spoof sequence for one workload.
pub fn commands(spec: &WorkloadSpec) -> Vec<CommandSpec> {
    let ns = ns_name(&spec.id);
    let tap = tap_name(&spec.id);
    let veth = veth_name(&spec.id);
    let mac = spec.assigned_mac();
    let gw_prefix = format!("{}/64", spec.gw6);

    let in_ns = |argv: &[&str]| {
        let mut full = vec!["ip", "netns", "exec", &ns];
        full.extend_from_slice(argv);
        CommandSpec::new(&full)
    };

    vec![
        // netns + veth pair; tap side moves into the netns
        CommandSpec::new(&["ip", "netns", "add", &ns]),
        CommandSpec::new(&["ip", "link", "add", &veth, "type", "veth", "peer", "name", &tap]),
        CommandSpec::new(&["ip", "link", "set", &tap, "netns", &ns]),
        // root side: enslaved to the bridge, isolated, up (no workload↔workload L2)
        CommandSpec::new(&["ip", "link", "set", &veth, "master", BRIDGE]),
        CommandSpec::new(&["bridge", "link", "set", "dev", &veth, "isolated", "on"]),
        CommandSpec::new(&["ip", "link", "set", &veth, "up"]),
        // netns side: loopback + tap up, gateway /64 on the tap
        in_ns(&["ip", "link", "set", "lo", "up"]),
        in_ns(&["ip", "link", "set", &tap, "up"]),
        in_ns(&["ip", "addr", "add", &gw_prefix, "dev", &tap]),
        // anti-spoof (SEC-43 subset): default-drop input from the tap, allow exactly
        // the assigned MAC and the assigned /128
        in_ns(&["nft", "add", "table", "ip6", "porche"]),
        in_ns(&["nft", "add", "chain", "ip6", "porche", "input", "{ type filter hook input priority 0 ; policy accept ; }"]),
        in_ns(&["nft", "add", "rule", "ip6", "porche", "input", "iifname", &tap, "ether", "saddr", "!=", &mac, "drop"]),
        in_ns(&["nft", "add", "rule", "ip6", "porche", "input", "iifname", &tap, "ip6", "saddr", "!=", &spec.ip6, "drop"]),
    ]
}

/// Netns teardown (part of the FR-3 sequence — `ip netns del` removes the veth pair
/// and the nft table with it).
pub fn remove_commands(id: &str) -> Vec<CommandSpec> {
    vec![CommandSpec::new(&["ip", "netns", "del", &ns_name(id)])]
}

#[cfg(test)]
mod tests {
    use super::*;
    use porche_core::VolumeSpec;

    fn spec() -> WorkloadSpec {
        WorkloadSpec {
            id: "w1".into(),
            image_digest: format!("sha256:{}", "e".repeat(64)),
            command: "true".into(),
            vcpu: 1,
            ram_mib: 64,
            volume: None,
            ip6: "fd00:abcd::1".into(),
            gw6: "fd00:abcd::ff".into(),
        }
    }

    #[test]
    fn derived_names_stay_under_ifname_limit() {
        let s = spec();
        assert!(ns_name(&s.id).len() <= 15);
        assert!(tap_name(&s.id).len() <= 15);
        assert!(veth_name(&s.id).len() <= 15);
    }

    #[test]
    fn commands_isolate_the_port_and_antispoof() {
        let cmds = commands(&spec());
        let flat: Vec<String> = cmds.iter().map(|c| c.display()).collect();
        let joined = flat.join("\n");
        assert!(joined.contains("bridge link set dev veth-w1 isolated on"));
        assert!(joined.contains(&format!("ether saddr != 02:00:00:00:00:01 drop")), "{joined}");
        assert!(joined.contains("ip6 saddr != fd00:abcd::1 drop"));
        // the tap is only touched root-side by its creation and its move into the
        // netns; everything else goes through `ip netns exec`
        for line in flat {
            if !line.starts_with("ip netns exec") && line.contains("tap-w1") {
                assert!(
                    line.contains("netns w-w1") || line.contains("type veth peer name"),
                    "tap configured outside the netns: {line}"
                );
            }
        }
        // hairpin never appears
        assert!(!joined.contains("hairpin"));
    }

    #[test]
    fn remove_deletes_the_netns() {
        let cmds = remove_commands("w1");
        assert_eq!(cmds[0].display(), "ip netns del w-w1");
    }
}
