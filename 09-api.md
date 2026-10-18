# 09 — API: Object Catalog (CRUD + Main Attributes)

Working document — v3, 2026-10-18. Resolves challenge B-3 (doc 08): the PORCHE API
specified as **objects with CRUD and main attributes** — deliberately at attribute level,
not full OpenAPI, until P0 feedback. Fills out FR-40. **v2**: Exposure object added
(ingress first-class v1 — E-2). **v3 (E-8)**: two API surfaces — tenant and admin.
**v4 (E-14)**: Exposure renamed **IngressRule** (+ `source_cidrs`); **EgressRule** and
**SecurityGroup** objects added.

Transport: HTTPS REST + gRPC mirror; AuthN per SEC-20 (short-lived tokens, rotation);
every call audit-logged (SEC-23). Scope: every object and attribute is visible only to its
owner (or, for platform objects, to platform operators) — view scoping per FR-14/SEC-16.
**Two API surfaces (E-8)**: the **tenant API** and the **admin API** — separate authN
domains, no surface spans both planes (FR-40/SEC-24); the object tables below mark
operator-owned mutations where relevant.

---

## Objects

### User
| | |
|---|---|
| Attributes | `id`, `state` (active/suspended), `default_tier_policy`, `quotas` (ref), `tokens` (refs) |
| CRUD | create (operator), read, update (policies, quotas — operator), delete (cascade rules per tier E/R) |
| Notes | max one per host; fleet scale ≈ max users ≈ 1,000+ (doc 05 §7) |

### Token (API key)
| | |
|---|---|
| Attributes | `id`, `user`, `scopes`, `expires_at`, `revoked` |
| CRUD | create, read (own), update (revoke), delete |

### Host
| | |
|---|---|
| Attributes | `id`, `hardware_profile` (CPU, RAM, NVMe size/class, GPU), `group` (ref), `state` (free/allocated/draining/maintenance/decommissioned), `version` (signed image digest, attestation result), `load` (live) |
| CRUD | read (own via Allocation; free hosts via `ListAvailableHosts` — profile, group, `cost_index` only — internal showback unit, E-15, not a customer price), update (operator: state), create/delete = fleet ops, not API CRUD |
| Notes | lifecycle owned by platform operator (doc 05 §7); users never mutate hosts |

### Allocation
| | |
|---|---|
| Attributes | `id`, `user`, `host`, `state` (active/draining/released), `created_at`, `released_at` |
| CRUD | create (`AllocateHost`), read (own), update (release request), delete = release |
| Notes | release triggers drain → attested wipe → host `free` (doc 05 §1.1) |

### HostGroup
| | |
|---|---|
| Attributes | `id`, `kind` (rack/power/maintenance), `metadata` |
| CRUD | read (catalog: groups reachable by the caller's allocations), create/update/delete (operator) |

### Vpc

| | |
|---|---|
| Attributes | `id`, `user`, `cidr` (RFC1918 — user-requested or platform-allocated, FR-23), `subnets` (refs), `state` (active/deleting) |
| CRUD | create (request a range — validated non-overlapping fleet-wide, E-13), read (own), update (add/remove subranges), delete (only when no workload /32s remain) |
| Notes | spans the user's entire allocation; workload /32s are assigned from it (doc 14 §3); overlap rejections carry machine-readable reasons |

### Workload
| | |
|---|---|
| Attributes | `id`, `user`, `image` (digest), `command`, `volumes` (refs), `vcpu`, `ram`, `affinity_tags`, `ttl`, `state` (pending/scheduled/starting/running/stopped/failed/reaped), `placement` (host, reason — incl. pending cause), `ingress_rules`, `security_groups` (refs), `console_ref`, `events` |
| CRUD | create (FR-1), read (own, incl. `GetPlacementDecisions`, `GetResilienceReport` projections), update (limited: stop/start, migrate — FR-34, re-tag), delete (deterministic teardown FR-3) |

### Volume
| | |
|---|---|
| Attributes | `id`, `user`, `size`, `tier` (E/R/S), `class` (hot/cold), `network_reservation`, `attached_to` (workload, host), `checkpoints` (refs), `state` |
| CRUD | create (size + tier + class + reservation, immutable per volume in v1), read (own), update (attach/detach at create-time in v1; checkpoint retention), delete (wipe + report, FR-3/FR-33) |

### Checkpoint
| | |
|---|---|
| Attributes | `id`, `volume`, `sealed_at`, `epoch`, `size`, `class` (hot/cold), `retention` |
| CRUD | create (`SyncVolume` — FR-33, opt-in app-consistent), read (list, own), update (retention), delete (release) |

### IngressRule

| | |
|---|---|
| Attributes | `id`, `user`, `workload`, `protocol` (tcp/udp), `guest_port`, `external_port`, `source_cidrs` (default 0.0.0.0/0), `state` (active/revoked), `created_at`, `revoked_at` |
| CRUD | create (explicit, auditable — FR-20), read (own), update (revoke), delete (revoke) |
| Notes | inbound access from outside the VPC; enforced by the per-host ingress proxy (I-14) — a revoked rule means **no listener exists**; created/changed/revoked through the control API only, never from the guest (E-2) |

### EgressRule

| | |
|---|---|
| Attributes | `id`, `user`, `vpc`, `workload` (nullable — null = VPC default), `destination` (fqdn | cidr), `protocol`, `port`, `state` (active/revoked) |
| CRUD | create, read (own), update (revoke), delete (revoke) |
| Notes | outbound internet access; enforced by the transparent egress proxy — **DNS is resolved by the proxy**, so FQDN rules cannot be bypassed by raw IP (doc 14 §4); empty set + no VPC default = no egress |

### SecurityGroup

| | |
|---|---|
| Attributes | `id`, `user`, `vpc`, `name`, `rules[]` (direction in/out, protocol, from/to port, cidr — within the user's VPC CIDR) |
| CRUD | create, read (own), update (rules), delete (when unreferenced) |
| Notes | east-west, workload ↔ workload within the VPC; attached to workloads at create (attachment immutable per workload in v1, like volumes); compiled to netns nftables (doc 14 §6); default deny |

### Image
| | |
|---|---|
| Attributes | `digest`, `name`, `tags`, `owner` (platform or user), `signed`, `sbom_ref` |
| CRUD | create (via build/intake pipeline — never direct execution, FR-31), read, update (re-tag), delete (GC when unreferenced) |

### Quota
| | |
|---|---|
| Attributes | `user`, `max_vcpu`, `max_ram`, `max_workloads`, `max_local_nvme`, `max_network_storage`, `max_egress`, `max_ttl` |
| CRUD | read (own), update (operator) |

---

## Read-only views (projections, FR-14)

`ListAvailableHosts`, `ListHosts`, `GetPlacement`, `GetDistribution`,
`GetPlacementDecisions`, `GetResilienceReport` — all derived from the objects above;
scoping rules per doc 05 §2.

## Conventions

- **Immutable IDs**; state machines per object as documented in 01/05.
- Every mutating call is idempotent (client retry-safe) and maps to one audit-log entry.
- Errors carry machine-readable constraint reasons (why pending, why rejected) — the
  explainability rule (doc 05 §2) applies to the API surface itself.
- Deletion semantics: workload/volume deletes are **operations** (teardown with reported
  wipe), not just row deletion — the audit trail outlives the object.
