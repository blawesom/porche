# 14 — Workload Networking: Data Path, Management, Isolation

Working document — v1, 2026-10-18. Specifies the workload network: the per-host data
path, addressing and routing, egress and ingress (Exposure), enforcement and isolation
rules, and the management flow. It resolves the last open data-path questions: C-6's CNI
adoption (superseded — native netns), the F45 VLAN-trust question (doc 02 §8), and the
Exposure enforcement point (I-14).

Implements: FR-20/21/22, SEC-7/24, I-7/I-14, E-2 (ingress first-class), E-8 (planes).
Adversary model (doc 01 §0): the compromised workload itself — it attacks the host, its
neighbors, and tries to bypass the control plane.

---

## 1. Design principles

1. **No VLANs in the workload path.** The F45 class (VLAN-tag trust bypass) is closed by
   not using VLANs for isolation at all. Isolation is netns + bridge-port isolation +
   nftables — enforced by the host, never by fabric configuration.
2. **The guest never participates in its own policy.** No DHCP, no guest agent, no
   cooperative anything: IP and gateway are set on the kernel command line we build;
   anti-spoof, shaping, and firewall rules live in the netns, outside the VM.
3. **Routed /32, no overlay.** Each workload holds one /32 from its user's VPC CIDR. No
   VXLAN/Geneve, no bridges spanning hosts. L2 exists exactly inside one host.
4. **Every byte crosses an enforcement point we own.** Egress: transparent per-host proxy.
   Ingress: per-host proxy listener (Exposure). Nothing is hairpinned past rules.
5. **Desired-state programmed, idempotent.** All dataplane objects (netns, veth, bridge
   ports, nft rules, tc shapers, routes) converge from the desired-state table (I-12) —
   a rebooted or drifted host rebuilds identical state.

## 2. Per-host data path (reference composition)

```
 root namespace (host)
   ┌─ brporche (Linux bridge, per host = per user, SEC-16)
   │    all workload ports: isolated on, hairpin off   ← no workload↔workload L2
   │    uplink port: fabric veth (workload fabric subnet)
   │
   ├─ per-workload netns  w-<workload-id>
   │    tap<id>  (CLH runs jailed inside this netns — SEC-12/13; tap never leaves it)
   │    veth pair  ↔ root side enslaved to brporche
   │    nftables: anti-spoof + security-group rules + egress redirect
   │    tc: token bucket both directions (FR-22)
   │    route 169.254.169.254/32 → metadata-proxy (C-3, token-gated)
   │
   ├─ egress-proxy  (host-agent module: allow-list per workload, NAT)
   ├─ ingress-proxy (host-agent module: Exposure listeners — §5)
   └─ fabric interfaces: control (mTLS out, I-11), workload fabric, storage net (FR-30)
```

- **One netns per workload** (FR-21): the VM's tap never shares an L2 with another
  workload. Isolated bridge ports mean even same-user neighbors cannot exchange frames —
  a compromised U1 agent cannot ARP-spoof, port-scan, or flood a neighbor workload on
  the same host.
- **CLH runs inside `w-<id>`** — the netns is part of the VMM jail; the process boundary
  (SEC-13) and the network boundary coincide.
- **vhost-net is not used in v1.** Kernel attack surface vs throughput; the untrusted-first
  call is userspace virtio. Measured against PERF-4; revisit as a per-workload-class,
  policy-gated option (I-7 pattern) — never default-on.
- **Offloads** (SEC-7/I-7): virtio-net offloads (checksum, TSO/GSO/GRO) **off by default**
  for untrusted classes, set per class by control-plane policy; tap offloads likewise.
  The PwnEscape vector is closed at both ends.

## 3. Addressing and routing

| Object | Value | Owner |
|---|---|---|
| User VPC CIDR | **user-defined within RFC1918** (E-13) — requested at VPC creation, validated non-overlapping fleet-wide; platform-allocated if unspecified | tenant plane, transactional |
| Workload IP | **one /32** from the user's VPC CIDR, assigned at create | tenant plane |
| Host fabric IP | stable per host, on the workload-fabric subnet | admin plane |
| Storage net | separate subnet/NIC to shards, **tc-shaped** per FR-30 | host-side |
| Guest config | `ip=<wip>::<gw>:<mask>::eth0:off` on the kernel command line | host-agent builds it |

- **Cross-host**: the root ns routes `wip/32 → <host-fabric-ip>`; routes are desired-state
  rows distributed by the tenant plane. No overlay, no BGP in v1 (fleet ≤ ~1,000 hosts,
  static routing suffices); BGP is a later scale seam behind the same route table.
- **Migration cutover** (doc 12 §6): target ready → **route switch** (withdraw on source,
  add on target, atomic in the state store) → Exposure listeners follow → source teardown.
  The /32 moves with the workload; connections re-establish through the ingress proxy.
- **Anti-spoof** (control-plane-enforced, never guest-cooperative): nftables in `w-<id>`
  accepts from the tap only the assigned source MAC and IP — everything else drops.

