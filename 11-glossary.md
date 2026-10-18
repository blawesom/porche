# 11 — Glossary and Controlled-Language Rules

Working document — v3, 2026-10-18. One word, one meaning — the lightweight controlled-English
contract (inspired by ASD-STE100, without its full cost; see doc 08 discussion). Applies to
the whole corpus from this version on. Legacy uses that predate this glossary are superseded
by the definitions below. **v2**: migration terms redefined — live migration is in scope
as a v1 goal (E-3, doc 08). **v3 (E-8)**: control plane split into planes.

---

## 1. Requirement keywords (RFC 2119)

In doc 01 (and wherever requirements are restated):

- **MUST** — normative; violation is a defect. *Default for every FR/SEC/PERF/OPS statement
  unless marked otherwise.*
- **SHOULD** — strong default; deviation requires a recorded justification (audit log /
  challenges doc).
- **MAY** — truly optional behavior.

## 2. Terms

| Term | Meaning | Do not use it for |
|---|---|---|
| **workload** | The tenant-facing unit of execution (API object, doc 09). The canonical word for "what a user creates and runs". | — (it is the default term) |
| **sandbox** | The execution unit of exactly one workload (one microVM + its VMM). Use in runtime context (per-sandbox cgroup). | the API object (say *workload*); the technology (say *microVM*) |
| **microVM** | The virtual machine as a technology object (guest kernel, VMM, device model). Use when discussing the VM layer itself. | the API object; the runtime unit |
| **VM** | Reserved for the incumbent design (doc 06) and generic virtualization discussion. Never a PORCHE object. | any PORCHE concept |
| **volume** | The storage object a workload attaches (API object, doc 09). | the ciphertext file (say *envelope*) |
| **envelope** | The LUKS2 container file that holds a volume's ciphertext on the storage pool. | the decrypted device (say *mapper device*) |
| **mapper device** | The `/dev/mapper/<volid>` decrypted block device handed to the runtime. | the volume; the envelope |
| **checkpoint** | A sealed, restorable image of a volume (API object, doc 09). | the operation (say *seal*) |
| **seal** | The operation that produces a checkpoint: quiesce → ship epoch → CRC-verify → record. Sealed = completed successfully. | — |
| **epoch** | The dirty-block tracking window between two seals (dm-era era). | — |
| **sync** | Not used in prose (ambiguous). The API call is named `SyncVolume` (FR-33); in prose the operation is a *seal* and the result a *checkpoint*. | — |
| **wipe** | The teardown operation that makes residual *data* unrecoverable. Storage regions: **crypto-erase** — zeroize the DEK, then TRIM/discard; never a physical overwrite (E-6: NAND wear). Memory: unmap/zero pages (SEC-14). | key material (say *zeroize*); physical overwrite (rejected — E-6) |
| **zeroize** | The destruction of *key material* (DEKs, kernel keyring entries) — includes crypto-erase where the mechanism allows. | data regions (say *wipe*) |
| **erase** | Not used (superseded by *wipe* / *zeroize*). | — |
| **attest** | Machine-verified evidence produced at boot (signature chain, sysctl checks) — verified by the platform, not self-declared. | host self-declarations (say *report*) |
| **report** | A host self-declaration (e.g., "wipe complete"). Reports are recorded; verification is separate (doc 08 A-3). | verified evidence (say *attest*) |
| **drain** | The host state/transition that removes workloads and excludes new placement (doc 05, doc 10). | — |
| **reimage** | Replace the host artifact by writing the spare A/B slot and booting into it (doc 10). | any in-place update (never used — image is immutable) |
| **migration** | Move a workload to another host within the owner's allocation. Two mechanisms: *checkpoint migration* and *live migration*. | — |
| **checkpoint migration** | Stop a workload + place it anew + start it (checkpoint-carrying for tiers R/S). The default mechanism, all tiers. | live migration |
| **live migration** | VMM-level transfer of a running workload; the guest sees only the cutover pause. Stateful workloads — P1 stretch (E-3, E-15; checkpoint migration is the P1 must). | checkpoint migration |
| **cutover** | The user-visible downtime window of one migration (for live migration: the pause at switchover). | — |
| **ingress rule** | The API object that allows inbound access to one workload port from a source range (doc 09 — IngressRule). Created and revoked through the control API only — never from the guest. | guest-initiated port opening, firewall edits from inside the workload |
| **egress rule** | The API object that allows outbound internet access to one destination — FQDN or CIDR (doc 09 — EgressRule). Enforced by the egress proxy, DNS included. | intra-VPC rules (say *security group*) |
| **security group** | The API object holding east-west rules within a VPC (doc 09 — SecurityGroup). Compiled to netns nftables. | internet-facing rules (say *ingress rule* / *egress rule*) |
| **allocation** | The binding of one host to one user (SEC-16; doc 05). | — |
| **tier** | Volume durability: E / R / S (doc 04). Nothing else is a tier. | performance level, storage location |
| **class** | Storage location/cost of a remote copy: hot / cold (doc 04 §2.2). | durability (say *tier*) |
| **seal** (noun) | Not used as a noun — the object is a *checkpoint*. | — |
| **DEK / KEK / KMS** | Data key (per volume) / key-encryption key (KMS-held) / the key service. | — |
| **host** | A bare metal machine (API object, doc 09). Always "host", never "server"/"node". | — |
| **HostGroup** | A failure domain (rack / power / maintenance; doc 05 §1.2). | — |
| **intake sandbox** | The restricted first-run sandbox for unvetted images (doc 02 §3.2). | — |
| **admin control plane** | The operator-only services: fleet lifecycle, version registry + deploy gates, image pipeline, security monitoring, user administration (E-8). | tenant-facing services (say *tenant control plane*) |
| **tenant control plane** | The tenant-facing services: workload/volume/network-rule (ingress/egress/security-group) orchestration, placement views, quotas, metering (E-8). | operator capability; fleet-global views |
| **data plane** | The workload execution path: hosts, microVMs, per-host proxies, storage shards. Tenant traffic never transits a control plane (I-14). | control-plane traffic (orders, metrics, audit) |
| **incumbent (design)** | The reference platform's design (doc 06 §4) — concepts only, no software. | anything PORCHE runs |

