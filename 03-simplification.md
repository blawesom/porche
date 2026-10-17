# 03 — Simplification Pass: What the New Build Does NOT Re-implement

Working document — v1, 2026-10-17. Analysis and recommendations applied to `02-architecture.md` (v3).

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

1. **No live migration, no pause-and-inspect, no block jobs** — gone with QMP. Not required
   (no snapshots; host drain = terminate + reschedule).
2. **Runtime volume attach** — CLH hotplug exists, but v1 simplifies: all volumes defined at
   workload creation, attach-at-boot only. Runtime attach is a phase-2 addition if a use case
   demands it.
3. **Guest-root can kill kata-agent** — same as the incumbent in-guest agent; the host never depends on
   guest cooperation for teardown (CRI stop → VMM kill → key zeroize).
4. **Image distribution** — OCI pull per workload adds latency; mitigated by a per-host
   pull-through cache. Must be measured against PERF-1/2 in P0.
5. **Density** — microVM per workload caps at ~100–500/host instead of the incumbent VM-per-process
   density; accepted (that is the isolation trade we chose in doc 06 §1).

---

## 5. Decision (recommended defaults)

- **Adopt the incumbent control-plane concepts** (CQS, pending calls, transactional
  placement) — the orchestration model is sound; the software architecture, languages and
  middleware are re-specified deliberately in a **later spec**, not inherited by habit.
- **Drop** the incumbent public-API surface: PORCHE exposes its own API (FR-40 — workloads, volumes, hosts, placement views), defined fresh rather than inherited.
- **Adopt** C-1..C-6 into `02-architecture.md` v3 (done).
