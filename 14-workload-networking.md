# 14 — Workload Networking: Data Path, Management, Isolation

Working document — v3, 2026-10-18. Specifies the workload network: the per-host data
path, addressing and routing, egress and ingress (IngressRule/EgressRule), enforcement and isolation
rules, and the management flow. It resolves the last open data-path questions: C-6's CNI
adoption (superseded — native netns), the F45 VLAN-trust question (doc 02 §8), and the
IngressRule enforcement point (I-14).

Implements: FR-20/21/22, SEC-7/24/43, I-7/I-14, E-2 (ingress first-class), E-8 (planes).
**v2 (E-20)**: static route rows ratified as the v1 route-propagation decision — no BGP
or any dynamic routing protocol in v1.
**v3 (E-22..E-25)**: **IPv6-first** — one /128 per workload (ULA /48 per VPC), family
translation only at the proxies, bounded legacy-v4 exception (E-23), attached IPv4 1:1 at
the edge (E-24), cmdline+init bootstrap and well-known v6 metadata address (E-25).
Adversary model (doc 01 §0): the compromised workload itself — it attacks the host, its
neighbors, and tries to bypass the control plane.

---

## 1. Design principles

1. **No VLANs in the workload path.** The F45 class (VLAN-tag trust bypass) is closed by
   not using VLANs for isolation at all. Isolation is netns + bridge-port isolation +
   nftables — enforced by the host, never by fabric configuration.
2. **The guest never participates in its own policy.** No DHCP, no guest agent, no
   cooperative anything: the address and gateway are set on the kernel command line we
   build and applied by the **platform init** (E-25 — every v1 guest runs the platform
   rootfs, E-17); anti-spoof, shaping, and firewall rules live in the netns, outside
   the VM.
3. **Routed single address, no overlay — IPv6 first (E-22).** Each workload holds exactly
   one **/128** from its VPC's ULA prefix (a legacy-v4 workload additionally holds one
   /32, E-23). No VXLAN/Geneve, no bridges spanning hosts. L2 exists exactly inside one
   host.
4. **Every byte crosses an enforcement point we own.** Egress: transparent per-host proxy.
   Ingress: per-host proxy listener (IngressRule). Nothing is hairpinned past rules.
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
   │    route fe80::a9fe:a9fe/128 → metadata-proxy (C-3, token-gated — E-25)
   │
   ├─ egress-proxy  (host-agent module: allow-list per workload, NAT)
   ├─ ingress-proxy (host-agent module: IngressRule listeners — §5)
   └─ fabric interfaces: control (mTLS out, I-11), workload fabric, storage net (FR-30)
