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
| Metal node 1 | `i-7482ee3c` | `tinav7.c8r16p1`, nested virt verified, CLH v54 boots guests; Rocky 9 placeholder; running (ephemeral public IP `171.33.73.205` while its EIP is unlinked) |
| Node 1 public IP | `eipalloc-84925ae5` (`148.253.126.10`) | allocated; currently **unlinked** (public IPs disassociate when a VM stops) |
| Management SG | `sg-ba94b9e2` (`porche-integ-sg`) | SSH/ICMP from office; worn by both metal nodes |
| Keypair | `porche-integ` | local private key `id_ed25519_research` |
| Dedicated group | `Test1` (`ded-53c8318b`, eu-west-2a, gen 7) | **shared — marked "in use" in doc 13 §6; never deleted** |
| Node 1 root BSU | `vol-679e783c` (50 GB gp2) | created with the VM |

## PORCHE-owned, MANAGED by `infra/integration/`

Everything named `porche-integ-*` that this configuration creates:
metal node 2 + its 200 GB BSU, node 1's 200 GB BSU (volume + link only —
the VM itself is untouched), the NFS VM, the control-plane VM, the
`hostcp`/`hostshard` SGs, and the public IPs of the new VMs.

Verified state after the 2026-10-18 partial apply: metal node 2
(`i-2727754e`) was created out-of-band and **terminated the same day**
(it no longer exists; the local TF state still records it — reconcile
before the next apply). What survives in the account: both 200 GB BSUs
(`vol-414ae55c` node 1, `vol-c14305ae` node 2 — both unlinked), the
`hostcp`/`hostshard` SGs with their rules, and three unlinked public
IPs (`eipalloc-7df5877e` node 2, `eipalloc-64230109` NFS,
`eipalloc-37f28ba3` control plane).

## NOT PORCHE — untouchable, present in the account

| Kind | Resources |
|---|---|
| VMs | `i-ad8b0ed9` (cyber_ble), `i-d149acce` (research-p3), `i-8dee2269` (research-p3, stopped), `i-a0c07e4b` (interview) |
| Volumes | `vol-d31b6f2b` (cyber_ble), `vol-6868b111` / `vol-163268f0` (research-p3), `vol-2e9cb6c1` (interview), `vol-6e80981f` (800 GB io1, unlinked — owner unknown), `vol-6bdc9f5a` (5 GB, unlinked — owner unknown) |
| Public IPs | `eipalloc-fcc51ea9` (unlinked), `eipalloc-8a611ae4` (unlinked) — not referenced by doc 13, treat as foreign |
| SGs | all except `sg-ba94b9e2` (default, test-moll, test2, SG PVN7, Test, test1, qa-test-*, windev, cyber agent bl, research-p3, interview, vpc-9159e604 defaults) |
| Keypairs | heckle-work2, skr2024, al_squadra, skr2025, cyber_ble, research-p3, research-oapi, interview |
| Net | `vpc-9159e604` (used by qa-test SGs — not porche's) |

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
