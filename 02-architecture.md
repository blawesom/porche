# 02 — Core Architecture Design (new orchestrator, incumbent design as reference)

Working document — v4, 2026-10-18. Implements `01-requirements.md`.
**v4 scope change** (doc 08 E-1..E-3): stateful workloads and ingress first-class v1;
live migration is a v1 goal — runtime selection re-opened (§4.5, doc 08 A-9).
Simplification rationale: `03-simplification.md` (C-1..C-6 applied here).
Storage durability tiers (local NVMe write-back + epoch sync, checkpoint semantics): `04-storage-tiers.md`.
Dedicated host allocation, user placement views, attract/repulse resiliency: `05-dedicated-hosts.md`.

**Position**: PORCHE is a **new orchestrator, built from scratch**. The incumbent
platform's design (summary and known weaknesses in doc 06 §4) is **reference and
inspiration — we adopt design concepts, not software**. There is no legacy code to keep,
extend, or migrate. **Software architecture, languages, and concrete control-plane
middleware are not specified in this corpus** — they are a later specification; CT-2 in
doc 01 remains guidance for that spec. What *is* fixed here: the security/performance
model and the microVM isolation model; the **concrete runtime stack (currently Kata + CLH)
is a v1 selection, not a commitment** — live-migration capability is a selection criterion
(E-3, doc 08 A-9). The incumbent design is improved by:

