# 07 — PoC Scope (pre-P0)

Working document — v9, 2026-10-19. **v2**: live migration explicitly deferred (v1 goal,
not PoC scope — E-3). **v3 (E-4)**: PoC re-targeted to **direct Cloud Hypervisor
orchestration** — no containerd, no Kata. **v4 (E-5)**: PoC host = **Fedora CoreOS,
stable stream** — the tracked upstream. **v5 (E-6/E-7)**: wipe = crypto-erase only;
escape suite mapped to hardening controls. **v6 (E-16/E-17/E-18)**: the PoC executes
**in the integration environment** (nested virt, doc 13) — PERF numbers there are
mechanism-only smoke signals, metal keeps every gate; the escape suite validates by
baseline **delta**; create takes platform images only. **v7 (E-22/E-25)**: the guest boots
**IPv6 from the cmdline** (single-host scope); cross-host v6, legacy v4, and attached v4
stay deferred. **v8 (test plan + sequencing)**: §7 concretizes the test plan — methods,
samples, thresholds, wipe probe, baseline toggles, results template; the harness develops
on node 1's Rocky interim, the recorded PoC runs on the FCOS OMI (E-5 unchanged —
sequencing, not scope). **v9 (implementation plan, 2026-10-19)**: §7.4 escape artifact
policy decided (**hybrid**); §8 opened — repository layout (E-27), workstreams
WS-A..WS-F, burst-mode sequencing, ratified defaults (§8.4). Defines the **first proof of
concept**: the smallest buildable system that validates the riskiest technical bets of
the design. Everything not listed in scope is **deferred by decision**, not forgotten —
see §3.

**Scope rule**: a feature enters the PoC only by evicting another one. If it is not needed
to answer the §1 questions, it is out.

---

## 1. What the PoC must answer (the risky bets)

1. **Data plane**: does **direct CLH orchestration** (host-agent → CLH API — no containerd,
   no Kata, E-4) boot microVMs, fast, with the doc 06 §5 hardening
   applied? (I-1, E-4 — the foundation of everything. Runs in the integration
   environment — E-16; "fast" is a smoke signal there, the gate is metal)
2. **Encrypted local volume**: can we hand a LUKS2-encrypted local NVMe volume to a microVM
   through dm-crypt, and wipe it at teardown? (I-2 reduced to tier E)
3. **Teardown determinism**: kill VM → unmap → wipe, in a loop, with no leak and no wedge.
   (FR-3/SEC-14 — the property the whole disposable-workload model rests on)

If these three hold, the architecture's core is sound. If any fails, we learn it in weeks,
not after building a control plane around it.

## 2. In scope

| Item | Content | Validates |
|---|---|---|
| 1 host — **integration environment** (E-16) | **Fedora CoreOS, stable stream** (E-5 — the tracked upstream) as a VM on the doc-13 dedicated group (nested virtualization; CLH boots guests on a gen-7 node — doc 13 §4). PERF and hardening *evidence* numbers are metal-only (E-16); here they are smoke signals. + kernel hardening **sysctls only** (io_uring off, userns off, core_pattern, modules_disabled). **Sequencing (§7.1)**: harness develops on node 1's Rocky 9 interim (doc 13 §6); the **recorded PoC run** is on the FCOS OMI once imported | SEC-11 subset |
| **Cloud Hypervisor (direct)** | host-agent drives CLH over its local API — no containerd, no shim, no annotations (E-4); devices from the frozen allow-list (SEC-3), resource ceilings in VM config (I-9) | I-1, SEC-3/5 |
| Guest image | minimal rootfs built in-house (kernel + initramfs + rootfs running the workload command), pinned by digest — the signing pipeline is P0 (doc 08 action 11); **init sets the v6 address from the cmdline key** (`porche.ip6=` — E-22/E-25: the bootstrap mechanism the PoC proves) | FR-31 subset |
| **host-agent skeleton** (the only new code) | CLI, 4 verbs: `create / status / stop / wipe`. **CLH API client** + `cryptsetup open` + teardown wipe. Written as the **role-neutral core of the one binary** (I-13) and shaped for **reconciliation** (I-12: verbs operate on a local desired-state table, so the P0 control plane replaces the CLI writer, not the agent) | C-4 shape, I-11..13 |
| Tier-E volume | LUKS2 file on local NVMe, raw sparse inside; **key from a local file** (mock KMS — one JSON file, root-only) | I-2 reduced |
| Workload spec | platform-image digest + command + volume size. That's all (FR-1 post-E-17) | FR-1 subset |
| Networking | per-VM netns set up by host-agent, **v6 single-host** (the guest gets its /128 via the cmdline key — E-22/E-25; no exposure, no cross-host v6 — deferred) | — |
| Test harness | scripted: boot-time measurement, 20 parallel VMs, 1,000× create/wipe loop, residual-memory probe, 3 public escape PoCs — **each run against an unhardened baseline and the hardened config, validated by the delta (E-7/E-18)**: virtio-net offload attack (PwnEscape class) → SEC-7/I-7; virtio device-model escape against CLH → SEC-3/4; 2026 page-cache/userns kernel cluster → SEC-11 (+ boot-time sysctl check). Reproducing a published escape is **outside** the PoC estimate (E-18) | PERF-1/5/8 subset (smoke — E-16), SEC-40 seed, doc 01 criteria 2/6/7 |

