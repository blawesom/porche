# Boundaries — what this configuration may and may not touch

Phase-0 read-only inventory of the account, taken 2026-10-18 before any
IaC was applied. **This account is shared: most of it is NOT PORCHE's.**
Regenerate with `make inventory` before any change that alters resource
boundaries, and update this file in the same commit.

Refreshed 2026-10-18 (evening) with `make inventory` (octl, sandbox
profile) after the partial apply and the out-of-band termination of
metal node 2 — see the managed-resources section below.

## PORCHE-owned, PRE-EXISTING — reference only, NEVER managed

Managed = appears nowhere in `*.tf` except as a variable value. The
provider has zero authority over these (no import, no lifecycle, no
destroy path).

| Resource | ID | Notes |
|---|---|---|
| Metal node 1 | `i-b48c9483` | `tinav7.c8r16p1`, nested virt, keypair `porche-integ2` **injected via UserData** (§4.12 — keypair injection served a stale key on this VM type); PoC-ready (doc 07 §8.5); **stopped at burst end 2026-10-19** — recreation path is scripted (§4.13) |
| Node 1 public IP | `eipalloc-84925ae5` (`148.253.126.10`) | linked to `i-b48c9483` (via octl) |
| Management SG | `sg-ba94b9e2` (`porche-integ-sg`) | SSH/ICMP from office; **rule added 2026-10-19: `171.33.93.229/32` tcp/22** (current office egress — was `171.33.98.94`); worn by both metal nodes |
| Keypair | `porche-integ2` (local `~/.ssh/porche_integ`) | active 2026-10-19; the OLD `porche-integ` keypair remains in the account but its private key (`id_ed25519_research`) is **lost** — do not rely on it |
| Dedicated group | `Test1` (`ded-53c8318b`, eu-west-2a, gen 7) | **shared — marked "in use" in doc 13 §6; never deleted** |
| Node 1 root BSU | created with the VM | DeleteOnVmDeletion=true |

## PORCHE-owned, MANAGED by `infra/integration/`

Everything named `porche-integ-*` that this configuration creates:
the `hostcp`/`hostshard` SGs, node 2 + its BSU, the NFS VM, the
control-plane VM, and the public IPs of the new VMs.

**2026-10-19 burst state**: node 1's 200 GB BSU is
`vol-ad0f8082` (created via TF, **linked via octl** — §4.14); the TF state
currently holds volume + EIP + SGs, with the VM and link records removed
(stale IDs after the node recreation) — reconcile with the
out-of-band-then-import pattern before the next TF use. A canary VM
(`i-b57d5cda`, `tinav7.c2r2p1`) used to prove group allocation was
**deleted** after the burst.

## NOT PORCHE — untouchable, present in the account

| Kind | Resources |
|---|---|
| VMs | `i-ad8b0ed9` (cyber_ble), `i-d149acce` (research-p3), `i-8dee2269` (research-p3, stopped), `i-a0c07e4b` (interview) |
| Volumes | `vol-d31b6f2b` (cyber_ble), `vol-6868b111` / `vol-163268f0` (research-p3), `vol-2e9cb6c1` (interview), `vol-6e80981f` (800 GB io1, unlinked — owner unknown), `vol-6bdc9f5a` (5 GB, unlinked — owner unknown) |
| Public IPs | `eipalloc-fcc51ea9` (unlinked), `eipalloc-8a611ae4` (unlinked) — not referenced by doc 13, treat as foreign |
| SGs | all except `sg-ba94b9e2` (default, test-moll, test2, SG PVN7, Test, test1, qa-test-*, windev, cyber agent bl, research-p3, interview, vpc-9159e604 defaults) |
| Keypairs | heckle-work2, skr2024, al_squadra, skr2025, cyber_ble, research-p3, research-oapi, interview |
| Net | `vpc-9159e604` (used by qa-test SGs — not porche's) |

## Exploit / PoC runs (doc 07 §7.4)

1. Exploit and PoC material runs **only inside our node VMs' guests** (the
   workload → nested-host boundary). Nothing is authored, aimed, or released
   against the Outscale boundary, the outer hypervisor, or any resource that
   is not PORCHE-owned (tables above).
2. **Abort on anomaly**: any unexpected process, listener, or connectivity
   outside the expected composition stops the run and is reported (SEC-41
   logic). The shared account makes this a hard rule, not a guideline.
3. Escape-run verdicts in this environment are **control-delta claims only**
   (E-16/E-18); security evidence is metal-only.

## Hard rules

1. `tofu plan` must contain **only `+` (create) lines** for
   `porche-integ-*` resources. Any update/destroy/replace line ⇒ stop.
2. No `tofu destroy` without an explicit `-target=<resource>` — and
   never targeting anything in the tables above.
3. `prevent_destroy = true` on every managed resource is load-bearing:
   do not remove it to "fix" a plan.
4. The dedicated group and the management SG are **shared** — creating
   node 2 allocates another physical server in the group (billing:
   stop/terminate metal VMs when idle, doc 13 §7).