```

- **One netns per workload** (FR-21): the VM's tap never shares an L2 with another
  workload. Isolated bridge ports mean even same-user neighbors cannot exchange frames —
  a compromised U1 agent cannot ND-spoof, port-scan, or flood a neighbor workload on
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
| User VPC prefix | **ULA /48–/56, RFC 4193** (E-22) — requested at VPC creation, validated non-overlapping fleet-wide; platform-allocated if unspecified | tenant plane, transactional |
| Legacy-v4 range (opt-in) | RFC1918 /24–/16, **only when the E-23 exception is in use** — from it, opt-in workloads take an *additional* /32 | tenant plane, transactional |
| Workload address | **one /128** from the VPC prefix, assigned at create (always present); **plus one /32** from the legacy range for legacy-v4 workloads (E-23) | tenant plane |
| Host fabric IP | stable per host, **v6** on the workload-fabric prefix (legacy-v4 workloads add a v4 fabric route row) | admin plane |
| Storage net | separate subnet/NIC to shards, **tc-shaped** per FR-30 | host-side |
| Guest config | **`porche.ip6=<addr>,<gw>`** custom kernel-command-line key, applied by the platform init (E-25 — Linux `ip=` is v4-only; SLAAC/RA deliberately unused: no guest-side family negotiation, anti-spoof keeps exactly one allowed source per family) | host-agent builds it |

- **Cross-host**: the root ns routes `wip6/128 → <host-fabric-ip6>` (plus an optional
  `wip4/32 → <host-fabric-ip4>` row for legacy-v4 workloads, E-23); routes are
  desired-state rows distributed by the tenant plane. No overlay, no BGP in v1 (E-20:
  static route rows are the v1 route-propagation decision; fleet ≤ ~1,000 hosts, static
  routing suffices); BGP is a later scale seam behind the same route table.
- **Migration cutover** (doc 12 §6): target ready → **route switch** (withdraw on source,
  add on target, atomic in the state store) → IngressRule listeners follow (attached IPv4
  listener + attachment move in the same transaction, E-24) → source teardown.
  The /128 moves with the workload; connections re-establish through the ingress proxy.
- **Anti-spoof** (control-plane-enforced, never guest-cooperative): nftables in `w-<id>`
  accepts from the tap only the assigned source MAC and the assigned address(es) —
  exactly one per family — everything else drops (SEC-43).

**Address-range policy (E-22/E-23; re-scopes E-13)**: a user may define the **v6 ULA
prefix** (min /48, max /56) from which their workloads' /128s are assigned — the VPC
object (doc 09) — and, only if they use the legacy-v4 exception, an RFC1918 range
(/24–/16) for opt-in workloads' /32s. The tenant plane validates each request
transactionally against every other user's ranges **and the platform-reserved ranges of
both families** (v6: fabric, storage, link-local, metadata; v4: platform edge pools); an
overlapping request is rejected with a machine-readable reason (the explainability rule,
doc 05 §2). Users see only their own ranges (FR-14 scoping — a CIDR choice is not a
probe). The choice changes *allocation policy only*: /128 assignment, routing, proxies,
and reachability rules are identical for user-defined and platform-allocated prefixes.

**Family model (E-22)**: the workload network is IPv6 — **all family translation lives
in the two proxies** (§4/§5), which re-originate in whatever family the destination
requires. No NAT64/DNS64 machinery exists; the guest never sees IPv4 (a legacy-v4
workload's /32 exists for *legacy v4 peers*, not for internet access — egress still goes
through the proxy, FR-20).

## 4. Egress (FR-20)

1. Default route from the netns → root ns.
2. nft **transparent redirect** (tcp/udp) → per-host **egress proxy** (host-agent module,
   L4, tokio): **EgressRule objects** (doc 09 — per-workload, or the VPC default) define
   the allow-list (FQDN or CIDR — family-agnostic), then the proxy **re-originates in
   whatever family the destination requires** (E-22: v6 or v4; no NAT64/DNS64 — the
   guest never sees IPv4), sourcing from the host egress address or the workload's
   **attached IPv4** (E-24 — return traffic then matches).
3. **Shaping**: token bucket on the veth pair (both directions), rate from the tenant
   quota (FR-22) — enforced by tc, reported like any host fact.
4. DNS: resolved **by the proxy** (allow-list applies to names, not just IPs) — the guest
   never talks to a resolver directly.
5. No split tunneling, no exceptions path: an empty allow-list means no egress, and the
   workload cannot change that (E-2 logic applied to egress).

## 5. Ingress (E-2 — IngressRule)

1. `IngressRule` object (doc 09) → desired state → the **ingress proxy** on the hosting
   host opens a listener `host-fabric-ip6:<external_port>` → forwards to
   `wip6:guest_port`, with the rule's **`source_ranges` filter applied at the listener**
   (family-agnostic — v4 clients reaching a v6 listener is the normal case; default all).
   If the workload has an **attached IPv4** (E-24, `PublicIpv4` object), the listener
   additionally binds that v4 address, 1:1 — the proxy stays the sole path (no DNAT rule
   exists anywhere).
2. The proxy **is** the enforcement point (I-14): every connection is logged (SEC-23
   events), revocation closes the listener immediately, and no DNAT rule exists that
   could bypass it.
3. Revocation/migration: listener follows the desired state — closed on the source host,
   opened on the target, in the same transaction as the route switch (§3) — the attached
   IPv4 and its listener move with the workload (E-24).
4. L7 features (TLS termination, host routing) are P2 — v1 is L4; the proxy module
   boundary is where L7 lands if ever needed.

## 6. Security groups (VPC model, adopted v1 — the `SecurityGroup` object, doc 09)

Tenant-authored rules (ports/protocols/CIDRs — family-agnostic) — the **SecurityGroup**
object — compile
to nftables chains **inside `w-<id>`** — same direction as every other rule: control
plane programs, guest cannot reach the firewall. Default: deny inbound (only IngressRule
listeners open anything), east-west deny, egress only via EgressRule. v6 is the default
family; v4 rules apply only between legacy-v4 workloads (E-23).

## 7. Management flow

- Desired-state rows: `netns(veth, tap, mac, ip6, [ip4], gw, rates, sg-rules, ingress rules, attached-v4)` per
  workload. The host-agent converges: create/teardown netns with the VM lifecycle (FR-3 —
  netns teardown is part of deterministic teardown), reprogram tc/nft on quota change.
- **Drift is impossible to miss**: convergence runs continuously (I-12); a rule that
  vanished gets rebuilt; an unexpected interface on the host is a security event (SEC-41).
- All changes originate in the planes — the guest's only lever is its own traffic.

## 8. Threat-model walkthrough (doc 01 §0 adversary)

| Attack | Control |
|---|---|
| Guest→neighbor L2 (ND/NA poison, RA games, flood, scan) | isolated bridge ports + per-workload netns — no shared L2 at all; **RA/ND emission from the workload side is dropped** (SEC-43 — the platform never SLAACs, nothing legitimate emits RA) |
| IP/MAC spoofing | netns nft anti-spoof — control-plane enforced, exactly one allowed source address per family (SEC-43) |
| IPv6 extension-header abuse (routing headers, hop-by-hop, fragmented floods) | SEC-43: unknown/unnecessary extension headers and all routing headers dropped, strict fragment policy (doc 01 §4.2) |
| Offload/device-model exploit (F45/PwnEscape) | SEC-7 offload policy at virtio-net and tap; frozen device model |
| Egress abuse / exfiltration | transparent proxy + name-level allow-list + token bucket |
| Ingress widening | IngressRule is an API object only (E-2); proxy is the sole path; no DNAT |
| Metadata attack (F0.1/F17) | `fe80::a9fe:a9fe` (E-25) → proxy + single-use boot token (C-3) — fixed well-known v6 address, no guest-influenced endpoint |
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
- **Doc 14 §10 (v2) "IPv6: later addition"** — superseded by E-22: IPv6 **is** the v1
  workload network; the tables above are the same model with v6 as the default family
  and legacy v4 as the bounded exception (E-23).

## 10. Open points

- [ ] tc profile: exact shaper (tbf vs CAKE) and the FR-30 storage-net reservation
      mapping — decide with the integration environment (doc 13 §8).
- [x] ~~IPv6: v1 is IPv4-only for workload /32s; IPv6 is a later addition to the same
      model.~~ — **resolved by E-22/E-25**: IPv6 is the v1 workload network (/128, ULA per
      VPC, cmdline+init bootstrap, well-known v6 metadata address, SEC-43 hardening);
      legacy v4 is the bounded exception (E-23).
- [ ] Metal v6-fabric validation plan: the integration environment carries cross-host v6
      over a v6-in-v4 tunnel (doc 13 §2) — define when the native-v6-fabric run happens
      on metal (before the P1 networking exit; alongside the metal validation run, E-16).
- [ ] SEC-43 fragment policy detail: drop-vs-rate-limit for fragments, and the exact
      extension-header allow-list (hbh/destopts needed for ND vs dropped) — with the
      doc-13 lab, folded into the nftables rule compilation tests.
- [ ] Conntrack sizing for high-PPS workloads through the proxies — measure in
      integration (PERF-4 impact of the transparent egress path).
- [ ] MTU: 1500 v1; jumbo frames on the storage net only.
- [ ] BGP seam: if static route distribution ever shows its limits, the route table is
      the interface — BGP enters behind it, like the bus (E-9 seam rule).
