# 08 — Design Review: Challenges and Gaps

Working document — v3, 2026-10-18. A red-team pass over decisions 01–07, plus the
scope-change register (§E). Each item: what we
decided, the challenge, verdict ( upheld / needs change / open ), and the action.
Verdicts marked **needs change** are applied to the docs in the same commit.
**v2**: A-9, E-1..E-3, B-2 amendment (live migration in scope as a v1 goal).
**v3**: E-4 — direct Cloud Hypervisor as the single runtime; Kata and containerd dropped
from the data plane. A-2, A-6, A-9 closed. **v4**: E-5 — Fedora CoreOS as the tracked
host-OS upstream. **v5**: E-6 (wipe = crypto-erase only) and E-7 (PoC escape suite
mapped to hardening controls). **v6**: E-8 — three-plane separation.

---

## A. Technical challenges

### A-1. The tier-R device stack is not implementable as written [needs change — fixed]

`04-storage-tiers.md` §3.1 specified `guest → dm-crypt → dm-era → local NVMe (hot) + remote
envelope (origin)`. **`dm-era` is a tracking-only target**: it marks blocks written per epoch
and passes I/O through to a single origin — it does **not** provide local write-back
caching. The design silently assumes a caching layer that the named primitives don't
supply. A write-back mechanism (dm-writecache, dm-cache, or a custom target) is required in
the stack, and its exact interaction with epoch tracking (what has/hasn't reached the
origin at seal time) is non-trivial.

**Action applied**: doc 04 §3.1 now marks the composition as a *candidate* requiring a
dedicated spike, listing the open problem. **New kill-adjacent gate**: no tier-R commitment
before the spike proves a working, seal-able stack.

### A-2. SEC-5 (empty annotation allow-list) may block the only viable volume attach path [RESOLVED — dissolved by E-4]

