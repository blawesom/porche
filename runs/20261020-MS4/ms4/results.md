# MS-4 escape delta — 2026-10-20

run-id: `20261020-MS4` (`runs/20261020-MS4/ms4/`) · milestone: **MS-4 — escape delta (WS-E)**
host: node 1 · safety: §7.4 — every probe runs inside PORCHE-owned guest VMs only.
Contract (E-18): **delta verdicts only** — "the hardening changes the observable
outcome", never an absolute-security claim.

## Class 1 — virtio-net offload attack surface (PwnEscape class, SEC-7/I-7)

- toggle: hardened = offloads off at virtio-net + tap; baseline = explicitly ON
  (a run defect first made the baseline a no-op: v54 defaults offloads to false,
  so omission ≠ baseline — fixed by emitting explicit `true` flags; commit follows)
- probe: 8 × 60 pings at 65,000 bytes through the tap (in-guest), both sides;
  mechanism evidence via the tap's ethtool state captured per side
- **hardened**: TSO off — RTT avg 0.29–0.32 ms; stress completed; clean wipe
- **baseline**: TSO **on** — RTT avg 0.51–0.52 ms; stress completed; clean wipe
- **DELTA: present** — the offload segmentation path is demonstrably engaged on
  baseline only (mechanism verified), with a measurable packet-path difference
  (~1.7× RTT at 65 KB). No crash on either side — recorded honestly: this probe
  demonstrates the delta, not an escape.
- raw: `c1/` (consoles, tap-offloads, create/wipe records, verdict)

## Class 2 — CLH device-model surface (SEC-3/4)

- toggle: hardened = the agent's frozen device set (virtio-blk volume + ONE
  virtio-net on the workload's netns tap + file serial); baseline = hand-driven
  CLH VM with an expanded set — virtio-rng + a SECOND net iface on a second tap
  bridged outside the workload netns
- mechanism evidence: CLH's own live `vm.info` for the baseline shows
  `net ifaces: 2, rng: {src: /dev/urandom}` (committed: `c2/vminfo3.json`);
  the hardened side's config is emitted by `VmConfig::to_clh_json` (unit-pinned)
- probe: both guests booted the same pinned kernel+initramfs, enumerated their
  virtio PCI devices (4 functions visible to each guest kernel — blk/net
  counts differ per side per the config above), DEVICE-ENUM-DONE both sides,
  clean teardown
- **DELTA: present** — the device-model surface the VM exposes is
  configuration-divergent (rng present/absent; one vs two net ifaces; the
  baseline's second iface bridges OUTSIDE the workload's isolated netns — the
  exact legacy surface SEC-3/4's allow-list exists to deny)
- raw: `c2/` (consoles both sides, create/boot records, vm.info, verdict)

## Class 3 — 2026 page-cache/userns kernel cluster (SEC-11)

(both legs on the same boot — the sysctls flip at runtime; only modules_disabled
is one-way and it stayed 1 throughout. Hardened = sealed state
io_uring_disabled=2 / max_user_namespaces=0; baseline = stock 0 / 63183.)

- probes (unprivileged `nobody`, host-side): `io_uring_setup` syscall —
  hardened **DENIED (EPERM)** vs baseline **ALLOWED (fd 3)** (one probe
  artifact corrected and recorded: NULL params gives EFAULT regardless of
  policy; the corrected probe with a real struct shows the true baseline).
  `unshare -Ur` — hardened **DENIED (ENOSPC, the max_user_namespaces=0
  form)** vs baseline **ALLOWED (mapped root)**.
- **DELTA: present** — both 2026 attack primitives closed under the sealed
  sysctls, open under stock defaults. Node re-sealed and verified after the
  baseline leg.
- raw: `c3/` (per-leg sysctl states, probe outputs, corrected-probe record)

## Harness defects found and fixed during this class (record)

1. template `$id` injected into the ip6 field → invalid spec (both runs failed
   identically — caught as an invalid "delta present")
2. guest rootfs applet list is fixed (`ping`/`tail` not symlinked) — commands
   must call `busybox <applet>`; the first stress died silently in a pipe
3. stress pacing: `ping -c 200` at 1 pps outran the harness's poll window —
   paced to `-i 0.1 -W 1`
4. **baseline no-op** (the important one): CLH v54 defaults offloads OFF, so
   `offloads_off=false` without explicit flags was identical to hardened.
   Fixed in `porche-clh` config: baseline emits explicit `true` flags.
5. class-2 stack: newline-in-spec rejection; curl heredoc mangling;
   `cloud-hypervisor` not on PATH under non-login `sudo bash` (must use the
   absolute `/usr/local/bin/` path); missing `/api/v1` prefix on the CLH
   HTTP API; missing `vm.boot` after `vm.create`; two net ifaces cannot share
   one tap ("Resource busy") — the baseline's second iface needs its own tap.

## Verdict & signature

- **every verdict backed by paired machine artifacts** — yes (c1/c2/c3: consoles,
  tap-offload states, vm.info, per-leg sysctl states, probe outputs).
- **delta verdicts only** — yes (E-18 honored throughout; no absolute-security
  claims anywhere in this record).
- **user test: the operator walked the delta report** — yes (operator read
  results.md + the three class records, 2026-10-20, verdict "read and ok").

```
operator: benjamin ble
date: 2026-10-20
verdict: full user test passed
signature: benjamin ble / 2026-10-20
```
