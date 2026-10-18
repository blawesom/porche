# 13 — Integration Environment (Outscale cloud)

Working document — v1, 2026-10-18. Defines and records the **integration environment**:
PORCHE's "proper infra" simulated on Outscale cloud VMs, used for test and integration
**before and alongside** the bare-metal PoC. This document is the contract between that
environment and the design corpus: what it validates, what it deliberately never
validates, how it is built, and how it is operated.

Related: doc 07 (PoC — stays on real metal for perf gates), doc 12 (software architecture
validated here), doc 08 A-1/A-9 (the spikes hosted here).

---

## 1. Position

- The integration environment exists to validate **mechanism** — protocol, orchestration,
  state machines, storage stacks, migration — cheaply and repeatably.
- The PoC (doc 07) exists to validate **performance and kill criteria** on real bare metal.
- **Neither substitutes for the other.** A number measured in this environment is a smoke
  test; a gate passed only here has not passed.

## 2. Fidelity contract

**Validated here (functional, fully):**

| What | Corresponding design element |
|---|---|
| Host-agent ↔ control-plane protocol (gRPC/mTLS, dual channel, desired-state reconciliation) | doc 12 §3/§6, I-11/I-12, E-8 rule 1 |
| Three services + Postgres, plane separation, two API surfaces | doc 12, E-8, FR-40/SEC-24 |
| CLH orchestration: create/boot/stop/wipe, teardown determinism, 1,000× loop | E-4, FR-3, PERF-8 *function* |
| Tier-E LUKS2 volume on a block device, dm-crypt attach, crypto-erase teardown | I-2, E-6 |
| **A-1 spike**: dm-writecache/dm-era sealable tier-R stack (seal → ship → restore) | doc 04 §3, A-1 gate |
| **A-9 spike**: CLH live migration between two nodes over TLS, KMS lease extension, cutover state machine, checkpoint fallback | FR-34, E-3, doc 12 §6 |
| Epoch shipping, `SyncVolume` seals, restore from checkpoint, storage-worker clone | doc 04 |
| Escape suite "must fail" runs (E-7) — functionally; kernel hardening applies inside the node | SEC-40/SEC-11 |
| Teardown + relaunch loops, wedge detection | doc 01 §8 criteria 6 |

**Never validated here (metal only):**

| What | Why it cannot transfer |
|---|---|
| PERF-1/2/5/9/10 absolute numbers | nested KVM inflates boot and I/O; BSU "local" storage is network-backed |
| Tier-E vs tier-R performance ratio and economics (doc 04 §2.1) | real NVMe is 10–50× faster than the NFS path; here both tiers are network — the ratio is distorted |
| Bare-metal security assertions (residual-memory probe as *evidence*) | the outer hypervisor is Outscale's; the threat model assumes we own the metal |
| L2/VLAN control (F45 class), IOMMU/VFIO, GPU | no L2 access, no VT-d passthrough in cloud VMs |
| Netboot/reimage (doc 10) | no PXE; approximated by image swap or rpm-ostree — drain/attest/wipe *logic* still exercised |
| PERF-13 migration pause absolute value | measured, but as a smoke signal only |

## 3. Topology

Five roles; only the metal nodes need dedicated tenancy and nested virtualization.

| # | PORCHE element | Outscale realization | Size (initial) |
|---|---|---|---|
| 1–2 | bare metal hosts | dedicated-group VM, `NestedVirtualization=true`, **p1 performance flag required**, tenancy = the group ID | `tinav7.c8r16p1`, root BSU 50 GB |
| 1–2 | local NVMe (tier E/R hot path) | **second BSU volume** per metal VM (`/dev/vdb`) — root disk is "host OS", second volume is the "NVMe region" | 100–200 GB gp2/io2 |
| 3 | NFS storage pool / hot shard | normal-tenancy VM + large BSU, NFS export (`/export/porche`); ciphertext only (A-4 — placement non-security-critical by design) | `tinav7.c4r16`, 500 GB gp2 |
| 4 | control plane | normal-tenancy VM: `porche tenant-plane` + `admin-plane` + `kms` + Postgres (single node; HA is a separate drill, A-8) | `tinav7.c4r8` |
| 5 | netboot/image service | folded into #4 initially (HTTP); PXE approximated — §2 | — |

**Both metal nodes land on the same physical server of the dedicated group** — cross-VM
live migration between them is a genuine VMM-level migration.

**Network**: metal nodes in **public mode** (see §4, the Net-tenancy subtlety);
segmentation expressed with security groups — one SG for management (SSH/ICMP from the
office), one for host↔control-plane (gRPC/mTLS), one for host↔shard (NFS + migration TLS
port). The FR-30 storage-network reservation is host-side `tc` shaping on the metal nodes —
testable here as-is.

## 4. Outscale mechanics (learned — reproduce exactly)

