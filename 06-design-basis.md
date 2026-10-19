# 06 — Design Basis (self-contained research record)

Working document — v2, 2026-10-18 (v1, 2026-10-17). Consolidates everything the PORCHE
documents trace to:
the isolation technology study, the 2021–2026 vulnerability history, the 2026 incumbent-platform
isolation assessment (findings F0–F45), the legacy base design, and the stack
decision. **v2**: vocabulary sweep only (GPU phase prose → P2, doc 08 E-16..E-21 pass).
**This directory is self-contained**: no external repository is required to read
or extend the PORCHE design docs. (French-language working originals of the research phase
are kept in `archive/`.)

Traceability tags used across docs 01–05:
- `[R-xx]` = finding from the incumbent-platform assessment → §3 below
- `[C-yy]` = CVE / cluster from the vulnerability corpus → §2 below
- `[D-yy]` = decision from the stack study → §5 below

---

## 1. Isolation technology landscape (study conclusion)

| Cat | Technology | Isolation | Boot | Runtime overhead | Density | GPU |
|---|---|---|---|---|---|---|
| 1 | kernel primitives (namespaces, cgroups, seccomp, bwrap/nsjail) | weak–medium (shared kernel) | < 10 ms | ≈ 0% | very high | yes |
| 2 | userspace containers (runc, Docker, Podman, LXC) | medium (config-dependent) | 10–100 ms | 1–5% | very high | yes |
| 3 | faux-kernel sandboxes (gVisor, libkrun, Kata) | high | 50–200 ms | 10–50% syscall-heavy | high | Kata/libkrun yes, gVisor no |
| 4 | microVMs (Firecracker, Cloud Hypervisor) | high (dedicated guest kernel, minimal device model) | ~125 ms | low (~5%) | high | CH: VFIO; FC: no |
| 5 | full VMs (QEMU/KVM, Xen, VMware…) | very high | 5–30 s | 3–10% | low (tens) | yes (VFIO) |

Decision criteria: who runs the code (trust), density/boot latency, GPU access, ecosystem.
**PORCHE conclusion**: untrusted workloads at high density with a GPU path → category 4
device model, delivered through the category-3 **Kata** API (container-compatible, minimal
glue) with **Cloud Hypervisor** as VMM. Rejected: runc (shared kernel — SEC-1), gVisor (no
GPU), Firecracker (no GPU, more glue), QEMU microVM (category-5 device-model surface and
CVE volume — judged against the **latest QEMU LTS at decision time**, per the comparison
rule in doc 11 §4).

## 2. Vulnerability history 2021–2026 (essentials)

Window 2021-01 → 2026-10; NVD 2.0, CISA KEV (1,748 entries), GitHub/vendor advisories;
filter: guest/container→host leaks, container-reachable LPE, hypervisor RCE. No unverifiable
CVE IDs retained.

**Cat 1 — shared kernel** (~20 CVE, 5+ exploited in the wild). Clusters: nf_tables (5 CVE,
2 ITW — e.g. CVE-2024-1086), overlayfs (Docker rootfs — CVE-2023-0386, GameOverlay),
2026 "page-cache" wave (CVE-2026-43284/43500 Dirty Frag actively exploited, 43494 PinTheft,
43499 GhostLock, 46300 Fragnesia, 80521 AF_UNIX). io_uring: CVE-2025-39698/40364,
CVE-2026-43494. **User namespaces are the key that makes these reachable from unprivileged
containers.**

**Cat 2 — userspace containers**: runc CVE-2024-21626 (Leaky Vessels), CVE-2025-31133,
CVE-2025-52565, CVE-2025-52881 (procfs core_pattern); three runc breakouts in one salve
(Nov 2025); BuildKit/Docker CVE-2024-23651/52/53 (build-time escapes, one CVSS 10).

**Cat 3 — faux kernels**: gVisor — 7 CVE in the window, zero host escapes 2018–2025, first
in 2026 (CVE-2026-96812, needs CUSE config); strict CVE policy (only boundary-crossing,
gVisor-specific). **Kata** — quiet 2021–2024, then 9 CVE + 2 GHSA in 16 months (2025-09 →
2026-08), several true guest→host escapes, mostly via **annotations and virtio-fs**:
CVE-2026-50540 (config_path annotation → host TOML → root RCE), CVE-2026-44210
(virtiofsd injection → mount host rootfs), CVE-2026-47243 (virtio-fs standalone →
/etc/cron.d RCE), GHSA-7fhf-v3p3-rp56 (sandbox_bind_mounts), CVE-2026-24054 (malformed
image → host device hotplug into VM), CVE-2026-24834 (pmem/DAX RCE in guest).

**Cat 4 — microVMs**: small CVE history for Firecracker/Cloud Hypervisor (Rust, rust-vmm,
fuzzed); no escape-class pattern comparable to QEMU's.

**Cat 5 — full VMs**: high QEMU CVE volume on the **latest LTS line** (the basis of
comparison — never EOL versions); the 2024→2026 incomplete-fix pattern on maintenance
lines is a process-risk observation, not part of the technical comparison.

**Standing lessons**: (1) version lag on any line eventually equals an open door;
(2) annotations/user-controllable config are a recurring escape vector; (3) the 2026
kernel clusters demand a hardened host kernel (io_uring, userns, core_pattern).