1. **Kata Containers + Cloud Hypervisor as the base VM runtime** (instead of the
   incumbent's per-VM QEMU execution path) — decision traceable to doc 06 §5, F43, F45.
   **Current default, not definitive** (E-3): the live-migration requirement re-opens the
   selection — §4.5, doc 08 A-9.
2. **Default block encryption** — every volume encrypted at rest by default (the
   incumbent stored plaintext volumes on NFS, replicated to a backup DC).

**Simplification (v3)**: the incumbent design's compensation machinery is **not
re-implemented** — per-VM VMM drivers, custom guest agents and their crypto protocol,
and qcow2 chain management are all dropped in favor of upstream components (doc 03).

---

## 1. Design concepts adopted from the incumbent design — and why

The incumbent architecture is sound where it matters; the isolation assessment confirmed
the weaknesses are concentrated in the **data-plane execution path (QEMU) and protocol
authentication**, not in the orchestration model. We adopt the following **concepts** in
the new build (their concrete implementation — middleware, DB, service decomposition,
languages — is the later software-architecture spec):

| Incumbent concept | Verdict | Why adopt / reject |
|---|---|---|
| Multi-agent + CQS (command/query separation, agents as execution units) | **Adopt (concept)** | Clean separation of command/query; scales operationally |
| Pending calls persisted before publication (async orders with crash-safe semantics) | **Adopt (concept)** | Exactly the right crash semantics for async orders |
| Message-bus plumbing between control plane and agents (e.g., AMQP patterns) | **Adopt (concept)** | Proven at fleet scale; concrete middleware = later spec |
| Transactional resource accounting (locks against over-allocation) | **Adopt (concept)** | Resource accounting must be transactional |
| **One supervisor process per VM** (incumbent's per-VM VMM driver) | **Reject** (C-1) | containerd + kata shim already provide one process per sandbox — a wrapper is pure glue (CT-3) |
| Per-VM process exec + QMP control | **Reject** | per-VMM driver + QMP control surface; the F43/F31/F44 class (judged against the incumbent's setup — for current-QEMU fairness see SEC-2's LTS comparison rule) |
| NFS shares mounted everywhere → any VM anywhere | **Adopt** | Location independence is a core cloud property |
| qcow2 thin volumes | **Reject format** (C-5) | No snapshots → no chains → no qcow2: raw sparse files inside LUKS2; removes `qemu-img` from the storage path |
| Dedicated storage agents for cold volume creation/replication | **Redesign** | Storage role runs in the new binary (I-13) on any NFS-mounting node; per-file encryption makes placement non-security-critical (doc 08 A-4) |
| Guest agent over vsock (custom protocol) | **Reject** (C-2) | kata-agent (mutual TLS over vsock) becomes the only guest agent — F42 fixed by deletion, not by redesign |
| Metadata service | **Adopt feature, simplify delivery** (C-3) | Per-host proxy + boot-injected single-use token — F0.1/F17 fixed structurally |
| VPC/security-group network model | **Adopt (v1)** | Out of the security-critical path being redesigned |

---

## 2. Improvement register

Each improvement is traceable to a requirement (doc 01) and a root cause (assessment finding).

| # | Improvement | Replaces / adds | Traces to |
|---|---|---|---|
| I-1 | **Kata 4.x + Cloud Hypervisor as base VM runtime**; per-VM VMM driver **not re-implemented** (C-1) | incumbent: custom exec of QEMU + QMP control | SEC-1..8, F43, F31, F44, F34, doc 06 §5 |
| I-2 | **Default block encryption** (LUKS2, per-volume keys, KMS); qcow2 **dropped** (C-5); **durability tiers E/R/S** with local-NVMe write-back + epoch sync (doc 04) | plaintext qcow2 on NFS, all-remote synchronous path | SEC-15, PERF-10/11 |
| I-3 | **Custom guest protocol deleted** (C-2) — kata-agent mTLS is the only guest channel; no key adoption surface exists | incumbent driver adopts guest-chosen key (F42) | SEC-8, F42 |
| I-4 | **Metadata token auth** (C-3) — single-use, per-boot, boot-injected token; no identity from headers/network position | metadata trusts requester position | SEC-21, F0.1, F17 |
| I-5 | **Version registry + no-EOL policy + CVE feed as deploy gate** | unobservable patch status (F27) | SEC-30/31/32, F43 root cause |
| I-6 | **Host kernel hardening + immutable host OS** (io_uring off, userns off, modules_disabled, verified boot) | mutable hypervisor OS | SEC-10/11 |
| I-7 | **Network offload policy per workload class** (csum/TSO off for untrusted) | always-on offloads | SEC-7, F45 |
| I-8 | **CI attack suite as deploy gate** | none | SEC-40 |
| I-9 | **Resource envelopes per workload** (cgroup, fds, I/O caps) | per-VM QEMU without envelope | OPS-4, F31/F44 self-DoS |
| I-10 | **Dedicated hosts** — one user per host, allocation lifecycle with attested wipe; user placement views; attract/repulse (soft/strict × host/host-group) ported from the incumbent design | shared hypervisor pool, opaque placement | SEC-16, FR-10/13/14, doc 05 |
| I-11 | **Outbound-only host-agent** (fuse pattern): hosts dial the control plane over mTLS, zero inbound ports on hosts — the inter-host fabric is hostile and no control relies on it | inbound-facing host agents | SEC-20; zero-trust fabric (doc 02 §8 discussion) |
| I-12 | **Desired-state reconciliation** (flintlock pattern): host-agent converges to a desired-state table, not push-only orders — replaces missed-order/rejoin/drift machinery | fire-and-forget push orders | OPS-2/3, FR-12 |
| I-13 | **One binary, multiple roles** (E2B pattern): host-agent, storage worker, build worker = subcommands of one audited binary | separate services per role | CT-3, SEC-30 |
| I-14 | **Standing rules**: workload traffic never transits the control plane (ingress = per-host proxy, **first-class v1** — E-2, Exposure object in doc 09); every artifact pinned by digest (kernel, runtime, images) in the version registry | control-plane-relayed data path; floating tags | SEC-30/31, FR-20 |

---

## 3. Target architecture

```
            PORCHE control plane (new build — concepts from the incumbent design;
            concrete middleware/languages = later software-architecture spec)
  ┌───────────────────────────────────────────────────────────────────────────┐
  │  PORCHE API (objects per doc 09) ──► state store (pending calls, objects)  │
  │        command/query separation: API ── orders ──► agents                  │
  │        agents ── feedback/notification ──► API/consumers                    │
  │                                        │                                    │
  │        + KMS service (volume DEK wrap/unwrap, workload tokens)             │
  │        + HostAllocation registry, placement views, affinity engine         │
  │          (scheduler candidates ⊆ owner's hosts; attract/repulse tags)      │
  └────────────────────────────────────────┼──────────────────────────────────┘
                                           │ outbound mTLS, desired-state pull (I-11/I-12)
     ┌─────────────────────────────────────┼──────────────────────────────────┐
     │ host-agent (new code, C-4)             │    IaaS — bare metal hosts       │
     │   hardened immutable OS (I-6), CNI      │                                  │
     │     │                                    │                                  │
     │     └─ containerd (CRI) ── runtime class kata-clh                         │
     │          └─ kata shim (1/sandbox) ── Cloud Hypervisor VMM (I-1)            │
     │               └── microVM ── [ kata-agent | workload ]  (no custom      │
     │                    guest agent — C-2)                                       │
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

---

## 4. Deep dive A — Kata as the base VM (I-1)

### 4.1 Execution path redesign

| Legacy today | PORCHE (v3) |
|---|---|
| the hypervisor agent forks a per-VM VMM driver process (one per VM) | host-agent talks **CRI to containerd** — no per-VM fork of our own (C-1) |
| the per-VM driver forks QEMU, drives it over a QMP unix socket | containerd starts the **kata shim** (one per sandbox) → **Cloud Hypervisor** VMM |
| the per-VM driver implements snapshot/migrate/hotplug via QMP block ops | dropped **with QMP** — migration re-enters as a v1 requirement by different mechanisms (FR-34: checkpoint migration + live migration, §4.5); volumes attach at create via CRI/direct assignment |
| QEMU device model (full) | CLH minimal device model, frozen by policy (SEC-3/4) |

The per-sandbox supervision property the incumbent design got from its per-VM driver now comes from the
kata shim (one process per sandbox, managed by containerd, per-sandbox logs and crash
isolation). **No wrapper, no ZMQ per-VM protocol, no QMP parsing of untrusted input** —
Its replacement is a small CRI client plus host duties (see §6).

Configuration: runtime class `kata-clh` per doc 06 §5 hardening points — annotation allow-list
empty (SEC-5), virtio-fs sources restricted (SEC-6), guest seccomp on, per-sandbox ceilings
(I-9).

### 4.2 Storage path relocation (block ops leave the VMM)

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

### 4.3 Guest agent: kata-agent is the only one (C-2)

The incumbent in-guest agent and its custom protocol are **deleted**, not adapted. The F42
flaw (the host driver adopting a guest-chosen key from an unauthenticated endpoint) disappears with the
protocol: kata-agent's channel is mutually authenticated TLS over vsock, owned and fuzzed
upstream, keys bound to the sandbox.

What survives, and how:

| Old incumbent-agent feature | v3 replacement |
|---|---|
| Lifecycle/stdio/exec | kata-agent (upstream) + CRI streaming |
| Metadata | metadata-proxy + boot-injected single-use token (C-3) — no guest-side agent needed |
| Guest metrics | v1: host-side only (VMM cgroup + virtio stats); in-guest scraping deferred |
| Snapshot sync | dropped (not a requirement) |

### 4.4 What this kills from the threat model

- The QEMU device surface of the assessment (F43, F31, F44, F34, F33, F35) — no QEMU in the
  data plane at all. (For the record: this rejection is maintained against the latest
  QEMU LTS, not those findings — see SEC-2's comparison rule; the findings motivate the
  *class* concern: device-model surface + version-governance risk.)
- QMP/virtqueue parsing complexity in a long-lived process — the VMM is disposable per
  workload (SEC-13) and no PORCHE code parses guest-controlled frames (F42's parser surface
  is gone).
- Version-lag blast radius shrinks to CLH/kata/kernel, which are covered by I-5/I-8.

### 4.5 Live migration and the runtime question (E-3 — new in v4)

Live migration is a **v1 goal** (FR-34, PERF-13), not PoC scope (doc 07 §3). This
deliberately re-opens the runtime choice:

- **Cloud Hypervisor** implements VMM-level live migration (upstream protocol); what
  transfers is guest RAM + device state — not locally attached block storage.
- **Kata Containers does not migrate sandboxes today** — the kata shim lifecycle has no
  hand-off of a running VMM. Kata is therefore **not a definitive choice** (operator
  decision, E-3): the v1 runtime selection is a gate with live migration as a criterion.
- Options on the table: **direct CLH orchestration** for stateful workloads (the escape
  hatch doc 06 §5 already listed — full VMM control, migration protocol reachable, but
  re-introduces per-VMM lifecycle glue that C-1 deleted — honest CT-3 cost); **upstream
  Kata live-migration support** (zero glue if it lands; out of our control); **two runtime
  paths** (Kata for ephemeral, direct CLH for stateful — hedged, but two data-plane
  surfaces to audit and attack-test, SEC-40 matrix cost). The runtime-class hedge (A-6)
  keeps any swap a tested configuration change (CT-5).
- **Volume dimension (the harder half)**: live migration is only clean when volumes are
  network-attached at cutover — tier S by construction; tier R **elevated to write-through
  for the migration window** (which itself depends on the A-1 sealable-stack spike); tier E
  stays on checkpoint migration (ephemeral by choice).

**Gate (doc 08 A-9)**: no v1 live-migration commitment before a two-host spike proves the
chosen path end-to-end — RAM/state transfer, encrypted-volume cutover, PERF-13 under load.
The PoC is unaffected (its three bets don't change); the spike follows the PoC and feeds
the v1 runtime selection.

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
(guest fs journal replays); app-consistency is opt-in via `fsfreeze` through kata-agent exec.

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
| Control plane (intel/manager/RMQ/Postgres/periodic) | **new control plane**, same concepts (CQS, multi-agent, pending calls, transactional placement) | re-specified in the later software-architecture spec; concept adoptions per §1 |
| Public API surface | **PORCHE API** (objects per doc 09) | not carried over; only workloads, volumes, hosts, placement |
| — | **KMS** (new) | volume DEK wrap/unwrap, workload tokens (I-2/I-4) |
| per-VM VMM driver | **not re-implemented** (C-1) | containerd + kata shim (upstream, one process per sandbox) |
| QEMU + QMP | **kata-clh runtime (CLH)** | rejected (I-1); no QEMU/QMP in data plane |
| guest agent (custom protocol) | **not re-implemented** (C-2) | kata-agent is the only guest agent (mTLS, upstream) |
| metadata service | **metadata-proxy** (C-3) | per-host HTTPS proxy + boot-injected single-use token (I-4) |
| storage agents | **storage role** in the one binary (I-13) | volume create/clone key-aware, raw+LUKS2 (I-2, C-5) |
| NFS storage pool | adopted | ciphertext only on the pool |
| hypervisor OS | immutable, hardened, verified boot | new ops model (I-6) |
| per-host networking | CNI plugins (C-6) | standardized; VPC model adopted (v1) |

---

## 7. Phasing

| Phase | Scope | Exit criteria |
|---|---|---|
| P0 | 1 host, control plane v0 (new build), host-agent + containerd + kata-clh, one encrypted volume round-trip (create→attach→write→detach) | doc 01 acceptance criteria 1–7 |
| P1 | Full create-instance flow on the new path; storage worker (clone) under encryption; KMS in HA; CNI networking; ingress data path (per-host proxy, E-2); **live-migration two-host spike → v1 runtime decision (A-9, E-3)** | PERF-1/5/8/9 met; F42-fix PoC (no guest-controlled channel exists to attack); F0.1-fix PoC (metadata without token fails); runtime decision recorded (PERF-13 evidence or runtime swapped, CT-5) |
| P2 | GPU (VFIO/MIG via CLH hotplug), runtime volume attach/detach (hotplug), live migration GA (from the P1 decision), CMK option for tenants | GPU PoC on A100; PERF-13 met; CMK wrap hierarchy reviewed |
| P3 | HA hardening, fleet version-registry enforcement, CI attack suite at full coverage | I-5 gate enforced (deploys blocked on EOL/CVE) |

---

## 8. Open questions

- [ ] Volume attach model in v1: CRI block mounts vs direct-assignment annotations — pick
      the least-privileged path that keeps `/dev/mapper` devices out of virtio-fs.
- [ ] Live-migration runtime path (A-9): direct CLH for stateful workloads vs upstream Kata
      support vs two runtime paths — decided by the two-host spike (P1); gate for v1.
- [ ] Does the metadata proxy live per-host or in the control plane? (Per-host = smaller blast radius;
      SI = one place to audit.)
- [ ] Image pull latency vs PERF-1: per-host pull-through cache sizing, and whether guest
      rootfs should be pre-seeded on the host image for the base catalog (decided in P0).
- [ ] Multi-tenant L2 on the adopted fabric/VPC model: revisit VLAN-tag trust (F45 used a
      2-VLAN-tag bypass in a lab PoC).
