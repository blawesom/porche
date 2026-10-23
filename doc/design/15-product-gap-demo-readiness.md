# Product-gap assessment — where the demo line sits (2026-10-23)

Status note (context for whoever picks this up): assessment requested after the
MS-6 scope was rebaselined (doc 07 v12 — integration gates software + function,
hardware run gates performance) and the `use2` tenant switch (doc 13 §4.10; OMI
rebuilt as `ami-c804ee38`). The trigger question: *the demo should demonstrate
functional capabilities to a platform user — API feature-complete and UX too;
where do we sit?*

## The one-paragraph answer

The PoC implements the ephemeral-workload slice of the frozen API map (~8 of
~40 doc-09 methods). Everything that makes a *user of the platform* — tenant
image intake, egress control, persistent volumes, migration, quotas/metering
views, host allocation — is at zero, by design, deferred to P0/P1. Today's
honest demo is a **platform-mechanism proof** ("the workload is the adversary
and it provably vanishes"), not a product demo. The product demo is the **P0
exit artifact**; the complete one is P1. This is sequencing, not shortfall:
doc 07's own words — "the PoC exists to earn the right to build P0" — and
MS-1..MS-4 did the earning.

## API surface: frozen map (doc 09, E-21/E-26) vs. implementation

| Doc 09 method | Status | Where it lands |
|---|---|---|
| `CreateWorkload` / `StopWorkload` / `DeleteWorkload` | **exists** (console + wipe report) | now |
| `DeleteVolume` | **exists** | now |
| `GetConsole` / `GetEvents` | **exists** (live tail, derived timeline — E-29 display rules) | now |
| `List`/`Get` reads (workload, volume, image) | **exists** | now |
| `StartWorkload` (restart a stopped VM) | missing — no verb exists in `ops`/CLI | P0 |
| `AllocateHost` / `ReleaseHost` / `ListAvailableHosts`; placement reads (`GetPlacement`, `GetDistribution`, `GetPlacementDecisions`) | missing — no Host object at all | P0/P1 |
| `ImportImage` / `DeleteImage` — controlled rebuild (E-17; *the* tenant intake story) | missing — images are operator-placed digest dirs (`data_dir/images/`) | P0 (build-worker, doc 08 action 14) |
| `CreateToken` / `RevokeToken` / real authN (SEC-20) | missing — static PoC tokens (E-31.3, marked; constant-time compare still open) | P1 planes |
| `CreateVpc`, `IngressRule`, `EgressRule`, `SecurityGroup`, `AttachPublicIpv4` | missing — single-host v6 subset only; **no egress control exists** (FR-20 is U1's dominant constraint) | P1 (doc 14) |
| `CreateVolume` (standalone), `SyncVolume`, `ReleaseCheckpoint`, `MigrateWorkload` | missing — no tier R/S (A-1 spike is the gate; tier-R = P1 critical path per E-15) | P1 |
| `GetQuota` (view — enforcement exists in web), `GetUsage` (FR-42 metering) | missing | P1 |
| Admin API (User CRUD, host onboard/offboard, HostGroup, version registry, `UpdateQuota`) | missing — read-only `/a` fleet + host pages | P0/P1 |

Structural note: by count ~20% of the map exists, but it is the *easy 20%* —
the tenant-facing product story (bring your image, control your egress, keep
your data, move your workload, see your usage) is entirely in the unbuilt
part. The two known **implementation debts** that gate the P0 build:

1. **`ConsoleApi` seam unimplemented** (doc 12 §9 / E-28): no such trait in
   `src/` — `porche-web/handlers.rs` calls `store`/`ops` directly. The "P1
   adapter swaps the backend; UIs do not change" promise is unearned until
   the trait exists; every page built before it is a P0 rewire.
2. **Store schema**: `spec_json`/`state_json` blobs, no origin-tagged rows
   (doc 12 §6: admin-gates-tenant, drain wins), no doc-09 state names
   (`pending/scheduled/starting/...` vs `Creating/Running/...`), no
   idempotent order ids. One migration now vs. under live data later.

Also-open code findings from the 2026-10-23 review (still valid): create-path
check-then-act races (`create_submit` quota/duplicate checks outside the
executor lock); audit "exactly one entry per mutation" vs. create's
"ordered"+settle pair (E-26 contract drift); 1,820-line `handlers.rs`.

## UX: same picture

Exists and good: two consoles, strict authN domains (401s), tenancy view
isolation + quota enforcement, create→detail→events→console-tail→wipe-report
flow, base-path proxying (E-30), state derivation from observable signals.

Missing (blocked on their API methods, not on front-end work): volumes CRUD,
tokens page, audit view (E-29 cut → P0/P2), quotas view, VPC/rules surfaces,
usage/metering, placement views, host allocation.

## Demo tiers (be explicit with stakeholders about which is asked)

- **v0 — now (MS-6 window, node 1 in `use2`)**: mechanism proof. Create →
  attacker's view (frozen device set, EPERM probes, sibling isolation via the
  uc-suite gate — green only when observed) → wipe report (E-6 five probes) →
  same-id re-create. Tenancy/quota/401 negatives. Frame: *platform mechanism*,
  not product. Essentially `uc-suite.sh` U1 + an adversarial `command`.
- **v1 — P0 exit**: first true "user of the platform" demo — real tenant
  REST/gRPC API over the two planes, **`ImportImage` end-to-end** (tenant OCI
  in → controlled rebuild out — the single most demo-worthy missing
  capability), quotas+TTL, tokens, placement views. Console speaks the same
  methods via the adapter.
- **v2 — P1 exit**: frozen map complete — rules (ingress/egress/SG), tier R/S
  volumes, `SyncVolume`, migration under load (PERF-13), usage/metering.

## Build order to v1 (when picked up)

1. Extract `ConsoleApi` trait (method names = the frozen map) + fix the
   create-path races; split `handlers.rs` along its commented seams.
2. Store schema to doc-09 shape (origin-tagged rows, state names, idempotent
   order ids) — single migration, before plane code.
3. P0 services: tenant-plane (scheduler in-process), admin-plane, KMS v0
   (dev-mode KEK per E-15); host-agent gains the tonic/mTLS dual channel
   (doc 12 §6) — reconcile shape (store+verbs) already survives from the PoC.
4. `ImportImage` → build-worker (controlled rebuild, FR-31/32) + version
   registry + signed pipeline (doc 01 §8.1 acceptance: "deployed by the signed
   pipeline, not by hand").
5. Host object + `AllocateHost`/`ReleaseHost` + placement reads; `StartWorkload`.
6. Console surfaces over the trait for everything the above unlocked
   (tokens, quota view, placement view).

Still-standing sequencing risks from the 2026-10-23 reevaluation (unchanged):
A-1 tier-R spike first thing in P1; two-host infra + A-9 migration spike
early (PERF-13 fallback trigger fires at end-of-P1 — too late to save a
late-P2 collapse); PERF-5 rerun at n=100 on metal; metal validation run owns
every absolute number (E-16).
