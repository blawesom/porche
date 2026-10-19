# 03 — Simplification Pass: What the New Build Does NOT Re-implement

Working document — v4, 2026-10-18. Analysis and recommendations applied to
`02-architecture.md` (v3). **v2**: tradeoff §4.1 rewritten for the scope change (live
migration is a v1 goal — doc 08 E-3). **v3 (E-4)**: the Kata/containerd adoption
recommended here is **superseded** — see §6. C-5/C-6 stand; C-1..C-4 logic inverts.
**v4 (E-17 + corpus vocabulary sweep)**: §5 item 4 restated — OCI images enter only via
`ImportImage`; no per-workload OCI pull exists; phase prose switched to P-codes.

**Principle**: every incumbent-design component whose job is already done by an upstream, audited
Kata/containerd component gets **deleted, not adapted**. The measure of success is the size
of the *remaining* in-house security-critical surface (constraint CT-3).

---

## 1. What Kata/containerd gives for free (vs incumbent custom code)

Measured against the incumbent codebase (the incumbent's per-VM VMM driver (≈ 7.3 kLOC), its in-guest agent
protocol, its hypervisor agent, and its metadata service):

| Capability | Legacy (custom, in-house) | Kata/containerd (upstream) |
|---|---|---|
| One supervisor process per VM | a per-VM VMM driver process (one per VM, started by the hypervisor agent) | kata shim — one per sandbox, managed by containerd |
| VMM lifecycle, command line, control socket | `qemu.py`, `osc_to_qemu.py`, `qemu_monitor_protocol.py` | shim drives CLH over its API — no QEMU, no QMP |
| Guest agent channel | in-guest agent with a custom vsock protocol + **flawed handshake (F42)** | kata-agent ttrpc over vsock with **mutual TLS** — upstream, audited, keys bound to the sandbox |
| Console / stdio / exec | `console.py` + QMP chardevs + ZMQ plumbing | CRI streaming API (exec/attach/logs) |
| Guest bootstrap | in-guest agent baked into the machine image | kata-agent in guest image — standard, patched with runtime |
| Resource envelopes (CPU/RAM ceiling per sandbox) | absent (bare QEMU cmdline) | sandbox config + cgroups |
| Guest syscall filtering | absent | kata-agent seccomp profile |
| Volume attach/detach | QMP block ops (via the per-VM driver) | CRI/virtio-blk direct assignment |

**Observation**: the F42 finding (unauthenticated key adoption in the host↔in-guest-agent
handshake) is not a bug to patch in our new design — the entire protocol is deleted and
replaced by kata-agent's mutually authenticated channel. Fixing by deletion beats fixing by
redesign.

---

## 2. Core change recommendations

### C-1 — Delete the per-VM VMM driver (and the v2 "workload-supervisor" idea)

containerd + the kata shim already provide one process per sandbox with per-VM logs, crash
isolation, and lifecycle. The v2 architecture (`02-architecture.md` v2) kept a per-VM
supervisor driving containerd — that is a thin wrapper around something containerd already
does, i.e. pure glue (CT-3 violation). **Delete it.** the hypervisor agent talks CRI to containerd
directly; per-sandbox supervision is the shim's job.

*Cost*: we lose the ZMQ per-VM order protocol — replaced by CRI calls (standard, typed).
*Risk*: none identified; the pattern is how every Kubernetes node works.

### C-2 — Delete the incumbent in-guest agent and its crypto protocol entirely

kata-agent becomes the **only** guest agent. What the incumbent agent did:

| Incumbent agent feature | Replacement |
|---|---|
| Snapshot sync (app-consistent) | already dropped (not a requirement) |
| Guest metrics | kata-agent exec of a metrics endpoint, or scrape from host (`/proc` of VMM cgroup + virtio stats) — v1: host-side only |
| Metadata access | see C-3 |
| Custom handshake (F42 flaw) | **deleted with the protocol** — kata-agent mTLS |

No more key-adoption surface, no custom protocol parser on the host path (F42 also showed the
guest can feed the host malformed frames — upstream kata-agent parsing is fuzzed upstream).

### C-3 — Metadata: token at boot, proxy at the edge

Keep the metadata *feature* (FR-41), simplify the delivery: the KMS mints a single-use,
per-boot token; it is injected into the guest at creation (kata-agent config/env, not a
network position); the guest fetches its metadata from a per-host HTTPS proxy presenting
that token. This structurally satisfies SEC-21/F0.1 (no identity from network position, no
XFF trust) with ~one small component (metadata-proxy) instead of a guest-side agent.

### C-4 — The hypervisor agent shrinks to a thin host-agent

Remaining responsibilities, all host-scoped, no per-VM logic: CRI client (create/stop/inspect
sandboxes), KMS client (fetch wrapped DEK, open LUKS, zeroize on detach), boot attestation +
kernel-hardening checks, drain protocol, metrics reporter. This is the **only** new privileged
daemon on a PORCHE host, and it is small enough to audit line-by-line.

### C-5 — Drop qcow2: raw sparse files inside LUKS2

With snapshots/backing chains/streaming already gone (v2), qcow2's only remaining job is
thin provisioning — which a sparse raw file does natively. Making volumes **raw inside the
LUKS2 envelope**:

- removes `qemu-img` (QEMU code!) from the storage worker — the "no QEMU anywhere in the
  production path" rule (SEC-2) becomes complete and consistent;
- removes a parser (qcow2 metadata) from the storage path entirely;
- virtual-size/thin behavior preserved via sparse files (`fallocate` + hole punching on TRIM).

