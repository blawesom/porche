# 05 — Dedicated Hosts, User Placement Views, Attract/Repulse Resiliency

Working document — v3, 2026-10-18. **v2**: §7.1 adds live migration as a v1 goal (E-3,
doc 08 A-9). **v3 (vocabulary sweep, E-16..E-21 pass)**: GPU phase prose → P2. Adds the
hosting model to `02-architecture.md`:

1. **A bare metal host is allocated to exactly one user** (dedicated hosts).
2. **The user has a view on their hosts and the VM/workload distribution across them.**
3. **the legacy resiliency model is ported**: attract/repulse placement tags (soft/strict,
   host/group scopes), peer-group semantics as implemented in the legacy affinity resolution
   (doc 06 §4, "Placement/affinity").

---

## 1. Host allocation model

### 1.1 Dedicated allocation

- A **host is allocated to at most one user at a time** — a hard scheduling boundary, not a
  preference. `HostAllocation` (user, host, state) is a control-plane object with lifecycle:
  `free → allocated(user) → draining → free`.
- The scheduler's candidate set for a workload is **always** ⊆ the owner's allocated hosts.
  Cross-user co-location is structurally impossible — no policy can violate it.
- Allocation granularity: single hosts, optionally constrained to host groups (see §1.2).
  Users may hold multiple allocations (e.g., hosts in different racks).
- Release: user-initiated or TTL-expired → `draining` (workloads terminated/rescheduled off,
  per tier E/R rules — R volumes restore from latest checkpoint on another of the user's
  hosts) → wipe (NVMe regions per SEC-14, LUKS envelopes detached) → `free`.

### 1.2 Failure domains (first-class)

- **HostGroup** = a failure domain: rack, power/net domain, or maintenance domain (the legacy
  "cluster" concept, renamed to avoid VM-cluster confusion). Every host belongs to exactly
  one HostGroup; groups carry metadata (rack id, power feed, maintenance calendar).
- Resiliency reasoning happens at group granularity: "my 3 replicas live in 3 different
  HostGroups" is checkable and reportable (§2).

### 1.3 Capacity & oversubscription

- A user's capacity = Σ their allocated hosts. Per-user oversubscription policy (CPU
  overcommit ratio, NVMe dirty-region budget across tiers E/R) — platform default with
  user-tunable bounds. The user *sees* the effects of their own oversubscription (§2), since
  noisy-neighbor impact is confined to their own workloads (a structural consequence of
  dedicated allocation — OPS-4's neighbor guarantee becomes intra-user only).
- **Local storage accounting is space, not performance**: tier-E volumes are fully
  resident on the host's NVMe; tier-R adds its dirty-region budget on the same device. The
  scheduler's hard filter is `Σ(E sizes) + Σ(R dirty budgets) ≤ host NVMe capacity` — a
  **capacity** constraint, enforced at placement on declared sizes, never a performance
  control. Local performance is the raw device, unallocated and unshaped (that is the point
  of dedicated metal); the only local backpressure is tier-R's durability rule (doc 04 §6,
  RPO-driven, not space-driven). If thin-provisioned local volumes oversubscribe the device
  and it fills, the guest sees `ENOSPC` on **its own** volume's region — the failure is
  confined to the offending workload, exactly like a real disk running out of space.

---

## 2. User placement views

