MS-1.M4 — host-agent wipe path (doc 07 §8.6 artifact rule)
run-id: 20261020-M1 (runs/20261020-M1/ms1/04-wipe/)
live: 2026-10-20T10:53Z–10:54Z on node 1 (i-f517519a), root, real path

== the run ==
create  wall 1.016 s  (vs 8.765 s pre-KDF-rule — the spike's effect, live)
wipe    wall 1.768 s  (unmap → luksErase → digest → probe scan)

== §7.2 wipe row — the machine's answer ==
wipe-report.json is the per-probe record (its own file, per §8.6): all five checks pass —
mapper absent, KMS keyfile entry removed, keyslots erased, region trimmed, region clean.
Per category, with probes taken live (see wipe-report.json "probe_detail").

This closes the pre-rollback "319 ms" evidence-gap class: the same claim (single-wipe
duration + all-pass probe report) is now a machine artifact tied to this run, not a
commit message.

== honest notes ==
- The create→wipe contract here used a marker-writing workload (dd's `35 bytes copied`
  records in serial-console.log prove the markers landed in the ciphertext region).
  pre-wipe grep of the CONSOLE for the marker strings returns 0 — by design: the probe
  scans the region file, not the console channel.
- ms1create1 (the M3 workload) is intentionally left running until M5 stops/wipes it
  under the console's DeleteWorkload flow — the UAT sees the same verbs a human does.
- cryptsetup note carried over from the KDF spike: --pbkdf-force-iterations floor is 4.