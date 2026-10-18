# ------------------------------------------------------------------
# Node 1's second BSU (doc 13 §6: "no second BSU yet" — resolved
# 2026-10-18). Volume created and linked online by this configuration;
# the VM itself is managed in metal.tf.
# ------------------------------------------------------------------

resource "outscale_volume" "metal_node1_nvme" {
  size           = var.nvme_bs_size
  volume_type    = "gp2"
  subregion_name = var.subregion

  tags {
    key   = "Name"
    value = "porche-integ-metal1-nvme"
  }

  lifecycle { prevent_destroy = true }
}

resource "outscale_volume_link" "metal_node1_nvme" {
  volume_id   = outscale_volume.metal_node1_nvme.volume_id
  vm_id       = outscale_vm.metal_node1.vm_id
  device_name = "/dev/sdb"

  lifecycle { prevent_destroy = true }
}
