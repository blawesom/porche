# 04 — Storage Tiers: Local Write-Back, Async Network Sync, Checkpoint Semantics

Working document — v2, 2026-10-18. **v2**: tier R/S confirmed as the v1 stateful-workload
path (E-1); live-migration volume semantics added (§2, E-3). Evaluates the proposal: **local storage for writes, async
write-out to network storage, and "snapshot" redefined as a sync-to-network checkpoint.**
Extends `02-architecture.md` §5 (I-2); supersedes the all-remote-NFS volume path for most
workload classes.

---

## 1. Problem with the v3 baseline

v3 keeps the incumbent storage topology: every guest I/O goes over the network to the NFS storage pool
(NFS), even though the bare metal hosts have local NVMe. Costs:

- Write latency: NFS round-trip (~0.5–2 ms) vs local NVMe (~20–100 µs) — a 10–50× penalty
  on the hot path, paid by every workload, always.
- The NFS server is on the synchronous path of every write — availability coupling (filer
  maintenance = fleet-wide I/O impact).

But pure local storage loses the two things network storage provides: durability across host
loss, and location independence (the incumbent "any VM on any host"). The proposal — local writes +
async remote sync — buys the performance back while keeping durability *at defined points
in time* instead of per-write.

**This is a deliberate durability-semantics change**: from "every acknowledged write is on
shared storage" to "every acknowledged write is on local NVMe; a crash-consistent image
exists on network storage as of the last sync point." That contract must be explicit,
per-volume, and visible to the tenant.

---

## 2. Tier model (the contract)

Every volume picks a tier at creation (immutable per volume in v1):

| Tier | Write path | Durability contract | RPO | For |
|---|---|---|---|---|
| **E — ephemeral** | local NVMe only | none past teardown; wiped at teardown | ∞ | U1 agents, U2 builds (the disposable 90%) |
| **R — replicated-async** | local NVMe, epoch sync to remote envelope | crash-consistent remote checkpoint | ≤ sync interval (default 60 s, tunable 10 s–1 h) | U3 dev envs, **U4 stateful apps (the long-lived default)**, long-running agents worth restarting |
| **S — synchronous** | write-through to remote envelope (v3 path) | every flush durable on network storage | 0 | control-plane data, databases, anything the tenant declares critical |

Restore semantics:
- **E** — no restore. Host loss = workload loss. Reschedule from image.
- **R** — restart from the **latest sealed checkpoint** on *any* host (location
  independence returns at checkpoint granularity). Guest filesystem journal replays
  (crash-consistency, see §3).
- **S** — v3 behavior; restart anywhere, no data loss on host loss.

**Live migration** (v1 goal, E-3): clean only when the volume is network-attached at
cutover — tier S by construction; tier R is **elevated to write-through (S semantics) for
the migration window**, then demoted. The sealable tier-R stack (§3.1, A-1 spike) is a
prerequisite for that elevation. Tier E stays on checkpoint migration (ephemeral by
choice). See doc 02 §4.5.

### 2.1 What each tier buys — performance, latency, and cost

Each tier is a **performance/latency/cost trade**, not just a durability level:

| Tier | Write latency | Read path | Remote footprint | Cost profile | Buys you |
|---|---|---|---|---|---|
| **E** | NVMe — fastest | NVMe only | none | local NVMe GB·s while attached — **cheapest** | max performance, zero storage cost |
| **R** | NVMe — same as E | warm: NVMe; cold miss: network | used blocks only, incremental epochs | local NVMe + hot-class GB·month + ship bytes ≈ 1× write volume (amortized, batched) | E-class write performance **plus** bounded RPO |
| **S** | network (~0.5–2 ms) | network | full, synchronous | hot-class IOPS + GB·month, NFS on the sync path — **most expensive** | zero RPO, location-independent always |

The decisive economic property: **tier R costs barely more than tier E** (the remote
envelope grows only with used blocks; epoch traffic amortizes to ≈1× the write volume,
batched and off the hot path), while **tier S pays the network latency tax and NFS IOPS
cost on every write, always**. Therefore:

- **R is the correct default** for anything expected to outlive a host hiccup — it is the
  only tier where you get NVMe-class writes *and* a survivable copy.
- **E is the cost/perf optimum for the disposable majority** (U1/U2): zero remote spend.
- **S is reserved** for declared-critical data — the tenant explicitly pays latency+IOPS for
  synchronous durability.

### 2.2 Second dimension: storage class of the remote copy

Durability tier (E/R/S) is orthogonal to **where the remote bytes live** — a cost lever of
its own, since sealed checkpoints are immutable and rarely read:

| Class | Medium | Content | Cost |
|---|---|---|---|
| **hot** | SSD share (NFS) | tier-S origins; latest sealed checkpoint of tier-R volumes | high IOPS, GB·month |
| **cold** | HDD / object store, erasure-coded, cross-DC | older retained checkpoints (immutable, read ≈ only at restore) | ≈ 5–10× cheaper per GB |

Consequences: retaining checkpoint history costs cold-class prices, not hot — the retention
policy ("latest + N") becomes a cheap insurance knob instead of a storage-bill driver; the
latest checkpoint stays hot so restore stays fast; compaction (§4) bounds the increment
set. Tier E has no remote footprint at all and therefore no class.

