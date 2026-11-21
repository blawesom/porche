# MS-5 recorded run — 2026-11-20

run-id: `20261120-node-usw1` (`runs/20261120-node-usw1/ms5/`) · milestone: **MS-5 — recorded run (WS-F)**
verdict classes (v12): §7.5.1 rows below are **software gates** (integration
verdicts); every row carrying a hardware-bound unit (ms, MB, ×) is
**recorded here** and **gated on the hardware run** (E-16).

host: node — `i-520a2bce` (`tinav5.c8r16p1`, gen-5/Skylake, nested virt,
dedicated group `porche-usw1-g5-1b` (ded-d326ad5a) @ us-west-1b, **Flatcar
4881.0.0+nightly-20261111-2100** kernel 6.18.58-flatcar, OMI `ami-67af010c`).
**Host-OS substitution, recorded (E-5 fallback + E-31.1)**: the MS-5 base was
specified as the FCOS stable OMI; FCOS cannot consume Outscale user-data or
keypair injection — Outscale's IMDS is EC2-style, Ignition's Outscale provider
exists only from 2.28.0 and FCOS stable 45 still ships 2.27.0 (verified in its
meta.json). Flatcar is the documented fallback; its Outscale variant carries
the patched Ignition/Afterburn, verified here (Ignition applied the node's
key via user-data). REVERT TRIGGER: FCOS stable ships Ignition ≥ 2.28 →
rebuild with `infra/images/build-fcos-omi.sh` and re-run this record.
harness: `ms5-suite.sh` + `ms5-{offload,device,sysctl}-delta.sh` (this run
dir; the `harness/ms3-harness.sh` + `harness/ms4-*-delta.sh` adaptations:
agent/CLH at `/var/lib/porche/bin` — read-only `/usr/local` on Flatcar — and
the python3 analyses ported to shell) — run as root on the node against the
live host-agent (commit `52911db`); every measurement below is
machine-recorded in this run dir. Fixtures: `fixtures.txt` (§7.1).

## PERF-1 — create→ready (n=100 after 10 warmups) — RECORDED (hardware-gated)

- method: T0 = wall clock immediately before the agent create; T1 = the
  `porche.ready` serial marker's arrival (the guest's own stamp, §7.2)
- **P50 = 1449 ms** (threshold < 1500 ms as a smoke signal here; the gate is
  the hardware run, E-16) — **PASS**
- **P99 = 1524 ms** — **PASS**
- raw: `perf1.csv`, `perf1.sorted`, `perf1.txt`

## PERF-5 — density (20 parallel VMs, 60 s settle, 10 min @ 1 Hz) — RECORDED

- method: procfs VmRSS sampler over every live CLH process; max per-VM RSS
- **max per-VM RSS = 105,020 kB ≈ 102.5 MB** (smoke threshold < 200 MB) —
  **PASS** (MS-3 reference: 105 MB)
- raw: `perf5.csv`, `perf5.txt`

## PERF-8 — 1,000-cycle create/wipe loop — THE SOFTWARE GATE

- method: each cycle creates a 1-vCPU/128 MiB workload with a 64 MiB tier-E
  volume (LUKS2), converges, wipes; per-cycle teardown timing,
  MemAvailable floor + agent RSS at cycles 200/1000, wedge = failed cycle
  or > 60 s cycle
- **wedge events = 0** (threshold 0) — **PASS**
- **memory drift**: cycle 200 = 0.35 %, cycle 1000 = 0.29 % (threshold < 5 %)
  — **PASS**; host-agent RSS flat at 4,488 kB across the whole loop (no leak)
- teardown timing (RECORDED, hardware-gated): **P50 = 368 ms, P99 = 394 ms**
  (smoke threshold P99 < 2000 ms)
- **GATE: PASS**
- raw: `perf8.csv`, `perf8.sorted`, `perf8.txt`, `wedge.txt`

## Wipe probe (E-6 semantics)

Each wiped volume's probe (mapper absent, keyfile entry removed, keyslots
erased, region trimmed, region clean) is embedded in `porche host-agent wipe`
— a failing probe fails the wipe, which the loop counts as a wedge event.
**1,000 / 1,000** probes passed. (TRIM verified via the discard report only —
the NAND claim is metal-only, E-16.)

## Escape delta (3 classes, §7.3 toggles, E-18) — delta verdicts only

| Class | Hardened | Baseline | Delta |
|---|---|---|---|
| 1 — virtio-net offload (SEC-7/I-7) | stress done, no panic, tap TSO **off** (`c1/hardened.tap-offloads`) | stress done, no panic, tap TSO **on** (`c1/baseline.tap-offloads`) | **present (mechanism)** — outcome identical both sides |
| 2 — CLH device model (SEC-3/4) | agent's frozen set (VmConfig, unit-pinned) | live `vm.info`: rng + 2 net taps (`c2/baseline.vminfo.json`) | **present** |
| 3 — kernel sysctl cluster (SEC-11) | io_uring EPERM (gate-blocked), userns DENIED | io_uring EFAULT (gate open), userns ALLOWED (mapped root) | **present** |

raw: `../ms5-delta/c1/`, `../ms5-delta/c2/`, `../ms5-delta/c3/`. Control claims
only (E-16/E-18); security evidence is metal-only.

**MS-4 c1 correction (found by this re-run)**: the MS-4 class-1 analysis
compared dicts that included the leg `label`, so `h == b` could never be true
— its "outcome differs" verdict line was a comparison bug. The MS-4 outcomes
were identical then too (as here); the class-1 delta evidence was and remains
the **mechanism** (tap offload state), which this run re-demonstrates.

## Kernel-hardening verification (SEC-11 subset)

- sysctl assert at boot: **PASS** (`io_uring_disabled=2`,
  `max_user_namespaces=0`, `core_pattern` locked, `modules_disabled=1`);
  re-asserted after the class-3 baseline leg (node re-sealed, `fixtures.txt`)
- nested virt verified on gen-5/Skylake (first time): vmx 16/16 CPUs,
  `/dev/kvm`, `kvm_intel nested=Y` (see `../00-run-record.txt`, coverage
  test)

## §7.5 exit-table rows (doc 07)

| Criterion | Method | n | Threshold | Measured | Verdict |
|---|---|---|---|---|---|
| PERF-8 loop (gate) | create/wipe | 1,000 | no wedge, < 5 % drift | 0 wedge, 0.29 % drift | **pass** |
| Wipe probe | markers + key slots | all | no recoverable data | 0 probe failures | **pass** |
| Escape delta | §7.3 toggles | 3 | delta present | 3/3 classes: mechanism/config delta present | **pass** |
| Sysctl script | boot assert | every boot | all pass | all 4 items ok (setup + re-assert) | **pass** |
| PERF-1 (recorded) | console marker | 100 | P50 < 1.5 s | P50 1449 ms / P99 1524 ms | pending — hardware run |
| PERF-5 (recorded) | RSS sampler | 20 | < 200 MB each | max 102.5 MB | pending — hardware run |
| PERF-8 timing (recorded) | per-cycle | 1,000 | P99 < 2 s | P50 368 ms / P99 394 ms | pending — hardware run |

## Verdict & signature

- **every row backed by machine artifacts** — yes (CSVs, sorted series,
  harness output, wedge log, delta subdirs, fixtures — all committed in this
  run dir)
- **user test: the operator walked the record** — pending (results.md + §7.5
  + raw artifacts; the signature below is the operator's to fill)

```
operator: ____
date: ____
verdict: ____
signature: ____
```