**Success criteria** (all measurable, nothing subjective; PERF rows are **smoke signals
here** — authoritative numbers are measured on metal at the P1 gates, E-16):

- [ ] Workload create → process running in guest: P50 < 1.5 s (PERF-1 — smoke)
- [ ] 20 parallel microVMs stable, < 200 MB RSS each at rest (PERF-5 — smoke)
- [ ] Teardown P99 < 2 s; 1,000 create/wipe cycles: no host memory growth, no wedge (PERF-8 — the loop is a gate, the timing is smoke)
- [ ] After wipe, the volume's NVMe region contains no **cryptographically recoverable**
  workload data (probe: DEK zeroized, region TRIMmed — ciphertext bytes may legitimately
  remain; physical overwrite is rejected, E-6)
- [ ] The 3 escape PoCs show the **delta**: fail on the hardened config *and* exercise the
  intended surface on the unhardened baseline (SEC-7 offload policy; SEC-3/4 frozen
  device model; SEC-11 kernel hardening — E-7/E-18). Failing on both proves only the
  version floor (SEC-31)
- [ ] Kernel-hardening sysctls verified by script at boot

## 3. Explicitly deferred (do not build, even if "easy")

| Deferred | Reason it can wait |
|---|---|
| Control plane (new build), PORCHE API | the PoC host-agent is driven by a local CLI; a control plane around an unproven data plane is waste |
| KMS service | mock = local key file; the wrap/unwrap hierarchy is design work, not PoC risk |
| Tier R and S — epoch sync, checkpoints, dm-era, storage worker, hot/cold classes | tier E alone proves the volume attach path; sync machinery is a separate, later PoC |
| Dedicated-host allocation, placement views, attract/repulse | single host, single user — nothing to schedule |
| Scheduler, quotas, TTL, metering | ditto |
| Metadata proxy + tokens | no multi-workload secret delivery needed yet |
| Zero-trust inter-host network (mTLS mesh, signed orders, host firewall) | one host = no inter-host network; design stands (doc 02 discussion), implementation deferred to the 2-host stage |
| Image signing pipeline, SBOM registry, CVE-feed gates | pin by digest for now |
| CI attack suite beyond the 3 PoCs | the 3 prove the harness works; coverage grows with the fleet |
| Workload networking beyond single-host v6 (doc 14: cross-host v6 fabric, legacy-v4 exception, attached IPv4, egress/ingress proxies, rules) | single-host v6 boot/isolation is what the PoC measures; the rest is P1 |
| Console/stdio recording (FR-6, E-19) | no audit consumer yet; the serial console is visible live during the PoC |
| `ImportImage` / controlled-rebuild pipeline (E-17) | create takes platform images only; tenant OCI intake is P0+ |
| Intake sandbox (E-17) | deferred from v1 entirely — on demand, P2 |
| GPU (VFIO/MIG), CMK, cross-DC, HA everything | P2+ per doc 02 §7 |
| **Live migration (E-3, A-9/E-4; P1 stretch per E-15)** | **not PoC scope**: needs a second host. The runtime question is settled (direct CLH) — the post-PoC spike validates CLH migration under load (PERF-13) on the PoC stack itself |
| dm-thin clone-at-create, snapshot-resume fast boot (borrowings) | perf optimizations on top of a working baseline |

## 4. Position in the roadmap

```
PoC (this doc)  →  P0 pilot (doc 02 §7)          →  P1 ...
1 host, CLI,      1 host + minimal control plane,   N hosts, scheduler, tiers R/S,
tier E only       KMS v0, signing pipeline          dedicated hosts, zero-trust fabric
```

