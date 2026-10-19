# 08 — Design Review: Challenges and Gaps

Working document — v15, 2026-10-18. A red-team pass over decisions 01–07, plus the
scope-change register (§E). Each item: what we
decided, the challenge, verdict ( upheld / needs change / open ), and the action.
Verdicts marked **needs change** are applied to the docs in the same commit.
**v2**: A-9, E-1..E-3, B-2 amendment (live migration in scope as a v1 goal).
**v3**: E-4 — direct Cloud Hypervisor as the single runtime; Kata and containerd dropped
from the data plane. A-2, A-6, A-9 closed. **v4**: E-5 — Fedora CoreOS as the tracked
host-OS upstream. **v5**: E-6 (wipe = crypto-erase only) and E-7 (PoC escape suite
mapped to hardening controls). **v6**: E-8 — three-plane separation. **v7**: E-9
(state authority, no bus in v1) and E-10 (3 services, Rust, Postgres HA) — doc 12 opened.
**v8**: E-11 — Outscale integration environment (doc 13 opened). **v9**: E-12 —
workload networking specified (doc 14 opened; C-6 superseded, F45 question closed).
**v10**: E-13 — user-defined workload address ranges (FR-23). **v11**: E-14 — network-rule
objects: IngressRule (renamed from Exposure), EgressRule, SecurityGroup. **v12**: E-15 — P1
scope gates (live migration tiered: checkpoint = P1 must, live = P1 stretch with predefined
fallback; A-5 re-gated to GA with dev-mode KEK for P0/P1) and metering semantics
(internal showback, no chargeback). **v13**: E-16..E-21, scope-review pass — PoC executes
in the integration environment (metal keeps every perf/security gate); v1 image path =
controlled rebuild only (`ImportImage`; intake sandbox deferred); escape suite validates by
baseline delta; console recording host-local and lifetime-bounded; static route rows
ratified; v1 API surface frozen at E-14. **v14**: E-22..E-25 — IPv6-first workload
network (v6 /128 per workload, ULA per VPC; family translation only at the proxies;
bounded legacy-v4 exception; attached IPv4 1:1 at the edge). **v15**: E-26 — v1 tenant
surface re-baselined (frozen method map in doc 09; Host/Allocation merge; three reads
enumerated; three surface cuts). **v16**: corpus restructure — doc 03 merged into §F
(its file is a pointer stub; `doc 03 §N` = `doc 08 §F.N`).

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

### A-5. KMS KEK loss = total data loss [re-gated by E-15 — dev-mode KEK for P0/P1, HSM/quorum is the GA gate]

Every volume DEK is wrapped under a KEK that "lives solely in the KMS". Destroy the KEK
and *all* tenant data is unreadable — forever. There is no key ceremony, no KEK backup
(e.g., Shamir split, offline copies), no recovery runbook in the corpus. This is the
single largest unrecoverable-failure mode in the design.

**Re-gate (2026-10-18, E-15)**: a **labeled dev-mode KEK** (software, single-node,
non-production) is permitted in P0/P1 so the trust-anchor schedule no longer blocks the
mechanism work; **HSM/quorum + key ceremony + KEK backup are the GA gate**. The invariant
is unchanged in both modes: the KEK never enters Postgres.

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
   QEMU-runtime; the P2 hardware validation gate stays mandatory.

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
not a v1 feature** (a minimal audited guest agent would be a P2 addition), and
**app-consistent seals are deferred** (crash-consistency is the default contract).

The two-host spike stands, now on the PoC stack itself (direct CLH), and merges with the
PoC's second phase: same runtime, second host, live migration under load (PERF-13).
Upstream watch item retained: if Kata ever ships sandbox live migration for CLH, the
single-runtime choice is re-evaluated per the doc 11 §4 comparison rule — the design must
not grow dependencies that make that re-entry harder.

**Tiering (2026-10-18, E-15)**: "v1 goal" is re-tiered into **P1 must** (checkpoint
migration, FR-34/PERF-12 — the guarantee that platform-initiated maintenance never
*terminates* a stateful workload; a bounded I/O pause is acceptable) and **P1 stretch**
(live migration, PERF-13, owned by this spike). **Fallback trigger (premade, not
mid-P1)**: if the spike does not demonstrate PERF-13 (P99 cutover < 1 s, convergence
under sustained dirty rate ≤ 250 MB/s) by the end of P1, live migration moves to P2 —
design unchanged, schedule only.

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
attributes** (User, Host, Workload, Volume, Checkpoint, Image, HostGroup,
Token, Quota) — deliberately attribute-level, not full OpenAPI, until P0 feedback.
(The Allocation object was merged into Host by E-26 — the binding is Host state.)

### B-4. No DR/backup story for control-plane state [partially resolved]

