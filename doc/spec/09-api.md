# 09 — API: Object Catalog (CRUD + Main Attributes)

Working document — v7, 2026-10-18. Resolves challenge B-3 (doc 08): the PORCHE API
specified as **objects with CRUD and main attributes** — deliberately at attribute level,
not full OpenAPI, until P0 feedback. Fills out FR-40. **v2**: Exposure object added
(ingress first-class v1 — E-2). **v3 (E-8)**: two API surfaces — tenant and admin.
**v4 (E-14)**: Exposure renamed **IngressRule** (+ `source_cidrs`); **EgressRule** and
**SecurityGroup** objects added. **v5 (E-17/E-21)**: Image create = **`ImportImage` →
controlled rebuild** (never direct execution); **v1 API surface frozen at E-14**.
**v6 (E-22..E-25)**: IPv6-first — `Vpc.prefix6` (ULA), Workload `ip6`/`ip4`/`public_ipv4`,
new **PublicIpv4** object, family-agnostic rule ranges.
**v7 (E-26)**: tenant **surface map** added (the list E-21 freezes against);
`GetUsage`/`GetConsole`/`GetEvents` enumerated (FR-2/FR-6/FR-40/FR-42 — existing scope,
newly named); **Allocation merged into Host** (allocate = create, release = delete — the
binding is Host state, FR-13); cuts → P2 register: `Vpc.subnets`, VPC subrange update,
`Image` re-tag. **v8 (E-28, 2026-10-19)**: **console surfaces (MVP)** — the UIs as
pure projections of the frozen map; page inventories for both consoles + Stage-1
demo scope.

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
| Notes | a token binds to **exactly one authN domain** (E-8/FR-40) — tenant or admin, never both planes; short-lived per SEC-20 (rotation = create + revoke) |

### Host (merged with Allocation — E-26)

