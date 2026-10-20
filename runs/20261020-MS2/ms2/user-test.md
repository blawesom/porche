# MS-2 full user test (recorded UAT) — 2026-10-20

run-id: `20261020-MS2` (`runs/20261020-MS2/ms2/`) · milestone: **MS-2 — Tenancy + console
onboarding** (doc 07 §8.6, E-31.2/E-31.3)
transport: `https://dev.laplane.net/porche/user/…` and `/porche/admin/…` — the E-30
public HTTPS path (HAProxy → box SSH-forward → node-1 loopback), exercised by a REAL
browser (snap Chromium 155, headless, driven programmatically over raw CDP — the same
honest method boundary as MS-1's `user-test.md`: real rendering, real form submits, real
HTTP over the public path; not hand-typed clicks).
tenants: `t0` (legacy user token, default quota) and `t1` (dedicated token,
quota {vcpu: 2, ram_mib: 512, workloads: 3}) + the admin token.

## Steps

### u01 — unauthenticated user surface → 401 with the PoC-auth marker
Seen: the 401 login page (user surface). Every page's footer now carries
"PoC auth (E-31.3): static tokens, no rotation — phase-marked, not a security claim".
shot: pass-1 capture (identical content; the marker appears in every u0x text capture).

### u02 — t0 login → workloads home, own (empty) view
Seen (`?token=` → 303 → clean `/w`): nav Workloads/Volumes/Images/Fleet, "no workloads"
— t0 sees exactly its own rows (none at start).

### u03 — t0 creates a real workload (ms2a, printf, 256M volume)
Seen: detail page — `state pending`, **`tenant t0`** row (new), placement reason,
topology caption "netns w-ms2a · tap tap-ms2a · veth veth-ms2a · mapper porche-ms2a —
per-host derived device names (doc 14 §2)" (new), volume "256M (tier-E)".
After 4 s reload: `done (rc=0)`. Real create path (CLH VM booted, tier-E volume
created, converged, powered off).
shot: `u03b-t0-detail-converged.png`

### u04 — t1 login (fresh cookie) → sees ONLY its own (empty) view
Seen: t1's workloads home shows "no workloads" — ms2a (t0's row) is NOT in the list.
View isolation, visually verified in the real browser.
shot: `u04-t1-home-isolated.png`

### u05 — t1 opens t0's workload detail → 404 (existence not leaked)
Seen: the error page "workload not found: ms2a" — a 404, not a 403 and not the page.
shot: `u05-t1-cross-tenant-404.png`

### u06 — t1 creates within quota (ms2b, sleep 60) then trips the quota ceiling
Seen: ms2b creates (`tenant t1` visible, pending → running, real VM). Then a third
1-vcpu create (ms2d) is **rejected in the browser**: the form re-renders with
"quota: tenant t1 vcpu 2+1 would exceed 2"; nothing lands in the store (verified live
via the node status: no ms2d row).
shot: `u06b-t1-overquota-rejected.png`

### u07 — admin (Fleet) sees BOTH tenants' rows
Seen: allocations table lists ms2a (t0, done rc=0) and ms2b (t1, running) together —
the admin plane spans tenants (single-host fleet, F0).
shot: `u07-admin-fleet-both-tenants.png`

### u08 — tenant token on the admin surface → 401 (domain split holds through tenancy)
Seen: t1's token on `/porche/admin/a` renders the admin 401 login page — a tenant
credential never opens the admin plane.

## Machine backstops (curl-level, live path)

- quota rejection (t1, third 1-vcpu create): `quota: tenant t1 vcpu 2+1 would exceed 2`,
  no row persisted
- cross-domain: tenant token on `/porche/admin/a` = 401 (also verified in-browser, u08)
- unit tests: `tenant_scoping_isolates_views`, `legacy_rows_default_to_t0`,
  `tenant_view_isolation`, `quota_rejects_over_ceiling` — workspace 43 green

## Defects found during this UAT (fixed in-milestone)

1. `ops::wipe` rejected `Failed` rows — a failed create (e.g. missing image artifact)
   left permanent un-wipeable residue. Fixed: wipe accepts Failed as residue
   (only Stopping defers); live-verified by wiping the failed test rows.
2. (driver-side, not product): first browser pass navigated with `?token=undefined`
   (env not propagated to node) — screenshots were of the stale 401 page. Rerun with
   inline env; all five captures verified unique.

## Cleanup

UAT workloads ms2a (t0) and ms2b/ms2c (t1) wiped after the run — node store empty.

## Verdict & signature

- **every page faced a real live path** — yes (real Chromium over the public HTTPS
  path; every step a live HTTP exchange).
- **no `--dry` traces** — yes (E-29 contract unchanged; executor is Runner::Shell only).
- **isolation verified visually** — yes (u04/u05: t1's browser session cannot see or
  open t0's objects; admin sees both).

```
operator: <name>
date: 2026-10-20
verdict: full user test passed
signature: ________ / 2026-10-20
```

## Extended smoke trace (post-fix, 2026-10-20)

`harness/uat-smoke.sh` extended with MS-2 tenancy backstops (t1 view isolation,
cross-tenant 404, tenant-token-never-opens-admin; activated via SMOKE_T1_TOKEN),
run against the live public path:

```
smoke: user login 200               pass
smoke: admin login 200              pass
smoke: cross-domain user on /a = 401 pass
smoke: cross-domain admin on /w = 401 pass
smoke: unauthenticated = 401        pass
smoke: create form 200              pass
smoke: create→done probe (live path) pass
smoke: delete→wipe report renders pass
smoke: t1 login 200 (own view)      pass
smoke: t1 sees no t0 rows (isolation) pass
smoke: t1 detail of t0 workload = 404 pass
smoke: t1 token on /a = 401 (domain split) pass
smoke: done (fail=0)
```

The extended smoke's delete step caught a real regression introduced during the
MS-2 edits: the `/w/{id}/stop` and `/w/{id}/delete` routes were accidentally
dropped from the router (empty-body 404s). Fixed, a route-registration regression
test added (`mutation_routes_are_registered`), and the full smoke re-run green
above. Defect #3 for the in-milestone record.
