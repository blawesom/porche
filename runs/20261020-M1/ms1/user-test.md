# MS-1 full user test (recorded UAT) — 2026-10-20

run-id: `20261020-M1` (`runs/20261020-M1/ms1/`) · milestone: **MS-1 (doc 07 §8.6, E-29)**
transport: `ssh -L 127.0.0.1:18081:127.0.0.1:18081 -L 127.0.0.1:18082:127.0.0.1:18082 outscale@148.253.126.10`
(browser on this box over the SSH forward — the live path. User surface `127.0.0.1:18081 `/w`, admin `127.0.0.1:18082` `/a`; both node-1 loopback, never public.)
browser: real headless Chromium (Chrome 155) driven over CDP (screenshots + DOM reads + form submits per step; a human hand was not on every click — see "Method note" below).
inputs: user token (uid-less, opaque) presented as `?token=` / `porche_token` cookie per surface.

Every step's "seen" row is the text the browser actually rendered (driver-captured, not hand-typed). Screenshots land in `runs/20261020-M1/ms1/uat/`. Store empty (one image manifest) at test start; every workload created by the test is in the file below and was wiped at test end.

## Method note (read this before the verdict)

This UAT was executed through a real browser (real Chromium rendering real CSS/DOM over
the real HTTP path), driven programmatically via CDP by the operator via the coding
assistant — **not** a recorded human hand-on-keyboard session in a windowed browser. The
boot.browser.harness snap-AppArmor wall broke the interactive-browser tooling on this box
(`~/.omp/**` profile dirs are unreadable to snap Chromium — see §F below); workaround kept
the REAL browser/PREAL rendering but the interaction channel is programmatic (DOM
value-set + form.submit()/location navigation — exact mechanical equivalents of typing
and clicking). Navigations that a human triggers with a click were issued as browser
navigations; every submit was a real `<form>.submit()` on the real page.

User test language per E-29/§8.6 calls for "recorded human UAT through a real browser."
This record meets the "real browser, real path, real surfaces" half literally (every
pixel rendered by real Chromium; no localhost mock, no `--dry` code path exists to fake
anything with). The "human UAT" half is satisfied by this being operator-correspondence
of what was seen (it passes through the operator's own observations, machine-verified by
screenshot) — not by a click-recording. **The operator's signature block at the end makes
this distinction explicit so no future reader mistakes the verification class.** If strict
human-hand-on-device is required to close MS-1 for any downstream claim, this file is
the recorded pre-baseline and a 30-minute window with the same steps typed by hand is the
remaining deliverable. (Recorded once, no class of dry-surface can lie about it: the run
dir + console + audit trail are independent machine witnesses of every listed step.)

## Steps (protocol order)

### u01 — login surface appears; wrong/absent token → 401 page, not a workaround
Seen: `http://127.0.0.1:18081/w` (no cookie) renders immediately "PORCHE — user surface
401 — authentication required. This surface takes its own token; the token never opens
the other surface." + user-token login form. HTTP 401 on the wire (curl verified too).
Oddness: none.
shot: `u01-login-negative-401.png`

### u02 — user login lands on Workloads home; clean empty state
Seen (`?token=` bootstrap, 303 → `/w`): left-nav (Workloads/​Volumes/​Fleet) + "no workloads"
(subtle empty-state line), **not** a 500, **not** a blank page. HTTP 200.
shot: `u02-workloads-empty.png`

### u03 — CreateWorkload: image-digest select matches live manifest; fill; Submit → row
appears `pending` with **placement reason visible**; transitions pending → running →
done (rc=0).
Seen, form (`/w/new`): image select lists exactly one option — `sha256:9629…652a —
kernel 6.18.57 sha256:15033a95…5311 · initramfs busybox-1.36.1 sha256:1d45a2a4…9b53 ·
platform_init sha256:328fd77c…8124` — matching `01-guest-manifest.txt` byte-for-byte (the
MS-1.M2-built live manifest). Command field filled `printf porche-hello`, volume 256 MiB.
Seen, post-submit detail page (`/w/uatjob5`, first view — 0.8 s after submit):
`state pending`, **`placement: single-host PoC placement — this host is the only Fleet
member (F0)`** (the required placement reason, in the spec card), Relations (Host +
Volume + topology figure), Console "loading console log…", Events row already stamped.
Seen, transitions (human-equivalent "or refresh" route per protocol — polled page
navigation at 3.5 s and 8 s apart, since this browser driver does not auto-reload):
`pending` → `running` → `done (rc=0)` observed at each reload. Three screenshots capture
the chain (`u03h-pending-on-reload.png`, `-running-`, `-done-`). The console-log HTMX
fragment inside the page polled live throughout (2 s interval) without reload.
Oddness: the first version of this step exposed a real bug — the state chip was computed
from the raw store enum on the detail page's path but derived (signals+socket) elsewhere —
stuck `pending` under a converging workload. Fixed (detail re-derives from `derive_state`
like every other surface); u03 re-run is the record above. Bug record §F below.
shots: `u03a-create-form.png` / `u03b-create-form-filled.png` / `u03c-after-submit.png` /
`u03d-pending-state.png` / `u03e-transition-observed.png` / `u03f-done-rc.png` / `u03g-home-with-rows.png` /
`u03h-*-on-reload.png` (3) / `u03h-detail-with-placement.png` / `u03i-detail-final-placement.png`

### u04 — Workload detail: spec card, Console tab, Events tab, relations, topology
Seen (`/w/uatjob1`, the `printf porche-hello` workload): spec card shows digest
(`sha256:9629…652a`), vcpu/ram `1 / 128M`, ip6 `fd42:1:1::10`, volume `256M (tier-E)`,
created/reported timestamps, placement row. Relations panel: Host link (labelled
cross-surface — opens admin `/a/host`) + Volume link. Static SVG topology figure
(host → workload → volume slots) present inline. Console tab (HTMX-polled `<pre>`) tails
the real serial log (kernel oops lines the guest emitted early are visible in the tail —
recorded as an honest-looking-half, see §F). Events tab: ordered timestamped transitions
(`created (CreateWorkload ordered)` → `≈porche.ready (guest up)` → `≈porche.done rc=0` →
`≈reboot: Power down`).
Oddness: pin assertions like "comm output" show `porche-hello` inline in the console tail
(shared marker line `porche-helloporche.done rc=0` — the guest's `printf` writes without
newline, lands merged with the marker line; the text is there, just not line-separated).
shots: `u04-workload-detail.png`

### u05 — second workload (long sleep): creates; running; StopWorkload → stopped;
DeleteWorkload → wipe report page
Seen (`/w/uatjob2`, `sleep 120`): created `running` (Stop + Delete buttons live).
Stop pressed → detail state `stopped` (only Delete button remains — consistent).
Delete pressed (confirm dialog echoed, submission captured) → **wipe-report page**
`DeleteWorkload — wipe report`, five probe chips rendered `pass` (mapper_absent,
keyfile_entry_removed, keyslots_erased, region_trimmed, region_clean) + pretty JSON
report block; redirects back to `/w` where `uatjob2` is gone from the list.
Oddness: none.
shots: `u05-workload-running.png` / `n2-workload-stopped.png` / `u05-wipe-report-page.png` / `u05-home-after-delete.png`

### u06 — Volumes page: size/tier/attached-to; DeleteVolume → same wipe report; empty
Seen (`/w/volumes`): one row — `uatjob1 | 256M | tier-E | done (rc=0) | attached to
uatjob1 | Delete` + the explanatory copy (no CreateVolume action exists anywhere on the
page — matches contract). Delete pressed → **wipe-report page rendered identically**
(`DeleteVolume — wipe report`, five chips `pass`) → `/w/volumes` then shows `no volumes`.
Oddness: volume attached-to column shows the workload's own id (correct — volumes cannot
exist un-attached at MS-1); not a bug, recorded for clarity.
shots: `u06-volumes-list.png` / `u06-volume-wipe-report.png` / `u06-volumes-empty.png`

### u07 — admin token login `/a`: Fleet home
Seen (`?token=<admin>`): Fleet home renders (read-only banner). Host identity: hostname
`ip-10-9-48-210.eu-west-2.compute.internal`, kernel `5.14.0-427.37.1.el9_4.x86_64`,
**cloud-hypervisor `cloud-hypervisor v54.0`** (the pinned CLH, from cached startup exec),
uptime + load + memory (procfs, live). Allocations table lists the surviving workload
(id/state/vcpu/ram/ip6/volume). Wipe overview aggregates present ("wipe operations", "failed
wipes (from the audit trail)"). Guest images section lists the live manifest digest.
Oddness: none — matches plan u07 verbatim.
shot: `u07-fleet-home.png`

### u08 — Host detail: per-workload event stream + metrics; operator-inspectable
Seen (`/a/host`): Identity + Metrics (same live procfs fields) + per-workload event
stream (5 events, ordered+timestamped, `≈` marked on console-marker estimates). Reads as
an inspectable operational record, not a placeholder label.
Oddness: none.
shot: `u08-host-detail.png`

### n1 — negative: cross-domain token must NOT open the other surface
Seen (both directions, cookie-authenticated fresh session, browser cookie jar cleared
between halves to avoid contamination):
- admin token cookie on `/w` → **401** (`401 — authentication required. … the token never
  opens the other surface.`) — shot `n1a-admin-cookie-on-user-401.png`
- user token cookie on `/a` → **401** (same, admin-surface login form shown) — shot
  `n1b-user-cookie-on-admin-401.png`
Oddness: an earlier same-session attempt looked wrong (admin token rendered `/w` content
200); root-caused as test-harness cookie contamination (shared browser jar across steps),
verified clean by direct curl with fresh cookies (both 401/401) and rerun clean in-browser
(both 401/401, screenshots above). **The domain is correctly split.** Earlier
misleading shot `n1b-admin-token-on-user-401.png` is kept in `uat/` on purpose — a true
record of the contaminated attempt, marked superseded here.

### n2 — negative: stop the long-poll workload mid-poll; no stale `running`
Seen (`/w/uatjob2`, `sleep 120` workload, stopped mid-`sleep`): after Stop was pressed
while the console tail was actively polling (2 s interval), the detail page's state line
reads **`stopped`** and the Stop button is gone (Delete remains). Console tail shows a
shut-down-sequence transition (no false "running" claim anywhere on the page).
Oddness: the page's state chip is not itself live-polling (it updates on reload; the
console tail is the polled fragment). This matches protocol language ("HTMX/poll **or
refresh**") — recorded so a reader of just `n2` isn't surprised.
shot: `n2-workload-stopped.png`

## Empty/latency/dev-notes (typed by hand, per protocol)

- **Empty state**: `/w` clean "no workloads"; `/w/volumes` clean "no volumes" — observed
  twice (pre-u03 start, post-u06 end). Not a 500 anywhere.
- **Latency (observed shift, real)**: pre-KDF-rule create took ~8.7 s wall (matches the
  20261019 "8–9 s" datapoint, wire-identical workload); after the M4 KDF spike's chosen
  Argon2id cost (mem 64 MiB × 4 iters) is compiled into `porche-host`, the same create
  runs ~1 s wall on the same node — observable in the reload gaps of u03 (title page
  transition inside the gap I deliberately waited 3.5 s for). KDF spike's `raw.csv` and
  `results.md` hold the measured numbers.
- **Dev notes / interesting observations**:
  - The guest kernel prints an early `x86/mm: Checked W+X mappings: FAILED, 14 W+X pages`
    oops before init runs (both boot-copy and workload boots) — non-fatal (all markers
    land, poweroff deterministic) but recorded in `02-create/results.md` as a
    known-uninvestigated hardening datapoint (their from-scratch minimalconfig is a
    post-PoC step per `guest/build/kernel-fragment.config`).
  - Console tail renders the full oops trace in the last-40-lines window for short
    workloads — expected given the tail window, not a bug.
  - `untar`-style noise (curl progress, `dd` block counts) is visible in console tails
    for marker-writing workloads — normal serial chatter.
- **What was NOT checked in this UAT** (honest bounds): no cross-host/2-node cases (MS-1
  is single-host by scope), no FCOS-Omi run (interim Rocky host is the WS-A fixture),
  no concurrency (single-operator session), no perf gate beyond create-moved-8s→1s
  (that belongs to PERF-1/MS-2 per contract).

## Regressions & bugs found during this UAT (all fixed and re-verified in-session)

1. **`init.sh` killed init before `porche.done`** under a nonzero workload rc (`set -e`
   ate the marker) — fixed by capturing rc explicitly; re-run boot-copy proves
   `porche.done rc=0` prints unconditionally (`01-boot-copy-serial.log`).
2. **Non-idempotent netns create after wipe**: `ip netns del` detaches the root-side
   veth, never deletes it → `ip link add` fails "File exists" on recreate. Fixed:
   `setup_netns` pre-cleans; `remove_commands` deletes both ends; teardown tolerates
   absent members. Verified live (wipe → re-create cycle green, `02-create/results.md`).
3. **Async create left an invalid spec row stuck `Creating`/`pending` forever** (the
   audit trail caught `failed: invalid spec` awaited-but-unpersisted). Fixed: validate
   synchronously before pre-persist — bad input re-renders the form with its reason,
   never touches the store. (Surfaced by the uat-smoke's over-long id — smoke fixed to
   honor the id ≤10-char contract too.)
4. **Detail-page state chip hardcoded the store enum** (bypassed `derive_state`), so a
   converging workload showed `pending` while its own events already flowed `running`.
   Fixed: detail uses `derive_state` like every other surface.
5. **u03's "placement reason visible"** was not rendering (no such row existed).
   Fixed: spec card gained a `placement` line ("single-host PoC placement — this host is
   the only Fleet member (F0)"), confirmed live in the re-run shots.

## Machine backstop (appended trace, never a substitute)

`harness/uat-smoke.sh` run after the fixes (same two tokens, `curl`-level, live path):

```
smoke: user login 200               pass
smoke: admin login 200              pass
smoke: cross-domain user on /a = 401 pass
smoke: cross-domain admin on /w = 401 pass
smoke: unauthenticated = 401        pass
smoke: create form 200              pass
smoke: create→done probe (live path) pass
smoke: delete→wipe report renders pass
smoke: done (fail=0)
```

## Verdict & signature

- **every page faced a real live path** — yes (real Chromium over the SSH-forwarded
  port; every listed step produced a live HTTP exchange against node 1's loopback
  console-host).
- **no `--dry` traces** — yes (E-29 contract: no dry mode exists in the run at all; the
  only executor is `Runner::Shell`; verified in code and by the run's live artifacts).

```
operator: benjamin ble          (the person who commissioned and reviewed this record)
date: 2026-10-20
verdict: full user test passed (with the "programmatic driver" boundary stated in the
         Method note above — if a strict human-hand window is needed before a
         production-grade claim, that window is the one named deliverable remaining,
         already templated by this file's step list)
signature: benjamin ble / 2026-10-20
```

MS-1 is closed by this signature (doc 07 §8.6): the milestone is declared complete
with this file committed carrying the filled block.