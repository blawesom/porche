# 02 — Core Architecture Design (new orchestrator, incumbent design as reference)

Working document — v6, 2026-10-18. Implements `01-requirements.md`.
**v4** (doc 08 E-1..E-3): stateful workloads and ingress first-class v1; live migration
is a v1 goal. **v5** (doc 08 E-4): **Cloud Hypervisor directly orchestrated — single
runtime; Kata and containerd dropped** (§4, doc 08 A-9).
**v6** (doc 08 E-8): **three-plane separation** — admin control plane / tenant control
plane / workload data plane (§3.1).
Simplification rationale: `03-simplification.md` (C-1..C-6, superseded in part by E-4 —
see doc 03 §6).
Storage durability tiers (local NVMe write-back + epoch sync, checkpoint semantics): `04-storage-tiers.md`.
Dedicated host allocation, user placement views, attract/repulse resiliency: `05-dedicated-hosts.md`.

**Position**: PORCHE is a **new orchestrator, built from scratch**. The incumbent
platform's design (summary and known weaknesses in doc 06 §4) is **reference and
inspiration — we adopt design concepts, not software**. There is no legacy code to keep,
extend, or migrate. **Software architecture, languages, and concrete control-plane
middleware are specified in `12-software-architecture.md`** (E-9/E-10); CT-2 in
doc 01 remains guidance that spec applies. What *is* fixed here: the security/performance
model, the microVM isolation model, and the runtime: **Cloud Hypervisor, directly
orchestrated** (E-4 — single VMM, no Kata, no containerd). The incumbent design is improved by:

