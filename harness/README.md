# harness/ — the PoC test harness (doc 07 §7)

One script per §7.2 method: same observation, same sample size, same threshold.
Everything writes into a run directory (`RUN_DIR`, default `runs/<timestamp>-<name>`)
and the filled §7.5 table is committed with the run (doc 07 §7.5).

| Script | §7.2 row | n | Threshold |
|---|---|---|---|
| `boot-time.sh` | PERF-1 create → running | 100 (10 warmups) | P50 < 1.5 s (**smoke**, E-16) |
| `density.sh` | PERF-5 density | 20 parallel | < 200 MB RSS each, 10 min anomaly-free |
| `loop.sh` | PERF-8 loop + timing + drift + wedge | 1,000 | no wedge, < 5 % drift (**gate**); P99 < 2 s (smoke) |
| `sysctl-assert.sh` | kernel hardening | every boot | all four SEC-11 items |
| `lib/fixtures.sh` | §7.1 fixture pinning | every run | recorded before anything else |

## Prerequisites

- `porche` built (`src/`), on `PATH` or via `PORCHE_BIN`
- node 1 up (burst — `make up` in `infra/integration/`), CLH v54 installed,
  `brporche` converged by the first `create`
- guest image installed under `$DATA_DIR/images/<digest>/` (`guest/README.md`)
- root (netns, cryptsetup, /proc scans) — the harness refuses to run otherwise

## Environment knobs

```sh
PORCHE_BIN=./porche          # the binary
DATA_DIR=/var/lib/porche     # agent data dir (must match the agent's)
STORE=/var/lib/porche/desired-state.sqlite3
SPEC=$PWD/harness/spec-loop.json   # base workload spec (loop.sh rewrites id/cmd)
```

T0 note (WS-C refinement): PERF-1 T0 is stamped by the harness immediately before
invoking `create`; an in-agent stamp (`created t0=<ns>` line) replaces it during
WS-C — the method (§7.2) stays the same.

## Escape-delta runs (§7.3) — separate, after WS-C

The baseline toggles are **config flips**, not harness scripts: offloads on/off
(`VmConfig.offloads_off`), device set (frozen vs expanded), sysctls (stock vs
`sysctl-assert.sh`). Each exercise runs twice — control first (§7.4 safety rules,
`infra/integration/BOUNDARIES.md`): inside our node VMs' guests only, abort on
anomaly.
