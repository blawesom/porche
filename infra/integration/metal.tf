# ------------------------------------------------------------------
# Metal nodes (doc 13 §3 rows 1-2).
#
# LIFECYCLE (operator rule): the environment is allocated only for
# integration test runs and released after — var.env_running toggles
# every VM between "running" and "stopped". A stopped metal VM releases
# its physical server back to the group (doc 13 §7): idle cost is BSU
# storage only. "make up" / "make down" are thin wrappers over the
# env_running variable.
#
# BOOTSTRAP NOTE: both nodes were created out-of-band (osc-cli at the
# time; octl today), NOT via this configuration, then imported — the provider only accepts
# placement_tenancy of "default"|"dedicated" and cannot express the
# dedicated-GROUP tenancy (doc 13 §4.1). Consequences:
#   - no block_device_mappings here: the root BSUs pre-exist and
#     declaring them forces a REPLACEMENT (prevent_destroy would
#     block the plan — by design).
#   - if a node is ever lost, recreate it with octl
#     (octl iaas api CreateVms --profile sandbox — typed flags: group
#     tenancy + nested virtualization are real booleans) and re-import.
#
# CRITICAL: both nodes must sit on the SAME physical server of the
# dedicated group — that is what makes cross-VM live migration genuine
# (doc 13 §3). No API knob exists: verified after every "make up"
# (make verify); if the scheduler split the nodes, stop/restart node 2
# until co-located, then record it in doc 13 §6.
# ------------------------------------------------------------------

# --- Metal node 1 (pre-existing, imported 2026-10-18) ---

resource "outscale_vm" "metal_node1" {
  image_id     = var.rocky_9_omi
  vm_type      = var.vm_type_metal
  keypair_name = var.existing_keypair_name

  placement_tenancy        = var.dedicated_group_id
  placement_subregion_name = var.subregion
  nested_virtualization    = true

  security_group_ids = [var.existing_mgmt_sg_id]

  state = var.env_running ? "running" : "stopped"

  tags {
    key   = "Name"
    value = "porche-integ-metal1"
  }

  lifecycle { prevent_destroy = true }
}

resource "outscale_public_ip" "metal_node1_ip" {
  tags {
    key   = "Name"
    value = "porche-integ-metal1"
  }

  lifecycle { prevent_destroy = true }
}

resource "outscale_public_ip_link" "metal_node1_ip" {
  public_ip_id = outscale_public_ip.metal_node1_ip.public_ip_id
  vm_id        = outscale_vm.metal_node1.vm_id

  lifecycle { prevent_destroy = true }
}

# --- Metal node 2 (created 2026-10-18 out-of-band, imported) ---

resource "outscale_vm" "metal_node2" {
  image_id     = var.rocky_9_omi
  vm_type      = var.vm_type_metal
  keypair_name = var.existing_keypair_name

  placement_tenancy        = var.dedicated_group_id
  placement_subregion_name = var.subregion
  nested_virtualization    = true

  security_group_ids = [var.existing_mgmt_sg_id]

  state = var.env_running ? "running" : "stopped"

  tags {
    key   = "Name"
    value = "porche-integ-metal2"
  }

  lifecycle { prevent_destroy = true }
}

# The "NVMe region" of node 2 — second BSU, shows up as /dev/sdb.
resource "outscale_volume" "metal_node2_nvme" {
  size           = var.nvme_bs_size
  volume_type    = "gp2"
  subregion_name = var.subregion

  tags {
    key   = "Name"
    value = "porche-integ-metal2-nvme"
  }

  lifecycle { prevent_destroy = true }
}

resource "outscale_volume_link" "metal_node2_nvme" {
  volume_id   = outscale_volume.metal_node2_nvme.volume_id
  vm_id       = outscale_vm.metal_node2.vm_id
  device_name = "/dev/sdb"

  lifecycle { prevent_destroy = true }
}

resource "outscale_public_ip" "metal_node2_ip" {
  tags {
    key   = "Name"
    value = "porche-integ-metal2"
  }

  lifecycle { prevent_destroy = true }
}

resource "outscale_public_ip_link" "metal_node2_ip" {
  public_ip_id = outscale_public_ip.metal_node2_ip.public_ip_id
  vm_id        = outscale_vm.metal_node2.vm_id

  lifecycle { prevent_destroy = true }
}