Cold clone-at-create stays (storage worker copies the decrypted view into a new envelope).

### C-6 — Networking via CNI; VPC model untouched

Per-host netns/veth/bridge moves to standard CNI plugins driven by host-agent. The VPC /
security-group / subnet model is database-level and adopted as-is (v1) — no change
to the API surface or the firewall service VMs.

---

## 3. What remains PORCHE-specific (the honest inventory)

After C-1..C-6, the complete list of new/in-house security-relevant code:

| Component | LOC estimate | Trust level | Audit burden |
|---|---|---|---|
| host-agent (CRI/KMS/keyring/attest/drain) | small | privileged | full audit |
| storage-worker (LUKS2 create/clone) | small | privileged (key-aware) | full audit |
| KMS (wrap/unwrap, tokens) | medium | control plane | full audit + key ceremony |
| metadata-proxy | tiny | unprivileged | review |
| control plane (concepts adopted, per doc 02 §1) | to be specified | control plane | later software-architecture spec |
| kata shim, kata-agent, CLH, containerd, kernel | upstream | data plane | upstream + our CI attack suite (SEC-40) |

Compare with the v2 architecture: a VMM-driver replacement + custom in-guest agent + custom handshake +
qemu-img storage ops all gone. **The only code standing between an untrusted guest and the
host is upstream, fuzzed, and disposable-per-workload.**

---

## 4. Tradeoffs accepted

1. **No QMP block jobs, no pause-and-inspect** — gone with QMP, permanently. **Live
   migration is in scope as a P1 stretch** (E-3, E-15, doc 08 — checkpoint migration is
   the P1 must), but by a different mechanism
   (VMM-level transfer, FR-34) — never by re-introducing QMP machinery. Consequence: the
   C-1/C-2 deletions stand unconditionally for the *ephemeral* workload class; for
   *stateful* workloads, the v1 runtime selection (A-9 gate, doc 02 §4.5) decides whether
   Kata is used at all. If stateful workloads run on direct CLH, the per-VMM lifecycle
   glue that C-1 deleted returns in reduced form — the honest CT-3 cost of live migration.
   Checkpoint migration (drain = seal + restore) remains the fallback primitive for all tiers.
2. **Runtime volume attach** — CLH hotplug exists, but v1 simplifies: all volumes defined at
   workload creation, attach-at-boot only. Runtime attach is a P2 addition if a use case
   demands it.
3. **Guest-root can kill kata-agent** — same as the incumbent in-guest agent; the host never depends on
   guest cooperation for teardown (CRI stop → VMM kill → key zeroize).
4. **Image distribution** — platform-rootfs distribution per workload adds latency;
   mitigated by a per-host pull-through cache (base catalog pre-seeded on the host
   image — doc 02 §8). OCI images now enter only via `ImportImage` (E-17) — no
   per-workload OCI pull exists. Must be measured against PERF-1/2 in P0.
5. **Density** — microVM per workload caps at ~100–500/host instead of the incumbent VM-per-process
   density; accepted (that is the isolation trade we chose in doc 06 §1).

---

## 5. Decision (recommended defaults)

- **Adopt the incumbent control-plane concepts** (CQS, pending calls, transactional
  placement) — the orchestration model is sound; the software architecture, languages and
  middleware are re-specified deliberately in a **later spec**, not inherited by habit.
- **Drop** the incumbent public-API surface: PORCHE exposes its own API (FR-40 — workloads, volumes, hosts, placement views), defined fresh rather than inherited.
- **Adopt** C-1..C-6 into `02-architecture.md` v3 (done).

---

## 6. Supersession — E-4: the simplification inverts (2026-10-18)

The runtime revalidation (doc 08 A-9, E-4) **supersedes this document's central
recommendation**: Kata and containerd are dropped; Cloud Hypervisor is orchestrated
directly by the host-agent. §1–§5 stand as the historical analysis of the v2→v3
architecture; the principle survives, the conclusion inverts:

- **C-1** (delete the per-VM VMM driver) — *inverted*: supervision returns to the
  host-agent as a **bounded lifecycle role** over CLH's API. No wrapper *process*
  reappears; the glue is API driving, not protocol parsing.
- **C-2** (kata-agent replaces the custom agent) — *superseded by something stronger*:
  **no guest agent at all**. F42's class is eliminated by having no host↔guest protocol
  (SEC-8 v3), not by replacing one channel with another.
- **C-3** (metadata token) — stands unchanged.
- **C-4** (host-agent scope) — *extended*: it gains the per-VM lifecycle role (E-4).
- **C-5** (raw sparse in LUKS2) — stands unchanged.
- **C-6** (CNI networking) — *superseded (E-4/doc 14)*: native netns/veth/bridge in the
  host-agent; CNI existed to standardize container-runtime integration that no longer
  exists. The host-agent's netlink module replaces the plugin ecosystem.

**The deeper simplification (doc 02 v5)**: this document asked "which incumbent glue
does upstream Kata replace?" E-4 asks the better question — "which *intermediary layers*
can be deleted entirely?" Answer: containerd, the kata shim, and the guest agent. The
data plane is **host-agent + CLH + kernel**. The CT-3 measure improves: the in-house
surface is one audited binary; the upstream surface is one VMM.

**What this costs** (recorded honestly, doc 08 A-9): exec into a running workload is not
a v1 feature; app-consistent seals are deferred; the guest rootfs image is built by our
pipeline (doc 08 action 11).
