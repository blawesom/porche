# runs/ — recorded run artifacts

One directory per recorded run: `runs/<YYYYMMDD-HHMMSS>-<name>/` (created by the
harness scripts). Contents:

- `fixtures.txt` — the §7.1 fixture record (node OS/kernel, CLH, guest digests,
  host-agent commit, hardening state)
- per-test CSVs (`perf1.csv`, `perf5.csv`, `perf8-*.csv`) and console logs as
  needed for the verdicts
- a copy of the filled `harness/results-template.md` (§7.5)

**Verdict classes (v12 gate split, doc 07 §7.5)**: every `results.md` header
carries the run's verdict class — **`software gate`** (function + software
perf: integration verdicts) or **`hardware gate — recorded here`** (latency,
throughput, RSS absolutes — recorded in integration, gated on the hardware
run). A recorded row never carries a pass/fail verdict outside its class.

The PoC's software suite (§7.5.1, MS-6) is green when its gates pass here or a
§6 kill criterion fired (doc 07 §7.5). The recorded PoC run happens on the
FCOS stable OMI (§7.1 sequencing); the hardware validation run reuses the same
harness and owns the authoritative PERF/security numbers (E-16).
