# 01 — Requirements Specification

Working document — v4, 2026-10-18. First normative document of the PORCHE project.
**v2** (doc 08 E-1..E-3): stateful enterprise workloads and ingress access
are first-class v1 citizens; live migration is a v1 goal.
**v3** (doc 08 E-4): **Cloud Hypervisor, directly orchestrated, is the single runtime** —
Kata and containerd are dropped from the data plane (CT-5).
**v4** (doc 08 E-8): **three-plane separation** — admin control plane / tenant control
plane / workload data plane.
**v5** (doc 08 E-9): FR-5 TTL re-scoped to per-workload-class policy (operator-waivable
for stateful U4).

**Conventions (RFC 2119, doc 11 §1)**: every FR/SEC/PERF/OPS statement is **MUST** —
normative, violation is a defect — unless explicitly marked **SHOULD** (deviation requires
recorded justification) or **MAY** (optional). Terms carry the doc 11 glossary meanings.

**Purpose**: define the requirements of a bare metal cloud orchestrator for untrusted workloads
(AI agents, jobs, builds) **and long-lived stateful enterprise applications**, GPU in phase 2.
First-in-class performance and security, by design.

**Reference adversary**: a compromised workload running on the pool — it attempts guest→host
escape, neighbor attacks, and control-plane bypass. Secondary adversary: the curious legitimate
tenant (platform enumeration, fingerprinting).

**Traceability sources** (all in doc 06 — this directory is self-contained):
- `[R-xx]` = finding from the incumbent-platform assessment — F-register, doc 06 §3
- `[C-yy]` = CVE / cluster from the vulnerability corpus — doc 06 §2
- `[D-yy]` = decision from the stack study — doc 06 §5

---

## 1. Scope

### 1.1 What PORCHE is

- An orchestrator that **provisions and manages bare metal hosts** (immutable, hardened OS) and
  **runs isolated workloads** on them (microVMs, container-compatible API).
- A platform where **stateful workloads are first-class citizens** (E-1): long-lived enterprise
  applications (databases, app servers, queues) run on persistent tier R/S volumes, serve
  ingress, and are **live-migratable** (v1 goal, E-3) for non-disruptive maintenance.
- Two control planes and a workload data plane (E-8): the **admin control plane** (host
  fleet lifecycle, version registry, image pipeline, monitoring — operators only) and the
  **tenant control plane** (workload/volume/network-rule orchestration, placement views, quotas,
  metering), plus the **workload data plane** (hosts, microVMs, per-host proxies, storage
  shards — tenant traffic never transits a control plane).
- A set of verifiable security guarantees (every SEC requirement is testable).

### 1.2 What PORCHE is not (non-goals)

- **Not a general-purpose public IaaS** — no commercial multi-region, no heterogeneous instance
  catalog, no customer billing.
- **Not Kubernetes** — no dependency on the K8s ecosystem (but OCI image compatibility).
- **No heterogeneous guest OSes** (category 5) in v1 — Windows is out of scope
  (re-evaluation possible in phase 3+). **Live migration is in scope as a v1 goal** (E-3):
  it applies to microVMs and does not require category-5 VMs. It is gated on the v1 runtime
  selection (doc 08 A-9), not deferred to phase 3+.
- **No trust in the workload** — no feature may depend on the goodwill of executed code
  (a "cooperative agent" is never the only control).

---

## 2. Target use cases (by priority)

| # | Use case | Profile | Dominant constraint |
|---|----------|---------|---------------------|
| U1 | Untrusted AI agent execution (generated code, tool-use) | 0.5–4 vCPU, 0.5–8 GB, ephemeral (min–hours), controlled egress | Strong isolation + fast boot |
| U2 | Disposable build/CI jobs | 1–8 vCPU, bursty I/O, controlled rootfs | Density + ephemerality |
| U3 | Dev environments | persistent, long sessions | Persistent volumes, long-lived VMs |
| U4 | Stateful enterprise applications (databases, app servers, queues) | 2–32 vCPU, 2–128 GB, tier R/S volumes, long-lived (weeks–years), serves ingress | Live migration + non-disruptive maintenance (E-1/E-2/E-3) |
| U5 | GPU workloads (inference, fine-tuning) | phase 2: A100 via VFIO passthrough or MIG | Isolation + GPU scheduling |

---

## 3. Functional requirements

### 3.1 Workload lifecycle

- **FR-1** — API to create a workload from: (a) an OCI image, (b) a command + minimal rootfs.
  Asynchronous response with observable state.
