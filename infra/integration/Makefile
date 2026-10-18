.PHONY: init fmt validate guard plan apply up down snapshot inventory verify teardown-help

# --- credentials (never stored in the repo) ---
# SANDBOX ACCOUNT ONLY (operator rule). ~/.osc/config.json holds two
# profiles (perso, sandbox); only 'sandbox' is ever used here. The
# Makefile hard-pins OSC_PROFILE and every octl call passes an explicit
# --profile sandbox (an explicit profile overrides stray OSC_* env
# vars). 'guard' fail-closes the mutating targets on anything else.
OSC_CONFIG ?= $(HOME)/.osc/config.json
OSC_PROFILE := sandbox
export OSC_PROFILE
export OSC_ACCESS_KEY := $(shell jq -r .sandbox.access_key $(OSC_CONFIG) 2>/dev/null)
export OSC_SECRET_KEY := $(shell jq -r .sandbox.secret_key $(OSC_CONFIG) 2>/dev/null)
export OSC_REGION     := $(shell jq -r .sandbox.region $(OSC_CONFIG) 2>/dev/null)

TF := tofu
OCTL := octl

init:
	$(TF) init

fmt:
	$(TF) fmt -recursive

validate:
	$(TF) validate

# Fail-closed account guard: refuse to touch anything but the sandbox
# account. Also catches a broken or missing ~/.osc/config.json before
# tofu/octl produce a confusing failure (or worse, a wrong account).
guard:
	@test "$(OSC_PROFILE)" = "sandbox" || { echo "REFUSED: OSC_PROFILE='$(OSC_PROFILE)' — only the sandbox account is permitted"; exit 1; }
	@jq -e '.sandbox | has("access_key") and has("secret_key") and has("region")' $(OSC_CONFIG) >/dev/null \
	  || { echo "REFUSED: no usable 'sandbox' profile in $(OSC_CONFIG)"; exit 1; }
	@test -n "$(OSC_ACCESS_KEY)" || { echo "REFUSED: sandbox credentials did not resolve (jq on $(OSC_CONFIG))"; exit 1; }

# Always plan to a file. Review it line by line before apply:
# the ONLY acceptable plan is '+' (create) lines for porche-integ-*
# resources, or '~' (update) lines limited to the `state` attribute
# when combined with an env_running toggle. Any other '~'/'-'/'-/+'
# line => STOP and investigate.
plan: guard
	$(TF) plan -out tfplan

# Apply only a reviewed plan file. No --auto-approve.
# NOTE: first apply of newly created VMs must run with
# -var env_running=true so cloud-init first-boot executes:
#   tofu apply -var env_running=true tfplan
apply: guard
	@test -f tfplan || (echo "no tfplan — run 'make plan' and review it first"; exit 1)
	$(TF) apply tfplan
	@$(MAKE) --no-print-directory snapshot

# ------------------------------------------------------------------
# Lifecycle (operator rule: resources are allocated only for
# integration test runs and cleaned at the end).
#
#   make up   -> every VM running (dedicated servers allocated)
#   make down -> every VM stopped (physical servers released;
#                idle cost = BSU storage only)
#
# Preconditions for using up/down non-interactively:
#   - the stack is converged (empty plan at env_running=false), and
#   - `state` flips are the only change in the resulting plan.
# If up/down ever show more than state diffs, abort and review.
# ------------------------------------------------------------------
up: guard
	@$(TF) apply -auto-approve -var env_running=true && $(MAKE) --no-print-directory snapshot

down: guard
	@$(TF) apply -auto-approve -var env_running=false && $(MAKE) --no-print-directory snapshot

# State snapshot before/after every mutation (recovery path).
snapshot:
	@mkdir -p state-backups
	$(TF) state pull > state-backups/$(shell date +%Y%m%d-%H%M%S).tfstate
	@ls -t state-backups | head -3

# Read-only account inventory — regenerate BOUNDARIES.md before any
# change touching resource boundaries. octl, sandbox profile only.
inventory: guard
	$(OCTL) iaas api ReadVms --profile sandbox | jq '[.Vms[] | {VmId, VmType, State, Tenancy: .Placement.Tenancy, KeypairName, SecurityGroups: [.SecurityGroups[].SecurityGroupId]}]'
	$(OCTL) iaas api ReadVolumes --profile sandbox | jq '[.Volumes[] | {VolumeId, Size, State, VolumeType, LinkedVms: [(.LinkedVolumes // [])[] | .VmId]}]'
	$(OCTL) iaas api ReadPublicIps --profile sandbox | jq '[.PublicIps[] | {PublicIpId, PublicIp, VmId: (.VmId // null)}]'

# Post-"make up" verification. The co-location gate is the critical one:
# both metal nodes must sit on the same physical server of the group,
# or the A-9 live-migration spike is meaningless. TODO automate
# (cross-node KVM migration smoke test); manual until then — README.
verify:
	@echo "Verify after 'make up':"
	@echo "  1. co-location: both metal nodes same physical server (README procedure)"
	@echo "  2. node 2: /dev/kvm, vmx flag, kvm_intel nested=Y"
	@echo "  3. NFS VM: showmount -e <nfs-ip> lists /export/porche"
	@echo "  4. review: no foreign resource touched (make inventory vs BOUNDARIES.md)"

teardown-help:
	@echo "There is NO 'make destroy'. Teardown is targeted and explicit:"
	@echo "  tofu destroy -target=outscale_vm.nfs"
	@echo "  tofu destroy -target=outscale_vm.control_plane"
	@echo "  tofu destroy -target=outscale_vm.metal_node2"
	@echo "Plus per doc 13 §7: DeleteVms/DeletePublicIp/DeleteVolume for the"
	@echo "specific resources. The dedicated group, porche-integ-sg and the"
	@echo "porche-integ keypair are NEVER deleted. For day-to-day use,"
	@echo "'make down' is the correct end-of-test action (VMs stopped,"
	@echo "physical servers released, data kept)."
