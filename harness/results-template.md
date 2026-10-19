# Results record — doc 07 §7.5 (fill per run, commit with the run)

Run: `<date>-<name>` · Directory: `runs/<date>-<name>/` · Fixtures: `fixtures.txt` (§7.1)

| Criterion | Method | n | Threshold | Measured | Verdict |
|---|---|---|---|---|---|
| PERF-1 (smoke) | console marker | 100 | P50 < 1.5 s | | |
| PERF-5 (smoke) | RSS sampler | 20 | < 200 MB each | | |
| PERF-8 loop (**gate**) | create/wipe | 1,000 | no wedge, < 5 % drift | | |
| PERF-8 timing (smoke) | per-cycle | 1,000 | P99 < 2 s | | |
| Wipe probe | markers + key slots | all | no recoverable data | | |
| Escape delta | §7.3 toggles | 3 | delta present | | |
| Sysctl script | boot assert | every boot | all pass | | |

## Notes

- PERF rows are **smoke signals** in the integration environment (E-16) — the
  authoritative numbers are the metal validation run's, same harness.
- Escape-delta verdicts are **control claims only** (E-16/E-18) — security evidence
  is metal-only.
- Wedge/liveness events and residue findings, if any, go here verbatim.