## 3. Platform assessment — findings digest (F-register, redacted)

A 2026 security assessment of a production QEMU-KVM cloud platform (the design reference
behind several PORCHE decisions). **Detailed findings are held in private notes, not in
this repository.** The public-safe lessons the design traces to:

| Finding | Public-safe lesson (what PORCHE does about it) |
|---|---|
| F43 | A VMM line past end-of-life yielded a deterministic guest→host heap disclosure. → No EOL branches in the fleet (SEC-31); minimal-device-model VMM (SEC-2/3) |
| F45 | Guest-supplied network-offload metadata enabled a host-kernel escape (public CVE-2026-99999, Linux net/ipv4). → Offloads disableable by policy, off for untrusted (SEC-7); hardened kernel (SEC-11) |
| F42 | An unauthenticated host↔guest handshake let the guest choose the host's key. → No custom guest protocol at all (C-2): upstream kata-agent mTLS (SEC-8) |
| F31/F44 | Guest-triggerable VMM aborts/wedges from device-level bugs (self-DoS). → Disposable VMM per workload + resource envelopes (SEC-13, OPS-4) |
| F34 | A dormant device (graphics) became live attack surface during operator console sessions. → Serial console only, audited (SEC-4) |
| F0.1/F17 | Metadata service trusted requester network position; identity headers spoofable. → Single-use boot-injected tokens, no header/network-position trust (SEC-21) |
| F19/F24 | Control plane trusted agents on network reachability alone. → Strong AuthN everywhere, no "internal network" exception (SEC-20) |
| F22 | Snapshot imports could carry backing-file escapes. → Structural rejection of backing chains at import (FR-32) |
| F27 | Fleet patch level was unobservable from outside. → Version registry + transparency (SEC-30) |
| F1/F3 | Cache/DRAM side channels: none found across VMs with controls active. → Dedicated hosts make the class structurally moot (SEC-16) |
| M1 | Side-channel PoC methodologies produce false positives (prefetcher effects). → Randomized probing mandated in validation (SEC-40) |

---

## 4. Incumbent base design — moved to private notes

The detailed summary of the incumbent platform's design (control plane, per-VM VMM driver,
in-guest agent protocol, storage topology, affinity model) **is not part of this
repository** — it describes internals that are not public. It is held in local private
notes (`~/porche-private/design-reference.md`, outside this repo).

What the public corpus needs from it is already abstracted:
- the **concept adoptions/rejections** table — doc 02 §1;
- the **concept mapping** (incumbent component role → PORCHE choice) — doc 02 §6;
- the affinity/attract/repulse semantics — doc 05 §3;
- the storage and upgrade models — docs 04 and 10.


## 5. Stack decision and hardening guidance

**Kata Containers 4.x + Cloud Hypervisor on containerd** (runtime class `kata-clh`):
container API unchanged, every sandbox a microVM with dedicated guest kernel; CLH gives a
future VFIO GPU path and is Rust/rust-vmm (fuzzed, smaller attack surface); minimal in-house
glue. Escape hatches: Firecracker for max density with zero GPU (more glue); direct CLH
orchestration for total VMM control. **Avoid QEMU microVM.**

**Scope change (2026-10-18, doc 08 E-3 → E-4 — resolved)**: live migration became a **v1
goal**, which re-opened this decision; the revalidation (evidence in doc 08 A-9) concluded
that **direct CLH orchestration is adopted** — Kata and containerd are dropped from the
data plane. Evidence snapshot at decision time: CLH live migration is native and mature
(TLS transport, documented n-2 version compatibility); Kata has no sandbox hand-off
(upstream issue #1690, open since 2021, no roadmap); the only working third-party Kata
live migration drives QEMU via QMP (excluded by SEC-2); Firecracker has no live migration
upstream. The Kata hardening points below (version floor, annotation allow-list) are
**superseded**; LSM confinement now covers host-agent + VMM. Full rationale: doc 08 A-9,
doc 02 §4.

Hardening essentials:
- Kata ≥ 4.0.0 (fixes CVE-2026-50540, CVE-2026-47243, CVE-2026-44210); annotation
  allow-list **empty by default** (never `io.katacontainers.config_path`,
  `sandbox_bind_mounts`); virtio-fs sources restricted, never `--sandbox=none`; guest
  seccomp on; per-sandbox vCPU/RAM ceilings.
- Host kernel: `kernel.io_uring_disabled=2`, `user.max_user_namespaces=0`, locked
  `core_pattern`, `modules_disabled=1` after boot; LSM confinement for containerd/shim/VMM;
  kernel ≥ recent 6.x (2026 page-cache cluster); automated CVE feeds + unattended upgrades
  on kernel/runtime.
- Prereqs: `/dev/kvm` (bare metal), ~64 MB RAM + 1 vCPU baseline per microVM.
- GPU (P2): VFIO passthrough (validate IOMMU groups/BIOS first) or MIG slices on A100.

Validation plan: install on test host → boot < 1 s, minimal `/dev`, no host devices visible →
run public escape PoCs (all must fail) → 20 parallel sandboxes (< 200 MB RSS each) →
rollout + update monitoring.
