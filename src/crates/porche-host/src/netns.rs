//! Per-workload netns composition (doc 14 §2, the single-host v6 subset the PoC
//! measures — E-22/E-25; cross-host fabric, proxies and rules are deferred, doc 07
//! §3):
//!
//! - one netns per workload (`w-<id>`) — CLH runs jailed inside (SEC-12/13:
//!   process and network boundary coincide);
//! - **`tap-<id>` is a real TUN/TAP device** (CLH attaches via `/dev/net/tun` +
//!   TUNSETIFF — a veth peer cannot be opened as a tap; observed on v54);
//! - the tap bridges to the uplink veth through an **in-netns bridge** `br-<id>`;
//!   the root-side veth `vr-<id>` is enslaved to `brporche` with **isolated on**
//!   (no workload↔workload L2 at all);
//! - the in-netns bridge carries the gateway /64; the guest holds the /128
//!   (cmdline key, E-25);
//! - anti-spoof (SEC-43 subset) rides the **netdev family ingress hook bound to
//!   the tap** — it sees guest frames before any bridging, exactly one allowed
//!   source MAC and one allowed v6 source, everything else drops.
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

/// In-netns bridge (tap ↔ uplink veth).
pub fn nsbr_name(id: &str) -> String {
    format!("br-{id}")
}

/// Veth root side — enslaved to brporche, isolated.
pub fn veth_root(id: &str) -> String {
    format!("vr-{id}")
}

/// Veth netns side — enslaved to the in-netns bridge.
pub fn veth_ns(id: &str) -> String {
    format!("ve-{id}")
}

/// The netns + tap + bridges + anti-spoof sequence for one workload.
pub fn commands(spec: &WorkloadSpec) -> Vec<CommandSpec> {
    let ns = ns_name(&spec.id);
    let tap = tap_name(&spec.id);
    let nsbr = nsbr_name(&spec.id);
    let vr = veth_root(&spec.id);
    let ve = veth_ns(&spec.id);
    let mac = spec.assigned_mac();
    let gw_prefix = format!("{}/64", spec.gw6);

    let in_ns = |argv: &[&str]| {
        let mut full = vec!["ip", "netns", "exec", &ns];
        full.extend_from_slice(argv);
        CommandSpec::new(&full)
    };

    vec![
        // netns + uplink veth pair; netns side moves in
        CommandSpec::new(&["ip", "netns", "add", &ns]),
        CommandSpec::new(&["ip", "link", "add", &vr, "type", "veth", "peer", "name", &ve]),
        CommandSpec::new(&["ip", "link", "set", &ve, "netns", &ns]),
        // root side: enslaved to brporche, isolated, up (no workload↔workload L2)
        CommandSpec::new(&["ip", "link", "set", &vr, "master", BRIDGE]),
        CommandSpec::new(&["bridge", "link", "set", "dev", &vr, "isolated", "on"]),
        CommandSpec::new(&["ip", "link", "set", &vr, "up"]),
        // netns side: real tap + in-netns bridge, tap and veth enslaved to it
        in_ns(&["ip", "tuntap", "add", "dev", &tap, "mode", "tap"]),
        in_ns(&["ip", "link", "add", &nsbr, "type", "bridge"]),
        in_ns(&["ip", "link", "set", &tap, "master", &nsbr]),
        in_ns(&["ip", "link", "set", &ve, "master", &nsbr]),
        in_ns(&["ip", "link", "set", "lo", "up"]),
        in_ns(&["ip", "link", "set", &nsbr, "up"]),
        in_ns(&["ip", "link", "set", &tap, "up"]),
        in_ns(&["ip", "link", "set", &ve, "up"]),
        // gateway /64 on the in-netns bridge (the guest's <gw>, E-25)
        in_ns(&["ip", "addr", "add", &gw_prefix, "dev", &nsbr]),
        // anti-spoof (SEC-43 subset): netdev ingress hook BOUND to the tap —
        // guest frames filtered before any bridging; exactly one allowed MAC
        // and one allowed v6 source
        in_ns(&["nft", "add", "table", "netdev", "porche"]),
        in_ns(&[
            "nft",
            "add",
            "chain",
            "netdev",
            "porche",
            "antispoof",
            &format!("{{ type filter hook ingress device {tap} priority 0 ; }}"),
        ]),
        in_ns(&[
            "nft", "add", "rule", "netdev", "porche", "antispoof",
            "ether", "saddr", "!=", &mac, "drop",
        ]),
        in_ns(&[
            "nft", "add", "rule", "netdev", "porche", "antispoof",
            "ip6", "saddr", "!=", &spec.ip6, "drop",
        ]),
    ]
}

/// Netns teardown (part of the FR-3 sequence — `ip netns del` removes the veth
/// pair, the tap, the in-netns bridge and the nft table with it).
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
        for n in [
            ns_name(&s.id),
            tap_name(&s.id),
            nsbr_name(&s.id),
            veth_root(&s.id),
            veth_ns(&s.id),
        ] {
            assert!(n.len() <= 15, "{n}");
        }
    }

    #[test]
    fn commands_build_a_real_tap_bridged_to_the_uplink() {
        let cmds = commands(&spec());
        let flat: Vec<String> = cmds.iter().map(|c| c.display()).collect();
        let joined = flat.join("\n");

        // the tap is a TUN/TAP device, created inside the netns (CLH attaches via
        // /dev/net/tun — a veth peer cannot serve as a tap)
        assert!(joined.contains("ip netns exec w-w1 ip tuntap add dev tap-w1 mode tap"));
        // in-netns bridge carries tap + veth + the gateway
        assert!(joined.contains("ip link add br-w1 type bridge"));
        assert!(joined.contains("ip link set tap-w1 master br-w1"));
        assert!(joined.contains("ip link set ve-w1 master br-w1"));
        assert!(joined.contains("ip addr add fd00:abcd::ff/64 dev br-w1"));
        // root side: isolated port on the host bridge
        assert!(joined.contains("bridge link set dev vr-w1 isolated on"));
        // anti-spoof: netdev ingress hook bound to the tap, one MAC + one v6
        assert!(joined.contains("netdev porche antispoof"));
        assert!(joined.contains("hook ingress device tap-w1 priority 0"));
        assert!(joined.contains(&format!("ether saddr != 02:00:00:00:00:01 drop")), "{joined}");
        assert!(joined.contains("ip6 saddr != fd00:abcd::1 drop"));
        // hairpin never appears; no VLANs anywhere
        assert!(!joined.contains("hairpin"));
        assert!(!joined.contains("vlan"));
    }

    #[test]
    fn remove_deletes_the_netns() {
        let cmds = remove_commands("w1");
        assert_eq!(cmds[0].display(), "ip netns del w-w1");
    }
}