**Address-range policy (E-13)**: a user may define the range (an RFC1918 block, min /24,
max /16) from which their workloads' /32s are assigned — the VPC object (doc 09). The
tenant plane validates the request transactionally against every other user's ranges
**and the platform-reserved fabric/storage/link-local ranges**; an overlapping request is
rejected with a machine-readable reason (the explainability rule, doc 05 §2). Users see
only their own ranges (FR-14 scoping — a CIDR choice is not a probe). The choice changes
*allocation policy only*: /32 assignment, routing, proxies, and reachability rules are
identical for user-defined and platform-allocated ranges.

## 4. Egress (FR-20)

1. Default route from the netns → root ns.
2. nft **transparent redirect** (tcp/udp) → per-host **egress proxy** (host-agent module,
   L4, tokio): per-workload allow-list (FQDN or CIDR per tenant policy), then NAT with
   the host egress address.
3. **Shaping**: token bucket on the veth pair (both directions), rate from the tenant
   quota (FR-22) — enforced by tc, reported like any host fact.
4. DNS: resolved **by the proxy** (allow-list applies to names, not just IPs) — the guest
   never talks to a resolver directly.
5. No split tunneling, no exceptions path: an empty allow-list means no egress, and the
   workload cannot change that (E-2 logic applied to egress).

## 5. Ingress (E-2 — Exposure)

1. `Exposure` object (doc 09) → desired state → the **ingress proxy** on the hosting
   host opens a listener `host-fabric-ip:<external_port>` → forwards to `wip:guest_port`.
2. The proxy **is** the enforcement point (I-14): every connection is logged (SEC-23
   events), revocation closes the listener immediately, and no DNAT rule exists that
   could bypass it.
3. Revocation/migration: listener follows the desired state — closed on the source host,
   opened on the target, in the same transaction as the route switch (§3).
4. L7 features (TLS termination, host routing) are phase-2 — v1 is L4; the proxy module
   boundary is where L7 lands if ever needed.

## 6. Security groups (VPC model, adopted v1)

Tenant-authored rules (ports/protocols/CIDRs) compile to nftables chains **inside
`w-<id>`** — same direction as every other rule: control plane programs, guest cannot
reach the firewall. Default: deny inbound (only Exposure listeners open anything),
allow-listed egress.

## 7. Management flow

- Desired-state rows: `netns(veth, tap, mac, ip, gw, rates, sg-rules, exposures)` per
  workload. The host-agent converges: create/teardown netns with the VM lifecycle (FR-3 —
  netns teardown is part of deterministic teardown), reprogram tc/nft on quota change.
- **Drift is impossible to miss**: convergence runs continuously (I-12); a rule that
  vanished gets rebuilt; an unexpected interface on the host is a security event (SEC-41).
- All changes originate in the planes — the guest's only lever is its own traffic.

## 8. Threat-model walkthrough (doc 01 §0 adversary)

| Attack | Control |
|---|---|
| Guest→neighbor L2 (ARP poison, flood, scan) | isolated bridge ports + per-workload netns — no shared L2 at all |
| IP/MAC spoofing | netns nft anti-spoof — control-plane enforced |
| Offload/device-model exploit (F45/PwnEscape) | SEC-7 offload policy at virtio-net and tap; frozen device model |
| Egress abuse / exfiltration | transparent proxy + name-level allow-list + token bucket |
| Ingress widening | Exposure is an API object only (E-2); proxy is the sole path; no DNAT |
| Metadata attack (F0.1/F17) | 169.254.169.254 → proxy + single-use boot token (C-3) |
| Self-DoS / neighbor DoS (F31/F44) | tc both directions per workload + per-netns limits (OPS-4) |
| Fabric/VLAN games (F45) | no VLANs in the workload path at all |
| Enumeration / co-residency | dedicated hosts (SEC-16) + scoped placement views (FR-14) |

## 9. What this supersedes / resolves

- **C-6 (CNI adoption)** — superseded: CNI existed to standardize container-runtime
  integration that E-4 deleted. Needs are ~200 lines of rtnetlink in the host-agent.
  (Doc 03 §6 amended.)
- **Doc 02 §8, multi-tenant L2 / VLAN trust (F45)** — resolved: no VLANs in the workload
  path; the fabric's VLANs (if any) are platform plumbing carrying no isolation authority.
- **I-14 ingress data path** — specified (§5).

## 10. Open points

- [ ] tc profile: exact shaper (tbf vs CAKE) and the FR-30 storage-net reservation
      mapping — decide with the integration environment (doc 13 §8).
- [ ] IPv6: v1 is IPv4-only for workload /32s; IPv6 is a later addition to the same model.
- [ ] Conntrack sizing for high-PPS workloads through the proxies — measure in
      integration (PERF-4 impact of the transparent egress path).
- [ ] MTU: 1500 v1; jumbo frames on the storage net only.
- [ ] BGP seam: if static route distribution ever shows its limits, the route table is
      the interface — BGP enters behind it, like the bus (E-9 seam rule).
