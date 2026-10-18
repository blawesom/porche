# ------------------------------------------------------------------
# New security groups (doc 13 §3 "Network", doc 13 §8 item 3).
#
# The existing management SG (sg-ba94b9e2) stays untouched and is the
# SG-member anchor for inter-host flows: it is attached to BOTH metal
# nodes (node 1 from day one, node 2 at creation), so new rules
# reference it instead of touching node 1's attachment set.
# ------------------------------------------------------------------

locals {
  common_tags = [
    { key = "project", value = var.tag_project },
    { key = "env", value = var.tag_env },
  ]
}

# On the control-plane VM: accept host-agent gRPC/mTLS from metal nodes.
resource "outscale_security_group" "host_cp" {
  security_group_name = "porche-integ-sg-hostcp"
  description         = "porche-integration-host-to-cp-grpc-mtls"

  tags {
    key   = "Name"
    value = "porche-integ-sg-hostcp"
  }

  lifecycle { prevent_destroy = true }
}

resource "outscale_security_group_rule" "host_cp_grpc" {
  flow              = "Inbound"
  security_group_id = outscale_security_group.host_cp.security_group_id

  # members = the management SG, which both metal nodes wear
  rules {
    ip_protocol     = "tcp"
    from_port_range = var.grpc_port
    to_port_range   = var.grpc_port
    security_groups_members {
      security_group_id = var.existing_mgmt_sg_id
    }
  }

  lifecycle { prevent_destroy = true }
}

# On the storage/NFS VM: accept NFS + live-migration TLS from metal nodes.
resource "outscale_security_group" "host_shard" {
  security_group_name = "porche-integ-sg-hostshard"
  description         = "porche-integration-host-to-shard-nfs-migration-tls"

  tags {
    key   = "Name"
    value = "porche-integ-sg-hostshard"
  }

  lifecycle { prevent_destroy = true }
}

resource "outscale_security_group_rule" "host_shard_nfs" {
  flow              = "Inbound"
  security_group_id = outscale_security_group.host_shard.security_group_id

  rules {
    ip_protocol     = "tcp"
    from_port_range = var.nfs_port
    to_port_range   = var.nfs_port
    security_groups_members {
      security_group_id = var.existing_mgmt_sg_id
    }
  }

  lifecycle { prevent_destroy = true }
}

resource "outscale_security_group_rule" "host_shard_migration" {
  flow              = "Inbound"
  security_group_id = outscale_security_group.host_shard.security_group_id

  rules {
    ip_protocol     = "tcp"
    from_port_range = var.migration_tls_port
    to_port_range   = var.migration_tls_port
    security_groups_members {
      security_group_id = var.existing_mgmt_sg_id
    }
  }

  lifecycle { prevent_destroy = true }
}