### 2.3 Metering (feeds FR-42)

Per-workload cost observability, per tier and class: local-NVMe GB·s, hot/cold GB·month,
epoch-ship bytes, seals per hour. Tenants see the trade they are making — the tier choice
becomes an informed cost decision, not a guess.

---

## 3. Consistency — the hard part, answered mechanism by mechanism

### 3.1 Single-writer invariant (no split brain, no consensus needed)

A volume is attached to **exactly one host at a time**, enforced by a control-plane state-store
lease (the incumbent design already uses DB locks against over-allocation — same machinery). Checkpoint
restore takes a new lease. No multi-writer, ever → no distributed-consistency protocol on
the data path. The only ordering authority is the single host.

### 3.2 Crash consistency of checkpoints (the epoch mechanism)

The checkpoint must be a **power-cut-equivalent image**: whatever the guest's filesystem can
recover after a crash, it can recover after a checkpoint restore. Mechanism: **epoch
tracking** on the encrypted device — kernel-native `dm-era` (built exactly for this:
incremental backup by marking blocks written per "era"):

1. Volume stack on the host (tier R) — **candidate composition, requires a spike (doc 08
   A-1)**: `dm-era` tracks dirty blocks per epoch but passes I/O through to a single
   origin; it does **not** provide local write-back caching. A caching mechanism (dm-
   writecache, dm-cache, or a custom target) must sit in the stack between dm-crypt and
   the origin, and its interaction with epoch sealing (what has/hasn't reached the origin
   at seal time) is an open engineering problem. Candidate shape:
   `guest → dm-crypt → dm-era (tracking) → dm-writecache (local NVMe hot) → remote LUKS
   envelope (origin via loop/NFS)` — writes land in the NVMe region, acknowledged at NVMe
   speed; the era marks every block written during the current epoch. **No tier-R
   commitment before the spike proves a working, seal-able stack.**
2. **Sync (checkpoint sealing)**: flush guest → quiesce I/O briefly (dm suspend or barrier) →
   ship only the epoch's dirty **ciphertext** blocks to the remote envelope → CRC-verify →
   atomically record "checkpoint N sealed" in the control-plane state store → advance the epoch.
3. Between checkpoints the remote image is possibly torn — it is *never* read except after a
   seal. After a seal it is exactly a crashed-disk image: guest fs journal replays.

Torn-write detection: every shipped block carries a CRC; a failed/short epoch ship aborts
the seal and **the previous checkpoint is never touched** (write-ahead discipline: epoch N+1
lands in a staging area of the envelope, swap on completion).

### 3.3 Ordering and flush semantics

- Guest `fsync`/flush (FUA) passes through dm-crypt to the **local NVMe** — honest,
  documented: flush = "durable against guest crash and host reboot", not against host loss.
  Tier S is the tier where flush means network-durable.
- dm-era + dm-crypt preserve write ordering per block; the epoch seal inserts a barrier
  (quiesce) so no write straddles a checkpoint boundary.
- Same-host crash (reboot, VMM crash): the NVMe region survives → epoch state and dirty
  blocks recover on re-attach; nothing is lost that a local disk wouldn't lose. Host
  **loss** (hardware dead, decommissioned): NVMe is gone → RPO window = one epoch, by
  contract.

### 3.4 App-consistent checkpoints (optional, opt-in per sync)

Crash-consistent is the default (fs journal replays — the state every OS is designed to
tolerate). For tenants who want more: before sealing, host-agent runs `fsfreeze` (or an
app-specific hook) **inside the guest via kata-agent exec** — this resurrects the useful
10% of the incumbent app-consistent-snapshot feature with zero custom guest protocol (the other 90%
— chains, block-stream, QMP — stays dead).

### 3.5 Cross-DC replication (distant storage)

Sealed checkpoints are immutable → asynchronous, retry-safe, checksummed shipping to the
second DC is trivial (this is exactly the incumbent replication-job shape). Cross-DC RPO = seal
interval + ship time. No synchronous cross-DC path in v1 (no workload class needs it).

---

## 4. "Snapshot" reframed as sync-to-network — exactly right, with one caveat

The user-facing API becomes **`SyncVolume` / checkpoint**:

- force-seal now (optionally app-consistent), record checkpoint id in the control-plane state store;
- restore = new workload booting from a sealed checkpoint on any host;
- list checkpoints = list of sealed epochs.

What this *avoids* versus legacy snapshots: no backing chains, no block-stream, no dead-leaf
GC, no QMP, no guest protocol — a checkpoint is just "the remote envelope as of epoch N".

**The caveat**: restoring checkpoint N needs the base image + epochs up to N, and keeping
K checkpoints means keeping K-1 epoch increments — a bounded, internal, single-parent
incremental history. To keep the "no chains" property *observable* to the platform, a
compaction job folds old epochs into the base (re-encrypt copy, storage-worker) whenever
increments exceed a size/time budget. This is chain management reduced to its floor: linear,
single-parent, background, and invisible to tenants. (If even that is unwanted: cap retained
checkpoints at 1 — "latest only" — and compaction is just the seal itself.)

