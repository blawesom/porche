# 07 — PoC Scope (pre-P0)

Working document — v4, 2026-10-18. **v2**: live migration explicitly deferred (v1 goal,
not PoC scope — E-3). **v3 (E-4)**: PoC re-targeted to **direct Cloud Hypervisor
orchestration** — no containerd, no Kata. **v4 (E-5)**: PoC host = **Fedora CoreOS,
stable stream** — the tracked upstream. Defines the **first proof of concept**: the smallest
buildable system that validates the riskiest technical bets of the design. Everything not
listed in scope is **deferred by decision**, not forgotten — see §3.

**Scope rule**: a feature enters the PoC only by evicting another one. If it is not needed
to answer the §1 questions, it is out.

---

## 1. What the PoC must answer (the risky bets)

1. **Data plane**: does **direct CLH orchestration** (host-agent → CLH API — no containerd,
   no Kata, E-4) boot microVMs on our bare metal, fast, with the doc 06 §5 hardening
   applied? (I-1, E-4 — the foundation of everything)
2. **Encrypted local volume**: can we hand a LUKS2-encrypted local NVMe volume to a microVM
   through dm-crypt, and wipe it at teardown? (I-2 reduced to tier E)
3. **Teardown determinism**: kill VM → unmap → wipe, in a loop, with no leak and no wedge.
   (FR-3/SEC-14 — the property the whole disposable-workload model rests on)

If these three hold, the architecture's core is sound. If any fails, we learn it in weeks,
not after building a control plane around it.

## 2. In scope

| Item | Content | Validates |
|---|---|---|
| 1 bare metal host | **Fedora CoreOS, stable stream** (E-5 — the tracked upstream: PERF and hardening numbers are measured on the actual production base) + kernel hardening **sysctls only** (io_uring off, userns off, core_pattern, modules_disabled) | SEC-11 subset |
| **Cloud Hypervisor (direct)** | host-agent drives CLH over its local API — no containerd, no shim, no annotations (E-4); devices from the frozen allow-list (SEC-3), resource ceilings in VM config (I-9) | I-1, SEC-3/5 |
| Guest image | minimal rootfs built in-house (kernel + initramfs + rootfs running the workload command), pinned by digest — the signing pipeline is P0 (doc 08 action 11) | FR-31 subset |
| **host-agent skeleton** (the only new code) | CLI, 4 verbs: `create / status / stop / wipe`. **CLH API client** + `cryptsetup open` + teardown wipe. Written as the **role-neutral core of the one binary** (I-13) and shaped for **reconciliation** (I-12: verbs operate on a local desired-state table, so the P0 control plane replaces the CLI writer, not the agent) | C-4 shape, I-11..13 |
| Tier-E volume | LUKS2 file on local NVMe, raw sparse inside; **key from a local file** (mock KMS — one JSON file, root-only) | I-2 reduced |
| Workload spec | image digest + command + volume size. That's all | FR-1 subset |
| Networking | CLH default (virtio-net, per-VM netns set up by host-agent; no exposure, egress NAT if it works out of the box) | — |
| Test harness | scripted: boot-time measurement, 20 parallel VMs, 1,000× create/wipe loop, residual-memory probe, 3 public escape PoCs must fail | PERF-1/5/8 subset, SEC-40 seed, doc 01 criteria 2/6/7 |

**Success criteria** (all measurable, nothing subjective):

- [ ] Workload create → process running in guest: P50 < 1.5 s (PERF-1)
- [ ] 20 parallel microVMs stable, < 200 MB RSS each at rest (PERF-5)
- [ ] Teardown P99 < 2 s; 1,000 create/wipe cycles: no host memory growth, no wedge (PERF-8)
- [ ] After wipe, the volume's NVMe region contains no recoverable workload data (probe)
- [ ] The 3 selected escape PoCs all fail
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
| CNI networking, egress proxy, ingress model | host-agent/CLH defaults suffice to measure boot/isolation |
| GPU (VFIO/MIG), CMK, cross-DC, HA everything | phase 2+ per doc 02 §7 |
| **Live migration (E-3, A-9/E-4)** | v1 goal, **not PoC scope**: needs a second host. The runtime question is settled (direct CLH) — the post-PoC spike validates CLH migration under load (PERF-13) on the PoC stack itself |
| dm-thin clone-at-create, snapshot-resume fast boot (borrowings) | perf optimizations on top of a working baseline |

## 4. Position in the roadmap

```
PoC (this doc)  →  P0 pilot (doc 02 §7)          →  P1 ...
1 host, CLI,      1 host + minimal control plane,   N hosts, scheduler, tiers R/S,
tier E only       KMS v0, CNI, signing             dedicated hosts, zero-trust fabric
```

The PoC is **smaller than P0**: P0 already assumes a (dev-mode) control plane and the full
acceptance list of doc 01 §8. The PoC exists to earn the right to build P0.

## 5. Estimated effort

- Setup (host, CLH, hardening): days
- Guest rootfs image (kernel + initramfs + rootfs): ~1 week
- host-agent skeleton (4 verbs, CLH API + cryptsetup): 1–2 weeks, Rust, test-first
- Test harness: 1 week
- Total: **~4–5 weeks, one developer**, no infrastructure beyond one machine

## 6. Kill criteria (what would send us back to the drawing board)

- Direct CLH cannot reach P50 < 1.5 s end-to-end on our hardware after tuning → revisit
  runtime choice (doc 06 §5 escape hatches; comparison per doc 11 §4 rule)
- dm-crypt attach via CLH's block-device API breaks or costs > 15% I/O → revisit the
  volume attach model
- VMM teardown leaves host state that requires manual cleanup → the disposable-VMM premise
  is wrong for this stack

Any of these is a *cheap failure* at PoC scale — which is the point.
