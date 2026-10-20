# MS-3 scale harness run — 2026-10-20

run-id: `20261020-MS3` (`runs/20261020-MS3/ms3/`) · milestone: **MS-3 — scale harness (WS-D)**
host: node 1 (`i-622da7f6`, Rocky 9.8 interim, dedicated tenancy `ded-53c8318b` — E-16:
integration environment; smoke verdicts, the gate is metal-scope where §7.2 says so)
harness: `harness/ms3-harness.sh` (commit `fc536dc`) — run as root on the node against
the live host-agent; every measurement below is machine-recorded in this run dir.

## PERF-1 — create→ready (n=100 after 10 warmups) — SMOKE

- method: T0 = wall clock immediately before the agent create; T1 = the
  `porche.ready` serial marker's arrival (the guest's own stamp, §7.2)
- **P50 = 1495 ms** (threshold < 1500 ms) — **PASS** (5 ms under — a genuinely
  tight pass; the KDF rule's luksFormat dominates and is deterministic, which
  the 16 ms P50→P99 spread reflects. On metal this row is re-cut, E-16)
- **P99 = 1511 ms** (threshold < 5000 ms) — **PASS**
- raw: `perf1.csv`, `perf1.sorted`, `perf1.txt`

## PERF-5 — density (20 parallel VMs, 60 s settle, 10 min @ 1 Hz) — SMOKE

- method: procfs VmRSS sampler over every live CLH process; max per-VM RSS
- **max per-VM RSS = 107,996 kB ≈ 105 MB** (threshold < 200 MB) — **PASS**
- all 20 VMs ran the full window with no anomalies (sampler rows: 12,000;
  `perf5.csv`), teardown clean after the window
- raw: `perf5.csv`, `perf5.txt`

## PERF-8 — 1,000-cycle create/wipe loop — THE GATE

- method: each cycle creates a 1-vCPU/128 MiB workload with a 64 MiB tier-E
  volume (LUKS2), converges, wipes; per-cycle teardown timing, MemAvailable
  floor + agent RSS at cycles 200/1000, wedge = failed cycle or > 60 s cycle
- **teardown P50 = 329 ms, P99 = 355 ms** (threshold P99 < 2000 ms) — **PASS**
- **wedge events = 0** (`wedge.txt` empty) — **PASS**
- **memory drift**: cycle 200 = 0.6 %, cycle 1000 = 0.85 % (threshold < 5 %) —
  **PASS**; host-agent RSS flat at 4176 kB across the whole loop (no leak)
- **GATE: PASS**
- raw: `perf8.csv`, `perf8.sorted`, `perf8.txt`, `wedge.txt`

## Wipe probe (E-6 semantics)

Each of the 1,000 gate cycles wiped a real LUKS2 volume; the wipe path's own
probe (mapper absent, keyfile entry removed, keyslots erased, region trimmed,
region clean) is embedded in `porche host-agent wipe` — a failing probe fails
the wipe, which the gate counts as a wedge event. Zero events ⇒ 1,000/1,000
wipe probes passed. (TRIM verified via the discard report only — the NAND
claim is metal-only, E-16.)

## §7.5 exit-table rows (doc 07)

| Criterion | Method | n | Threshold | Measured | Verdict |
|---|---|---|---|---|---|
| PERF-1 (smoke) | console marker | 100 | P50 < 1.5 s | P50 1495 ms / P99 1511 ms | **pass** |
| PERF-5 (smoke) | RSS sampler | 20 | < 200 MB each | max 105 MB | **pass** |
| PERF-8 loop (**gate**) | create/wipe | 1,000 | no wedge, < 5 % drift | 0 wedge, 0.85 % drift | **pass** |
| PERF-8 timing (smoke) | per-cycle | 1,000 | P99 < 2 s | P50 329 ms / P99 355 ms | **pass** |
| Wipe probe | markers + key slots | all (1,000) | no recoverable data | 0 probe failures | **pass** |

## Verdict & signature

- **every row backed by machine artifacts** — yes (CSVs, sorted series, harness
  output, wedge log — all committed in this run dir).
- **user test: the operator walked the results report** — yes (operator reviewed
  results.md + §7.5 + the raw CSVs, 2026-10-20, verbal verdict "ok good").

```
operator: benjamin ble
date: 2026-10-20
verdict: full user test passed
signature: benjamin ble / 2026-10-20
```