- **FR-2** — Managed states: `pending → scheduled → starting → running → stopped/failed → reaped`.
  Every transition timestamped and queryable (append-only audit log).
- **FR-3** — Deterministic teardown: the workload and **all** of its resources (ephemeral volume,
  netns, residual memory) are wiped; the wipe is **reported** in the audit log (host
  self-report — verification is external and proportional: residual-memory probe at PoC,
  randomized spot audits on host release, TPM-backed measurement as a phase-3 option; see
  doc 08 A-3). **Wipe semantics (E-6)**: storage regions are wiped by **crypto-erase only** —
  zeroize the volume DEK, then TRIM/discard the region; **physical overwrite is never
  performed** (NAND wear — operator decision). Memory is wiped by unmap/zero (SEC-14).
  Verification therefore means "no *cryptographically recoverable* data".
- **FR-4** — Guaranteed delay between "API accepts" and "process visible inside the guest":
  P50 < 1.5 s, P99 < 5 s (see PERF-1).
- **FR-5** — Per-tenant quotas: vCPU, RAM, workload count, egress, **local NVMe space**,
  **storage-network reservations (network tiers only)**, max lifetime (mandatory TTL —
  **policy per workload class** (E-9): enforced locally on the host, fail-safe under
  partition; operator-waivable for stateful workloads (U4); never waivable by tenant
  self-service).
- **FR-6** — Console/stdio: access to stdout/stderr streams and a serial console, **recorded**
  (not just relayed) for execution audit. Mechanism (E-4): the workload's stdio is its
  serial console (virtio-console), recorded host-side. **Exec into a running workload is
  not a v1 feature** (no guest agent exists — E-4; a minimal audited guest agent is a
  phase-2 addition if a use case demands it).

### 3.2 Scheduling and host allocation

- **FR-10** — Placement within the **owner's allocated hosts only**: a bare metal host is
  allocated to at most one user at a time (hard boundary). Attract/repulse affinity tags
  (soft/strict × host/host-group scopes, peer-group semantics, doc 05 §3) drive spread and
  co-location; strict constraints never silently dropped — unsatisfiable ⇒ `pending` with a
  machine-readable reason.
- **FR-11** — GPU scheduler (phase 2): IOMMU group inventory, MIG partitions, queue with
  preemption of CPU workloads.
- **FR-12** — Drain: host drain (release, maintenance) moves/terminates workloads without
  per-workload manual intervention; tier-R volumes restore from their latest sealed
  checkpoint on another of the user's hosts.
- **FR-13** — Host allocation lifecycle: `free → allocated(user) → draining → free`, with
  attested wipe (NVMe regions, detached envelopes) before returning to the pool.