## 3. Writing rules (procedure documents, esp. doc 10)

1. One instruction per numbered step; imperative mood; verb from the approved list below.
2. Each step states its **end state**; the next step's precondition is the previous end state.
3. Conditions precede actions ("If attestation fails, the bootloader falls back." not
   "The bootloader falls back if attestation fails.").
4. Sentence ≤ 20 words in procedural text; no semicolon-chained instructions.
5. No synonyms: the glossary term is the only term (e.g., never "cleanup", "scrub",
   "clear" for *wipe*).

**Approved operator-action verbs** (doc 10): attach, boot, check, detach, drain, halt,
mark, migrate, pause, place, pull, quarantine, record, release, reimage, restart, resume,
seal, set, start, stop, verify, wipe, zeroize. Domain verbs (cryptsetup open, CRC-verify)
stay as technical names.

## 4. Comparison rule (alternatives are judged at their best)

When a document compares PORCHE's choices against an alternative technology (QEMU, Firecracker,
gVisor, Kata variants, …), the comparison MUST use the alternative's **latest LTS release at
decision time** (or latest stable if no LTS exists) — never EOL, historical, or misconfigured
deployments. Incidents on EOL/misconfigured lines are cited as *process* lessons (version
governance, operational risk), not as technical disqualification of the current product.
Re-evaluation triggers: each new LTS of a rejected alternative. (Origin: SEC-2, QEMU.)

---

## 5. Retrofit status

- Doc 11: RFC 2119 keywords added to conventions (§0) — requirement statements are MUST by
  default.
- Doc 11 v2 (2026-10-18): *migration* redefined into *checkpoint migration* / *live
  migration* per scope change E-3; earlier "never live migration" uses are superseded.
- Doc 10: procedural sections restructured to the rules above.
- Doc 01 SEC-2 / 02 §1+§4.4 / 06 §1+§2 / 08 A-6: QEMU comparisons restated against the
  latest LTS at decision time (§4 rule); EOL incidents re-cast as process lessons.
- Older uses of deprecated terms (sync, erase, VM-as-PORCHE-object) remain in early-version
  history for traceability; new edits use the glossary terms.
