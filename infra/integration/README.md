# PORCHE integration environment — Infrastructure as Code

This is doc 13 (integration environment) as executable code: the
provisioning recipe of `13-integration-environment.md` §5, made
repeatable. The doc keeps the topology, the fidelity contract and the
learned Outscale mechanics; **this directory is the recipe**.

Tooling: **OpenTofu** (MPL-2.0 — CT-4 compliant) + the `outscale`
provider, version-pinned in `versions.tf`; **octl** v0.0.34 (BSD-3,
static Go binary, Sigstore-verified at install) for day-to-day API
calls — the successor of the legacy `osc-cli`, whose typed flags
replace the old `bin/osc-api.py` boolean workaround (deleted). Boring
and battle-tested, per the corpus's stack rule.

## Safety model (read `BOUNDARIES.md` first)

The Outscale account is **shared**. This configuration therefore:

- **targets the `use2` tenant — a NEW cloud tenant on a new region
  (2026-10-23; doc 13 §4.10: the old `sandbox` tenant's pinned resources are
  historical and do not exist there — the environment is provisioned
  fresh) and uses only its profile**: the `use2` profile of
  `~/.osc/config.json` is the only permitted one; other profiles that
  may exist in that file (e.g. `perso`) are never referenced. The
  Makefile hard-pins `OSC_PROFILE`, every `octl` call passes an
  explicit `--profile use2` (an explicit profile overrides stray
  `OSC_*` env vars), and `make guard` fail-closes the mutating
  targets on anything else.
- **manages only net-new resources** (node 2, BSUs, NFS VM, CP VM, new
  SGs). Existing resources — node 1, the management SG, the keypair,
  the dedicated group — appear only as **variable values**; the
  provider has zero authority over them.
- sets `prevent_destroy = true` on everything it manages,
- provides **no `make destroy`** — teardown is targeted and explicit
  (`make teardown-help`), matching doc 13 §7.

## First run

```sh
export OSC_PROFILE=use2
export OSC_ACCESS_KEY=$(jq -r .use2.access_key ~/.osc/config.json)
export OSC_SECRET_KEY=$(jq -r .use2.secret_key ~/.osc/config.json)
export OSC_REGION=$(jq -r .use2.region ~/.osc/config.json)

make init
make plan          # writes tfplan
# REVIEW tfplan: must contain ONLY '+' (create) lines, all
# porche-integ-*. Any '~'/'-'/'-/+' line => stop and investigate.
make apply         # applies the reviewed plan; snapshots state after
make verify        # co-location gate + node checks (manual for now)
```

State is local (`terraform.tfstate`, gitignored) with automatic
snapshots under `state-backups/` after each apply. Single-operator
setup; move to an Outscale OOS remote state when a second person works
here.

## Post-apply verification (the co-location gate)

Both metal nodes **must land on the same physical server** of the
dedicated group — otherwise the A-9 live-migration spike is meaningless.
Outscale has no "same host" knob, so this is a gate, not a property:

1. SSH to node 2 (`ssh outscale@$(tofu output -raw ... )`).
2. Check `/dev/kvm`, `vmx` in `/proc/cpuinfo`, `kvm_intel nested=Y` —
   same as node 1's validation (doc 13 §4.3).
3. Attempt (or later: automate) a cross-node KVM/CLH action proving
   co-location; if the group scheduler split the nodes, terminate and
   recreate node 2 until they share a physical server, then record the
   outcome in doc 13 §6.

Automation of this check is an open item (doc 13 §8 / `make verify`).

## Cost rule (doc 13 §7)

Dedicated servers are billed while the group's VMs run. **Stop the
metal VMs when idle** (`octl iaas api UpdateVms --profile use2
--VmIds ... --State stopped` — or the console; `tofu` state edit is
NOT the way; TF manages lifecycle, not day-2 stop/start).

## What runs where (target state)

| VM | Role | Notes |
|---|---|---|
| metal ×2 | bare metal hosts (nested virt, `p1`, group tenancy) | host-agent + CLH (PoC, doc 07) |
| NFS | storage pool / hot shard | `/export/porche`, ciphertext only |
| control-plane | doc-12 services + Postgres | single node; HA is a drill (A-8) |

Node 1 stays on its Rocky placeholder until the FCOS derivative OMI
exists (doc 08 action 11); its transition to a managed FCOS VM will be a
deliberate, documented step — not a plan diff.