- **FR-14** — **Placement views** (scoped strictly to the caller's allocations): list hosts,
  workload-per-host distribution, per-group aggregates, placement decisions with reasons
  (incl. pending causes), and a resilience report (replica spread, single points of failure).
  Nothing about other users or the global fleet is exposed. Provisioning catalog
  (`ListAvailableHosts`: hardware profile, group, price) is likewise scoped.

### 3.3 Networking

- **FR-20** — Default egress NAT through an **authenticated, filtered** proxy governed by
  **EgressRule objects** (doc 09: per-workload or VPC-default allow-list of FQDNs/CIDRs;
  DNS resolved by the proxy — a name rule cannot be bypassed by raw IP); **no ingress by
  default**, but inbound access is a **first-class v1 capability** (E-2) governed by
  **IngressRule objects** (doc 09): explicit, auditable, revocable, enforced by the
  per-host data path, never by guest cooperation. Both rule types are created, changed,
  and revoked **through the control API only** — the guest has no path to open, alter, or
  close one (a compromised workload cannot widen its own network surface).
- **FR-21** — Strict L2 isolation between workloads (netns per VM, no bridge shared between
  tenants); verifiable by test (see SEC-30).
- **FR-22** — Per-workload egress bandwidth cap (token bucket); the cap is enforced by the
  control plane, not by the guest.
- **FR-23** — **User-defined address range**: a tenant may define the RFC1918 range from
  which their workloads' /32s are assigned (bounded size, /24–/16); the platform validates
  non-overlap against all other users' ranges **and platform-reserved ranges**
  (fabric, storage net, link-local), transactionally, rejecting conflicts with
  machine-readable reasons; if unspecified, the platform allocates. Range choice affects
  **allocation policy only** — never reachability (FR-20/21 unchanged; doc 14 §3). [E-13]

### 3.4 Storage and images

- **FR-30** — Volumes carry an immutable **durability tier** chosen at creation:
  **E** (ephemeral, local NVMe only, wiped at teardown), **R** (local NVMe + async epoch
  sync to the network envelope, RPO ≤ 60 s default), **S** (synchronous write-through to the
  network envelope). Tier-R/S remote copies pick a **storage class** (hot SSD shard for the
  latest checkpoint / synchronous origin; cold erasure-coded for older immutable
  checkpoints). The full **volume specification** = size + tier + class + (network tiers
  only) **storage-network bandwidth reservation** (token-bucket on storage traffic;
  effective throughput = min(reservation, class capability), effective latency = published
  host↔shard path latency). **Local storage is bounded by space only**: capacity is
  accounted physically at placement — `Σ(E sizes) + Σ(R dirty budgets) ≤ host NVMe` — and
  local performance is the raw device, unallocated and unshaped; the space bound and the
  perf bound are distinct contracts (local = space, network = performance). **Persistent
  volumes (tier R/S, encrypted) are v1** — the stateful-workload path (E-1); runtime
  attach/detach of a volume to a *running* workload is phase 2.
- **FR-31** — Guest image pipeline: immutable, signed, versioned rootfs; **no tenant-supplied
  image is executed directly** — it is either rebuilt (controlled build) or run through an
  intake sandbox (see SEC-40).
- **FR-32** — Image import: **structural rejection of backing chains** and of any format
  requiring host file access (lesson R-F22). Imported images are re-packed by the pipeline
  into the platform format.
- **FR-33** — **Checkpoint/sync API** (replaces snapshotting): `SyncVolume` seals a
  crash-consistent checkpoint of a tier-R volume on network storage (opt-in app-consistency
  via in-guest freeze); any host can boot a workload from a sealed checkpoint; retention
  "latest + N" with background compaction — no user-visible backing chains.
- **FR-34** — **Workload migration**, two primitives, both available at will (drain,
  rebalance, user-initiated): **checkpoint migration** (tier R/S: seal → restore on target;
  tier E: terminate + reschedule) and **live migration** for long-lived stateful workloads
  (v1 goal, E-3 — near-zero-cutover VMM-level transfer; gated on the runtime selection,
  doc 08 A-9). Targets always within the same user's allocation; affinity honored
  (doc 05 §7.1).
- **FR-35** — **Rolling upgrades, user-action-free**: platform-initiated host reimage in
  canary rings with version gates and drain concurrency limits per user (doc 05 §7.2).

### 3.5 Control plane & operations

- **FR-40** — **Two API surfaces with separate authN domains (E-8)**: the **tenant API**
  (workloads, volumes, ingress/egress rules, security groups, quotas, placement views, metrics) and the **admin API**
  (fleet, version registry, users) — REST/gRPC. AuthN via limited-lifetime tokens; AuthZ
  via roles; every call logged. **No API surface spans both planes.**
- **FR-41** — Per-workload metadata service (HTTP API inside the VM): exposes only the calling
  workload's own data, **upon presentation of a single-use token** (lesson R-F0.1 — never
  IMDSv1-style).
