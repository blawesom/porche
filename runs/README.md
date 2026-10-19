# runs/ — recorded run artifacts

One directory per recorded run: `runs/<YYYYMMDD-HHMMSS>-<name>/` (created by the
harness scripts). Contents:

- `fixtures.txt` — the §7.1 fixture record (node OS/kernel, CLH, guest digests,
  host-agent commit, hardening state)
- per-test CSVs (`perf1.csv`, `perf5.csv`, `perf8-*.csv`) and console logs as
  needed for the verdicts
- a copy of the filled `harness/results-template.md` (§7.5)

The PoC exits when the §7.5 table is green — or a §6 kill criterion fired
(doc 07 §7.5). The recorded PoC run happens on the FCOS stable OMI (§7.1
sequencing); the metal validation run reuses the same harness and owns the
authoritative PERF/security numbers (E-16).