| | |
|---|---|
| Attributes | `id`, `hardware_profile` (CPU, RAM, NVMe size/class, GPU), `group` (ref), `state` (free/allocated(user)/draining/maintenance/decommissioned — FR-13), `user` (the binding — set by allocate), `allocated_at`, `version` (signed image digest, attestation result), `load` (live) |
| Tenant CRUD | **create = `AllocateHost`** (bind a free host from the caller's catalog); read (own: state, group, profile, load, version + attestation); free hosts via `ListAvailableHosts` (profile, group, `cost_index` only — internal showback unit, E-15, not a customer price); **delete = `ReleaseHost`** (release request) |
| Admin CRUD | onboard/offboard (fleet ops, not tenant API), update (state: maintenance/decommission) |
| Notes | the binding is **Host state, not a separate object** (E-26 — FR-13 already modeled it so; doc 05's `HostAllocation` naming drift closed); release triggers drain → attested wipe → host `free` (doc 05 §1.1); release history lives in the audit trail (SEC-23), not in fields; users never mutate host configuration — only the binding |

### HostGroup
| | |
|---|---|
| Attributes | `id`, `kind` (rack/power/maintenance), `metadata` |
| CRUD | read (catalog: groups reachable by the caller's allocations), create/update/delete (operator) |

### Vpc

| | |
|---|---|
| Attributes | `id`, `user`, `prefix6` (**ULA /48–/56**, RFC 4193 — user-requested or platform-allocated, FR-23/E-22), `cidr4` (RFC1918 /24–/16 — **legacy-v4 enablement only**, E-23; absent = no v4 in this VPC), `state` (active/deleting) |
| CRUD | create (request prefixes — validated non-overlapping fleet-wide, E-22/E-23), read (own), delete (only when no workload addresses remain) — **prefix immutable after create** (E-26: no subnet mechanism exists in doc 14 — /128s are assigned from the VPC prefix directly; subrange management → P2 register) |
| Notes | spans the user's entire allocation; workload **/128s** are assigned from `prefix6` (doc 14 §3); overlap rejections carry machine-readable reasons |

### Workload
| | |
|---|---|
| Attributes | `id`, `user`, `image` (digest), `command`, `volumes` (refs), `vcpu`, `ram`, `affinity_tags`, `ttl`, `state` (pending/scheduled/starting/running/stopped/failed/reaped), `placement` (host, reason — incl. pending cause), `ip6` (/128 — always present, E-22), `ip4` (nullable /32 — legacy exception, E-23), `public_ipv4` (ref, nullable — E-24), `ingress_rules`, `security_groups` (refs), `console_ref`, `events` |
| CRUD | create (FR-1), read (own, incl. the placement projections), operations: `StartWorkload` / `StopWorkload` / `MigrateWorkload` (FR-34), delete (deterministic teardown, FR-3 — an operation, not a row delete) |

### Volume
| | |
|---|---|
| Attributes | `id`, `user`, `size`, `tier` (E/R/S), `class` (hot/cold), `network_reservation`, `attached_to` (workload, host), `checkpoints` (refs), `state` |
| CRUD | create (size + tier + class + reservation + attach-at-create, immutable per volume in v1), read (own), update (checkpoint retention), delete (wipe + report — an operation, FR-3/FR-33) |

### Checkpoint
| | |
|---|---|
| Attributes | `id`, `volume`, `sealed_at`, `epoch`, `size`, `class` (hot/cold), `retention` |
| CRUD | create (`SyncVolume` — FR-33, opt-in app-consistent), read (list, own), update (retention), delete (`ReleaseCheckpoint` — an operation) |

### IngressRule

| | |
|---|---|
| Attributes | `id`, `user`, `workload`, `protocol` (tcp/udp), `guest_port`, `external_port`, `source_ranges` (family-agnostic CIDR list — v4 or v6; default all, E-22/E-24), `state` (active/revoked), `created_at`, `revoked_at` |
| CRUD | create (explicit, auditable — FR-20), read (own), revoke (`RevokeIngressRule` — update and delete are both revoke) |
| Notes | inbound access from outside the VPC; enforced by the per-host ingress proxy (I-14) — a revoked rule means **no listener exists**; listeners bind the workload's fabric identity and, if attached, its **attached IPv4** (E-24) — v4 clients reaching a v6 listener is the normal case; created/changed/revoked through the control API only, never from the guest (E-2) |

### EgressRule

| | |
|---|---|
| Attributes | `id`, `user`, `vpc`, `workload` (nullable — null = VPC default), `destination` (fqdn | cidr — family-agnostic, E-22), `protocol`, `port`, `state` (active/revoked) |
| CRUD | create, read (own), revoke (`RevokeEgressRule` — update and delete are both revoke) |
| Notes | outbound internet access; enforced by the transparent egress proxy — **DNS is resolved by the proxy**, so FQDN rules cannot be bypassed by raw IP (doc 14 §4); empty set + no VPC default = no egress |

### SecurityGroup

| | |
|---|---|
| Attributes | `id`, `user`, `vpc`, `name`, `rules[]` (direction in/out, protocol, from/to port, cidr — family-agnostic, within the user's VPC ranges: v6 default, v4 only between legacy-v4 workloads) |
| CRUD | create, read (own), update (rules), delete (when unreferenced) |
| Notes | east-west, workload ↔ workload within the VPC; attached to workloads at create (attachment immutable per workload in v1, like volumes); compiled to netns nftables (doc 14 §6); default deny |

### PublicIpv4

| | |
|---|---|
| Attributes | `address` (v4, platform pool), `workload` (ref, nullable), `state` (available/attached) |
| CRUD | create (operator — pool), `AttachPublicIpv4` / `DetachPublicIpv4` (tenant, to own workloads — **1:1, both directions**, E-24), read (own attachments; operators see the pool), delete (operator) |
| Notes | inbound = ingress-proxy listener on the attached v4 (I-14 — the sole enforcement point, no DNAT bypass); outbound = SNAT through it so return traffic matches; moves with the workload on migration (same transaction as the route switch); quota `max_public_ipv4` (FR-5). **Runtime attach/detach is allowed** (unlike volume hotplug, P2): it reprograms an ingress-proxy listener only — no VMM involvement |

### Image
| | |
|---|---|
| Attributes | `digest`, `name`, `tags`, `owner` (platform or user), `signed`, `sbom_ref` |
| CRUD | create (**`ImportImage` → controlled rebuild** into the platform format — never direct execution, FR-31/FR-32, E-17), read, delete (GC when unreferenced) — **no re-tag in v1** (E-26: the digest is the identity, I-14; tags → P2 register) |

### Quota
| | |
|---|---|
| Attributes | `user`, `max_vcpu`, `max_ram`, `max_workloads`, `max_local_nvme`, `max_network_storage`, `max_egress`, `max_ttl`, `max_legacy_ipv4` (workload count, E-23), `max_public_ipv4` (attachments, E-24) |
| CRUD | read (own), update (operator) |

---

## Read-only views (projections, FR-14)

`ListAvailableHosts`, `ListHosts`, `GetPlacement`, `GetDistribution`,
`GetPlacementDecisions`, `GetResilienceReport` — all derived from the objects above;
scoping rules per doc 05 §2. Plus the three reads **enumerated by E-26** (existing
scope, newly named):

- **`GetUsage`** (FR-40 "metrics" / FR-42): per workload + per tenant — storage
  tiers/classes, vCPU/RAM-hours, host-hours per allocation, attached-IPv4 count, egress
  bytes. **Showback** — a cost signal for spread-vs-packing decisions, not an invoice
  (E-15).
- **`GetConsole`** (FR-6/E-19): live stream + recorded log of the caller's **own**
  workloads only; lifetime-bounded per E-19 (host-local, destroyed with the workload).
- **`GetEvents`** (FR-2): own-object lifecycle events (transitions with timestamps).
  The platform audit trail itself (SEC-23) stays with the admin plane.

**Attribute visibility is per projection** (FR-14/SEC-16): `ListAvailableHosts` exposes
**only** hardware profile, group, `cost_index`; the caller's own hosts additionally
expose `state`, `load`, `version` (image digest + attestation result). Nothing exposes
other users' information or fleet totals.

## Tenant surface map (v1 — the list E-21 freezes; re-baselined by E-26)

**Tenant API — mutations and operations:**

- Hosts: `AllocateHost` (create the binding), `ReleaseHost` (delete the binding)
- Workloads: `CreateWorkload`, `StartWorkload`, `StopWorkload`, `MigrateWorkload`
  (FR-34), `DeleteWorkload` (teardown operation)
- Volumes: `CreateVolume`, `DeleteVolume` (wipe + report), `SyncVolume` (FR-33),
  `ReleaseCheckpoint`, checkpoint-retention update
- Networking: `CreateVpc`, `DeleteVpc` (prefix immutable); `CreateIngressRule`,
  `RevokeIngressRule`; `CreateEgressRule`, `RevokeEgressRule`; `CreateSecurityGroup`,
  `UpdateSecurityGroupRules`, `DeleteSecurityGroup`; `AttachPublicIpv4`,
  `DetachPublicIpv4`
- Images: `ImportImage`, `DeleteImage`
- Identity: `CreateToken`, `RevokeToken`

**Tenant API — reads:** per-object `Get`/`List` (own objects only), `GetQuota`;
`ListHosts` + `ListAvailableHosts`; placement projections (`GetPlacement`,
`GetDistribution`, `GetPlacementDecisions`, `GetResilienceReport`); `GetUsage`,
`GetConsole`, `GetEvents` (scoping above).

**Admin API** (operator, separate authN domain): User create/update/delete, host
onboard/offboard + state, HostGroup create/update/delete, PublicIpv4 pool
create/delete, `UpdateQuota`, version registry + deploy gates, image-pipeline
triggers (doc 12 §3).

No method exists outside this map in v1; additions require a register decision that
evicts equivalent scope (E-21). Cut to the P2 register by E-26: `Vpc.subnets`, VPC
subrange update, `Image` re-tag.

## Conventions

- **Immutable IDs**; state machines per object as documented in 01/05.
- **The v1 API surface is frozen** (E-21, re-baselined by E-26): the freeze binds
  against the **tenant surface map** above — a new v1 object, field, or method must
  evict equivalent scope; additions queue in the P2 register.
- Every mutating call is idempotent (client retry-safe) and maps to one audit-log entry.
- Errors carry machine-readable constraint reasons (why pending, why rejected) — the
  explainability rule (doc 05 §2) applies to the API surface itself.
- Deletion semantics: workload/volume deletes are **operations** (teardown with reported
  wipe), not just row deletion — the audit trail outlives the object.

## Console surfaces (MVP) — E-28

**Rule**: a console is a **projection of this API — it may do nothing that is not
a method above** (the E-21 freeze binds the UIs exactly as it binds SDKs).
Attribute visibility rides the same per-projection rule (FR-14/SEC-16): the
tenant UI shows what the tenant reads see — never fleet internals.

Two consoles, two authN domains (E-8/FR-40 — same as the API): the **user
console** (tenant plane) and the **admin console** (admin plane). One web service
hosts both (the **console-host**, `porche-web` — doc 12 §9); the split is
enforced at the adapter, not in the UI.

### User console (tenant plane) — page inventory

| Page | Reads | Mutations |
|---|---|---|
| Workloads (home) | `Get`/`List`: state, **placement reason** (why pending — the explainability rule), ip6, public v4, image digest, vcpu/ram | `CreateWorkload` (form), `StopWorkload`, `DeleteWorkload` |
| Workload detail | spec + `GetConsole` (live stream + recorded log — tab), `GetEvents` (tab), attached volumes/rules | `MigrateWorkload` (P1 button — FR-34) |
| Volumes | `Get`/`List`: size, tier, attached-to | `CreateVolume`, `DeleteVolume` (**renders the wipe report** — deletion is an operation), `SyncVolume`, checkpoint retention |
| Network | Vpc, IngressRule/EgressRule/SecurityGroup lists, PublicIpv4 attachments | create/revoke rules, `UpdateSecurityGroupRules`, `AttachPublicIpv4`/`DetachPublicIpv4` (E-24) |
| Usage | `GetUsage`: per workload + summary (FR-40; showback-ready FR-42) | — |
| Tokens | own tokens | `CreateToken`, `RevokeToken` |

### Admin console (admin plane) — page inventory

| Page | Shows | Mutations |
|---|---|---|
| Fleet (home) | host states, groups, profiles, load, **version + attestation**, allocations | host onboard/offboard/state (drain, reimage) |
| Host detail | metrics, per-host event stream (the audit trail lives here — SEC-23) | — |
| HostGroups | capacity per group | HostGroup CRUD |
| Quotas | tenant quota matrix | `UpdateQuota` |
| Users | admin identity list | User CRUD |
| Images & versions | version registry, deploy-gate states | image-pipeline triggers, gate promote/hold (doc 12 §3) |
| PublicIpv4 pool | pool state | create/delete |
| Audit | every mutating call: actor, object, time (the E-26 convention as a page) | — |

**MVP cuts (P2)**: image import UI (`ImportImage` stays CLI-first), tenant-side
quota editing (read-only), migration UX, billing detail, SSO (authN domains are
token-based until the planes land).

**Stage 1 (PoC demo — doc 07 §8)**: `porche-web` runs the user surface
(workloads, console tab, wipe report) and a single-host admin surface (fleet of
one, audit) over the **PoC adapter** — the desired-state store + host-agent
verbs, with method names shaped like this map. The P1 adapter swaps the backend
to the real planes; the UIs do not change. Mutations serialize through one
executor lock (I-12 reconciliation replaces this).
