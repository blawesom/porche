# Results record — doc 07 §7.5 (fill per run, commit with the run)

Run: `20261019-1201-miniloop` (5-cycle stability smoke — NOT the §7.5 record; the
full 1,000-cycle gate run is pending) · Fixtures: `fixtures.txt` (§7.1) ·
Node: i-b48c9483 (interim Rocky 9.4 — §7.1 sequencing; the recorded run is FCOS)

| Criterion | Method | n | Threshold | Measured | Verdict |
|---|---|---|---|---|---|
| PERF-1 (smoke) | console marker | not run | P50 < 1.5 s | — | — |
| PERF-5 (smoke) | RSS sampler | not run | < 200 MB each | — | — |
| PERF-8 loop (**gate**) | create/wipe | 5 (stability smoke) | no wedge, < 5 % drift | no wedge, no residue, all wipes all-pass | **stable** (mini) |
| PERF-8 timing (smoke) | per-cycle | 5 | P99 < 2 s | P50 1325 ms, P99 1328 ms | **PASS** (mini) |
| Wipe probe | markers + key slots | all 5 | no recoverable data | 5/5 all-pass (mapper, keyfile, keyslots, trim, region) | **PASS** |
| Escape delta | §7.3 toggles | not run | delta present | — | — |
| Sysctl script | boot assert | every boot | all pass | 4/4 (io_uring=2, userns=0, modules_disabled=1, core_pattern) | **PASS** |

## Notes

- **First complete create→ready→done→poweroff→wipe cycle on the integration node**
  (2026-10-19): guest boots 6.18.57 (bzImage) under nested KVM, applies its /128
  from the `porche.ip6=` cmdline key, prints `porche.ready`, runs the decoded
  `porche.cmd`, prints `porche.done rc=0`, powers off; CLH exits with the guest.
- Create ≈ 8–9 s per cycle, dominated by `cryptsetup luksFormat` (Argon2id on the
  256 MiB volume) — a nested-virt smoke signal (E-16); flag for the PERF-1
  measurement: KDF cost sits inside the create→ready window.
- Wipe (teardown) ≈ 320 ms single, P50 1.33 s in-loop (incl. probe scan).
- Serial console lines end with CR — harness greps are unanchored (fixed).
- Platform quirks hit during bring-up are recorded in doc 13 §4 (items 12–14).