- **FR-42** — Metering: per workload and per tenant, across **storage tiers and classes**
  (local-NVMe GB·s, hot/cold GB·month, epoch-ship bytes, seals) plus vCPU/RAM/GPU-hours and
  **host-hours per allocation** (dedicated-host billing unit), with per-host utilization
  (the user's cost signal for spread-vs-packing decisions) — tier and placement choices
  must be informed cost decisions.
- **FR-43** — Observability: per-host and per-workload metrics (CPU, RAM, I/O, network, boot
  time), centralized logs, alerts (including security alerts, see OPS-10).
- **FR-44** — Bare metal fleet management: hardware inventory, immutable OS provisioning
  (signed image, verified boot), lifecycle (drain, reimage, decommission), and **allocation
  to users** — one user per host at a time, with attested wipe between allocations (doc 05).

---

## 4. Security requirements

### 4.1 Data-plane isolation (workload → host boundary)

- **SEC-1** — Every workload runs in a microVM (dedicated guest kernel, minimal device model).
  No host-kernel sharing with workloads. [D-03, C-cat3/4]
- **SEC-2** — The VMM is **Cloud Hypervisor** (Rust, rust-vmm), never QEMU. QEMU is excluded
  from the data plane. **Comparison rule**: this judgment is made against the **latest
  QEMU LTS at decision time** — not against EOL or historical lines. Grounds: device-model
  surface area and CVE volume of the current LTS line; the F43 EOL incident is a *process*
  lesson (SEC-31: version governance), not a technical disqualification of current QEMU.
  Re-evaluate this decision at each new QEMU LTS.
- **SEC-3** — Minimal device model, **frozen by policy**: allow-list of virtual devices exposed
  to guests; any addition is an architecture change requiring security review. [R-F16,
  R-implications]
- **SEC-4** — No USB/legacy devices, no persistent VNC (serial console only; any interactive
  console session is an audited event). [R-F34, R-F44]
- **SEC-5** — **No guest- or tenant-controllable runtime configuration channel exists**:
  workload definitions (devices, memory, CPU, volumes) are set solely by the control plane
  (host-agent). The runtime (direct CLH, E-4) exposes no annotation surface at all.
  [C-CVE-2026-50540, C-CVE-2026-44210, C-GHSA-7fhf — the lesson class stands even though
  the Kata annotation mechanism no longer exists]
- **SEC-6** — virtio-fs: never expose an arbitrary host path; source restricted to a
  per-sandbox dedicated directory; never `--sandbox=none`. [C-CVE-2026-47243] The direct-CLH
device model (E-4) uses **block devices only** — virtio-fs is not part of the v1 device
  model; this requirement governs any future introduction.
- **SEC-7** — Virtio network offloads disableable **by control-plane policy** and disabled by
  default for untrusted tenants (checksum offload included — the PwnEscape vector).
  [R-F45/CVE-2026-99999]
- **SEC-15** — **Default block encryption**: every volume and image is encrypted at rest
  (LUKS2 envelope) with a per-volume data key; keys never appear in plaintext on shared
  storage; no opt-out at creation.
- **SEC-16** — **Dedicated hosts**: one user per bare metal host (hard scheduler boundary);
  placement views scoped strictly to the caller's allocations — no cross-tenant placement,
  inventory, or co-residency information exposed. Closes the cross-tenant co-location class
  structurally (F0.1 oracle inverse); the compromised-workload adversary and all controls
  above still apply — a user is not a trust boundary for the platform.
- **SEC-8** — Host↔guest protocol: **none exists in v1** — there is no guest agent (E-4).
  F42's class (a guest choosing the host's key over an unauthenticated channel) is
  eliminated by having no channel. Any future guest-facing channel (exec, app-consistent
  seal trigger) is a new requirement requiring security review before introduction. [R-F42]

### 4.2 Host hardening (workload → host-kernel boundary)

- **SEC-10** — Immutable host OS (signed image, read-only root filesystem, /usr and /etc
  verified at boot), no package manager on production hosts.
- **SEC-11** — Hardened host kernel by default, verified by automated boot-time test:
  `kernel.io_uring_disabled=2`, `user.max_user_namespaces=0`, locked `core_pattern`, module
  loading disabled after boot (`modules_disabled=1`), kptr_restrict, dmesg_restrict.
  [C-CVE-2025-39698, C-CVE-2026-43494, C-CVE-2025-52881, C-userns cluster]
- **SEC-12** — Mandatory LSM: confinement profile per data-plane daemon (host-agent,
  VMM) — the VMM runs **unprivileged** with the strict minimum of file descriptors; dedicated
  jailer/namespace for the VMM process.
- **SEC-13** — **One disposable VMM process per workload**, with a resource cgroup (max CPU,
  RAM) and `no_new_privs`. A compromised VMM has no reachable neighbors and no possible
  persistence.
- **SEC-14** — Minimal host memory mapping for the VMM: no inter-workload shared memory;
  guest memory wiped (madvise MADV_DONTNEED / KVM_SET_USER_MEMORY_REGION unmap) at teardown.
  [FR-3]

### 4.3 Control plane & identity management

- **SEC-20** — No control API without strong authentication (limited-lifetime tokens,
  rotation); no "internal network" exception. [R-F19/F24]
- **SEC-21** — Metadata service: single-use token injected at boot, scoped per workload; the
  control plane **never** trusts client-supplied identity headers (no XFF-trust). [R-F0.1,
  R-F17]
- **SEC-22** — Tenant secrets: encrypted at rest, decryptable only by the designated workload,
  never in logs, never in another workload's metadata.
- **SEC-23** — Append-only, signed audit log (Merkle chain); covers workload transitions,
  operator actions, scheduler decisions, security alerts.