Postgres backup + audit Merkle-head continuity noted as P1 scope; the KEK ceremony
(A-5) remains the critical item — re-gated by E-15 to the **GA gate** (dev-mode KEK
authorized for P0/P1).

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
| 4 | Key ceremony + KEK backup procedure (A-5) | **GA gate** (E-15; dev-mode KEK authorized for P0/P1) |
| 5 | Manager HA verification (A-8) | before claiming OPS-1 |
| 6 | Operating model: users, staffing, SLA owners (B-1) | before P1 |
| 7 | API specification (B-3) | before P0 control plane |
| 8 | ~~Kata project-health review cadence (A-6)~~ | **closed by E-4** — replaced by CLH project-health cadence (OPS-10) |
| 9 | Live-migration two-host spike on the direct-CLH stack (A-9, E-4) — RAM/state transfer + encrypted-volume cutover + PERF-13 under load | runs in the integration environment (doc 13) after PoC; before v1 commitment |
| 10 | Ingress data-path design: per-host proxy + IngressRule object (E-2; named Exposure until E-14) | before P1 |
| 11 | Guest rootfs image for the direct-CLH runtime (kernel + initramfs + rootfs, signed, digest-pinned) | PoC scope (E-4) |
| 12 | Minimal guest-agent design (exec, app-consistent seal) — only if a use case demands it | P2, on demand (E-4) |
| 13 | Lean FCOS derivative compose (coreos-assembler): strip container tooling, add CLH + host-agent + hardening config; signed, digest-pinned | before P0 (E-5) |
| 14 | Image intake/rebuild service design (admin plane, `build-worker` — doc 12 §3: `ImportImage` → controlled rebuild into the platform format; FR-31/FR-32) | before P0 (E-17) |
| 15 | Legacy-v4 exception bounds (E-23): eligibility criteria, quota design (`max_legacy_ipv4`), justification recorded at create | before P1 |

---

## E. Scope-change register (operator decisions)

Rule (E-21): the v1 API surface is **frozen at E-14** — a new v1 object, field, or plane
must evict equivalent scope; additions queue in the P2 register. A **new corpus file**
is itself a scope change and requires a row here; new topics land as sections of
existing documents.