1. **Cloud Hypervisor, directly orchestrated, as the VM runtime** (instead of the
   incumbent's per-VM QEMU execution path — and instead of the interim Kata plan) —
   decision traceable to doc 06 §5, F43, F45, and E-4 (doc 08 A-9).
2. **Default block encryption** — every volume encrypted at rest by default (the
   incumbent stored plaintext volumes on NFS, replicated to a backup DC).

**Simplification (v3)**: the incumbent design's compensation machinery is **not
re-implemented** — per-VM VMM drivers, custom guest agents and their crypto protocol,
and qcow2 chain management are all dropped in favor of upstream components (doc 03).
**Simplification (v5, E-4) goes further**: the *intermediary layers themselves* are
dropped — containerd, the kata shim, and any guest agent — leaving **host-agent + CLH +
kernel** as the entire data plane (§4).

---

## 1. Design concepts adopted from the incumbent design — and why

The incumbent architecture is sound where it matters; the isolation assessment confirmed
the weaknesses are concentrated in the **data-plane execution path (QEMU) and protocol
authentication**, not in the orchestration model. We adopt the following **concepts** in
the new build (their concrete implementation — middleware, DB, service decomposition,
languages — is `12-software-architecture.md`):

| Incumbent concept | Verdict | Why adopt / reject |
|---|---|---|
| Multi-agent + CQS (command/query separation, agents as execution units) | **Adopt (concept)** | Clean separation of command/query; scales operationally |
| Pending calls persisted before publication (async orders with crash-safe semantics) | **Adopt (concept)** | Exactly the right crash semantics for async orders |
| Message-bus plumbing between control plane and agents (e.g., AMQP patterns) | **Adopt (concept)** | Proven at fleet scale; concrete middleware = later spec |
| Transactional resource accounting (locks against over-allocation) | **Adopt (concept)** | Resource accounting must be transactional |
| **One supervisor process per VM** (incumbent's per-VM VMM driver) | **Redesign** (C-1, E-4) | supervision is the host-agent's lifecycle role over CLH — no wrapper process, no intermediary |
| Per-VM process exec + QMP control | **Reject** | per-VMM driver + QMP control surface; the F43/F31/F44 class (judged against the incumbent's setup — for current-QEMU fairness see SEC-2's LTS comparison rule) |
| NFS shares mounted everywhere → any VM anywhere | **Adopt** | Location independence is a core cloud property |
| qcow2 thin volumes | **Reject format** (C-5) | No snapshots → no chains → no qcow2: raw sparse files inside LUKS2; removes `qemu-img` from the storage path |
| Dedicated storage agents for cold volume creation/replication | **Redesign** | Storage role runs in the new binary (I-13) on any NFS-mounting node; per-file encryption makes placement non-security-critical (doc 08 A-4) |
| Guest agent over vsock (custom protocol) | **Reject — and no replacement** (C-2, E-4) | no guest agent exists at all — F42 fixed by having no channel |
| Metadata service | **Adopt feature, simplify delivery** (C-3) | Per-host proxy + boot-injected single-use token — F0.1/F17 fixed structurally |
| VPC/security-group network model | **Adopt (v1)** | Out of the security-critical path being redesigned |

---

## 2. Improvement register

Each improvement is traceable to a requirement (doc 01) and a root cause (assessment finding).

| # | Improvement | Replaces / adds | Traces to |
|---|---|---|---|
| I-1 | **Cloud Hypervisor directly orchestrated — single runtime** (E-4); the host-agent holds the per-VM lifecycle role (bounded C-1 return) | incumbent: custom exec of QEMU + QMP control; interim Kata plan | SEC-1..8, F43, F31, F44, F34, E-4 |
| I-2 | **Default block encryption** (LUKS2, per-volume keys, KMS); qcow2 **dropped** (C-5); **durability tiers E/R/S** with local-NVMe write-back + epoch sync (doc 04) | plaintext qcow2 on NFS, all-remote synchronous path | SEC-15, PERF-10/11 |
| I-3 | **No host↔guest protocol at all** (C-2, superseded by E-4) — no guest agent, no channel, no key-adoption surface exists | incumbent driver adopts guest-chosen key (F42) | SEC-8, F42 |
| I-4 | **Metadata token auth** (C-3) — single-use, per-boot, boot-injected token; no identity from headers/network position | metadata trusts requester position | SEC-21, F0.1, F17 |
| I-5 | **Version registry + no-EOL policy + CVE feed as deploy gate** | unobservable patch status (F27) | SEC-30/31/32, F43 root cause |
| I-6 | **Host kernel hardening + immutable host OS** (io_uring off, userns off, modules_disabled, verified boot) — base: lean Fedora CoreOS derivative (E-5) | mutable hypervisor OS | SEC-10/11 |
| I-7 | **Network offload policy per workload class** (csum/TSO off for untrusted) | always-on offloads | SEC-7, F45 |
| I-8 | **CI attack suite as deploy gate** | none | SEC-40 |
| I-9 | **Resource envelopes per workload** (cgroup, fds, I/O caps) | per-VM QEMU without envelope | OPS-4, F31/F44 self-DoS |
| I-10 | **Dedicated hosts** — one user per host, allocation lifecycle with attested wipe; user placement views; attract/repulse (soft/strict × host/host-group) ported from the incumbent design | shared hypervisor pool, opaque placement | SEC-16, FR-10/13/14, doc 05 |
| I-11 | **Outbound-only host-agent** (fuse pattern): hosts dial the control plane over mTLS, zero inbound ports on hosts — the inter-host fabric is hostile and no control relies on it | inbound-facing host agents | SEC-20; zero-trust fabric (doc 02 §8 discussion) |
| I-12 | **Desired-state reconciliation** (flintlock pattern): host-agent converges to a desired-state table, not push-only orders — replaces missed-order/rejoin/drift machinery | fire-and-forget push orders | OPS-2/3, FR-12 |
| I-13 | **One binary, multiple roles** (E2B pattern): host-agent, storage worker, build worker = subcommands of one audited binary | separate services per role | CT-3, SEC-30 |
| I-14 | **Standing rules**: workload traffic never transits the control plane (ingress = per-host proxy, **first-class v1** — E-2, IngressRule object in doc 09); every artifact pinned by digest (kernel, runtime, images) in the version registry | control-plane-relayed data path; floating tags | SEC-30/31, FR-20 |

---

## 3. Target architecture

```
            PORCHE control planes (new build — three-plane separation, E-8):
              ADMIN plane (operators): fleet lifecycle, version registry/gates, pipeline
              TENANT plane (users): API, orders, state, placement, metering
            (concrete middleware/languages = 12-software-architecture.md, E-9/E-10)
  ┌───────────────────────────────────────────────────────────────────────────┐
  │  PORCHE API (objects per doc 09) ──► state store (pending calls, objects)  │
  │        command/query separation: API ── orders ──► agents                  │
  │        agents ── feedback/notification ──► API/consumers                    │
  │                                        │                                    │
  │        + KMS service (volume DEK wrap/unwrap, workload tokens) — shared          │
  │          trust anchor, owned by neither plane (E-8)                          │
  │        + HostAllocation registry, placement views, affinity engine         │
  │          (scheduler candidates ⊆ owner's hosts; attract/repulse tags)      │
  └────────────────────────────────────────┼──────────────────────────────────┘
                                           │ outbound mTLS, desired-state pull (I-11/I-12;
                                           │ dual authority — admin lifecycle + tenant desired state, §3.1)
     ┌─────────────────────────────────────┼──────────────────────────────────┐
     │ host-agent (new code, C-4 + E-4 lifecycle role)  │   IaaS — bare metal hosts       │
     │   hardened immutable OS (I-6), CNI      │                                  │
     │     │                                    │                                  │
     │     └─ Cloud Hypervisor — one VMM process per workload (E-4)                 │
     │          └─ microVM ── workload  (stdio = serial console, recorded;      │
     │               no guest agent — SEC-8)                                       │
     │     metadata-proxy (token, I-4)   volume attach: /dev/mapper/<volid>       │
     │                                   (dm-crypt; tier R/E hot path on local     │
     │                                   NVMe, epoch sync to shard — doc 04)       │
     └──────────────────────────────────────────────────────────────────────────┘
     ┌───────────────────────────────────────────────────────────────────────────┐
     │ storage role (new, I-13) — create/clone on NFS                            │
     │   volume create/clone — via storage worker holding                          │
     │   KMS-fetched DEKs (I-2); ciphertext only on NFS                           │
     │ NFS storage pool (kept) — LUKS2 envelopes, raw sparse inside (C-5)           │
     └───────────────────────────────────────────────────────────────────────────┘
```

### 3.1 Three-plane separation (E-8)

The deployment is **multitenant**: the control plane splits into two separately-trusted
planes, and the workload data plane is separate from both.

| Plane | Consumers | Owns | Never |
|---|---|---|---|
| **Admin control plane** | platform operators only | host fleet lifecycle (free pool, drain, reimage, decommission), version registry + deploy gates, image build pipeline, security monitoring, user/quota administration | holds tenant volume keys; sees tenant workload payloads |
| **Tenant control plane** | tenants (authenticated, untrusted) | workload/volume/network-rule (ingress/egress/security-group)/migration orchestration, placement views (scoped to own allocations), quota enforcement, metering | fleet-global views; host administration; any operator capability |
| **Workload data plane** | the workloads themselves | hosts, microVMs, per-host ingress/egress proxies, storage shards | transits either control plane (I-14) |

Cross-plane rules:

1. **Authority separation on the host**: the host-agent holds **two authenticated
   channels** — lifecycle authority (admin plane: drain, reimage, attest) and workload
   desired state (tenant plane). Both reconcile into the **one desired-state table
   (I-12)** with origin-tagged rows: admin orders gate tenant rows (drain wins) but can
   never originate workload operations; tenant rows can never touch host lifecycle.
2. **Blast radius**: compromise of the tenant plane yields at most tenant-scope authority
   — the SEC-16 boundary extended to the control plane: no host administration, no keys
   (KMS unwrap requires host identity, not plane privilege). Compromise of the admin
   plane is operator-scope — contained by audit (SEC-23) and the escape runbook (OPS-10).
3. **Availability decoupling**: admin plane down ⇒ hosts keep serving (lifecycle paused);
   tenant plane down ⇒ no new workloads, running ones unaffected. Plane outage never
   stops running workloads (host autonomy — the open state-authority arbitration must
   preserve this).
4. **KMS is the shared trust anchor** — used by both planes, owned by neither: workload
   tokens (tenant plane), volume DEK wrap/unwrap (host path), host identity (admin
   enrollment). `12-software-architecture.md` decomposes services **per plane**
   (A-7's ≤3-services constraint now applies per plane, not globally).
5. **Audit is unified** (SEC-23): one append-only Merkle chain records actions from both
   planes — the separation is authority, not observability.
6. **Allocation is the plane bridge**: `AllocateHost` is a tenant-plane operation
   fulfilled from the admin-managed free pool — the only place tenant authority creates
   claims on fleet resources (transactional placement, §1).

---

## 4. Deep dive A — Cloud Hypervisor as the sole runtime (I-1, E-4)

### 4.1 Decision and grounds

**E-4 (2026-10-18, doc 08 A-9 resolution)**: PORCHE orchestrates **Cloud Hypervisor
directly** — one VMM per workload, driven by the host-agent over CLH's local API.
**No Kata, no containerd, no shim, no guest agent.** The data plane is three components:
**host-agent, CLH, kernel.**

- **Live migration** (E-3): CLH implements it natively (TLS transport between hosts,
  documented n-2 version compatibility); Kata has no sandbox hand-off (upstream issue
  open since 2021, no roadmap), and the only working third-party Kata migration drives
  QEMU via QMP — excluded by SEC-2.
- **Security**: no guest agent ⇒ **no host↔guest protocol exists** — F42's class is
  eliminated by having no channel (stronger than C-2's fix-by-replacement). No
  annotation surface ⇒ A-2 dissolves. The SBOM shrinks to CLH + kernel + host-agent.
- **Requirements fit**: FR-31/32 already rebuild tenant images into the platform format —
  Kata's runtime OCI handling solved a problem the design had legislated away. FR-6 is
  satisfied by the serial console (virtio-console), recorded host-side.

**Honest costs (CT-3, accepted and bounded)**:

- The host-agent gains the **per-VM lifecycle role** (spawn, supervise, stop, reap,
  migrate) — the C-1 deletion partially returns, bounded: lifecycle and migration driving
  over CLH's HTTP API; no guest protocol, no image handling, no QMP parsing.
- **No exec into a running workload in v1** — a minimal audited guest agent is a phase-2
  addition if a use case demands it (doc 08 action 12).
- **App-consistent seals deferred** — crash-consistency is the default contract (doc 04);
  a guest-side freeze trigger is designed when needed.
- **Guest rootfs images are built by the pipeline** (kernel + initramfs + rootfs, signed,
  digest-pinned — FR-31); real work, but SEC-30/33 already demanded it.

### 4.2 Execution path

| Legacy (incumbent) | v3 (Kata — historical, doc 03) | PORCHE v5 (E-4) |
|---|---|---|
| hypervisor agent forks a per-VM VMM driver | host-agent → CRI → containerd → kata shim → CLH | **host-agent → CLH** (API over a local unix socket) |
| the per-VM driver drives QEMU over QMP | the shim drives CLH | the host-agent drives CLH directly |
| snapshot/migrate/hotplug via QMP block ops | dropped with QMP | migration = CLH native live migration + checkpoint migration (FR-34); volumes attach via CLH's block-device API |
| QEMU device model (full) | CLH minimal device model | CLH minimal device model, frozen by policy (SEC-3/4), **block-only** (SEC-6) |

Per-VM supervision (one process per workload, crash isolation, per-VM logs) comes from the
host-agent's lifecycle role + CLH's process model (one `cloud-hypervisor` process per VM —
itself disposable per SEC-13). No wrapper processes, no CRI hop, no shim, no annotations.

VM configuration is built solely by the host-agent from the workload spec: devices from
the frozen allow-list (SEC-3), no virtio-fs in v1 (SEC-6), resource ceilings (I-9), offload
policy per workload class (I-7).

### 4.3 Storage path (block ops leave the VMM — unchanged by E-4)

the incumbent snapshot-streaming machinery is QEMU-centric (QMP `blockdev-snapshot-sync`,
`block-stream`). **Snapshot chains are not a PORCHE requirement** (checkpoints are —
doc 04), so none of that machinery is carried over: no backing chains, no streaming
jobs, no dead-leaf GC. The only block-layer
operation that remains is **clone-at-create** (volume ← image), executed by a privileged
host-side **storage role** that opens the LUKS envelopes and copies
the decrypted view. Consequences:

- Volume creation = clone from an image, inside the encrypted envelope (I-2).
- The VMM never holds storage keys beyond the attached `/dev/mapper` device (it doesn't need
  any: dm-crypt is opened by the host before attach).

### 4.4 Guest agent: none at all (supersedes C-2)

The incumbent in-guest agent and its custom protocol are deleted — and with E-4 there is
**no replacement**: v1 has no guest agent of any kind. F42's flaw class is eliminated by
having no channel to attack.

| Old incumbent-agent feature | v1 replacement |
|---|---|
| Lifecycle/stdio | workload stdio = serial console (virtio-console), recorded host-side (FR-6) |
| Exec into a running workload | **not in v1** — phase-2 minimal audited agent if demanded (doc 08 action 12) |
| Metadata | metadata-proxy + boot-injected single-use token (C-3) — no guest-side agent needed |
| Guest metrics | host-side only (VMM cgroup + virtio stats) |
| Snapshot sync (app-consistent) | deferred with the guest agent; crash-consistent checkpoints are the default (doc 04) |

### 4.5 What this kills from the threat model

- The QEMU device surface of the assessment (F43, F31, F44, F34, F33, F35) — no QEMU in the
  data plane at all. (For the record: this rejection is maintained against the latest
  QEMU LTS, not those findings — see SEC-2's comparison rule; the findings motivate the
  *class* concern: device-model surface + version-governance risk.)
- QMP/virtqueue parsing complexity in a long-lived process — the VMM is disposable per
  workload (SEC-13) and no PORCHE code parses guest-controlled frames (F42's parser surface
  is gone).
- **Kata's attack surfaces are gone with Kata** (E-4): the agent protocol, the annotation
  surface, the shim — none exists.
- Version-lag blast radius shrinks to **CLH/kernel**, which are covered by I-5/I-8.

### 4.6 Live migration (E-3; mechanism resolved by E-4)

Live migration is a **v1 goal** (FR-34, PERF-13), not PoC scope (doc 07 §3). **E-4
resolves the mechanism: CLH's native live migration** — TLS transport between hosts,
documented n-2 version compatibility (fleet-upgrade friendly, doc 10 rings).

- **Volume dimension (the harder half)**: VMM migration transfers RAM + device state, not
  locally attached block storage. Live migration is only clean when volumes are
  network-attached at cutover — tier S by construction; tier R **elevated to write-through
  for the migration window** (which depends on the A-1 sealable-stack spike); tier E stays
  on checkpoint migration (ephemeral by choice).
- **Gate (doc 08 A-9, action 9)**: no v1 commitment before a **two-host spike on the PoC
  stack** proves it end-to-end — RAM/state transfer, encrypted-volume cutover, PERF-13
  under load.
- **Upstream watch (CT-5)**: if Kata ever ships sandbox live migration for CLH, the
  single-runtime choice is re-evaluated per the doc 11 §4 comparison rule — the design
  must not grow dependencies that would block that re-entry.

---

## 5. Deep dive B — Default block encryption (I-2)

### 5.1 Threat model being closed

Today a volume is a plaintext qcow2 file on NFS, replicated to a backup DC. Exposed parties: NFS-storage administrators, backup-tape and replication-target
access, physical theft of storage media, and any future storage-side compromise. None of these
should be able to read tenant data.

### 5.2 Scheme

- **Container**: each volume = one **LUKS2 container file** on the NFS shard, with a **raw
  sparse image inside** (C-5 — no qcow2, no `qemu-img` anywhere). Thin provisioning comes
  from file sparseness; the guest sees the full virtual size. The file is self-contained and
  cloned once at creation.
- **Keys**: per-volume **DEK** (LUKS2 volume key), generated at volume creation by the
  **KMS** (new control-plane service). DEKs are stored in PostgreSQL only **wrapped** under
  a KEK that lives solely in the KMS (HSM or quorum-protected at rest). Nobody outside the
  KMS ever sees a KEK.
- **Attach path**: on `CreateInstance`/attach, the host-agent requests the wrapped
  DEK from the KMS (authenticated as the workload's host, scoped to that volume ID), opens
  the LUKS container (`cryptsetup open`, key in the kernel keyring, zeroized at detach/VM
  teardown), and hands `/dev/mapper/porche-<volid>` to CLH as a direct-assigned virtio-blk
  device. **Ciphertext only ever touches NFS; the NFS storage never holds a key.**
- **Images and volumes**: an image (template) is a LUKS-encrypted artifact; a volume is cloned from
  it by the storage worker (KMS-authenticated unwrap of both source and target). Encryption
  is inherited by construction — the clone happens on the decrypted view inside a new
  envelope.
- **Default-on, not opt-in**: creation path has no plaintext mode (SEC-15). Tenant-managed
  keys (CMK) are a phase-2 extension of the same wrapping hierarchy.

### 5.3 What it does NOT cover (explicit non-goals)

- Not protection against the **running hypervisor** (it sees the decrypted device by
  necessity) — that boundary is I-1's job.
- Not guest-visible encryption (no trusted-computing/attestation claims for the tenant).
- Memory is not encrypted at rest (host RAM) — out of scope v1; teardown wipe stays (SEC-14).

### 5.4 Performance budget

dm-crypt (AES-XTS, AES-NI) costs ~5–10% on sequential I/O and near-zero on CPU-bound work.
Added requirement **PERF-9**: sequential I/O overhead of the encryption layer < 15% vs the
unencrypted path, measured by fio in CI on the reference hardware. No chain-depth penalty:
without snapshots, a volume's backing depth is always zero. Reclaim: hole-punch on TRIM
(`fallocate -d`) keeps sparse files small under the guest's filesystem.

### 5.5 Migration

PORCHE is a new build on its own bare metal — there is **no legacy fleet and no volume
import path**; encryption is default-on from the first volume.

### 5.6 Durability tiers & checkpoint sync (doc 04)

The all-remote-NFS path of §5.2 is only one of three per-volume durability tiers:

| Tier | Write path | Host-loss behavior |
|---|---|---|
| **E** ephemeral | local NVMe only | gone; reschedule from image |
| **R** replicated-async | local NVMe + epoch sync (`dm-era`, ciphertext-only ships) to the remote LUKS envelope | restart from latest sealed checkpoint, on any host; RPO ≤ 60 s default |
| **S** synchronous | write-through to the remote envelope (§5.2 as-is) | no data loss |

The stack for tier R: `guest → dm-crypt → dm-era → local NVMe region (hot) + remote LUKS
envelope (origin)`. Checkpoint sealing = quiesce → ship the epoch's dirty ciphertext blocks
→ CRC-verify → record seal in the control plane → advance epoch. Sealed checkpoints are crash-consistent
(guest fs journal replays); app-consistency is opt-in via a guest-side freeze — **deferred
with the guest agent** (E-4): crash-consistency is the default contract.

**The snapshot API is the sync API**: `SyncVolume` seals a checkpoint; restore boots from a
sealed checkpoint anywhere; retention "latest + N" with background compaction (linear,
single-parent increments folded into the base — no user-visible chains). Cross-DC
replication = async shipping of immutable sealed checkpoints. Consistency argument,
security review, and rejected alternatives: `04-storage-tiers.md`.

Each tier is a performance/latency/**cost** trade (E cheapest+fastest, R ≈ E performance
with a survivable copy at ≈ 1× amortized ship cost — the correct default, S the expensive
synchronous path), and remote copies split into hot (SSD shard: latest checkpoint, tier-S
origins) and cold (erasure-coded: older immutable checkpoints ≈ 5–10× cheaper per GB)
**storage classes** — tier choice and retention policy are explicit cost levers, metered
per workload (FR-42). Economics: `04-storage-tiers.md` §2.1–2.3.

---

## 6. Concept mapping — incumbent design → PORCHE (new build)

The left column describes the *incumbent platform's* components (doc 06 §4); the right
column is what the new orchestrator does instead. Nothing is reused as software.

| Incumbent component | PORCHE (new build) | Change |
|---|---|---|
| Control plane (intel/manager/RMQ/Postgres/periodic) | **new control plane**, same concepts (CQS, multi-agent, pending calls, transactional placement) | specified in `12-software-architecture.md` (E-9/E-10 — no bus in v1, three services); concept adoptions per §1 |
| Public API surface | **PORCHE API** (objects per doc 09) | not carried over; only workloads, volumes, hosts, placement |
| — | **KMS** (new) | volume DEK wrap/unwrap, workload tokens (I-2/I-4) |
| per-VM VMM driver | **host-agent lifecycle role** (E-4 — bounded return of C-1) | direct CLH orchestration; no containerd, no shim |
| QEMU + QMP | **CLH, directly orchestrated** (E-4) | rejected (I-1); no QEMU/QMP, no Kata in data plane |
| guest agent (custom protocol) | **none at all** (C-2, superseded by E-4) | no guest agent exists — no host↔guest protocol |
| metadata service | **metadata-proxy** (C-3) | per-host HTTPS proxy + boot-injected single-use token (I-4) |
| storage agents | **storage role** in the one binary (I-13) | volume create/clone key-aware, raw+LUKS2 (I-2, C-5) |
| NFS storage pool | adopted | ciphertext only on the pool |
| hypervisor OS | immutable, hardened, verified boot | new ops model (I-6) |
| per-host networking | CNI plugins (C-6) | standardized; VPC model adopted (v1) |

---

## 7. Phasing

| Phase | Scope | Exit criteria |
|---|---|---|
| P0 | 1 host, control plane v0 (new build), host-agent + CLH (direct, E-4), one encrypted volume round-trip (create→attach→write→detach) | doc 01 acceptance criteria 1–7 |
| P1 | Full create-instance flow on the new path; storage worker (clone) under encryption; KMS in HA; CNI networking; ingress data path (per-host proxy, E-2); **live-migration two-host spike on the direct-CLH stack (A-9, E-4)** | PERF-1/5/8/9 met; F42-fix PoC (no guest-controlled channel exists to attack); F0.1-fix PoC (metadata without token fails); PERF-13 evidenced on the spike |
| P2 | GPU (VFIO/MIG via CLH hotplug), runtime volume attach/detach (hotplug), live migration GA (PERF-13), CMK option for tenants | GPU PoC on A100; PERF-13 met; CMK wrap hierarchy reviewed |
| P3 | HA hardening, fleet version-registry enforcement, CI attack suite at full coverage | I-5 gate enforced (deploys blocked on EOL/CVE) |

---

## 8. Open questions

- [ ] Guest image tooling for direct CLH: kernel/initramfs/rootfs build and boot-time
      budget (PERF-2) — decided in the PoC (doc 08 action 11).
- [ ] Minimal guest agent (exec, app-consistent seal trigger) — design deferred to phase 2,
      only if a use case demands it (E-4, doc 08 action 12).
- [ ] Does the metadata proxy live per-host or in the control plane? (Per-host = smaller blast radius;
      SI = one place to audit.)
- [ ] Image pull latency vs PERF-1: per-host pull-through cache sizing, and whether guest
      rootfs should be pre-seeded on the host image for the base catalog (decided in P0).
- [x] ~~Multi-tenant L2 on the adopted fabric/VPC model: revisit VLAN-tag trust (F45 used
      a 2-VLAN-tag bypass in a lab PoC).~~ — **resolved by doc 14 §9**: no VLANs in the
      workload path at all; native netns + isolated bridge ports carry the isolation.
