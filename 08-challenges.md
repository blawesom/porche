# 08 — Design Review: Challenges and Gaps

Working document — v2, 2026-10-18. A red-team pass over decisions 01–07, plus the
scope-change register (§E). Each item: what we
decided, the challenge, verdict ( upheld / needs change / open ), and the action.
Verdicts marked **needs change** are applied to the docs in the same commit.
**v2**: A-9, E-1..E-3, B-2 amendment (live migration in scope as a v1 goal).

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

### A-2. SEC-5 (empty annotation allow-list) may block the only viable volume attach path [open]

Direct-assignment of `/dev/mapper` block devices to Kata sandboxes is typically driven by
annotations — the very mechanism SEC-5 freezes by default. As written, the two requirements
can conflict, and doc 02 §8 lists the attach model as an open question *on the PoC critical
path* (it is kill-criterion #2).

**Recommendation**: resolve at PoC in favor of one of (a) CRI block mounts (no annotations),
or (b) a *curated, platform-owned* allow-list containing exactly the direct-assignment
annotation — which is not user-controllable input and therefore not the SEC-5 threat
(tenant-chosen annotations). Amend SEC-5 wording once decided.

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

### A-6. Kata dependency risk is real and recent [upheld — mitigation adopted: dual-runtime hedge]

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

### A-9. Live migration vs the Kata runtime [open — v1 gate (scope change E-3)]

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
| 2 | Resolve volume attach model (A-2) | PoC (kill criterion) |
| 3 | Storage worker placement decision (A-4) | before P1 |
| 4 | Key ceremony + KEK backup procedure (A-5) | P1 entry criterion |
| 5 | Manager HA verification (A-8) | before claiming OPS-1 |
| 6 | Operating model: users, staffing, SLA owners (B-1) | before P1 |
| 7 | API specification (B-3) | before P0 control plane |
| 8 | Kata project-health review cadence (A-6) | ongoing, OPS-10 |
| 9 | Live-migration two-host spike + v1 runtime selection (A-9, E-3) | after PoC; before v1 commitment |
| 10 | Ingress data-path design: per-host proxy + Exposure object (E-2) | before P1 |

---

## E. Scope-change register (operator decisions)

| # | Date | Decision | Applied in |
|---|------|----------|-----------|
| E-1 | 2026-10-18 | **Stateful workloads are first-class v1 citizens** — any enterprise application (databases, app servers, queues); Windows guest OSes stay out of scope | doc 01 §1/§2/FR-30/OPS-2, doc 04 §2 |
| E-2 | 2026-10-18 | **Ingress access is first-class v1** — still explicit/auditable/revocable, never on by default, never guest-controlled | doc 01 FR-20, doc 02 I-14/§7, doc 09 (Exposure) |
| E-3 | 2026-10-18 | **Live migration is a v1 goal** (not PoC scope); **Kata is not a definitive choice** — runtime selection is a v1 gate with live migration as a criterion (A-9) | doc 01 FR-34/PERF-13/CT-5, doc 02 §4.5/§7/§8, doc 03 §4.1, doc 04 §2, doc 05 §7.1, doc 07 §3, doc 10 §5, doc 11 (migration terms) |
