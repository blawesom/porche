# infra/images — the FCOS OMI build

Imports the pinned Fedora CoreOS image as an OMI into **every in-scope
Outscale region**. OMIs are region-scoped: a build that exists in one
region is invisible in another (the 2026-11-19 us-west-1 probe hit
`5023 InvalidResource` on the use2 OMI), so the process targets all
in-scope regions by construction — currently `use2` (us-east-2) and
`usw1` (us-west-1); `euw2` stays out of scope (BOUNDARIES.md).

```sh
build-fcos-omi.sh                 # every in-scope region (use2 usw1)
build-fcos-omi.sh usw1            # one region
DRYRUN=1 build-fcos-omi.sh        # print the calls, mutate nothing
```

Behavior per region:

1. **Idempotent**: an already-available OMI of the pinned name
   (`porche-fcos-<build>`) makes that region a `SKIP` — re-running
   after a use2 build only builds the missing region.
2. Download + sha256-verify the pinned qcow2 once (shared across
   regions), multipart-upload it to an OOS bucket
   (`porche-<profile>-images`), presign (15 min — the snapshot import
   pulls server-side), `CreateSnapshot` with the qcow2 **virtual**
   size, `CreateImage`, wait for `available`, then clean the bucket
   object and bucket.
3. Every invocation writes a run record to
   `runs/<YYYYMMDD-HHMMSS>-omi-build/record.txt` — snapshot + image
   IDs per region. New IDs are then pinned where needed (doc 13
   §4.10 addendum, run records).

The pin (`FCOS_BUILD`, digests, virtual size) is at the top of the
script and **must stay the MS-5 line** — the guest bundle under
`guest/out` is built against that FCOS build, and the fixtures record
(doc 07 §7.1) keys off it. Bumping the pin is a deliberate change,
never a side effect.

All the OOS/octl mechanics encoded in the script are recorded lessons
from the two manual imports (`runs/20261020-MS5/ms5/01-omi-import.txt`,
`runs/20261023-use2-tenant/ms5/01-omi-import-use2.txt`): OOS needs
exported `OSC_*` credentials, the upload must be multipart (64 MiB
parts), the complete payload comes from the server-side `ListParts`,
`SnapshotSize` is the virtual size, and `CreateImage` needs the
`--payload` JSON form. `--Body` for `UploadPart` requires a real file.
Additional lessons from the first scripted run (2026-11-20):
`--raw` is an **iaas-api-only** global flag — the `storage api`
commands reject it (use their default JSON output; `ListParts`
prints `Parts: null` until parts exist — that, not a flag, was the
2026-10-23 "default output prints null" note).