| # | Date | Decision | Applied in |
|---|------|----------|-----------|
| E-1 | 2026-10-18 | **Stateful workloads are first-class v1 citizens** — any enterprise application (databases, app servers, queues); Windows guest OSes stay out of scope | doc 01 §1/§2/FR-30/OPS-2, doc 04 §2 |
| E-2 | 2026-10-18 | **Ingress access is first-class v1** — still explicit/auditable/revocable, never on by default, never guest-controlled | doc 01 FR-20, doc 02 I-14/§7, doc 09 (Exposure → IngressRule, E-14) |
| E-3 | 2026-10-18 | **Live migration is a v1 goal** (not PoC scope); **Kata is not a definitive choice** — runtime selection is a v1 gate with live migration as a criterion (A-9) | doc 01 FR-34/PERF-13/CT-5, doc 02 §4.5/§7/§8, doc 03 §4.1, doc 04 §2, doc 05 §7.1, doc 07 §3, doc 10 §5, doc 11 (migration terms) |
| E-4 | 2026-10-18 | **Cloud Hypervisor, directly orchestrated, is the single runtime** — Kata and containerd are dropped from the data plane (A-9 resolution; A-2 dissolved; A-6 closed). Consequences: no guest agent in v1 (no exec; app-consistent seals deferred); host-agent gains the per-VM lifecycle role (C-1 partially returns, bounded); guest rootfs images built by the pipeline; SBOM = CLH + kernel + host-agent | doc 01 (SEC-5/8/12/32/40, CT-5), doc 02 §1/§3/§4/§5.6/§6/§7/§8, doc 03 (superseded, §6), doc 04 §3.4, doc 06 §5, doc 07 (PoC re-target), doc 10 §2/§5, doc 11 (sandbox) |
| E-5 | 2026-10-18 | **Fedora CoreOS is the tracked host-OS upstream** — criteria: lightest *maintained* base, stable stream discipline, fastest kernel-CVE cadence (biweekly atomic updates on a fresh line, ≥ 6.6 for `io_uring_disabled`). Immutable/atomic (rpm-ostree A/B — doc 10's slot model uses upstream machinery), SELinux enforcing by default (SEC-12), Ignition reproducible provisioning. Production image = **lean coreos-assembler derivative** (container tooling stripped; CLH + host-agent added — doc 08 action 13). PoC runs the **stock stable stream**. Fallback: **Flatcar** (switch trigger: FCOS stream churn operationally too hot at fleet scale; comparison per doc 11 §4). Rejected: **Bottlerocket** (ships two containerd instances + orchestrator-agent model — E-4 violation), classic distros (no native immutable A/B; slower kernel lines) | doc 07 §2 (PoC host), doc 02 I-6, doc 10 §2 (image contents) |
| E-6 | 2026-10-18 | **Wipe = crypto-erase only** (operator decision: no physical teardown — overwrite passes accumulate on NAND cells and reduce NVMe drive lifetime). Storage regions: **zeroize the DEK + TRIM/discard** — never a physical overwrite. Memory: unmap/zero pages (SEC-14, unchanged). Consequences: verification criterion = "no **cryptographically recoverable** data" (ciphertext bytes legitimately remain on the medium); PERF-8's 2 s teardown budget becomes achievable (overwrite of a multi-GB region cannot fit it). TRIM writes nothing — it *reduces* NAND wear (lower write amplification on reuse) | doc 01 FR-3/PERF-8, doc 04 §2 (tier E), doc 07 (success criteria + probe), doc 11 (wipe term) |
| E-7 | 2026-10-18 | **PoC escape suite is selected to validate the hardening choices** (one PoC per control), not as three arbitrary bugs: (1) virtio-net **offload attack** (PwnEscape class, R-F45) → validates **SEC-7/I-7** (offloads disabled for untrusted workloads); (2) **virtio device-model escape against CLH** (blk or vsock, from the doc 06 §2 corpus) → validates **SEC-3/4** (frozen minimal device model, no legacy devices); (3) **2026 page-cache/userns kernel cluster** → validates **SEC-11** (+ the scripted boot-time sysctl check). The suite *proves the hardening works*; it grows with the fleet per SEC-40 | doc 07 (test harness row, success criteria), consistent with doc 01 SEC-40 |
| E-8 | 2026-10-18 | **Three-plane separation** (multitenant deployment, operator decision): **admin control plane** (operators: fleet lifecycle, version registry/gates, image pipeline, monitoring, user admin) / **tenant control plane** (workload/volume/exposure orchestration, placement views scoped to own allocations, quotas, metering) / **workload data plane** (hosts, microVMs, per-host proxies, shards — tenant traffic never transits a control plane, I-14). Host-agent dual authority: admin lifecycle + tenant desired state, origin-tagged in one desired-state table (admin orders gate, never originate). KMS = shared trust anchor owned by neither plane. A-7's ≤3-services constraint now applies **per plane**. Audit stays unified (SEC-23). `AllocateHost` is the plane bridge (transactional). Availability decoupled: a plane outage never stops running workloads | doc 01 §1.1/FR-40/SEC-24, doc 02 §3.1 (diagram + plane model), doc 09 (two API surfaces), doc 11 (plane terms) |
| E-9 | 2026-10-18 | **State authority + order flow**: the control plane owns *intent* (objects + pending calls, transactional), the host owns *reality* (reports); **host autonomy unbounded on partition** — TTL and quotas enforced locally on the host (fail-safe), no lease required to keep running (preserves E-8 rule 3). **No message bus in v1**: orders = rows in Postgres, hosts pull desired-state deltas over mTLS, events = append-only table. The bus is a *scale component for async tasks*, deliberately deferred; seam rule: when introduced it consumes the event table and dispatches async tasks — order flow never moves onto it. FR-5 consequence: TTL re-scoped to per-workload-class policy (mandatory ephemeral, operator-waivable stateful, never tenant-waivable) | doc 01 FR-5, doc 12 §1–2, §6 |
| E-10 | 2026-10-18 | **Decomposition, language, store**: three services total — tenant-plane (API + in-process scheduler + desired-state computation + migration state machine), admin-plane (fleet/registry/gates/pipeline/monitoring), KMS (shared trust anchor). One Rust workspace, role-selected at startup (I-13 extended). **Boring, battle-tested stack** (operator: "boring and battle tested is best"): tokio + tonic (gRPC/mTLS) + sqlx + PostgreSQL. One Postgres cluster, separate databases per plane, primary + synchronous replica + Patroni-style failover (OPS-1 by drill). Excluded v1: bus, mesh, k8s, etcd, per-plane DBs, multi-region | doc 12 (opened), doc 02 pointers |
| E-11 | 2026-10-18 | **Integration environment on Outscale cloud** — "proper infra" simulated on cloud VMs to validate *mechanism* before and alongside the metal PoC: dedicated-group VMs with nested virtualization play the bare-metal hosts (verified: CLH v54.0 boots guests on a gen-7 dedicated node), a normal-tenancy NFS VM plays the storage pool, a control-plane VM hosts the doc-12 services. **Fidelity contract is explicit**: protocol/orchestration/A-1 spike/A-9 spike/tier machinery = validated here; perf numbers, tier economics, bare-metal security assertions = never (metal only). Outscale mechanics recorded (group-as-tenancy, p1 flag, nested-virt flag, Net-tenancy subtlety) | doc 13 (opened), doc 07 §4 pointer, action 9 note |
| E-12 | 2026-10-18 | **Workload networking design** (doc 14): **native netns/veth — CNI superseded** (it standardized container-runtime integration that E-4 deleted); **no VLANs in the workload path** — F45 class closed by not using VLANs; one bridge per host (per user, SEC-16) with **isolated ports** — no workload↔workload L2; CLH jailed inside the per-workload netns; **routed /32 per workload, no overlay** (BGP behind the route-table seam); guest IP on the kernel command line — no DHCP, no guest cooperation; egress = transparent per-host proxy (name-level allow-list, NAT, token bucket); ingress = per-host proxy as the **sole** ingress enforcement point (IngressRule — E-14; no DNAT bypass); anti-spoof and security groups compiled to netns nftables; **vhost-net off in v1**; migration cutover = atomic route switch + listener move | doc 14 (opened), doc 03 §6 (C-6 line amended), doc 02 §8 (F45 item resolved) — **/32 model superseded by E-22** (IPv6-first) |
| E-13 | 2026-10-18 | **User-defined workload address ranges**: tenants may define the RFC1918 range (/24–/16) from which their workloads' /32s are assigned — enterprise migration need (U4). Constraints: **fleet-wide uniqueness enforced transactionally** (all users' ranges + platform-reserved fabric/storage/link-local; machine-readable rejection reasons); platform-allocated default when unspecified; users see only their own ranges (FR-14 — a CIDR choice is not a probe). **Allocation-policy change only** — data path, routing, proxies, reachability identical (FR-20/21 unchanged). VPC API object added | doc 01 FR-23, doc 09 (Vpc object), doc 14 §3 (address-range policy) — **re-scoped by E-22/E-23** (demoted to the legacy-v4 exception path) |
| E-14 | 2026-10-18 | **Network-rule objects** (operator: "lets simplify: egressRule, ingressRule, securitygroup"): **Exposure renamed IngressRule** (+ `source_cidrs` allow-list field — a revoked rule means no listener exists); **EgressRule** added (per-workload or VPC default, FQDN/CIDR; proxy resolves DNS — name rules cannot be bypassed by IP); **SecurityGroup** added (east-west within the VPC, compiled to netns nftables, default deny; attachment immutable per workload in v1). **NetworkPolicy rejected**: K8s label-selector semantics we do not have, and "not Kubernetes" is a non-goal (doc 01 §1.2); SecurityGroup is the IaaS-industry term and the model the corpus already adopted (doc 02 §1). Three rule types, three directions, one mental model | doc 09 (three objects; Exposure renamed), doc 01 FR-20/FR-40, doc 11 (glossary terms), doc 14 §4/§5/§6, doc 02 (I-14, §3.1), doc 12 §1 |
| E-15 | 2026-10-18 | **P1 scope gates + metering semantics**: (1) **E-3 re-tiered** — checkpoint migration (FR-34, PERF-12) is the **P1 must** for non-disruptive U4 maintenance; live migration (PERF-13) is a **P1 stretch** owned by the A-9 two-host spike; **fallback trigger**: no PERF-13 demonstration (P99 cutover < 1 s, convergence ≤ 250 MB/s dirty) by end of P1 ⇒ live migration moves to P2 (design unchanged, schedule only); OPS-2 restated as "platform-initiated maintenance never *terminates* a stateful workload; bounded I/O pause is acceptable". (2) **A-5 re-gated** — labeled dev-mode KEK (software, single-node) authorized for P0/P1; HSM/quorum + key ceremony + KEK backup remain the **GA gate**; the KEK never enters Postgres in either mode. (3) **Metering is showback** — FR-42's cost signal is internal (placement decisions, packing efficiency); **no customer-facing chargeback** (doc 01 §1.2); doc 09 `price` renamed `cost_index` | doc 01 (v6 header, §1.1/§1.2, FR-34, FR-42, OPS-2), doc 02 §4.6/§7 (P1 definition of done), doc 05 §6, doc 09 (Host/`ListAvailableHosts`) |
| E-16 | 2026-10-18 | **PoC executes in the integration environment (nested virt); evidence gates re-gated to metal**: the PoC's three questions (direct-CLH buildability, tier-E encrypted attach + wipe, teardown determinism) are answered on the doc-13 environment — dedicated-group VMs with nested virtualization — not on a physical host first. Consequences: the PoC's PERF-1/5/8 numbers there are **mechanism-only smoke signals** (doc 13 §2: nested KVM inflates boot; BSU "local" is network-backed); the authoritative perf gate stays **metal-only** (doc 02 §7 top-3 item 3 — PERF-1/5/8/9 on metal; doc 01 §8); security assertions (residual-memory probe as *evidence*, escape-suite security claims) never transfer from nested virt. **Kill criteria split**: functional kills (dm-crypt attach breakage, teardown leak/wedge) are judged in the nested environment; the perf kill (P50 < 1.5 s) is judged on the **metal validation run** before P0 commitment | doc 07 (§1/§2/§5/§6), doc 13 §1/§2, doc 11 (metal validation run), doc 02 §7 note |
| E-17 | 2026-10-18 | **v1 image path = controlled rebuild only**: workload create accepts **platform images** (signed, digest-pinned rootfs from the catalog) or command + minimal rootfs; FR-1(a) "create from an OCI image" becomes **`ImportImage`** — an asynchronous call that rebuilds the tenant image into the platform format (FR-31/FR-32) and returns a runnable catalog image. Nothing tenant-supplied is ever executed as supplied. The **intake sandbox is deferred from v1** (like the guest agent: on demand, P2). "Container-compatible API" phrasing retired — the API is OCI-*image*-compatible **at intake only** (E-4 already deleted the runtime's OCI handling). The rebuild service is admin-plane scope (`build-worker`, doc 12 §3); design action opened (§D action 14) | doc 01 (§1.1/§1.2, FR-1, FR-31), doc 09 (Image), doc 11 (intake sandbox), doc 03 §4 item 4 (§F.4 here), doc 07 §3 |
| E-18 | 2026-10-18 | **Escape suite validates controls by delta, not by failure** (E-7 refinement): each PoC is run against an **unhardened baseline** (control disabled — virtio-net offloads on, permissive device model, stock sysctls) *and* against the hardened config; a hardening control is evidenced by the **delta**, never by failure alone (a PoC that fails on both evidences only the version floor, SEC-31). Where a public PoC cannot be reproduced on the FCOS/CLH floor, the harness exercises the intended surface and the delta claim moves to the metal run / SEC-40 — reproducing a published escape is **outside** the PoC's ~4–5 week estimate | doc 07 §2 (harness row, success criteria), doc 13 §2 |
| E-19 | 2026-10-18 | **Console/stdio recording is host-local and lifetime-bounded** (FR-6 scope): the host-agent records the workload's serial console to a per-workload **local** log — destroyed with the workload (FR-3) for ephemeral classes, retained **≤ workload TTL + 24 h** for stateful; readable b-workload **local** log — destroyed with the workload (FR-3) for ephemeral classes, retained **≤ workload TTL + 24 h** for stateful; readable by the tenant via the tenant API for their **own** workloads. **No central console-log store in v1** | doc 01 FR-6, doc 12 §6, doc 07 §3 |
| E-20 | 2026-10-18 | **v1 route propagation = static routes as desired-state rows** (ratifies doc 14 §3 as a scope decision): cross-host /32 routes are desired-state rows compiled into each host's root ns; **no BGP or any dynamic routing protocol in v1** — BGP stays behind the route-table seam for scale. No fabric-level protocol exists in v1 | doc 14 §3, doc 12 §6 |
| E-21 | 2026-10-18 | **v1 API surface is frozen at E-14** (governance): no new v1 objects, fields, or planes without evicting equivalent scope; additions queue in the P2 register | doc 09 header + conventions, doc 08 §E rule |
| E-22 | 2026-10-18 | **IPv6 is the workload network**: every workload holds exactly one **/128** from its VPC's **ULA /48–/56** (RFC 4193 — user-defined or platform-allocated; fleet-wide uniqueness stays transactional, now across v6 ranges + platform-reserved v6 fabric/storage/link-local/metadata). Routing stays "routed single address, no overlay" — route rows carry `ip6/128 → host-fabric-ip6`. **All family translation lives in the two proxies** (L4 terminate + re-originate): the egress proxy connects out in whatever family the destination requires; **no NAT64/DNS64 machinery — the guest never sees IPv4**. Metadata endpoint moves to a fixed well-known v6 address (E-25) | doc 01 (§1.1, FR-20/FR-23, FR-41), doc 09 (Vpc/Workload/IngressRule/EgressRule/SecurityGroup), doc 14 (§1–§10), doc 12 §6, doc 07 §2/§3, README |
| E-23 | 2026-10-18 | **Legacy-v4 is a bounded, documented exception** (debt — the U4 migration case): a workload may **additionally** hold one v4 /32 from a user-defined RFC1918 range — E-13's mechanism survives, demoted to this path. **Invariant: every workload has a v6 /128; v4 is additive.** Bounded by: explicit opt-in at create, a tenant v4 range in use, a quota on v4-carrying workloads, and v4 nowhere else (metadata, proxies, control paths stay v6) | doc 01 (§1.1, FR-23, FR-5), doc 09 (Vpc `cidr4`, Workload `ip4`, Quota), doc 14 §3, doc 08 action 15 |
| E-24 | 2026-10-18 | **Attached IPv4 = 1:1 both directions** (Outscale PublicIp semantic, platform pool): inbound lands on the workload's **per-host ingress-proxy listener** (sole enforcement point, I-14 — no DNAT bypass); outbound SNATs through the attached v4 so return traffic matches. `IngressRule.source_cidrs` becomes family-agnostic (v4 clients → v6 listener is the normal case). On migration the listener + attachment move in the same transaction as the route switch | doc 01 FR-20, doc 09 (PublicIpv4 object, Workload `public_ipv4`, IngressRule `source_ranges`), doc 14 §5, doc 12 §6 |
| E-25 | 2026-10-18 | **Mechanism bundle (IPv6-first)**: guest bootstrap = **platform init + custom cmdline key** (`porche.ip6=<addr>,<gw>` — SLAAC/RA ruled out: no guest-side family negotiation, anti-spoof keeps exactly one allowed source per family); metadata endpoint = fixed well-known v6 address (`fe80::a9fe:a9fe`, the 169.254.169.254 analog) routed to the metadata-proxy, token gating unchanged; new **SEC-43** (v6 hardening: drop unknown/unnecessary extension headers, no routing headers, strict fragment policy, RA/ND suppression from workload side, source-address enforcement); lab cross-host v6 = **v6-in-v4 (SIT/GRE) transport substitution** in doc 13's fidelity contract — **native v6 fabric = metal-only** (Outscale Nets are RFC1918 v4-only, doc 13 §4) | doc 01 (SEC-43, FR-41), doc 14 (§2/§3/§8/§10), doc 13 (§2/§4/§8) |
| E-26 | 2026-10-18 | **v1 tenant surface re-baselined** (makes the E-21 freeze binding): doc 09 gains the **frozen per-plane method map**. Three reads **enumerated as existing scope** (not new): `GetUsage` (FR-40 "metrics" + FR-42 showback), `GetConsole` (FR-6/E-19), `GetEvents` (FR-2, own objects). **Allocation merged into Host** — allocate = create, release = delete; the binding is Host state (FR-13 already modeled it so; doc 05's `HostAllocation` naming drift closed; release history stays in the audit trail, SEC-23). **Cuts → P2 register**: `Vpc.subnets` + VPC subrange update (no subnet mechanism exists in doc 14; prefix immutable after create) and `Image` re-tag (the digest is the identity, I-14). Token binds to exactly one authN domain; `PublicIpv4` runtime attach justified (ingress-proxy listener only — not volume-hotplug class, which stays P2) | doc 09 (v7: surface map, Host merge, cuts), doc 01 (FR-6, FR-40), doc 05 §1.1, doc 02 §3 (diagram), doc 11 (allocation), doc 12 (§1, §3) |
| E-27 | 2026-10-19 | **Implementation code enters the repository** (operator): the corpus moves under `doc/` and the code trees land at the top level — `src/` (Rust workspace, one binary per I-13: porche-core / porche-clh / porche-host), `guest/` (action 11 image build), `harness/` (§7 scripts), `runs/` (recorded artifacts); `infra/` unchanged. Doc numbers remain stable IDs; the growth rules govern `doc/` only — code-tree layout is build tooling, not corpus scope. Doc 07 §8 opened as the implementation plan; host desired-state store = **SQLite** (closes doc 12 §8); §7.4 escape artifact policy decided: **hybrid** | README (structure), doc 07 (§7.4, §8), doc 12 §8 |
| E-28 | 2026-10-19 | **Console MVP = pure projections of the frozen API** (operator): a console may do nothing that is not a doc-09 method — the E-21 freeze binds the UIs exactly as it binds SDKs. Two consoles, two authN domains (E-8/FR-40), one web service (**console-host**, workspace binary `porche-web`): `/w` user console (tenant plane), `/a` admin console. Adapter seam: method names = the frozen map — PoC adapter (desired-state store + host-agent verbs) first, P1 adapter (tonic to the planes) swaps the backend without UI change. Stack: axum + askama (server-rendered) + HTMX **vendored** — no SPA, no JS toolchain, live updates are polled fragments. MVP page inventories + cuts recorded in doc 09; host-agent binary stays dependency-minimal (no async runtime — E-5), hence separate binary, fold-or-keep decided at P1. Stage-1 mutation serialization: one executor lock (I-12 removes it) | doc 09 (§Console surfaces, v8), doc 12 (§4 row, §9) |

---

## F. Superseded rationale: the simplification pass (was doc 03)

*Merged here in the corpus restructure (2026-10-18); `doc/decisions/03-simplification.md`
is a pointer stub. **Mapping: `doc 03 §N` = `doc 08 §F.N`** — the section numbering is
preserved.*

**Was doc 03, v4, 2026-10-18.** Analysis and recommendations applied to
`02-architecture.md` (v3). **v2**: tradeoff §F.4.1 rewritten for the scope change (live
migration is a v1 goal — doc 08 E-3). **v3 (E-4)**: the Kata/containerd adoption
recommended here is **superseded** — see §F.6. C-5/C-6 stand; C-1..C-4 logic inverts.
**v4 (E-17 + corpus vocabulary sweep)**: §F.4 item 4 restated — OCI images enter only
via `ImportImage`; no per-workload OCI pull exists; phase prose switched to P-codes.

**Principle**: every incumbent-design component whose job is already done by an upstream,
audited Kata/containerd component gets **deleted, not adapted**. The measure of success
is the size of the *remaining* in-house security-critical surface (constraint CT-3).

### F.1 What Kata/containerd gives for free (historical)

Measured against the incumbent codebase (the incumbent's per-VM VMM driver (≈ 7.3 kLOC), its in-guest agent
protocol, its hypervisor agent, and its metadata service):

| Capability | Legacy (custom, in-house) | Kata/containerd (upstream) |
|---|---|---|
| One supervisor process per VM | a per-VM VMM driver process (one per VM, started by the hypervisor agent) | kata shim — one per sandbox, managed by containerd |
| VMM lifecycle, command line, control socket | `qemu.py`, `osc_to_qemu.py`, `qemu_monitor_protocol.py` | shim drives CLH over its API — no QEMU, no QMP |
| Guest agent channel | in-guest agent with a custom vsock protocol + **flawed handshake (F42)** | kata-agent ttrpc over vsock with **mutual TLS** — upstream, audited, keys bound to the sandbox |
| Console / stdio / exec | `console.py` + QMP chardevs + ZMQ plumbing | CRI streaming API (exec/attach/logs) |
| Guest bootstrap | in-guest agent baked into the machine image | kata-agent in guest image — standard, patched with runtime |
| Resource envelopes (CPU/RAM ceiling per sandbox) | absent (bare QEMU cmdline) | sandbox config + cgroups |
| Guest syscall filtering | absent | kata-agent seccomp profile |
| Volume attach/detach | QMP block ops (via the per-VM driver) | CRI/virtio-blk direct assignment |

**Observation**: the F42 finding (unauthenticated key adoption in the host↔in-guest-agent
handshake) is not a bug to patch in our new design — the entire protocol is deleted and
replaced by kata-agent's mutually authenticated channel. Fixing by deletion beats fixing by
redesign.

### F.2 Core change recommendations

#### C-1 — Delete the per-VM VMM driver (and the v2 "workload-supervisor" idea)

containerd + the kata shim already provide one process per sandbox with per-VM logs, crash
isolation, and lifecycle. The v2 architecture (`02-architecture.md` v2) kept a per-VM
supervisor driving containerd — that is a thin wrapper around something containerd already
does, i.e. pure glue (CT-3 violation). **Delete it.** the hypervisor agent talks CRI to containerd
directly; per-sandbox supervision is the shim's job.

*Cost*: we lose the ZMQ per-VM order protocol — replaced by CRI calls (standard, typed).
*Risk*: none identified; the pattern is how every Kubernetes node works.

#### C-2 — Delete the incumbent in-guest agent and its crypto protocol entirely

kata-agent becomes the **only** guest agent. What the incumbent agent did:

| Incumbent agent feature | Replacement |
|---|---|
| Snapshot sync (app-consistent) | already dropped (not a requirement) |
| Guest metrics | kata-agent exec of a metrics endpoint, or scrape from host (`/proc` of VMM cgroup + virtio stats) — v1: host-side only |
| Metadata access | see C-3 |
| Custom handshake (F42 flaw) | **deleted with the protocol** — kata-agent mTLS |

No more key-adoption surface, no custom protocol parser on the host path (F42 also showed the
guest can feed the host malformed frames — upstream kata-agent parsing is fuzzed upstream).

#### C-3 — Metadata: token at boot, proxy at the edge

Keep the metadata *feature* (FR-41), simplify the delivery: the KMS mints a single-use,
per-boot token; it is injected into the guest at creation (kata-agent config/env, not a
network position); the guest fetches its metadata from a per-host HTTPS proxy presenting
that token. This structurally satisfies SEC-21/F0.1 (no identity from network position, no
XFF trust) with ~one small component (metadata-proxy) instead of a guest-side agent.

#### C-4 — The hypervisor agent shrinks to a thin host-agent

Remaining responsibilities, all host-scoped, no per-VM logic: CRI client (create/stop/inspect
sandboxes), KMS client (fetch wrapped DEK, open LUKS, zeroize on detach), boot attestation +
kernel-hardening checks, drain protocol, metrics reporter. This is the **only** new privileged
daemon on a PORCHE host, and it is small enough to audit line-by-line.

#### C-5 — Drop qcow2: raw sparse files inside LUKS2

With snapshots/backing chains/streaming already gone (v2), qcow2's only remaining job is
thin provisioning — which a sparse raw file does natively. Making volumes **raw inside the
LUKS2 envelope**:

- removes `qemu-img` (QEMU code!) from the storage worker — the "no QEMU anywhere in the
  production path" rule (SEC-2) becomes complete and consistent;
- removes a parser (qcow2 metadata) from the storage path entirely;
- virtual-size/thin behavior preserved via sparse files (`fallocate` + hole punching on TRIM).

Cold clone-at-create stays (storage worker copies the decrypted view into a new envelope).

#### C-6 — Networking via CNI; VPC model untouched

Per-host netns/veth/bridge moves to standard CNI plugins driven by host-agent. The VPC /
security-group / subnet model is database-level and adopted as-is (v1) — no change
to the API surface or the firewall service VMs.

### F.3 What remains PORCHE-specific (the honest inventory)

After C-1..C-6, the complete list of new/in-house security-relevant code:

| Component | LOC estimate | Trust level | Audit burden |
|---|---|---|---|
| host-agent (CRI/KMS/keyring/attest/drain) | small | privileged | full audit |
| storage-worker (LUKS2 create/clone) | small | privileged (key-aware) | full audit |
| KMS (wrap/unwrap, tokens) | medium | control plane | full audit + key ceremony |
| metadata-proxy | tiny | unprivileged | review |
| control plane (concepts adopted, per doc 02 §1) | to be specified | control plane | later software-architecture spec |
| kata shim, kata-agent, CLH, containerd, kernel | upstream | data plane | upstream + our CI attack suite (SEC-40) |

Compare with the v2 architecture: a VMM-driver replacement + custom in-guest agent + custom handshake +
qemu-img storage ops all gone. **The only code standing between an untrusted guest and the
host is upstream, fuzzed, and disposable-per-workload.**

### F.4 Tradeoffs accepted

1. **No QMP block jobs, no pause-and-inspect** — gone with QMP, permanently. **Live
   migration is in scope as a P1 stretch** (E-3, E-15, doc 08 — checkpoint migration is
   the P1 must), but by a different mechanism
   (VMM-level transfer, FR-34) — never by re-introducing QMP machinery. Consequence: the
   C-1/C-2 deletions stand unconditionally for the *ephemeral* workload class; for
   *stateful* workloads, the v1 runtime selection (A-9 gate, doc 02 §4.5) decides whether
   Kata is used at all. If stateful workloads run on direct CLH, the per-VMM lifecycle
   glue that C-1 deleted returns in reduced form — the honest CT-3 cost of live migration.
   Checkpoint migration (drain = seal + restore) remains the fallback primitive for all tiers.
2. **Runtime volume attach** — CLH hotplug exists, but v1 simplifies: all volumes defined at
   workload creation, attach-at-boot only. Runtime attach is a P2 addition if a use case
   demands it.
3. **Guest-root can kill kata-agent** — same as the incumbent in-guest agent; the host never depends on
   guest cooperation for teardown (CRI stop → VMM kill → key zeroize).
4. **Image distribution** — platform-rootfs distribution per workload adds latency;
   mitigated by a per-host pull-through cache (base catalog pre-seeded on the host
   image — doc 02 §8). OCI images now enter only via `ImportImage` (E-17) — no
   per-workload OCI pull exists. Must be measured against PERF-1/2 in P0.
5. **Density** — microVM per workload caps at ~100–500/host instead of the incumbent VM-per-process
   density; accepted (that is the isolation trade we chose in doc 06 §1).

### F.5 Decision (recommended defaults — historical)

- **Adopt the incumbent control-plane concepts** (CQS, pending calls, transactional
  placement) — the orchestration model is sound; the software architecture, languages and
  middleware are re-specified deliberately in a **later spec**, not inherited by habit.
- **Drop** the incumbent public-API surface: PORCHE exposes its own API (FR-40 — workloads, volumes, hosts, placement views), defined fresh rather than inherited.
- **Adopt** C-1..C-6 into `02-architecture.md` v3 (done).

### F.6 Supersession — E-4: the simplification inverts (2026-10-18)

The runtime revalidation (doc 08 A-9, E-4) **supersedes this pass's central
recommendation**: Kata and containerd are dropped; Cloud Hypervisor is orchestrated
directly by the host-agent. §F.1–§F.5 stand as the historical analysis of the v2→v3
architecture; the principle survives, the conclusion inverts:

- **C-1** (delete the per-VM VMM driver) — *inverted*: supervision returns to the
  host-agent as a **bounded lifecycle role** over CLH's API. No wrapper *process*
  reappears; the glue is API driving, not protocol parsing.
- **C-2** (kata-agent replaces the custom agent) — *superseded by something stronger*:
  **no guest agent at all**. F42's class is eliminated by having no host↔guest protocol
  (SEC-8 v3), not by replacing one channel with another.
- **C-3** (metadata token) — stands unchanged.
- **C-4** (host-agent scope) — *extended*: it gains the per-VM lifecycle role (E-4).
- **C-5** (raw sparse in LUKS2) — stands unchanged.
- **C-6** (CNI networking) — *superseded (E-4/doc 14)*: native netns/veth/bridge in the
  host-agent; CNI existed to standardize container-runtime integration that no longer
  exists. The host-agent's netlink module replaces the plugin ecosystem.

**The deeper simplification (doc 02 v5)**: this pass asked "which incumbent glue
does upstream Kata replace?" E-4 asks the better question — "which *intermediary layers*
can be deleted entirely?" Answer: containerd, the kata shim, and the guest agent. The
data plane is **host-agent + CLH + kernel**. The CT-3 measure improves: the in-house
surface is one audited binary; the upstream surface is one VMM.

**What this costs** (recorded honestly, doc 08 A-9): exec into a running workload is not
a v1 feature; app-consistent seals are deferred; the guest rootfs image is built by our
pipeline (doc 08 action 11).
