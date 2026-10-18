# 10 — Operator Runbook: Metal Host Upgrades and Workload Migration

Working document — v2, 2026-10-18. Detailed operator process behind doc 05 §7 (summary) and
FR-34/35. **v2**: §5 adds live migration as a v1 goal (E-3, doc 08 A-9).
**Procedural sections follow the doc 11 §3 writing rules** (one instruction per
step, approved verbs, explicit end states). Fleet scale: up to ~1,000+ metal nodes, one user
per host (SEC-16). Host lifecycle and security are **platform-operator responsibility**;
the process is **user-action-free** (users see effects, never perform steps).

**Standing principles**

1. **Immutable host = one artifact**: rootfs, kernel, containerd, Kata, CLH, host-agent and
   its config ship as a **single signed image** (digest-pinned, SEC-30). There are no
   in-place package upgrades, no config drift — *every* host change (kernel, runtime, agent
   config) is a reimage with a new artifact version.
2. **Fail-closed, rollback-ready**: a host that fails any gate never serves workloads; the
   previous signed image is always recoverable (A/B slots, §4).
3. **Version-gated**: a rollout starts only if the artifact passed the attack suite
   (SEC-40), is registered (SBOM, CVE check per SEC-31/32), and the registry records the
   affected host set.
4. **Bounded blast radius per user**: drain concurrency ≤ N (default 1) hosts per user;
   never two hosts of the same allocation in `draining` simultaneously.

---

## 1. Artifact and version model

| Item | Content |
|---|---|
| Host image (the unit of upgrade) | host OS rootfs + kernel + containerd + kata + CLH + host-agent + config — one digest |
| A/B slots | two rootfs partitions per host: active + spare; upgrade writes the spare, boots once into it, commits on attestation; bootloader falls back to the previous slot on failure |
| Version registry | per host: current digest, attestation result, upgrade history; per fleet: which hosts run which digest (drives ring selection and targeted emergency rollout) |
| Guest images | independent of host upgrades — workloads are unaffected by the host image change except for the reboot/migration itself |

## 2. Pre-flight (pipeline gates — before any host is touched)

1. Build host image → sign → register (SBOM, CVE scan vs SEC-32 feed).
2. **Attack suite green** on the new image (SEC-40, dual-runtime matrix per doc 08 A-6).
3. **Staging soak**: N staging hosts (no users) run the image under synthetic load for the
   soak window; health metrics compared against baseline (boot time, VMM crash rate,
   throughput within 10% — the PERF regression rule).
4. **Rollout plan generated** (dry-run, reviewed by operator): rings, per-ring host lists,
   affected users, estimated duration, expected workload downtime per user class.
   `plan` output is the contract; `apply` executes exactly what `plan` showed.

## 3. Fleet rollout — rings

```
ring 1: 1%   soak 24 h      (≈10 hosts)
ring 2: 5%   soak 4 h
ring 3: 25%  soak 4 h
ring 4: 100% continuous, rate-limited
```

- Progression to the next ring requires: zero unexplained VMM crashes, zero attest failures,
  PERF regressions within budget, migration cutover times within PERF-12 — else **halt**
  (remaining hosts keep the old image; investigation; fix produces a *new* digest, the
  campaign restarts from ring 1 of the fixed artifact).
- Soak windows and ring sizes are policy knobs, not code.
- **Halt/pause/resume** are first-class operator actions, all audit-logged.

## 4. Per-host upgrade sequence (state machine)

```
allocated ──(ring scheduled)──► upgrade-pending ──► draining ──► drained
   ▲                                                        │
   │                                                        ▼
rejoined ◄──(soak ok)── attesting ◄──(boot into new slot)── reimaging
```

1. **`upgrade-pending`**: host still serves; scheduler knows it will drain (placement
   decisions show "host upgrade — ring N" to the user, doc 05 §2).
2. **`draining`**: scheduler stops placing; workload migration runs (§5). Concurrency
   guard: no second host of the same user is draining.
3. **`drained`** (verified, not assumed): the platform checks each condition. Verify
   zero sandboxes (CRI query). Verify zero attached volumes (lease store). Verify zero
   `/dev/mapper` devices. Verify the local NVMe regions are wiped (host report — external
   spot-audit per doc 08 A-3). If any check fails, halt this host.
4. **`reimaging`**: the sequence on the host is:
   1. Pull the new image to the spare slot. Verify its signature.
   2. Set boot-once to the spare slot. Reboot.
   3. Boot into the new slot. Attest it (step 5).
   (The old-slot agent performs steps 1 and 2. An out-of-band netboot path exists for a
   dead agent — same artifact, same gates.)
5. **`attesting`**: on boot, the platform checks the verified boot chain, the image
   signature, the kernel-hardening sysctls (SEC-11), and the expected digest. If
   attestation fails, the bootloader falls back to the previous slot automatically. The
   host rejoins on the old version. The platform flags it for investigation. A host never
   serves workloads on an unattested boot.
6. **`rejoined`**: host returns to its allocation with the new digest; post-join soak
   window before it counts as healthy for ring-gate metrics.

## 5. Workload migration mechanics (the primitive behind drain, rebalance, user moves)

