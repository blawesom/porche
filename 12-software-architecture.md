# 12 — Software Architecture: Control Plane and Host-Side Software

Working document — v2, 2026-10-18. The software-architecture spec A-7 deferred ("a
dedicated doc, post-PoC"); seeded by operator decisions **E-9/E-10** (doc 08 §E).
It specifies: state authority, order flow, service decomposition, language/stack, and the
state store. It does **not** re-open the plane model (E-8, doc 02 §3.1) or the data plane
(E-4/E-5). Requirement references: doc 01 (FR/SEC/PERF/OPS), glossary per doc 11.
**v2 (E-15/E-17/E-19/E-20)**: A-5 wording aligned with the E-15 re-gate (dev-mode KEK
P0/P1, HSM/quorum = GA); `build-worker` named as the `ImportImage` rebuild path (E-17);
console recording added to the host-agent role (E-19); static route rows confirmed as the
v1 route-propagation decision (E-20).

---

## 1. State authority (E-9)

**The control plane owns intent; the host owns reality.**

| Concern | Owner | Mechanism |
|---|---|---|
| Desired state (workloads, volumes, network rules, allocations) | control plane | objects + pending calls in the state store, transactional |
| Reported state (running, stopped, failed, migrated) | host | reports over the host channel; never trusted where security matters (report/attest split, doc 08 A-3) |
| TTL + quota enforcement under partition | host | enforced locally — fail-safe: the host knows the TTL; no control-plane lease is required to keep running |
| Convergence | host | reconciliation to desired state (I-12) |

**Host autonomy is unbounded**: on partition, the host runs its last known desired state
until its own TTL expires or an admin drain arrives. Nothing in the design stops a workload
because a control plane is unreachable (E-8 rule 3). Corollary: TTL and quota parameters
are delivered to the host **as part of the create order** — never fetched lazily.

**FR-5 consequence (applied, doc 01 v5)**: TTL is a **per-workload-class policy** —
mandatory for ephemeral classes (U1/U2), operator-waivable for stateful (U4); never waivable
by tenant self-service.

## 2. Order flow — no message bus in v1 (E-9)

The incumbent's AMQP existed to *push orders*; I-12 deleted push. Therefore:

- **Orders are rows**: a mutating API call writes objects + a pending call in one
  transaction (the adopted crash semantics). Publication = commit.
- **Hosts pull**: each connected host receives desired-state deltas over its mTLS channel;
  on reconnect it resynchronizes — the desired-state table is the protocol state.
- **Events are rows**: an append-only event table is the outbound stream (audit, metering,
  placement decisions, notifications); consumers poll or tail it.

**Message bus = a scale component, deliberately deferred** (operator decision). Its eventual
job is **async task execution** (bulk operations, fleet-wide rollouts, checkpoint
compaction), not order transport. **Seam rule**: when a bus is introduced, it is a *consumer
of the event table and a dispatcher of async tasks* — order flow never moves onto it. The
event table is the permanent contract; the bus is an optimization behind it.

## 3. Service decomposition (E-10)

Three services total; one per plane plus the shared trust anchor:

| Service | Plane | Contents |
|---|---|---|
| **tenant-plane** | tenant | API front (REST + gRPC mirror), **scheduler as in-process component** (shares the placement transaction), per-host desired-state computation, migration workflow state machine, quota enforcement, metering events |
| **admin-plane** | admin | fleet registry + lifecycle, version registry + deploy gates, image pipeline triggers (`ImportImage` → controlled rebuild, `build-worker` — E-17), monitoring ingestion, user administration |
| **KMS** | shared trust anchor (owned by neither plane, E-8) | DEK wrap/unwrap, token minting, host enrollment (attestation-gated) |

- **Scheduler in-process**: PERF-7 (1,000 decisions/s) is one process's transaction budget;
  a separate scheduler would split the transactional placement A-7 mandates. If scale ever
  demands extraction, the extraction point is the event table — not a re-architecture.
- **One Rust workspace, role-selected at startup** (I-13 extended to the control plane):
  `porche tenant-plane`, `porche admin-plane`, `porche kms`, `porche host-agent`,
  `porche storage-worker`, `porche build-worker` — same audited codebase, shared
  protocol/state types. Serialization boundaries between our own components disappear.
- **Host identity, two channels**: the host-agent presents one host identity (certificate)
  to both planes; the admin channel carries lifecycle authority, the tenant channel carries
  workload desired state (E-8 rule 1).

## 4. Language and stack (E-10)

**Rust everywhere** (CT-2 hardened): the tenant API parses untrusted input; the host-agent
is already Rust (doc 07); one language = one audit surface. Go acceptable for CI tooling
only; C forbidden outside audited components (CT-2 unchanged).

**Boring, battle-tested choices only** (operator decision):

| Layer | Choice | Why |
|---|---|---|
| Runtime | tokio | the default async runtime; nothing exotic in the trust path |
| RPC | tonic (gRPC) | typed contracts, mTLS, streaming — the host channel + API gRPC mirror |
| DB access | sqlx | compile-time-checked SQL; no ORM magic in the transactional core |
| State store | PostgreSQL | already the corpus assumption (wrapped DEKs); transactional placement needs ACID |
| Deployment | static binaries in the FCOS derivative image (E-5) | no runtime dependencies on hosts |

## 5. State store and availability (E-10)

**One PostgreSQL cluster, shared by both planes, separate databases** (`tenant`, `admin`,
`kms`): one HA unit to operate instead of two or three (CT-3), with plane isolation
preserved at the database level (separate credentials; no cross-plane grants).

- **Topology**: primary + **synchronous** streaming replica + automated failover
  (Patroni-style). Synchronous = zero lost committed orders; failover is the entire OPS-1
  story (99.9% = 43 min/month budget — proven by drill before the claim, A-8).
- **Audit Merkle chain** (SEC-23): append-only table in the `admin` database, fed by both
  planes' events (audit is unified — E-8 rule 5), with periodic head-signing.
- **KMS state**: wrapped DEKs in the `kms` database; the KEK never enters Postgres (A-5
  as re-gated by E-15: labeled dev-mode KEK authorized for P0/P1; HSM/quorum + key
  ceremony = the GA gate — in either mode the KEK stays outside Postgres).
- **Scale check**: ≤ ~1,000 hosts, one user per host — object counts are small; the only
  hot path is transactional order flow/placement, and it is one primary's job by design.

## 6. Host-agent ↔ control-plane protocol

gRPC over mTLS (tonic), **host dials out only** (I-11 — zero inbound ports on hosts):

- One long-lived bidirectional stream per plane: desired-state deltas down, reports and
  events up; reconnect with backoff + full resync of the desired-state table.
- Orders are idempotent and versioned (client retry-safe — doc 09 conventions); the
  desired-state table makes redelivery harmless (I-12).
- **Console recording** (FR-6, E-19): the host-agent records each workload's serial
  console to a per-workload **local** log — lifetime-bounded (destroyed with the workload;
  ≤ TTL + 24 h for stateful); the tenant reads it over the tenant channel for their own
  workloads. No central console store exists in v1.
- **Route rows** (E-20): cross-host workload /32 routes are desired-state rows compiled
  into each host's root ns (doc 14 §3) — static, no BGP or dynamic routing protocol in
  v1; the route table stays the seam where BGP enters later.
- **Migration workflow** (FR-34, E-3) runs as a state machine in the tenant-plane service:
  `requested → preparing (target: volume lease + KMS scoped unwrap) → transferring (CLH
  live migration) → cutover → done`, with **checkpoint migration as the failure fallback**
  at every state. DEK availability on two hosts during the window = time-bounded,
  volume-scoped **lease extension in the KMS** (SEC-24: unwrap requires host identity —
  both hosts hold identities; the lease bounds *what* and *how long*).

## 7. Explicitly excluded from v1

Message bus (async tasks at scale later, §2), service mesh, Kubernetes, etcd/ZooKeeper,
per-plane databases, multi-region, read replicas for the tenant API, ORM, anything not in
§4's table in the trust path. Each can enter later **behind the event table or the plane
boundary** — the two seams this design preserves on purpose.

## 8. Open points

- [ ] KMS API surface (wrap/unwrap, migration lease extension, token minting) — spec
      before P1.
- [ ] Desired-state table on the host (SQLite vs file-based) — decide with the PoC
      host-agent (doc 07 open point).
- [ ] Metering aggregation cadence from the event table (FR-42 precision vs load).
- [ ] Admin-plane monitoring ingestion (metrics ride the admin channel — volume and
      retention to size).
- [ ] Failover drill procedure (A-8: prove OPS-1 before claiming it).
