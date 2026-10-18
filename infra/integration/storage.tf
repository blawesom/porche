# ------------------------------------------------------------------
# Storage pool / hot shard (doc 13 §3 row 3): normal-tenancy VM,
# NFS export /export/porche. Ciphertext only (A-4).
#
# First-boot configuration via cloud-init (Rocky). When the FCOS
# derivative OMI exists, this moves to Ignition (doc 13 §8).
# ------------------------------------------------------------------

resource "outscale_vm" "nfs" {
  image_id     = var.rocky_9_omi
  vm_type      = var.vm_type_storage
  keypair_name = var.existing_keypair_name

  # Normal tenancy: placement_* intentionally omitted.

  security_group_ids = [
    var.existing_mgmt_sg_id, # SSH from office
    outscale_security_group.host_shard.security_group_id,
  ]

  block_device_mappings {
    device_name = "/dev/sda1"
    bsu {
      volume_size           = 50
      volume_type           = "gp2"
      delete_on_vm_deletion = true
    }
  }
  # The 500 GB data BSU (the actual pool).
  block_device_mappings {
    device_name = "/dev/sdb"
    bsu {
      volume_size           = var.storage_bs_size
      volume_type           = "gp2"
      delete_on_vm_deletion = true
    }
  }

  user_data = base64encode(<<-EOT
    #!/bin/sh
    # NFS pool first-boot setup. Device naming varies across OMI drivers:
    # try the usual second-disk names.
    DATA=""
    for d in /dev/sdb /dev/xvdb /dev/vdb; do
      [ -b "$d" ] && DATA="$d" && break
    done
    if [ -z "$DATA" ]; then
      echo "PORCHE-INTEG: no second disk found, NFS pool NOT set up" >&2
      exit 0
    fi
    if ! blkid "$DATA"; then
      mkfs.xfs "$DATA"
    fi
    mkdir -p /export/porche
    grep -q "$DATA" /etc/fstab || echo "$DATA /export/porche xfs defaults,nofail 0 2" >> /etc/fstab
    mount -a
    dnf -y install nfs-utils
    echo "/export/porche *(rw,sync,no_root_squash,insecure)" > /etc/exports
    systemctl enable --now nfs-server
    systemctl restart nfs-server
  EOT
  )

  # Lifecycle toggle: NFS pool runs only during integration tests.
  state = var.env_running ? "running" : "stopped"

  tags {
    key   = "Name"
    value = "porche-integ-nfs"
  }

  lifecycle { prevent_destroy = true }
}

resource "outscale_public_ip" "nfs_ip" {
  tags {
    key   = "Name"
    value = "porche-integ-nfs"
  }

  lifecycle { prevent_destroy = true }
}

resource "outscale_public_ip_link" "nfs_ip" {
  public_ip_id = outscale_public_ip.nfs_ip.public_ip_id
  vm_id        = outscale_vm.nfs.vm_id

  lifecycle { prevent_destroy = true }
}
