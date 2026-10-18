output "metal_node1" {
  value = {
    vm_id     = outscale_vm.metal_node1.vm_id
    public_ip = outscale_public_ip.metal_node1_ip.public_ip
    nvme_volume_id = outscale_volume.metal_node1_nvme.volume_id
  }
}

output "metal_node2" {
  value = {
    vm_id          = outscale_vm.metal_node2.vm_id
    public_ip      = outscale_public_ip.metal_node2_ip.public_ip
    nvme_volume_id = outscale_volume.metal_node2_nvme.volume_id
  }
}

output "metal_node1_nvme_volume_id" {
  value = outscale_volume.metal_node1_nvme.volume_id
}

output "nfs" {
  value = {
    vm_id     = outscale_vm.nfs.vm_id
    public_ip = outscale_public_ip.nfs_ip.public_ip
    export    = "/export/porche"
  }
}

output "control_plane" {
  value = {
    vm_id     = outscale_vm.control_plane.vm_id
    public_ip = outscale_public_ip.control_plane_ip.public_ip
    grpc_port = var.grpc_port
  }
}

output "security_groups" {
  value = {
    host_cp    = outscale_security_group.host_cp.security_group_id
    host_shard = outscale_security_group.host_shard.security_group_id
  }
}