Migration has two primitives (E-3, v1 goal). **Checkpoint migration**: stop the workload,
place it anew, start it. The checkpoint preserves the data. **Live migration** (stateful
workloads): transfer the VMM to the target host, then cut over. Pause the guest only at
cutover. Elevate a tier-R volume to write-through for the migration window (doc 02 §4.5).
Tier S volumes are already network-attached. Tier E workloads use checkpoint migration
only (ephemeral by choice).

Failure handling stays trivial. A failed checkpoint migration is a stopped workload.
Restart it on any eligible host. A failed live migration falls back to checkpoint
migration. There is no half-migrated in-between state.

| Step | Tier E (ephemeral) | Tier R (replicated-async) | Tier S (synchronous) |
|---|---|---|---|
| 1. Choose target | same user's allocation, affinity + capacity (incl. NVMe budget) | same + storage-network reservation | same |
| 2. Pre-warm target | pull image (digest) | pull image | pull image |
| 3. Quiesce | — | optional `fsfreeze` via kata-agent exec (app-consistent) | optional `fsfreeze` |
| 4. Seal | — | force seal: ship dirty epoch to envelope, CRC-verify, record checkpoint | not needed — envelope always current |
| 5. Stop | CRI stop sandbox | CRI stop sandbox | CRI stop |
| 6. Release | wipe local region (report) | release volume lease; wipe dirty region (report) | release lease |
| 7. Start on target | fresh volume from image | attach (KMS unwrap, dm-crypt open) → create sandbox → boot; guest fs journal replays | attach → boot |
| 8. Verify | workload healthy (user check or agent probe) | idem | idem |
| 9. Record | audit + placement events; source resources reaped | idem | idem |

- **Downtime**: E ≈ cold start; S ≈ cold start; R ≈ seal + cold start (PERF-12: P50 < 60 s
  for ≤ 10 GB dirty). Live migration (stateful): cutover pause only (PERF-13: P99 < 1 s) —
  preferred for drain and rebalance of long-lived workloads when the runtime supports it
  (A-9 gate).
- **Allocation boundary (SEC-16)**: targets are always within the owner's allocation —
  checkpoint ciphertext never transits another user's host.
- **Single-host user edge case**: no in-allocation target exists. Resolution: the platform
  **provisions a temporary replacement host** into the user's allocation for the migration
  (borrowed from the free pool, released after the original host rejoins). If the pool is
  exhausted: E workloads restart on the same host post-reimage; R/S cutover waits for a
  free host, surfaced as a placement-decision reason. *Users never lose data because of an
  upgrade.*
- Standalone use: `MigrateWorkload` (FR-34) runs the same primitive on demand — drain,
  rebalance, user-initiated move.

## 6. Failure handling

| Failure | Automatic response | Operator action |
|---|---|---|
| Migration step fails (seal, attach, boot) | workload marked `failed` with reason; retryable placement; source data intact (R/S envelope never left) | none required; investigate if repeated |
| Host fails to rejoin / attest | bootloader falls back to previous slot; host flagged | pull host, out-of-band reimage, quarantine if persistent |
| Ring gate trips (crashes, PERF) | rollout halts; remaining hosts untouched | root-cause; new digest restarts campaign at ring 1 |
| Host dies mid-drain | its workloads: E lost (by contract), R/S restart from last sealed checkpoint on another in-allocation host (or replacement host, §5) | none required; host replaced by fleet ops |
| **Security emergency (P1 CVE / active exploit)** | targeted emergency rollout: only affected digests (registry knows the host set), compressed rings, soak waived with explicit operator sign-off; if an escape is *active*, affected hosts → `quarantined` (workloads terminated, not migrated — checkpoint may carry compromised state; the user restarts from image or a pre-incident checkpoint), out-of-band wipe | incident process per OPS-10 |

## 7. Operator tooling (spec-level; part of the control plane, later spec)

- `upgrade plan --image <digest>` → dry-run plan (rings, hosts, users, durations)
- `upgrade apply [--ring N | --host X]` / `upgrade pause` / `upgrade halt`
- `upgrade rollback --host X` → re-boot previous slot (or force netboot of old digest)
- `migrate --workload W [--target H]` → the §5 primitive on demand
- Everything emits audit events (SEC-23) and feeds the placement-decision explanations the
  user sees (FR-14): "host upgrade — ring 2", "migration cutover 41 s", "replacement host
  provisioned".

## 8. Timing budget (defaults — policy knobs)

| Step | Target |
|---|---|
| Pre-flight (build, gates, staging soak) | hours; soak window is the dominant term |
| Ring soak | ring 1: 24 h; rings 2-3: 4 h |
| Per host: drain → drained | bounded by migration cutover (PERF-12) + verify |
| Reimage + attest + rejoin | < 15 min (slot write dominates; netboot fallback allowed 30 min) |
| Post-join soak | 1 h |
| Full campaign, 1,000 hosts, default knobs | ≈ 3–5 days (formula: hosts × per-host time / ring concurrency + soaks) |

## 9. Invariants (checked by tooling, every campaign)

- [ ] No host reimaged with attached volumes or live sandboxes (verified `drained`, §4.3)
- [ ] No two hosts of one user draining simultaneously
- [ ] No workload placed on an unattested host
- [ ] No migration target outside the owner's allocation
- [ ] Previous signed image recoverable on every host at every step (A/B)
- [ ] Every user-visible downtime carries a placement-decision reason
- [ ] Emergency paths require explicit operator sign-off and are audit-logged as such
