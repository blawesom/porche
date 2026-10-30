# MS-5 recorded run — <DATE>

run-id: `<RUN-ID>` (`runs/<RUN-ID>/ms5/`) · milestone: **MS-5 — recorded FCOS run (WS-F)**
verdict classes (v12): §7.5.1 rows below are **software gates** (integration
verdicts); every row carrying a hardware-bound unit (ms, MB, ×) is
**recorded here** and **gated on the hardware run** (E-16).

host: node 1 — `<VM-ID>` (`<VM-TYPE>`, nested virt, dedicated group `<DED-ID>`,
FCOS `<STREAM/VERSION>`, kernel `<UNAME-R>`) — the E-5 contract base (FCOS
stable OMI `ami-____`), not the Rocky interim of MS-3/MS-4.
harness: `harness/ms3-harness.sh` + `harness/ms4-{offload,device,sysctl}-delta.sh`
+ `harness/sysctl-assert.sh` (commit `<COMMIT>`) — run as root on the node
against the live host-agent; every measurement below is machine-recorded in
this run dir. Fixtures: `fixtures.txt` (§7.1).

## PERF-1 — create→ready (n=100 after 10 warmups) — RECORDED (hardware-gated)

- method: T0 = wall clock immediately before the agent create; T1 = the
  `porche.ready` serial marker's arrival (the guest's own stamp, §7.2)
- **P50 = ____ ms** (threshold < 1500 ms as a smoke signal here; the gate is
  the hardware run, E-16) — **____**
- **P99 = ____ ms** — **____**
- raw: `perf1.csv`, `perf1.sorted`, `perf1.txt`

## PERF-5 — density (20 parallel VMs, 60 s settle, 10 min @ 1 Hz) — RECORDED

- method: procfs VmRSS sampler over every live CLH process; max per-VM RSS
- **max per-VM RSS = ____ kB ≈ ____ MB** (smoke threshold < 200 MB) — **____**
- raw: `perf5.csv`, `perf5.txt`

## PERF-8 — 1,000-cycle create/wipe loop — THE SOFTWARE GATE

- method: each cycle creates a 1-vCPU/128 MiB workload with a 64 MiB tier-E
  volume (LUKS2), converges, wipes; per-cycle teardown timing,
  MemAvailable floor + agent RSS at cycles 200/1000, wedge = failed cycle
  or > 60 s cycle
- **wedge events = ____** (threshold 0) — **____**
- **memory drift**: cycle 200 = ____ %, cycle 1000 = ____ % (threshold < 5 %) — **____**
- teardown timing (RECORDED, hardware-gated): **P50 = ____ ms, P99 = ____ ms**
  (smoke threshold P99 < 2000 ms)
- **GATE: ____**
- raw: `perf8.csv`, `perf8.sorted`, `perf8.txt`, `wedge.txt`

## Wipe probe (E-6 semantics)

Each wiped volume's probe (mapper absent, keyfile entry removed, keyslots
erased, region trimmed, region clean) is embedded in `porche host-agent wipe`
— a failing probe fails the wipe, which the loop counts as a wedge event.
____ / ____ probes passed. (TRIM verified via the discard report only — the
NAND claim is metal-only, E-16.)

## Escape delta (3 classes, §7.3 toggles, E-18) — delta verdicts only

| Class | Hardened | Baseline | Delta |
|---|---|---|---|
| 1 — virtio-net offload (SEC-7/I-7) | ____ | ____ | **____** |
| 2 — CLH device model (SEC-3/4) | ____ | ____ | **____** |
| 3 — kernel sysctl cluster (SEC-11) | ____ | ____ | **____** |

raw: `ms4-offload/`, `ms4-device/`, `ms4-sysctl/` subdirs. Control claims
only (E-16/E-18); security evidence is metal-only.

## Kernel-hardening verification (SEC-11 subset)

- sysctl assert at boot: **____** (`io_uring_disabled=2`,
  `max_user_namespaces=0`, `core_pattern` locked, `modules_disabled=1`)

## §7.5 exit-table rows (doc 07)

| Criterion | Method | n | Threshold | Measured | Verdict |
|---|---|---|---|---|---|
| PERF-8 loop (gate) | create/wipe | 1,000 | no wedge, < 5 % drift | ____ | **____** |
| Wipe probe | markers + key slots | all | no recoverable data | ____ | **____** |
| Escape delta | §7.3 toggles | 3 | delta present | ____ | **____** |
| Sysctl script | boot assert | every boot | all pass | ____ | **____** |
| PERF-1 (recorded) | console marker | 100 | P50 < 1.5 s | ____ | pending — hardware run |
| PERF-5 (recorded) | RSS sampler | 20 | < 200 MB each | ____ | pending — hardware run |
| PERF-8 timing (recorded) | per-cycle | 1,000 | P99 < 2 s | ____ | pending — hardware run |

## Verdict & signature

- **every row backed by machine artifacts** — ____ (CSVs, sorted series,
  harness output, wedge log, delta subdirs — all committed in this run dir)
- **user test: the operator walked the record** — ____ (results.md + §7.5 +
  raw artifacts, `<DATE>`, verbal verdict "____")

```
operator: ____
date: ____
verdict: ____
signature: ____ / ____
```