The PoC is **smaller than P0**: P0 already assumes a (dev-mode) control plane and the full
acceptance list of doc 01 §8. The PoC exists to earn the right to build P0.

**The PoC runs in** the Outscale **integration environment** (doc 13 — E-16): the
A-1/A-9 spikes and the doc-12 protocol work share the same nodes. The fidelity contract
(doc 13 §2) keeps every **gate** — performance, economics, security evidence — on metal:
the **metal validation run** (same harness, physical host) owns the PERF-1/5/8 and
security-evidence numbers before P0 commitment.

## 5. Estimated effort

- Setup (host, CLH, hardening): days
- Guest rootfs image (kernel + initramfs + rootfs): ~1 week
- host-agent skeleton (4 verbs, CLH API + cryptsetup): 1–2 weeks, Rust, test-first
- Test harness: 1 week
- **Excluded**: reproducing public escape PoCs (E-18 — harness exercises the intended
  surface; delta claims move to the metal run / SEC-40); the metal validation run itself
  (one machine, days, once the PoC passes functionally)
- Total: **~4–5 weeks, one developer**, no infrastructure beyond the doc-13 environment
  (already provisioned)

## 6. Kill criteria (what would send us back to the drawing board)

Split by evidence (E-16): **functional kills are judged in the integration environment**;
the **perf kill is judged on the metal validation run**, before P0 commitment — in the
nested environment it is watched as a smoke signal only (gross pathology ⇒ investigate
early, not kill).

- Direct CLH cannot reach P50 < 1.5 s end-to-end **on the metal validation run** after
  tuning → revisit runtime choice (doc 06 §5 escape hatches; comparison per doc 11 §4 rule)
- dm-crypt attach via CLH's block-device API breaks or costs > 15% I/O → revisit the
  volume attach model
- VMM teardown leaves host state that requires manual cleanup → the disposable-VMM premise
  is wrong for this stack

Any of these is a *cheap failure* at PoC scale — which is the point.

## 7. Test plan: methods, samples, thresholds

Every success criterion in §2 becomes an acceptance test here: one observation method,
one sample size, one threshold, one verdict. The PoC exits with **§7.5 filled in and
committed** alongside the run artifacts.

### 7.1 Fixture pinning

Each run records its fixtures (SEC-30-style traceability):

| Fixture | Recorded at run time |
|---|---|
| Node OS + kernel | image digest + `uname -r` (Rocky interim or FCOS OMI — sequencing below) |
| CLH | version + digest (v54.0 verified on node 1 — doc 13 §4) |
| Guest image | kernel + rootfs digests (action 11 artifact) |
| Host-agent | commit hash of the run |
| Hardening state | sysctl script output (§7.2) + offload/allow-list state |

**Sequencing (operator decision, 2026-10-18)**: the harness develops on node 1's
Rocky 9 interim; the **recorded PoC run** is on the **FCOS stable OMI** once imported
(doc 13 §8) — E-5 unchanged; this is sequencing, not scope.

### 7.2 Methods and thresholds

| Criterion | Observation method | n | Threshold |
|---|---|---|---|
| PERF-1 — create → running | host-agent stamps `create` (T0); guest init prints a `porche.ready` marker on the serial console; harness timestamps the line | 100 runs after 10 warmups | P50 < 1.5 s (**smoke** — E-16) |
| PERF-5 — density | procfs RSS sampler per CLH process (1 Hz); steady state = 60 s after boot | 20 parallel VMs | < 200 MB RSS each; zero VM/host-agent anomalies over 10 min |
| PERF-8 — teardown timing | per-cycle teardown duration across the loop | all 1,000 cycles | P99 < 2 s (**timing smoke**); the loop itself is a **gate** |
| PERF-8 — memory drift | `MemAvailable` floor + host-agent cgroup RSS sampled each cycle | cycles 200 and 1,000 | drift < 5 % |
| Wedge detection | host-agent liveness self-probe + create probe | continuous | liveness timeout 30 s, or a create > 60 s ⇒ wedge ⇒ fail |
| Wipe (E-6 semantics) | guest writes known markers before wipe; after wipe: mapper device absent, keyfile + kernel keyring empty, LUKS2 key slots nulled, region scan (as root) finds no *cryptographically recoverable* marker material; discard/trim report recorded | every wiped volume | all checks pass. **TRIM is verified in the lab via the discard report only — the NAND/physical claim is metal-only (E-16)** |
| Escape delta (E-18) | each PoC/exercise runs twice — hardened config and unhardened baseline (§7.3) | per §7.3 | **the delta**: fails on hardened *and* exercises the surface on baseline; failing on both = version floor only (SEC-31) |
| Sysctl verification | boot script asserts each SEC-11 item | every boot | `kernel.io_uring_disabled=2`, `user.max_user_namespaces=0`, `core_pattern` locked, `modules_disabled=1` |

