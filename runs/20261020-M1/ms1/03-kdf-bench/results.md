MS-1.M4.KDF — KDF spike results (doc 07 §8.2 WS-D/MS-1.M4, §8.6 artifact rule)
run-id: 20261020-M1 (runs/20261020-M1/ms1/03-kdf-bench/)
live: 2026-10-20T10:49:52Z–10:50:36Z on node 1's region disk (/var/lib/porche, 200 GB ext4 discard)
tool: cryptsetup 2.8.1 (node), volume size 256 MiB unless noted; data = raw.csv (wall-clock s, `date +%s.%N`)

summary (median of reps, 256 MiB):
  default argon2id (out-of-box)      5.960 s   ← the "create ≈ 8–9 s" driver of 20261019
  default, --pbkdf-memory 65536      5.989 s   (memory knob alone does not move it)
  --pbkdf-memory 65536 --pbkdf-force-iterations 4   0.687 s  ← CHOSEN
  --pbkdf-force-iterations 4 (256 MiB memory)  2.034 s
  size 128 MiB / 512 MiB at chosen vals  0.695 / 0.686 s  (size-independent — key derivation,
                                                          not I/O, dominates)
  reuse variant: one-time format 0.681 s, then per-cycle luksOpen+close 0.11 s

Chosen setting: argon2id --pbkdf-memory 65536 --pbkdf-force-iterations 4, cost ~0.7s — the create→ready contract for MS-1

rationale (recorded in results.md per §8.6 exact wording): the luksFormat "passphrase" is a
32-byte CSPRNG DEK from the mock KMS — not a human mnemonic. The Argon2id PBKDF derives a
passphrase-derived key to protect that input; with no human entropy to amplify, the ~6 s
default cost buys no additional secrecy on this path (the data-plane secret is the DEK
itself, held root-only in /etc/porche/keys.json with keyfile hygiene 0600). A bounded cost
(65536 B × 4 iters) keeps Argon2id as the KDF while fitting PERF-1's P50 < 1.5 s budget for
the create→ready contract.

PERF-1 note (plan-governed): PERF-1's T0 semantics are UNTOUCHED here (E-16 honest-to-smoke
stance — MS-2 owns the metrics passage and re-cuts it from data). The 1.5 s class of budget
is what this cost was fitted against; no §7.5 row is claimed by this spike.

NOT chosen — recorded for the MS-2/M4-KDF decision review:
  - pre-format reuse variant (one-time Argon2id at volume-create, luksOpen per cycle,
    0.11 s): preserves default Argon2id cost — kept as a fallback if a future attack
    analysis ever requires high Argon2id cost on the mock-KMS path. Not chosen for MS-1:
    it adds an indirection layer (paused/reopened device state) for no security gain
    given the DEK-is-the-secret rationale above.
  - default Argon2id (6 s): rejected — breaches the create budget for no secrecy gain.
  - mem256-iter4 (2 s): rejected — 3× slower for a knob (memory cost) that dominates
    nothing here.

cryptsetup note observed live: --pbkdf-force-iterations < 4 is refused
("minimum is 4"), first 2× rep loop hit this and was rerun; raw.csv is the clean rerun.