New API surface (scoped **strictly to the caller's allocations** — contrast F0.1, where the
platform's co-residency oracle leaked cross-tenant placement; here co-residency is by design
the user's own business, but *nothing about other users or the global fleet* is exposed):

| Call | Returns |
|---|---|
| `ListAvailableHosts` | free hosts allocatable by the caller: hardware profile (CPU/RAM/NVMe size+class, GPU), HostGroup, `cost_index`/host-hour (internal showback, E-15) — the provisioning catalog (scoped: no other users' information, no fleet totals) |
| `ListHosts` | the user's hosts: state, group, hardware profile, current load, allocated capacity |
| `GetPlacement` | per host: the workloads running there (id, tier, resources, affinity tags); per workload: its host |
| `GetDistribution` | aggregate view: per group/host — workload count, vCPU/RAM/NVMe usage, per-tier breakdown |
| `GetPlacementDecisions` | recent placement decisions with reasons — including *why a workload is pending* (e.g., "no host in allocation satisfies `repulse_server_strict=g1` with free memory") |
| `GetResilienceReport` | per affinity group: spread across hosts/groups, single-points-of-failure callouts ("group `web` has 2/3 replicas in HostGroup R7") |

Design rules:

- **Explainability is a requirement**: a pending workload must always carry a machine- and
  human-readable constraint-failure reason. No silent constraint dropping, no "it will fit
  eventually" mystery (the legacy `no_candidates_check` details, ported).
- Views are read-only projections of scheduler state (PostgreSQL) — no new trusted path, no
  host-agent involvement in serving them.
- Every placement *change* is an event in the audit log (SEC-23) and subscribable
  (notification stream) for the user's own workloads.

---

## 3. Attract/repulse semantics (ported from legacy, kept compatible)

Affinity is expressed as **VM tags with shared group values**, resolved against the *current
placement of the owner's other VMs* (peer-group semantics — unchanged from legacy, and with
dedicated hosts all peers are the owner's own workloads):

| Tag | Scope | Strength | Meaning |
|---|---|---|---|
| `attract_server` | host | soft | prefer hosts already running my group |
| `attract_server_strict` | host | hard | must run on a host already running my group (else pending) |
| `attract_group` | HostGroup | soft | prefer my group's HostGroup (rack-locality) |
| `attract_group_strict` | HostGroup | hard | must be group-local |
| `repulse_server` | host | soft | avoid hosts running my group (spread) |
| `repulse_server_strict` | host | hard | never co-locate with my group |
| `repulse_group` | HostGroup | soft | prefer other HostGroups |
| `repulse_group_strict` | HostGroup | hard | never same HostGroup (rack spread) |

Renaming note: the legacy `*_cluster` → `*_group` (cluster was ambiguous); the legacy
`*_server` → `*_host`. Semantics otherwise identical — tags are evaluated at scheduling
time against live placement; strict violations make the workload `pending` (with the §2
reason), never drop the constraint.

### Resiliency recipes (documented, first-class)

| Goal | Tags |
|---|---|
| Replicas spread across hosts | one tag per replica set: `repulse_server_strict=web` |
| Replicas spread across racks | `repulse_group_strict=web` |
| Latency-critical co-location | `attract_server=db` on the app VMs |
| Licensing / hardware pinning | `attract_group_strict=gpu-rack` |

---

## 4. Scheduler

Per workload (order preserved from the legacy slot-locking model):

1. **Candidate set** = owner's allocated hosts (hard boundary, §1.1).
2. **Hard filters**: strict attract/repulse (peer-group resolution), resources (vCPU/RAM),
   NVMe budget for tier E/R volumes, GPU availability (P2), host state = `allocated`.
3. **Preference ordering**: soft attract/repulse, then bin-packing/topology (NUMA, core
   pinning), then load balancing *within* the allocation.
4. **Commit**: PostgreSQL transaction locking the host row + slots (the legacy
   `with_for_update` slot locking, reused as-is) → write placement → audit + notify.
5. **Starvation handling**: pending workloads with unsatisfiable strict constraints are
   reported (§2) and re-evaluated on every placement change in the allocation; an
   operator/tenant action (release constraint, add host) is the remedy — the scheduler does
   not silently relax constraints.

Because candidates are per-user, the global scheduling problem decomposes into independent
per-allocation problems — smaller lock domains in PostgreSQL than the legacy fleet-wide slots
(a throughput win, aligned with PERF-7).

---

## 5. Security notes

- **Posture unchanged**: dedicated hosts close the *cross-tenant co-location* class
  structurally (side-channel neighbors, noisy neighbors of other users, F0.1-style
  co-residency oracles about *others*), but the reference adversary remains a **compromised
  workload attacking its own host** — all SEC-1..15 stand. A user is not a trust boundary
  for the platform.
- **View scoping** (§2) is the new sensitive surface: `GetPlacement`-family calls are
  filtered to the caller's allocations server-side; host identifiers of other users, group
  inventories beyond the caller's, and fleet totals are never returned.
- Host release wipe (§1.1) is reported in the audit log like any teardown (FR-3 — host self-report; external verification: spot audits on release, doc 08 A-3).

---

## 6. Metering & billing hooks

Feeds FR-42: host-hours per allocation (the unit of dedicated-host accounting), plus
per-host utilization so the user sees packing efficiency of their own allocation
(idle-host cost is *their* cost signal for the §3 spread decisions — spreading has a
visible price). **Metering is internal showback** (E-15): host-hours is an accounting
unit, not an invoice line — no customer-facing chargeback exists.

---

## 7. Fleet operations — rolling upgrades and workload migration

**Summary** — full operator runbook in `10-upgrade-migration.md` (artifact/A-B model,
ring gates, per-host state machine, migration mechanics per tier, failure handling,
invariants).

Fleet scale: up to **~1,000+ metal nodes** (one user per host = max user count). Metal
host lifecycle and security is **platform-operator responsibility**; users never touch
host administration (B-1 resolution, doc 08).

### 7.1 Workload migration (the flexibility primitive)

Migration = **checkpoint migration** (all tiers — the P1 must) or **live migration**
(long-lived stateful workloads — P1 stretch, E-3/E-15; gated on the A-9 two-host spike). No QMP
block jobs in either path. Both available at will (drain, rebalance, user-initiated,
upgrade-driven):

| Tier | Checkpoint migration | Live migration (stateful, E-3) | Downtime |
|---|---|---|---|
| E | terminate + reschedule from image | not applicable (ephemeral by choice) | a cold start (by design) |
| R | seal checkpoint (optional app-consistent `fsfreeze`) → restore on target host → cutover | elevate to write-through for the window (doc 04 §2) → VMM transfer → cutover | checkpoint: seal + ship dirty set + restore (PERF-12); live: cutover pause (PERF-13) |
| S | same as R; the envelope is always current, restore is immediate | preferred path — volumes already network-attached | live: ≈ cutover pause |

**Hard constraint (SEC-16)**: the migration target must be **within the same user's
allocation** — migration never crosses the allocation boundary; checkpoint data stays
inside the owner's hosts. If the user holds a single host, drain requires either a new
allocation or (tier E) termination — surfaced to the user by `GetPlacementDecisions`
before drain starts. Affinity tags (§3) are honored at migration placement like at
initial placement.

### 7.2 Rolling upgrade process (platform-initiated, user-action-free)

1. **Version gate** (I-5/SEC-31): new host image signed, registered, attack suite green
   (SEC-40) — otherwise the rollout does not start.
2. **Canary rings**: 1% → 5% → 25% → 100% of the fleet, with soak time and automated
   health checks between rings; any regression halts the rollout (previous signed image
   remains the fallback — A/B by reimage).
3. **Per host**: mark `draining` (scheduler stops placing) → migrate workloads per §7.1 →
   verify empty (no volumes attached, NVMe regions reported wiped) → netboot the signed
   image → attest (kernel hardening, image signature) → rejoin as `allocated(user)` →
   user-visible only as a placement event (FR-14 views) and the migration cutover itself.
4. **Rate limits**: max drain concurrency per user (an upgrade must not take down a
   user's entire allocation at once) and per fleet ring.

### 7.3 What users see

Nothing about the host OS. They see: their workloads migrate (with cutover windows),
placement decisions explain drains ("host upgrade — ring 2"), and host-hours continue
unbroken (the host stays billed to them through reimage). Host security state is
trustworthy *because* the operator owns it — verified boot, attestation, and version
registry are operator attestations, not user-verifiable claims in v1 (tenant-facing
attestation remains the open question, doc 02 §8).

## 8. Open questions

- [ ] ~~May a user's allocation span hosts whose hardware profiles differ~~ — resolved:
  yes, via `ListAvailableHosts` (hardware profile, group, `cost_index` per host); a user may hold
  heterogeneous allocations (e.g., a GPU rack group + a compute group).
- [ ] Reservations: pre-allocate capacity for a group ("keep 2 hosts free for `web`
      failover") — is this a tag, a quota, or a group-level policy?
- [ ] Should `attract`/`repulse` also constrain *checkpoint restore placement* (tier R), or
      only live scheduling?