### 7.3 Baseline toggles for the delta runs (E-18 — the reproducible part)

| Test | Hardened config (the control) | Unhardened baseline | Validates |
|---|---|---|---|
| virtio-net offload attack (PwnEscape class) | offloads **off** at virtio-net + tap | offloads **on** (checksum, TSO/GSO/GRO) | SEC-7/I-7 |
| CLH device-model escape | frozen device allow-list, no legacy devices | expanded device set (what the PoC requires) | SEC-3/4 |
| 2026 page-cache/userns kernel cluster | hardened sysctls | stock sysctl defaults | SEC-11 |

### 7.4 Escape-run safety rules (shared account — `infra/integration/BOUNDARIES.md`)

- Exploit/PoC material runs **only inside our node VMs' guests** (workload → nested-host
  boundary). Nothing is authored, aimed, or released against the Outscale boundary or any
  resource that is not PORCHE-owned.
- **Abort on anomaly**: any unexpected process, listener, or connectivity outside the
  expected composition stops the run and is reported (SEC-41 logic).
- Delta verdicts are *control* claims, never security-evidence claims (E-16/E-18) —
  security evidence is metal-only.

**Decided (operator, 2026-10-19): hybrid.** Synthetic surface exercises carry the delta
controls (every run); real public PoCs run only where safely reproducible inside our node
VMs' guests; full real-PoC artifacts move to the metal run / SEC-40.

### 7.5 Results record (the PoC exit artifact)

| Criterion | Method | n | Threshold | Measured | Verdict |
|---|---|---|---|---|---|
| PERF-1 (smoke) | console marker | 100 | P50 < 1.5 s | | |
| PERF-5 (smoke) | RSS sampler | 20 | < 200 MB each | | |
| PERF-8 loop (**gate**) | create/wipe | 1,000 | no wedge, < 5 % drift | | |
| PERF-8 timing (smoke) | per-cycle | 1,000 | P99 < 2 s | | |
| Wipe probe | markers + key slots | all | no recoverable data | | |
| Escape delta | §7.3 toggles | 3 | delta present | | |
| Sysctl script | boot assert | every boot | all pass | | |

The filled table plus the §7.1 fixture record is committed with the run; the PoC is done
when the table is green — or a §6 kill criterion fired.

---

## 8. Implementation plan (opened with the build, 2026-10-19)

The working plan the implementation follows; §1–§7 remain the contract. Status lives in
§8.5 and is updated as workstreams complete.

### 8.1 Repository layout (E-27)

| Tree | Role | First content |
|---|---|---|
| `doc/` | the corpus (moved 2026-10-19; doc numbers are stable IDs, unchanged) | — |
| `src/` | Rust workspace, **one binary role-selected at startup** (I-13): `porche-core` (types + desired-state store + verb orchestration), `porche-clh` (CLH API client), `porche-host` (`porche host-agent` — the PoC CLI) | WS-C |
| `guest/` | guest image build (action 11): pinned kernel + initramfs + BusyBox rootfs + platform init; artifacts digest-pinned at fetch | WS-B |
| `harness/` | the §7 test harness (scripted) | WS-D |
| `infra/` | integration environment IaC (doc 13) — unchanged | — |
| `runs/` | recorded run artifacts; the filled §7.5 table is committed with its run | WS-F |

Growth rules (README) govern `doc/` only; code-tree layout is build tooling.

### 8.2 Workstreams