Recommendation: default retention = latest checkpoint only + one configurable history
depth for tier R; compaction folds everything older.

---

## 5. Security review (does this weaken I-2?)

- **Ciphertext-only invariant holds**: dm-crypt sits *above* the cache/era layers — the NVMe
  region and every shipped block are ciphertext; the remote envelope stays a LUKS2 file;
  plaintext never leaves host RAM. The NFS storage still never holds keys.
- **Local NVMe regions** are per-volume partitions, wiped (secure erase / crypto-erase) at
  teardown — SEC-14 applies to NVMe, not just RAM. NVMe holds ciphertext anyway, but wipe
  prevents cross-workload *reuse* of stale regions.
- **Checkpoint metadata** (epoch numbers, CRCs, seal records) is integrity-protected in the
  audit/DB path; a forged "sealed" record without matching ciphertext is detected at
  restore (CRC of blocks + envelope size).
- New kernel components (dm-era, dm-writecache if used for the region) are kernel attack
  surface — but they are in-kernel, mainline, and exercised by the CI attack suite (SEC-40
  gains a "storage stack" category: era boundary conditions, malformed CRCs, epoch races).

---

## 6. Performance model (to be validated in P0)

| Metric | v3 (all-NFS) | Proposed tier R | Tier E |
|---|---|---|---|
| Write latency (4k, queue depth 1) | ~0.5–2 ms (network) | ~20–100 µs (NVMe) | ~20–100 µs |
| Read (cold block) | network | network (first), NVMe (warm) | NVMe only |
| Remote write traffic | 1× every write | ~1× every write (amortized, batched) | 0 |
| Host-loss data loss | 0 | ≤ 1 epoch (default 60 s) | everything since image |
| Filer on synchronous path | yes | no (only at seals) | no |

Backpressure (tier R only, and durability-driven — never a space or performance control on
local storage): the NVMe dirty region is bounded per volume. If a workload's write rate
exceeds the epoch-ship rate, writes throttle — self-DoS only (OPS-4 respected), never
neighbor impact. **Local-storage performance is the raw device**: tier E (and tier R's hot
path) are unallocated and unshaped — space is the only local constraint (doc 05 §1.3).
Performance shaping exists **only** on the network path (§6.1).

### 6.1 Storage-network bandwidth as an allocated resource

Network storage performance is bounded by **two explicit levers**, not by best-effort
sharing of the host NIC:

1. **Per-volume storage-network reservation** (declared at creation, like size and tier):
   a token-bucket cap on the volume's storage traffic — tier-S synchronous writes and
   tier-R epoch ships. Effective throughput = min(reservation, storage-class capability).
   Unreserved (best-effort) volumes get a platform-default share — the point of the
   reservation is to make "my volume's performance" a *contract*, not a hope.
2. **The RPO backpressure rule** (tier R): the epoch-ship bandwidth must ≥ the volume's
   sustained dirty rate, or the RPO contract (OPS-11) is unmet. Enforced by throttling the
   *guest's* writes (self-DoS), never by silently extending RPO.

**Effective latency** is reported, not just promised: per host↔shard path the control plane
publishes the measured RTT/histograms (tier-S write latency ≈ network RTT + filer service
time), so a volume's tier-S latency expectation is grounded in observed path latency, and
the tier choice (§2.1) is an informed one. A user story made explicit: *"network storage is
bound in performance by the network bandwidth allocated and the effective latency"* —
both terms are now first-class: reservation (throughput) and published path latency
(latency).

New/changed requirements: **PERF-10** (tier-R write latency ≤ 3× raw NVMe), **PERF-11**
(seal overhead: guest I/O pause at seal < 100 ms P99), **REL-RPO** (default tier-R RPO ≤ 60 s,
enforced).

---

## 7. Recommendation

**Adopt.** With the following decisions:

1. Three durability tiers (E / R / S) as the volume's defining attribute — the performance
   win of E/R covers the disposable majority of target workloads; S preserves legacy-class
   durability where actually needed.
2. Epoch mechanism: `dm-era` + ciphertext epoch shipping + sealed checkpoints (kernel-native,
   no new parsers — consistent with C-5 philosophy).
3. Snapshot API **becomes** the sync/checkpoint API (§4); retention policy "latest + N",
   background compaction; no user-visible chains.
4. Cross-DC = async shipping of sealed checkpoints only.
5. `fsfreeze`-style app-consistency as an opt-in per checkpoint via kata-agent exec.

Rejected alternatives (for the record):

- **dm-cache writeback / dm-writecache to an NFS origin**: origin-on-loop-over-NFS is
  fragile across NFS server blips, and writeback flushes are not checkpoint-shaped (no
  tenant-visible sync point, no clean RPO story). dm-era gives the same performance with
  *defined* consistency points.
- **File-level sync (rsync/ZFS-send of the guest fs)**: requires filesystem awareness
  inside the host — layering violation, breaks the block-device contract.
- **Synchronous remote writes for everyone** (v3 as-is): keeps the 10–50× latency tax for
  durability most target workloads do not need.
