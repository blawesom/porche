# PORCHE — Portable Cloud Orchestrator

In-house cloud orchestrator: **bare metal**, first-in-class performance, security by
design.

## Objectives

- **Own the metal**: bare-metal hosts, one user per host — no legacy virtualization
  layer between the orchestrator and the hardware.
- **Untrusted workloads by default**: the reference adversary is the workload itself
  (compromised AI agent, malicious build) — every design decision starts there.
- **Disposable by construction**: a workload is a microVM (Cloud Hypervisor directly
  orchestrated — no Kata, no containerd) with encrypted local storage and a
  deterministic teardown: kill → unmap → wipe (crypto-erase), no residue, no wedge.
- **Security by design, not by patching**: every security requirement traces to a
  real-world lesson (2026 incumbent-platform isolation assessment, 2021–2026 CVE
  corpus — doc 06); hardening is validated by an escape suite, not assumed.
- **Measurable performance**: quantified SLOs (PERF-1..13) with test plans and
  thresholds — not adjectives.
- **Boring where it doesn't matter**: battle-tested components in the trust path,
  one language, one audited codebase.

## Core requirements (the essentials)

| Class | The contract |
|---|---|
| Workload lifecycle | create from platform images only (controlled rebuild — nothing tenant-supplied runs as supplied); deterministic teardown (FR-3) |
| Isolation | one microVM per workload, frozen device model, kernel hardening (SEC-11), per-workload netns — no shared L2, no VLAN trust |
| Storage | tiers: encrypted local NVMe write-back (E), async epoch sync (R), archival (S); wipe = crypto-erase only (E-6) |
| Networking | IPv6-first: one /128 per workload from the VPC ULA; egress/ingress proxies are the sole enforcement points; no BGP in v1 |
| Control plane | three planes (admin / tenant / KMS shared trust anchor); hosts own reality — unbounded autonomy on partition; orders are rows, no message bus in v1 |
| Operations | A/B reimaging, canary rings, platform maintenance never terminates stateful workloads |

Full requirements (FR / SEC / PERF / OPS + traceability): doc 01.

## Start here (read order)

1. **[doc 06 — design basis](doc/decisions/06-design-basis.md)** — why these
   choices: isolation landscape, vulnerability history, the stack decision
2. **[doc 01 — requirements](doc/spec/01-requirements.md)** — the contract
3. **[doc 02 — architecture](doc/design/02-architecture.md)** — how the system
   works

Then: doc 07 (PoC scope) · doc 08 (design review + E-register) · doc 12 (software
architecture) · doc 14 (workload networking). Full set: docs 01–14 live under
`doc/{spec,design,decisions,operations}/` — doc numbers are stable IDs
(`doc 14 §3` = §3 of `doc/design/14-workload-networking.md`); new topics land as
sections of existing documents, a new corpus file needs an E-register row (doc 08 §E).

## Repository layout

| Tree | Content |
|---|---|
| `doc/` | the corpus — spec/ (normative) · design/ (informative) · decisions/ (why) · operations/ (procedures) |
| `src/` | Rust workspace — one binary, role-selected at startup; the host-agent is the PoC role |
| `guest/` | guest platform image build: pinned kernel + BusyBox + platform init |
| `harness/` | PoC test harness (doc 07 §7) |
| `infra/` | integration environment IaC (Outscale — doc 13) |
| `runs/` | recorded run artifacts + the filled §7.5 results table |