Direct-assignment of `/dev/mapper` block devices to Kata sandboxes is typically driven by
annotations — the very mechanism SEC-5 freezes by default. As written, the two requirements
can conflict, and doc 02 §8 lists the attach model as an open question *on the PoC critical
path* (it is kill-criterion #2).

**Resolution (2026-10-18, E-4)**: with Kata dropped, **no annotation mechanism exists at
all** — volumes attach through Cloud Hypervisor's native block-device API, configured solely
by the host-agent. The conflict and the SEC-5 amendment both evaporate; SEC-5 is reworded
to state the invariant directly (no guest- or tenant-controllable runtime configuration
channel). Kill-criterion #2 of the PoC now tests the CLH block path.

### A-3. "Attested wipe" is actually "reported wipe" [needs change — fixed]

FR-3/FR-44 say teardown wipes are "attested in the audit log" — but the attestor is the
host itself. A compromised host-agent can claim a wipe it never performed. Self-report is
not proof.

**Action applied**: FR-3 now says *reported*; verification is external and proportional —
the residual-memory probe at PoC (doc 07), randomized spot audits on returned hosts
(doc 05 §1.1), and TPM-backed measurement as a phase-3 option. The honest contract:
*dedicated hosts confine a lying host to its own user's data*; cross-user wipe fraud is
structurally limited by allocation.

### A-4. Storage worker placement [RESOLVED — per operator decision]

**Resolution**: the storage pool is **plain NFS space** — no assumptions about the NFS
server (no vendor, no ONTAP, no filer-side agents). Security is carried entirely by
per-file encryption: each volume file is a LUKS2 envelope under the customer's key
hierarchy, passed to the VM for read/write through the host's dm-crypt attach path.
Because only ciphertext ever exists on the pool, the storage-role placement is a
scheduling concern, not a security boundary: the storage role (same binary, I-13) runs
on any platform node with NFS access + KMS reachability. Legacy filer agents are not
part of the new storage path.

### A-5. KMS KEK loss = total data loss [open — must close before P1]

Every volume DEK is wrapped under a KEK that "lives solely in the KMS". Destroy the KEK
and *all* tenant data is unreadable — forever. There is no key ceremony, no KEK backup
(e.g., Shamir split, offline copies), no recovery runbook in the corpus. This is the
single largest unrecoverable-failure mode in the design. **Action required**: key ceremony
+ KEK backup procedure as a P1 entry criterion.

### A-6. Kata dependency risk is real and recent [closed by E-4 — Kata dropped from the data plane]

We banned QEMU for CVE volume (judged, per the comparison rule, against its **latest LTS**) —
then adopted Kata, whose 2025-26 record includes 9 CVEs and true guest→host escapes
(doc 06 §2). The comparison must stay symmetric: both stacks are judged at their best. Exploration of the actual risk shape:

1. **The sharpest issue is not the CVE count — it is that our chosen pairing is the
   least-exercised combination.** Most production Kata deployments run the QEMU runtime
   or Dragonball (confidential containers); the **kata-clh** runtime is a niche inside a
   niche. We get CLH's smaller attack surface but also its smaller validation base —
   defensible, but it means *we* are part of the validation.
2. **We own kata-agent's protocol surface now.** We deleted our custom guest protocol
   (C-2) for theirs — the right call, but every kata-agent parsing bug is now our boundary
   bug. Mitigation: guest seccomp, minimized agent API surface in config, upstream
   fuzzing + our attack suite (SEC-40) covering kata escape PoCs.
3. **Version churn vs our no-EOL policy**: Kata releases fast; SEC-31 requires continuous
   tracking at 1,000-node scale — real recurring work (B-1 staffing).
4. **GPU via kata-clh+VFIO is assumed, not verified** — Kata's GPU stories are mostly
   QEMU-runtime; the phase-2 hardware validation gate stays mandatory.

**Mitigation adopted (structural)**: the **runtime-class hedge**. Containerd runtime
classes make the VMM nearly swappable (`kata-clh` today, `kata-fc`/`kata-qemu` as
fallbacks). The CI attack suite (SEC-40) runs a **second runtime** in the matrix, so a
runtime swap is a *tested configuration change*, never a port. CLH stays primary (Rust,
minimal surface, GPU path); the hedge converts an existential dependency into an
operational choice. Plus: Kata project-health review cadence (OPS-10), pin-by-digest,
and the GPU validation gate unchanged.

**Closure (2026-10-18, E-4)**: the entire challenge is **closed by deletion** — Kata
(and containerd) are dropped from the data plane; Cloud Hypervisor is orchestrated
directly by the host-agent. The niche-inside-a-niche exposure (point 1), the kata-agent
protocol surface (point 2), and the Kata release-churn burden (point 3) all disappear
with the dependency. What remains of this item: the **CLH project-health review cadence**
replaces the Kata one (OPS-10), and the GPU/VFIO validation gate (point 4) is unchanged —
it now tests a *smaller* assumed surface. The dual-runtime hedge is retired: with a
single VMM, the hedge is version pinning + the escape hatch of re-evaluating runtimes
(CT-5, doc 11 §4 comparison rule).

### A-7. Control-plane software architecture [RESOLVED by clarification — new build, later spec]

**Correction of premise (operator)**: PORCHE is a **new orchestrator — there is no legacy
to keep or extend**. The incumbent platform is design reference only (doc 06 §4); we
adopt *concepts* (multi-agent, CQS, pending calls, transactional placement), not
software. The earlier "strangler pattern / keep the legacy monolith" discussion is moot.

What stands:

- **Software architecture, languages, and concrete middleware are a later specification**
  (a dedicated doc, post-PoC). CT-2 (doc 01) remains guidance for that spec (Rust for
  untrusted-input paths, minimal C), not a decision.
- **Design constraints already fixed** (these are architecture decisions, not language
  choices): outbound-only host-agents with zero inbound ports (I-11), desired-state
  reconciliation (I-12), one-binary-multiple-roles for host-side code (I-13), at most
  three control-plane services, transactional placement accounting.
- **The concept-adoption list is explicit** (doc 02 §1) so the later spec starts from a
  clean sheet without re-litigating the orchestration model.

### A-8. Control-plane HA [rewritten — was about the incumbent reply-consumer]

The original challenge (verify the incumbent reply-consumer's HA) is moot — nothing is kept.
OPS-1's 99.9% target is a requirement **on the new build**: the later software-architecture
spec must design control-plane HA from scratch (replication, quorum, failover of the state
store and API tier) and prove it before OPS-1 is claimed.

### A-9. Live migration vs the Kata runtime [RESOLVED — direct CLH, single runtime (E-4)]

Live migration is a **v1 goal** (operator decision, 2026-10-18 — E-register below), not
PoC scope. The challenge: **Cloud Hypervisor implements VMM-level live migration, but
Kata Containers does not migrate sandboxes** — the kata shim lifecycle has no hand-off of
a running VMM. The v1 requirement therefore re-opens the runtime choice, and the operator
has ruled that **Kata is not a definitive choice**. Options on the table:

1. **Direct CLH orchestration** for stateful workloads (escape hatch, doc 06 §5) — full
   control, migration protocol reachable; cost: re-introduces per-VMM lifecycle glue that
   C-1 deleted (CT-3 tension, honest cost).
2. **Upstream Kata live-migration support** — zero glue if it lands; out of our control,
   no date.
3. **Two runtime paths** (Kata for ephemeral, direct CLH for stateful) — hedged, but two
   data-plane surfaces to audit and attack-test (SEC-40 matrix cost).

Second, harder half — **volumes**: VMM migration transfers RAM + device state, not
locally attached block storage. Live migration is only clean when volumes are
network-attached at cutover: tier S by construction, tier R **elevated to write-through
for the migration window** (which depends on the A-1 sealable-stack spike). Tier E stays
on checkpoint migration (ephemeral by choice).

**Gate**: a two-host spike, end-to-end (RAM/state transfer + encrypted-volume cutover +
PERF-13 under load), before any v1 live-migration commitment. The PoC is unaffected — its
three bets (doc 07 §1) do not change; the spike follows the PoC and feeds the v1 runtime
selection (CT-5). Full analysis: doc 02 §4.5.

**Resolution (2026-10-18, E-4 — operator decision, evidence-based revalidation)**: after
re-running the doc 06 §5 decision matrix against the *new* requirement set (E-1..E-3),
**Cloud Hypervisor is orchestrated directly — single runtime, no Kata, no containerd**.
Grounds:

- **Live migration**: CLH implements it natively and with operational maturity (TLS
  transport, documented n-2 version compatibility for migrations); Kata has no sandbox
  hand-off (issue #1690 open since 2021, no roadmap). The only working Kata live
  migration found (third-party) drives QEMU via QMP — excluded by SEC-2.
- **Security**: no guest agent exists at all — the F42 class is eliminated by having no
  host↔guest protocol (stronger than C-2's fix-by-replacement); no annotation surface
  (A-2 dissolves); the SBOM shrinks to CLH + kernel + host-agent.
- **Requirements fit**: FR-31/32 already mandate that tenant images are rebuilt into the
  platform format — Kata's runtime OCI handling solved a problem the design had
  legislated away. FR-6 is satisfied by the serial console (virtio-console), recorded
  host-side.
- **CT-3 cost (accepted, honest)**: the host-agent grows a per-VM lifecycle role — the C-1
deletion partially returns, bounded (lifecycle + migration driving over CLH's HTTP API;
no guest protocol, no image handling). Consequences: **exec into a running workload is
not a v1 feature** (a minimal audited guest agent would be a phase-2 addition), and
**app-consistent seals are deferred** (crash-consistency is the default contract).

The two-host spike stands, now on the PoC stack itself (direct CLH), and merges with the
PoC's second phase: same runtime, second host, live migration under load (PERF-13).
Upstream watch item retained: if Kata ever ships sandbox live migration for CLH, the
single-runtime choice is re-evaluated per the doc 11 §4 comparison rule — the design must
not grow dependencies that make that re-entry harder.

## B. Gaps in objectives

### B-1. No operating model [RESOLVED — per operator decision]

**Resolution**: fleet scale **up to ~1,000+ metal nodes (the max user count — one user
per host, SEC-16)**. Metal host lifecycle and security is **platform operator
responsibility, not user responsibility** — the user's surface is their workloads and
views, never host administration. Consequences: (a) host upgrades are platform-initiated
and must not require user action — the **rolling upgrade and workload migration process
is now specified** (doc 05 §7: canary rings, checkpoint migration, version gates);
(b) recurring security work (CVE watch, key ceremonies, attack suite — OPS-10/SEC-32)
is staffed platform work, sized for a 1,000-node fleet, not a hobby.

### B-2. "First in class performance" is self-referential [RESOLVED — per operator decision]

**Resolution**: the comparison class is **cold-boot microVM platforms**, and the two
performance priorities are **cold start** and **migration flexibility** — not
snapshot-resume boot (deliberately not adopted; doc 07 §3). Concretely: PERF-1 (cold
start P50 < 1.5 s) is measured against published cold-boot baselines, and a new SLO
**PERF-12** covers checkpoint migration cutover (FR-34). **Superseded (2026-10-18, E-3)**:
live migration is now a **v1 goal** — PERF-13 (cutover pause) added, mechanism question
opened as A-9. The comparison class and PERF-1/12 stand unchanged; migration = seal
checkpoint + restore *or* live transfer, available at will (drain, rebalance,
user-initiated) — that is the flexibility metric.

### B-3. PORCHE API is a one-line requirement [RESOLVED — object catalog specified]

**Resolution**: `09-api.md` specifies the API as **objects with CRUD and main
attributes** (User, Host, Allocation, Workload, Volume, Checkpoint, Image, HostGroup,
Token, Quota) — deliberately attribute-level, not full OpenAPI, until P0 feedback.

### B-4. No DR/backup story for control-plane state [partially resolved]

Postgres backup + audit Merkle-head continuity noted as P1 scope; the KEK ceremony
(A-5) remains the critical item and the P1 entry criterion.

## C. What survives challenge (explicitly)

- Dedicated hosts as the structural cross-tenant fix (A-3 strengthens rather than breaks it)
- Deleting the custom guest protocol (F42 class eliminated, not patched)
- The deferral-table / kill-criteria discipline (this document exists because of it)
- Fail-closed defaults; ciphertext-only on shared storage
- Traceability to real findings rather than best practices

## D. Action list

| # | Action | Due |
|---|---|---|
| 1 | Tier-R dm-composition spike (A-1) | before P1 (gate) |
| 2 | ~~Resolve volume attach model (A-2)~~ | **resolved — dissolved by E-4** (CLH native block attach) |
| 3 | Storage worker placement decision (A-4) | before P1 |
| 4 | Key ceremony + KEK backup procedure (A-5) | P1 entry criterion |
| 5 | Manager HA verification (A-8) | before claiming OPS-1 |
| 6 | Operating model: users, staffing, SLA owners (B-1) | before P1 |
| 7 | API specification (B-3) | before P0 control plane |
| 8 | ~~Kata project-health review cadence (A-6)~~ | **closed by E-4** — replaced by CLH project-health cadence (OPS-10) |
| 9 | Live-migration two-host spike on the direct-CLH stack (A-9, E-4) — RAM/state transfer + encrypted-volume cutover + PERF-13 under load | after PoC; before v1 commitment |
| 10 | Ingress data-path design: per-host proxy + Exposure object (E-2) | before P1 |
| 11 | Guest rootfs image for the direct-CLH runtime (kernel + initramfs + rootfs, signed, digest-pinned) | PoC scope (E-4) |
| 12 | Minimal guest-agent design (exec, app-consistent seal) — only if a use case demands it | phase 2, on demand (E-4) |
| 13 | Lean FCOS derivative compose (coreos-assembler): strip container tooling, add CLH + host-agent + hardening config; signed, digest-pinned | before P0 (E-5) |

---

## E. Scope-change register (operator decisions)

| # | Date | Decision | Applied in |
|---|------|----------|-----------|
| E-1 | 2026-10-18 | **Stateful workloads are first-class v1 citizens** — any enterprise application (databases, app servers, queues); Windows guest OSes stay out of scope | doc 01 §1/§2/FR-30/OPS-2, doc 04 §2 |
| E-2 | 2026-10-18 | **Ingress access is first-class v1** — still explicit/auditable/revocable, never on by default, never guest-controlled | doc 01 FR-20, doc 02 I-14/§7, doc 09 (Exposure) |
| E-3 | 2026-10-18 | **Live migration is a v1 goal** (not PoC scope); **Kata is not a definitive choice** — runtime selection is a v1 gate with live migration as a criterion (A-9) | doc 01 FR-34/PERF-13/CT-5, doc 02 §4.5/§7/§8, doc 03 §4.1, doc 04 §2, doc 05 §7.1, doc 07 §3, doc 10 §5, doc 11 (migration terms) |
| E-4 | 2026-10-18 | **Cloud Hypervisor, directly orchestrated, is the single runtime** — Kata and containerd are dropped from the data plane (A-9 resolution; A-2 dissolved; A-6 closed). Consequences: no guest agent in v1 (no exec; app-consistent seals deferred); host-agent gains the per-VM lifecycle role (C-1 partially returns, bounded); guest rootfs images built by the pipeline; SBOM = CLH + kernel + host-agent | doc 01 (SEC-5/8/12/32/40, CT-5), doc 02 §1/§3/§4/§5.6/§6/§7/§8, doc 03 (superseded, §6), doc 04 §3.4, doc 06 §5, doc 07 (PoC re-target), doc 10 §2/§5, doc 11 (sandbox) |
| E-5 | 2026-10-18 | **Fedora CoreOS is the tracked host-OS upstream** — criteria: lightest *maintained* base, stable stream discipline, fastest kernel-CVE cadence (biweekly atomic updates on a fresh line, ≥ 6.6 for `io_uring_disabled`). Immutable/atomic (rpm-ostree A/B — doc 10's slot model uses upstream machinery), SELinux enforcing by default (SEC-12), Ignition reproducible provisioning. Production image = **lean coreos-assembler derivative** (container tooling stripped; CLH + host-agent added — doc 08 action 13). PoC runs the **stock stable stream**. Fallback: **Flatcar** (switch trigger: FCOS stream churn operationally too hot at fleet scale; comparison per doc 11 §4). Rejected: **Bottlerocket** (ships two containerd instances + orchestrator-agent model — E-4 violation), classic distros (no native immutable A/B; slower kernel lines) | doc 07 §2 (PoC host), doc 02 I-6, doc 10 §2 (image contents) |
| E-6 | 2026-10-18 | **Wipe = crypto-erase only** (operator decision: no physical teardown — overwrite passes accumulate on NAND cells and reduce NVMe drive lifetime). Storage regions: **zeroize the DEK + TRIM/discard** — never a physical overwrite. Memory: unmap/zero pages (SEC-14, unchanged). Consequences: verification criterion = "no **cryptographically recoverable** data" (ciphertext bytes legitimately remain on the medium); PERF-8's 2 s teardown budget becomes achievable (overwrite of a multi-GB region cannot fit it). TRIM writes nothing — it *reduces* NAND wear (lower write amplification on reuse) | doc 01 FR-3/PERF-8, doc 04 §2 (tier E), doc 07 (success criteria + probe), doc 11 (wipe term) |
| E-7 | 2026-10-18 | **PoC escape suite is selected to validate the hardening choices** (one PoC per control), not as three arbitrary bugs: (1) virtio-net **offload attack** (PwnEscape class, R-F45) → validates **SEC-7/I-7** (offloads disabled for untrusted workloads); (2) **virtio device-model escape against CLH** (blk or vsock, from the doc 06 §2 corpus) → validates **SEC-3/4** (frozen minimal device model, no legacy devices); (3) **2026 page-cache/userns kernel cluster** → validates **SEC-11** (+ the scripted boot-time sysctl check). The suite *proves the hardening works*; it grows with the fleet per SEC-40 | doc 07 (test harness row, success criteria), consistent with doc 01 SEC-40 |
| E-8 | 2026-10-18 | **Three-plane separation** (multitenant deployment, operator decision): **admin control plane** (operators: fleet lifecycle, version registry/gates, image pipeline, monitoring, user admin) / **tenant control plane** (workload/volume/exposure orchestration, placement views scoped to own allocations, quotas, metering) / **workload data plane** (hosts, microVMs, per-host proxies, shards — tenant traffic never transits a control plane, I-14). Host-agent dual authority: admin lifecycle + tenant desired state, origin-tagged in one desired-state table (admin orders gate, never originate). KMS = shared trust anchor owned by neither plane. A-7's ≤3-services constraint now applies **per plane**. Audit stays unified (SEC-23). `AllocateHost` is the plane bridge (transactional). Availability decoupled: a plane outage never stops running workloads | doc 01 §1.1/FR-40/SEC-24, doc 02 §3.1 (diagram + plane model), doc 09 (two API surfaces), doc 11 (plane terms) |
