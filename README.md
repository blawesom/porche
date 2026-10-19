# PORCHE — Portable Cloud Orchestrator

In-house cloud orchestrator: **bare metal**, first-in-class performance, security by design.

**Base**: a **new orchestrator**, built from scratch. Its design starts from the
incumbent production platform's design (reference only, doc 06 §4) and improves it —
**Cloud Hypervisor directly orchestrated as the single VM runtime (E-4: no Kata, no
containerd)**, default block encryption, tiered storage
durability, dedicated hosts. Software architecture and languages: later spec. **This corpus is self-contained**: every fact the design
documents rely on (research findings, vulnerability history, the incumbent base design, the
stack decision) is recorded in doc 06. No external repository is needed to read or extend
the design.

## Philosophy

- **Bare metal first**: we own the hardware; no legacy virtualization layer between the
  orchestrator and the metal.
- **Security by design, not by patching**: every security requirement is traceable to a
  real-world lesson (2026 incumbent-platform isolation assessment, 2021–2026 CVE corpus —
  doc 06).
- **Untrusted workloads by default**: the reference adversary is the workload itself
  (compromised AI agent, malicious build).
- **Measurable performance**: quantified SLOs, validated by a test plan — not adjectives.

## Structure and growth rules

The corpus is four thematic directories plus this index:

| Directory | Role | Documents |
|---|---|---|
| `spec/` | the contract (normative) | 01 requirements · 09 API + tenant surface map · 11 glossary |
| `design/` | how it works (informative) | 02 architecture · 04 storage tiers · 05 dedicated hosts · 12 software · 14 networking |
| `decisions/` | why — evidence + chronology | 03 simplification (merged into doc 08 §F — stub) · 06 design basis · 08 review + E-register |
| `operations/` | build & run (procedures) | 07 PoC · 10 upgrade & migration · 13 integration environment |

Rules:

- **Doc numbers are stable IDs.** The corpus cites itself as `doc NN §X` (255+ references);
  the number resolves to the document wherever it lives — `doc 14 §3` means the section
  `§3` of doc 14, in `design/14-workload-networking.md`.
- **New topic → a new section inside an existing document.** A new corpus file is itself a
  scope change and requires a row in the E-register (doc 08 §E); the directory set is
  frozen.
- `archive/` holds superseded originals; nothing normative lives there.

## Document index

| # | Document | Role |
|---|----------|------|
| 01 | [requirements](spec/01-requirements.md) | Requirements specification (FR / SEC / PERF / OPS) + traceability |
| 02 | [architecture](design/02-architecture.md) | Core architecture (new build, incumbent design as reference): layers, improvement register |
| 03 | [simplification](decisions/03-simplification.md) | **Merged into doc 08 §F** — pointer stub (`doc 03 §N` = `doc 08 §F.N`) |
| 04 | [storage tiers](design/04-storage-tiers.md) | Local NVMe write-back + async epoch sync; checkpoint ("snapshot") semantics, consistency, economics |
| 05 | [dedicated hosts](design/05-dedicated-hosts.md) | One user per bare metal host; placement views; attract/repulse resiliency model |
| 06 | [design basis](decisions/06-design-basis.md) | Self-contained research record: isolation landscape, CVE history, assessment findings (F-register), incumbent design, stack decision |
| 07 | [PoC scope](operations/07-poc.md) | First proof of concept: minimal scope, explicit deferrals, kill criteria |
| 08 | [challenges](decisions/08-challenges.md) | Design review: challenged decisions, contradictions, objective gaps, action list + scope-change register (E) + §F superseded rationale (was doc 03) |
| 09 | [API](spec/09-api.md) | Object catalog + frozen tenant surface map: Host (allocate/release), Workload, Volume, Checkpoint, network rules, PublicIpv4, Image, … |
| 10 | [upgrade & migration](operations/10-upgrade-migration.md) | Operator runbook: host image/A-B reimage, canary rings, workload migration mechanics, failure handling |
| 11 | [glossary](spec/11-glossary.md) | Controlled-language contract: one word one meaning, RFC 2119 keywords, procedure writing rules |
| 12 | [software architecture](design/12-software-architecture.md) | Control-plane and host-side software: state authority, order flow (no bus in v1), three services, Rust/tonic/sqlx, Postgres HA |
| 13 | [integration environment](operations/13-integration-environment.md) | The "proper infra" simulated on Outscale cloud: topology, fidelity contract, provisioning recipe, operations |
| 14 | [workload networking](design/14-workload-networking.md) | Workload data path: IPv6-first (routed /128, ULA per VPC), netns/isolated-bridge, egress/ingress proxies, enforcement rules, threat-model walkthrough |

Read order for a new session: **06 → 01 → 02**, then 03/04/05/07/08 as needed; the
software architecture is **12** (after 02).

## Archive

`archive/` holds the French-language working originals of the research phase
(isolation comparison, vulnerability history, recommended stack) — superseded by the
English digests in doc 06, kept for historical reference only.
