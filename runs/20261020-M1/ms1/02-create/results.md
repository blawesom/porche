MS-1.M3 — host-agent create path (doc 07 §8.6 artifact rule)
run-id: 20261020-M1 (runs/20261020-M1/ms1/02-create/)
live: 2026-10-20T10:59Z on node 1 (i-f517519a), root — real path only; no dry surfaces exist (E-29)

== what ran ==
/usr/local/bin/porche host-agent --store /root/poc/ms1/ds.sqlite3 --keys-file /etc/porche/keys.json \
    create --spec /root/poc/ms1/spec.json
host-agent binary sha256:2184febb791124b073cbf57f3d06ad1fcea397b31038532a684eedc183c61e2a
(install + sha256sum of BOTH the bundle copy and /usr/local/bin/porche match — the
binary that produced this run is pinned by digest, closing the 20261019 "commit: unknown" gap)

== result ==
exit: created (rc=0)
wall: 0.994 s  (user 0.480 s, sys 0.093 s)
(create ≈1 s under the chosen KDF rule — the spike's effect is live-proven; the
 pre-rule run of the same spec on the same node took 8.765 s)

== the create→ready→done→poweroff chain (serial-console.log, live 10:59) ==
porche.ready                        (PERF-1 T1 marker — guest, 8 ms after init start)
porche-helloporche.done rc=0        (workload output + PERF-1 exit marker, rc=0)
ACPI PM: Preparing to enter system sleep state S5
reboot: Power down                  (init's poweroff -f — deterministic)

== post-create host state (all observed live) ==
store row:       ms1create1  state=running (create-verified; terminal "done" is derived
                 from console markers + VM gone — see 04-wipe/results.md and src/README.md)
volume:          /var/lib/porche/volumes/ms1create1.img (256 MiB sparse)
mapper:          /dev/mapper/porche-ms1create1 || netns: w-ms1create1 (id 0)
mock KMS entry:  /etc/porche/keys.json 0600, 1 entry (ms1create1)

== defect found + fixed during this run (recorded live) ==
Wipe-then-recreate of the same id hit RTNETLINK "File exists": `ip netns del` DETACHES
the root-side veth (vr-<id>) instead of deleting it, so the next `ip link add` failed.
Fixed in src/crates/porche-host: setup_netns now pre-cleans (ip netns del + ip link del
vr-<id>, failures ignored) and remove_commands returns both removals; teardown tolerates
absent members. Verified live: wipe ms1create1 (5/5) → host fully clean (no-mapper,
no-netns, no-root-veth, no-key) → re-create green at 1 s → this artifact.

== host-agent pin ==
source content: MS-1 tree over rollback commit a44c44c (git rev-parse HEAD at build
time; the MS-1 code commit that closes this milestone lands the same content).
test base: cargo test --workspace → 33 passed (6 clh + 21 core + 6 host) — spec
validation, store state-machine and wire contract kept; argv-shape self-assertions and
every dry/dry-run surface do not return (E-29).

== stderr noise observed (not a defect in scope) ==
Guest kernel occasionally oopses early (x86/mm "Checked W+X mappings: FAILED, 14 W+X
pages found") before init runs; init proceeds and all markers land. Carried to the
MS-1 write-up as a known-uninvestigated guest-kernel hardening datapoint (the from-
scratch minimalconfig tightening is post-PoC per guest/build/kernel-fragment.config).