# ------------------------------------------------------------------
# Control-plane VM (doc 13 §3 row 4): normal tenancy. Will run the
# doc-12 services (tenant-plane, admin-plane, kms) + Postgres
# single-node (HA is a separate drill, A-8). For now: an empty Rocky
# box on the right network paths.
# ------------------------------------------------------------------

resource "outscale_vm" "control_plane" {
  image_id     = var.rocky_9_omi
  vm_type      = var.vm_type_control_plane
  keypair_name = var.existing_keypair_name

  security_group_ids = [
    var.existing_mgmt_sg_id, # SSH from office
    outscale_security_group.host_cp.security_group_id,
  ]

  block_device_mappings {
    device_name = "/dev/sda1"
    bsu {
      volume_size           = var.cp_root_bs_size
      volume_type           = "gp2"
      delete_on_vm_deletion = true
    }
  }

  # Lifecycle toggle: control plane runs only during integration tests.
  state = var.env_running ? "running" : "stopped"

  tags {
    key   = "Name"
    value = "porche-integ-cp"
  }

  lifecycle { prevent_destroy = true }
}

resource "outscale_public_ip" "control_plane_ip" {
  tags {
    key   = "Name"
    value = "porche-integ-cp"
  }

  lifecycle { prevent_destroy = true }
}

resource "outscale_public_ip_link" "control_plane_ip" {
  public_ip_id = outscale_public_ip.control_plane_ip.public_ip_id
  vm_id        = outscale_vm.control_plane.vm_id

  lifecycle { prevent_destroy = true }
}