Region `eu-west-2`; subregion `eu-west-2a`. Credentials: local `~/.osc/config.json`
(never in this document or the repository).

1. **Dedicated groups are the tenancy**: `CreateVms` with `Placement.Tenancy = <group-id>`
   (e.g. `ded-53c8318b`) — not `dedicated`, and there is no separate group parameter.
2. **The `p1` performance flag is mandatory in dedicated groups** (`tinavW.cXrYp1`);
   `p2`/`p3` are rejected there.
3. **Nested virtualization** is a `CreateVms` flag: `NestedVirtualization=true`. Result:
   `vmx` CPU flag + `/dev/kvm` + `kvm_intel nested=Y` inside the VM. Verified on gen-7
   (Xeon Sapphire Rapids) with CLH v54.0 booting a guest kernel.
4. **Subregion goes inside `Placement`** (`Placement.SubregionName`), not top-level.
5. **A Net (VPC) must itself carry the dedicated group as tenancy** to host
   dedicated-group VMs — and that would pull *all* its VMs onto dedicated servers.
   Hence public mode for metal nodes (SG-segmented) until proven otherwise.
6. **SSH user on Rocky OMIs is `outscale`** (not `rocky`/`cloud-user`).
7. **Office access path**: via VPN/NAT — egress seen by the cloud as `46.231.147.8`
   (~0.4 ms RTT; fully workable for the migration spike). Direct egress is `171.33.98.94`.
8. Public IPs are allocated and linked explicitly (`CreatePublicIp` + `LinkPublicIp`).
9. CLH v54 CLI: `--cpus boot=N`, `--memory size=NM` — first thing the host-agent encodes.

## 5. Provisioning recipe (reproducible)

```text
prereq:  keypair "porche-integ" (imported local public key)
         SG "porche-integ-sg"   (SSH+ICMP: office egress ranges)

metal ×2:
  CreateVms  ImageId=<Rocky-9 OMI>            # placeholder until the FCOS OMI exists
             VmType=tinav7.c8r16p1
             KeypairName=porche-integ
             NestedVirtualization=true
             SecurityGroupIds=[porche-integ-sg]
             Placement={Tenancy: ded-53c8318b, SubregionName: eu-west-2a}
             BlockDeviceMappings=[root 50 GB]
  CreateVolume 200 GB → LinkVolume as /dev/vdb     # the "NVMe region"
  CreatePublicIp → LinkPublicIp

storage:
  CreateVms  VmType=tinav7.c4r16 (normal tenancy), 500 GB BSU
             → NFS server, export /export/porche (ciphertext only)

control plane:
  CreateVms  VmType=tinav7.c4r8 (normal tenancy)
             → Postgres (databases tenant/admin/kms), porche services, HTTP image svc
```

## 6. Current state (2026-10-18)

| Resource | ID | Status |
|---|---|---|
| Dedicated group `Test1` | `ded-53c8318b` (eu-west-2a, gen 7) | pre-existing, in use |
| Metal node 1 (validation) | `i-7482ee3c` (`tinav7.c8r16p1`, nested virt) | **deployed, verified** — CLH v54.0 boots guests; no second BSU yet |
| Public IP (node 1) | `eipalloc-84925ae5` → `148.253.126.10` | linked |
| SG `porche-integ-sg` | `sg-ba94b9e2` | deployed |
| Keypair `porche-integ` | local `id_ed25519_research` | deployed |
| Metal node 2, second BSUs, NFS VM, control-plane VM | — | **not yet provisioned** (recipe §5) |

Note: node 1 currently runs stock Rocky 9 — it was the nested-virt validation vehicle.
Metal nodes move to the **FCOS derivative OMI** once built (action 11); Rocky + hardening
sysctls is the acceptable interim.

## 7. Operations

- Access: `ssh outscale@<public-ip>` with the local key.
- Cost control: dedicated servers are allocated while group VMs run — **stop or
  terminate metal VMs when idle**; a stopped VM releases the physical server back
  (re-allocation happens on next start).
- Teardown: `DeleteVms` + `DeletePublicIp` + `DeleteVolume` per resource; the group
  (`Test1`) and SG/keypair stay.
- This environment is **not** a security boundary: test workloads only, mock KMS (local
  key file per doc 07) until the real KMS service lands here (doc 12).

## 8. Open items

- [ ] Provision metal node 2 + second BSUs + NFS VM + control-plane VM (§5).
- [ ] FCOS derivative OMI (import qcow2 → OMI; Ignition via user-data — verify metadata
      compatibility; fallback: bake config into the OMI). Unblocks action 11 validation.
- [ ] SG set for host↔control-plane and host↔shard flows (gRPC, NFS, migration TLS port).
- [ ] Net-tenancy question: verify whether a dedicated-group Net can host default-tenancy
      VMs (if yes, migrate topology off public mode for cleaner segmentation).
- [ ] Decide the tc shaping profile simulating FR-30 storage-network reservations.