| # | Workstream | Deliverable | Acceptance (§7.5 row) |
|---|---|---|---|
| WS-A | Host baseline — node 1, **burst** | `make up`; SEC-11 sysctl script applied + asserted; `brporche` + isolated-port config; fixture capture (§7.1) | sysctl script |
| WS-B | Guest image (action 11) | `guest/` build: pinned kernel + rootfs; init applies `porche.ip6=`, prints `porche.ready` | PERF-1 anchor; guest digests (§7.1) |
| WS-C | host-agent — the only new code; **test-first** | `porche host-agent` create/status/stop/wipe over the desired-state store; CLH client; LUKS2 + crypto-erase wipe; per-VM netns | the code under every §7.5 test |
| WS-D | Harness | §7.2 methods scripted: boot timing, density, 1,000× loop, drift, wedge, wipe probe | all rows |
| WS-E | Escape delta (hybrid, §7.4) | 3 classes × hardened/baseline toggles (§7.3), inside our guests only (§7.4 safety rules) | escape delta |
| WS-F | Recorded run | FCOS stable OMI (doc 13 §8) → full §7 suite → §7.5 filled + fixtures committed | exit |

### 8.3 Sequencing (burst mode — cost rule, doc 13 §7)

Node 1 runs only for integration bursts (`make up` at burst start, `make down` at end;
dedicated servers are billed while group VMs run). Development that does not need the
node happens locally.

| Week | Work |
|---|---|
| 1 | restructure (done) + WS-A + WS-B (guest build is local) |
| 2–3 | WS-C on node 1 in bursts: CLH/netns/volume integration; store + verb orchestration + CLH client are test-first offline |
| 4 | WS-D harness (interim Rocky 9 is acceptable, §7.1) + WS-E delta exercises |
| 5 | WS-F: FCOS OMI, recorded run, results |

### 8.4 Ratified defaults (operator, 2026-10-19)

| Point | Decision | Note |
|---|---|---|
| Desired-state store | **SQLite** (rusqlite, WAL) | closes doc 12 §8 open point; verbs are table writers — the P0 control plane replaces the CLI writer, not the agent (I-12) |
| Guest init | POSIX sh on BusyBox | platform init per E-25; `porche.ready` = the PERF-1 T1 marker |
| Harness language | bash (+ jq/python3 for stats) | CI-tooling class, not the trust path (doc 12 §4 allows) |
| Core runtime | sync-first PoC core; tokio/tonic arrive with the P0 long-running agent | the reconcile shape that must survive is store + verbs, not the async runtime |
| Escape artifacts | hybrid (§7.4) | synthetic deltas every run; real PoCs only where safely reproducible |

### 8.5 Status

| Workstream | State |
|---|---|
| repo restructure (E-27) | done 2026-10-19 |
| WS-A | **done 2026-10-19** — node 1 PoC-ready (interim Rocky 9.4; recorded run still FCOS per §7.1): CLH v54.0 digest-pinned, all four SEC-11 sysctls asserted, modules warmed **and verified** before `modules_disabled` seals, `/var/lib/porche` on the 200 GB BSU region (ext4, discard) |
| WS-B | **done 2026-10-19** — kernel 6.18.57 **bzImage** (vmlinux lacks the PVH header CLH wants; bzImage boots via the Linux boot protocol) + BusyBox rootfs + platform init, digest-pinned, installed on node |
| WS-C | **first full cycle green 2026-10-19** — create → `porche.ready` → `porche.done rc=0` → poweroff; wipe report all-pass (319 ms); CLH v54 API contract pinned on the node (payload strings, net sequence, serial `{mode,file}`, memory bytes, disk `image_type: Raw`, Content-Length HTTP reads, real TUN/TAP in-netns bridge + netdev anti-spoof). Residual: create ≈ 8–9 s (Argon2id luksFormat dominates — PERF-1 must account for KDF cost) |
| WS-D | **exercised 2026-10-19** — 5-cycle stability loop on node: no wedge, no residue, teardown P50 1.33 s / P99 1.33 s (§7.5-mini PASS); boot-time + density scripts ready, not yet run at §7.2 scale |
| WS-E | not started (after WS-C/D at scale) |
| WS-F | not started (FCOS OMI → recorded run) |
| **UX MVP (E-28)** | **console-host `porche-web` built 2026-10-19** — both surfaces over the PoC adapter (doc 09 §Console, doc 12 §9): workloads list/create/detail (console-tab poll), stop, **DeleteWorkload → wipe-report page**, admin fleet-of-one + audit stream; 5 unit tests + full dry-mode cycle verified locally (create→running→stop→wipe all-pass). **Live-mode node demo deferred** — the gen-7 metal pool in eu-west-2a went `InsufficientCapacity` mid-burst (also the StartVms-504 root cause, doc 13 §4); runs when capacity returns |

First-run artifacts: `runs/20261019-1201-miniloop/` (fixtures + results).