- **SEC-24** — **Plane separation (E-8)**: the tenant control plane holds **no operator
  capability** — host administration, fleet-global views, registry writes, and user
  management live in a separate admin plane with separate authentication. Compromise of
  the tenant plane yields at most tenant-scope authority (the SEC-16 boundary extended to
  the control plane); the KMS never grants unwrap on plane authority (host identity
  required, not plane privilege). Availability is decoupled: an admin-plane or
  tenant-plane outage never stops running workloads (host autonomy). [E-8]

### 4.4 Supply chain & patch management

- **SEC-30** — Full SBOM of every layer (host OS, kernel, VMM, runtime, guest images);
  version inventory queryable **by an external observer** (patch-status transparency — a guest
  must not serve as an oracle, but the operator must be able to prove fleet state). [R-F27]
- **SEC-31** — **No EOL branches in the fleet**: formalized support policy — a deployed version
  must be within the upstream maintenance window; version rollout is automated and tested; the
  fleet is deemed non-compliant after N days of lag. [R-F43 — root cause: an EOL VMM line never
  patched]
- **SEC-32** — Automated CVE watch (NVD, CISA KEV, GitHub advisories of components:
  cloud-hypervisor, rust-vmm crates, kernel) with triage SLA; data-plane components are
  **priority 1**. (Component list shrinks with E-4: no containerd, no Kata.)
- **SEC-33** — Host and guest image build pipeline: reproducible, signed, tested (see SEC-40);
  no builds on the production fleet (the 2024 BuildKit/Docker CVEs must be unreachable).
  [C-CVE-2024-23651/52/53]

### 4.5 Detection & continuous validation

- **SEC-40** — **Continuous attack suite in CI**: public escape PoCs are run against every
  data-plane build — virtio device-model PoCs against CLH (net/blk/vsock), the 2026
  page-cache/userns kernel cluster, and the historical runc/kata corpus retained where the
  attack class applies to our stack; **any bypass blocks the deployment**. Goal: 0 bypass, proven — not assumed.
- **SEC-41** — Fleet security monitoring: version drift, VMM crashes, guest panics, I/O
  anomalies (signatures of F31-style wedges), control-plane enumeration attempts.
- **SEC-42** — Every incident/finding gets a registry entry with status
  (open/mitigated/closed); temporary mitigations carry an expiration date.

---

## 5. Performance requirements ("first in class")

| # | Requirement | Target | Measurement |
|---|-------------|--------|--------------|
| PERF-1 | MicroVM boot (API→process in guest) | P50 < 1.5 s; P99 < 5 s | Automated benchmark suite |
| PERF-2 | Raw VMM boot (spawn→guest ready) | < 250 ms | same |
| PERF-3 | Isolation CPU overhead (vs native process) | < 5% on CPU-bound workload | SPEC-like benchmark |
| PERF-4 | I/O overhead (vs runc container) | < 15% on sequential I/O | fio |
| PERF-5 | Rest-state density | ≥ 100 active microVMs per 64 GB host, < 200 MB RSS per idle VM | stress + RSS measurement |
| PERF-6 | Control-plane API latency (workload CRUD) | P99 < 100 ms (excluding scheduling) | |
| PERF-7 | Scheduler throughput | ≥ 1,000 decisions/s | |
| PERF-8 | Full workload teardown | < 2 s P99 (resources freed, memory wiped) | |
| PERF-9 | Encryption-layer I/O overhead (LUKS2/dm-crypt vs unencrypted path) | < 15% on sequential I/O | fio on reference hardware |
| PERF-10 | Tier-R/E write latency vs raw local NVMe | ≤ 3× raw NVMe (4k, qd=1) | fio on reference hardware |
| PERF-11 | Checkpoint seal guest I/O pause (tier R) | < 100 ms P99 | seal under load in CI |
| PERF-12 | Checkpoint migration cutover (tier R, ≤ 10 GB dirty set) | P50 < 60 s; P99 < 300 s | migration under load in CI |
| PERF-13 | Live-migration cutover pause (stateful workload, ≤ 32 GB RAM, tier R elevated to write-through for the window) | P99 < 1 s; convergence under sustained dirty rate ≤ 250 MB/s | migration under load in CI |

**Comparison class (B-2, doc 08)**: cold-boot microVM platforms. Cold start (PERF-1) is
measured against published cold-boot baselines (e.g., Firecracker-class ~125 ms VMM boot);
snapshot-resume boot is deliberately not adopted — **cold start and migration flexibility
are the two performance priorities**.

**Rule**: no performance number is accepted without an automated CI test; a regression > 10%
blocks the release.

