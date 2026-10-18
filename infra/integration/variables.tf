# ------------------------------------------------------------------
# Environment shape (doc 13 §3) and Outscale mechanics (doc 13 §4).
# Defaults encode the learned gotchas so they are not re-learned.
# ------------------------------------------------------------------

variable "region" {
  description = "Outscale region (OSC_REGION env var wins if set)"
  type        = string
  default     = "eu-west-2"
}

variable "subregion" {
  description = "Subregion — goes inside Placement, not top-level (doc 13 §4.4)"
  type        = string
  default     = "eu-west-2a"
}

# --- Pre-existing resources: READ-ONLY REFERENCES (BOUNDARIES.md) ---
# These are NOT managed by this configuration. The provider has zero
# authority over them: no import, no lifecycle, no destroy path.
# Changes here are boundary changes — edit only with the boundary map open.

variable "existing_mgmt_sg_id" {
  description = "porche-integ-sg — pre-existing management SG (SSH/ICMP from office)"
  type        = string
  default     = "sg-ba94b9e2"
}

variable "existing_keypair_name" {
  description = "porche-integ — pre-existing keypair (local private key: id_ed25519_research)"
  type        = string
  default     = "porche-integ"
}

# --- New resources (created and managed by this configuration) ---

variable "env_running" {
  description = "Lifecycle toggle (operator rule: allocate only for integration tests). true = every VM running (physical servers allocated, billed); false = everything stopped (idle cost = BSU storage only). 'make up' / 'make down' set this. First apply of a new VM must use env_running=true (cloud-init first boot)."
  type        = bool
  default     = false
}

variable "rocky_9_omi" {
  description = "Rocky 9 OMI — placeholder until the FCOS derivative OMI exists (doc 13 §8)"
  type        = string
  default     = "ami-19a6bfa1" # the OMI node 1 runs — same image keeps the pair identical
}

variable "dedicated_group_id" {
  description = "Dedicated group = the tenancy. NOT the literal 'dedicated' (doc 13 §4.1). Shared: 'Test1', in use by others."
  type        = string
  default     = "ded-53c8318b"
}

variable "vm_type_metal" {
  description = "Metal node shape. The p1 performance flag is MANDATORY in dedicated groups; p2/p3 are rejected (doc 13 §4.2)"
  type        = string
  default     = "tinav7.c8r16p1"
}

variable "vm_type_storage" {
  type    = string
  default = "tinav7.c4r16"
}

variable "vm_type_control_plane" {
  type    = string
  default = "tinav7.c4r8"
}

variable "metal_root_bs_size" {
  description = "Metal node root BSU (the 'host OS' disk)"
  type        = number
  default     = 50
}

variable "nvme_bs_size" {
  description = "Second BSU per metal node — plays the 'NVMe region' (/dev/sdb)"
  type        = number
  default     = 200
}

variable "storage_bs_size" {
  description = "NFS VM data BSU — the storage pool (/export/porche)"
  type        = number
  default     = 500
}

variable "cp_root_bs_size" {
  type    = number
  default = 50
}

variable "grpc_port" {
  description = "host-agent <-> control-plane gRPC/mTLS port"
  type        = number
  default     = 7443
}

variable "nfs_port" {
  description = "NFS port (host <-> shard flow)"
  type        = number
  default     = 2049
}

variable "migration_tls_port" {
  description = "CLH live-migration TLS port (host <-> shard flow; doc 13 §3)"
  type        = number
  default     = 49152
}

variable "tag_project" {
  type    = string
  default = "porche"
}

variable "tag_env" {
  type    = string
  default = "integration"
}