---

## 6. Reliability & operations

- **OPS-1** — Control-plane availability: 99.9% (replicated components, quorum).
- **OPS-2** — Host failure: ephemeral workloads are terminated and reschedulable; stateful
  ones (U3/U4) survive on tier R/S storage and are restartable (RTO < 10 min). **Host
  maintenance is non-disruptive for stateful workloads**: drain migrates them live
  (v1 goal, E-3) or by checkpoint — platform-initiated action never requires terminating a
  user's long-lived workload (doc 05 §7.2).
- **OPS-3** — Data-plane updates (kernel, VMM, runtime) without full downtime: rolling drain
  (see FR-12).
- **OPS-4** — A workload cannot degrade its host beyond its quotas (CPU, RAM, I/O, fds,
  guest processes) — self-DoS bugs (F31, F44) must affect neither neighbors nor require manual
  host intervention.
- **OPS-10** — Security alerts: dedicated channel, triage SLA (P1 < 24 h), semi-annual
  incident-response exercise (escape runbook) — lesson from F43/F45 handling.
- **OPS-11** — Tier-R RPO contract: sealed-checkpoint interval enforced by the control plane
  (default ≤ 60 s, tenant-tunable 10 s–1 h); host loss never exceeds one epoch of data loss
  for tier-R volumes, and never anything for tier S.

---

## 7. Constraints

- **CT-1** — Hardware: x86_64 servers with KVM (VT-x), ≥ 64 GB RAM; IOMMU (VT-d) required for
  the GPU phase. ARM64: opportunistic, not blocking.
- **CT-2** — Languages: **specified in `12-software-architecture.md` (E-10): Rust
  everywhere**. Guidance retained: Rust for anything handling untrusted input;
  Go acceptable for tooling; C forbidden outside audited components.
- **CT-3** — Small team: the design must minimize the amount of in-house "glue" components
  that need continuous security auditing (lesson from doc 06 §5: glue code is the risk
  when leaving an integrated stack).
- **CT-4** — Licensing: open-source components only (Apache/MIT/AGPL acceptable), no dependence
  on a single vendor.
- **CT-5** — **Runtime choice** (E-3/E-4): the runtime is **Cloud Hypervisor, directly
  orchestrated** — single VMM, no intermediary layers (no Kata, no containerd). The choice
  stays open per the doc 11 §4 comparison rule (e.g., re-evaluate if Kata ships sandbox
  live migration for CLH); the design must not grow dependencies that would block such a
  re-entry.

---

## 8. Acceptance criteria (pilot phase P0)

1. 1 host, 20 microVMs, doc-03 stack deployed by the signed pipeline (not by hand).
2. SEC-40 suite: 0 bypass on known public PoCs (maintained list, seed: doc 06 §2).
3. PERF-1/5/8 measured and met across 20 parallel workloads.
4. Boot-time kernel-hardening verification (SEC-11): failure = host out of the pool.
5. Version registry (SEC-30) queryable and complete (host, VMM, runtime, images).
6. Teardown + relaunch looped 1,000× with no host memory leak and no wedge.
7. Workload destruction: verification that host residual memory contains no workload data
   (correlation probe, F43 methodology inverted).

---

## 9. Traceability matrix (lessons → requirements)

| Lesson (source) | Requirements |
|-----------------|--------------|
| R-F43 — EOL VMM line heap disclosure (deterministic, virtio) | SEC-2, SEC-31, SEC-3 |
| R-F45 — PwnEscape (virtio-net csum → host RCE) | SEC-7, SEC-11, SEC-31 |
| R-F31 — vring hijack / brick self-DoS | OPS-4, SEC-13 |
| R-F0.1/F17 — metadata without token, XFF-spoof | SEC-21, FR-41 |
| R-F42 — unauthenticated vsock handshake | SEC-8 |
| R-F22 — snapshot backing-file escape | FR-32 |
| R-F27 — unobservable patch status | SEC-30 |
| R-F19/F24 — "network-only trust" control plane | SEC-20 |
| R-F34/F44 — live VGA / UHCI abort | SEC-4 |
| C-Kata 2025-26 (annotations, virtio-fs, bind-mounts) | SEC-5, SEC-6, SEC-33 |
| C-runc/BuildKit 2024-25 | FR-31, SEC-33 |
| C-2026 page-cache/userns kernel cluster | SEC-11, SEC-12 |
| R-M1 — side-channel false positives (methodology) | SEC-40 (randomized probing) |
