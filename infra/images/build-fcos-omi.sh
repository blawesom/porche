#!/usr/bin/env bash
# FCOS OMI build — import the pinned Fedora CoreOS image into EVERY
# in-scope Outscale region.
#
# WHY THIS EXISTS (doc 13 §4.10 addendum, 2026-11-19): OMIs are
# region-scoped — the use2 import (ami-c804ee38) does not exist in
# us-west-1 (5023 in the 20261119-redeploy probe). The image build
# therefore targets ALL in-scope regions from now on; the operator
# scope is use2 + usw1 (euw2 stays OUT — BOUNDARIES.md).
#
# Provenance: the 2026-10-23 use2 import
# (runs/20261023-use2-tenant/ms5/01-omi-import-use2.txt) and the
# MS-5.M1 original (runs/20261020-MS5/ms5/01-omi-import.txt), made
# repeatable. Comments marked [lesson] are mechanics recorded there —
# do not "simplify" them away.
#
# Usage:
#   build-fcos-omi.sh [profile ...]     default: use2 usw1
#   DRYRUN=1 build-fcos-omi.sh ...      print the calls, mutate nothing
#
# Per region (idempotent — an existing available OMI of the pinned
# name makes the region a SKIP):
#   multipart-upload the qcow2 to OOS bucket porche-<profile>-images
#   -> presign (900 s is enough: CreateSnapshot pulls server-side)
#   -> CreateSnapshot (the qcow2 VIRTUAL size, not the file size)
#   -> CreateImage -> wait available -> clean the bucket object
#   -> record "<profile> OK <snapshot> <image>" to the run record.
#
# Output: one run record per invocation under
#   runs/<YYYYMMDD-HHMMSS>-omi-build/record.txt   (+ stdout)
set -euo pipefail

# ------------------------------------------------------------------
# The pin — must stay the MS-5 line (the guest bundle in guest/out is
# built against this FCOS build; doc 07 §7.1 fixtures record).
# ------------------------------------------------------------------
FCOS_BUILD=44.20260913.3.2
FCOS_VARIANT=openstack
XZ_SHA256=ff3079bf7a5bd799ffbf974ad8c161ee06255948d909f9e1a954f3665f0ce901
QCOW2_SHA256=59f357c672dc8f8323fe49afbe96ab60382d55cca62fdbfe306f0071baf2bba9
VIRTUAL_SIZE=10737418240   # 10 GiB — qcow2 VIRTUAL size [lesson MS-5.M1:
                           # the compressed file size lands VolumeSize=1]
ROOT_DEVICE=/dev/sda1
ROOT_SIZE_GIB=10
PART_SIZE=$((64 * 1024 * 1024))  # 64 MiB parts [lesson: ~80 s/part, 31 parts
                                 # ≈ 42 min; a single PutObject busts the
                                 # 900 s shell ceiling]
IMAGE_NAME="porche-fcos-${FCOS_BUILD}"
BASE_URL="https://builds.coreos.fedoraproject.org/prod/streams/stable/builds/${FCOS_BUILD}/x86_64"
XZ_FILE="fedora-coreos-${FCOS_BUILD}-${FCOS_VARIANT}.x86_64.qcow2.xz"
QCOW2_FILE="${XZ_FILE%.xz}"     # the object key is the uncompressed qcow2
DESCRIPTION="PORCHE host OMI — FCOS stable ${FCOS_BUILD}, ${FCOS_VARIANT} variant"

# ------------------------------------------------------------------
# Scope guard: ONLY in-scope profiles (operator decision 2026-11-19,
# infra/integration/BOUNDARIES.md). euw2 (eu-west-2) and anything else
# are refused fail-closed. Edit this array only with BOUNDARIES.md open.
# ------------------------------------------------------------------
IN_SCOPE=(use2 usw1)

# ------------------------------------------------------------------
# Environment knobs
# ------------------------------------------------------------------
WORK="${WORK:-/tmp/opencode/fcos-omi}"       # download + split parts live here
DRYRUN="${DRYRUN:-0}"                        # 1 = print calls, mutate nothing
SNAP_TIMEOUT_MIN="${SNAP_TIMEOUT_MIN:-60}"  # snapshot import completion bound

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
RUN_DIR="${REPO_ROOT}/runs/$(date -u +%Y%m%d-%H%M%S)-omi-build"
RECORD="${RUN_DIR}/record.txt"
OSC_CONFIG="${OSC_CONFIG:-$HOME/.osc/config.json}"

mkdir -p "$WORK"

say()  { printf '%s %s\n' "$(date -u +%H:%M:%S)" "$*"; }
note() { printf '%s\n' "$*" >> "$RECORD"; printf '%s\n' "$*"; }

# run-or-print: the DRYRUN escape hatch. Everything mutating goes
# through r(); read-only calls (Read*, List*) are always executed so a
# DRYRUN still reports per-region SKIP vs BUILD truthfully.
r() {
  if [ "$DRYRUN" = 1 ]; then
    printf '%s %s DRY:' "$(date -u +%H:%M:%S)" "$PROFILE"
    printf ' %q' "$@"
    printf '\n'
  else
    "$@"
  fi
}

# profile env for OOS calls [lesson 2026-10-23: octl storage does NOT
# read iaas-style profiles — OSC_* must be exported per region]
set_profile_env() {
  PROFILE=$1
  export OSC_ACCESS_KEY OSC_SECRET_KEY OSC_REGION
  OSC_ACCESS_KEY="$(jq -r ".\"$PROFILE\".access_key" "$OSC_CONFIG")"
  OSC_SECRET_KEY="$(jq -r ".\"$PROFILE\".secret_key" "$OSC_CONFIG")"
  OSC_REGION="$(jq -r ".\"$PROFILE\".region" "$OSC_CONFIG")"
  [ -n "$OSC_ACCESS_KEY" ] && [ "$OSC_ACCESS_KEY" != null ] \
    || { say "REFUSED: no usable '$PROFILE' profile in $OSC_CONFIG"; exit 1; }
}

guard_profile() {
  local p=$1 ok=0
  for s in "${IN_SCOPE[@]}"; do [ "$p" = "$s" ] && ok=1; done
  [ "$ok" = 1 ] || {
    say "REFUSED: profile '$p' is out of scope (in scope: ${IN_SCOPE[*]})."
    say "Boundary changes go through infra/integration/BOUNDARIES.md."
    exit 1
  }
}

# ------------------------------------------------------------------
# Source acquisition (once — shared by every region)
# ------------------------------------------------------------------
ensure_source() {
  cd "$WORK"
  if [ "$DRYRUN" = 1 ] && [ ! -f "$QCOW2_FILE" ]; then
    say "DRY: would download + sha256-verify ${XZ_FILE}, decompress to $QCOW2_FILE"
    cd - >/dev/null
    return 0
  fi
  if [ ! -f "$XZ_FILE" ]; then
    say "downloading ${XZ_FILE} ..."
    curl -fL --retry 3 -o "$XZ_FILE" "${BASE_URL}/${XZ_FILE}"
  fi
  echo "$XZ_SHA256  $XZ_FILE" | sha256sum -c - \
    || { say "FATAL: xz digest mismatch for $XZ_FILE"; exit 1; }
  if [ ! -f "$QCOW2_FILE" ]; then
    xz -dk "$XZ_FILE"
  fi
  echo "$QCOW2_SHA256  $QCOW2_FILE" | sha256sum -c - \
    || { say "FATAL: qcow2 digest mismatch for $QCOW2_FILE"; exit 1; }
  # [lesson 2026-10-23: no qemu-img on the import box — the virtual size
  # is asserted from the pin, not re-derived locally]
  say "source ok: $QCOW2_FILE ($(stat -c%s "$QCOW2_FILE") bytes, virtual size pinned $VIRTUAL_SIZE)"
  cd - >/dev/null
}

# ------------------------------------------------------------------
# One region
# ------------------------------------------------------------------
build_region() {
  local profile=$1
  guard_profile "$profile"
  set_profile_env "$profile"
  local bucket="porche-${profile}-images"
  local pdir="${WORK}/${profile}"
  mkdir -p "$pdir"

  say "== $profile ($OSC_REGION): $IMAGE_NAME =="

  # --- idempotency: an existing available OMI of the pinned name -> SKIP
  local existing
  existing=$(octl iaas api ReadImages --profile "$profile" \
    --Filters.ImageNames "$IMAGE_NAME" 2>/dev/null \
    | jq -r '[.Images[] | select(.State == "available") | .ImageId] | first // empty')
  if [ -n "$existing" ]; then
    note "$profile SKIP $existing (available image of the pinned name exists)"
    say "$profile: SKIP — $existing already available"
    return 0
  fi

  ensure_source

  # --- OOS bucket (create if missing; tolerate an existing bucket)
  if ! octl storage api ListBuckets 2>/dev/null \
       | jq -e --arg b "$bucket" '[.Buckets[]?.Name] | index($b) != null' >/dev/null; then
    if ! r octl storage bucket create --bucket "$bucket"; then
      say "bucket create refused — assuming '$bucket' already exists; continuing"
    fi
  fi

  # --- resumability: if the object is already in the bucket (a prior
  # run completed the upload but failed later — e.g. at the snapshot
  # import), the whole multipart phase is a no-op.
  local obj_bytes
  obj_bytes=$( (octl storage api HeadObject --Bucket "$bucket" --Key "$QCOW2_FILE" 2>/dev/null \
       | jq -r '.ContentLength // empty') || true)
  if [ -n "$obj_bytes" ]; then
    say "object already present ($obj_bytes bytes) — skipping upload"
  else

  # --- multipart upload [all lessons from the 2026-10-23 run]
  local upid
  if [ "$DRYRUN" = 1 ]; then
    say "DRY: would CreateMultipartUpload bucket=$bucket key=$QCOW2_FILE"
    upid="DRYRUN-UPLOADID"
  else
    local resp
    resp=$(octl storage api CreateMultipartUpload \
      --Bucket "$bucket" --Key "$QCOW2_FILE" 2>/dev/null) \
      || { say "FATAL: CreateMultipartUpload failed"; return 1; }
    upid=$(jq -r '.UploadId // empty' <<<"$resp")
    [ -n "$upid" ] || { say "FATAL: no UploadId in response: $resp"; return 1; }
  fi
  say "upload id: $upid"

  # abort hint if anything below fails [lesson: abort cleanly, then
  # verify no dangling upload via ListMultipartUploads]
  trap 'say "upload $upid left open — abort with: octl storage api AbortMultipartUpload --Bucket '"$bucket"' --Key '"$QCOW2_FILE"' --UploadId '"$upid"' ; verify ListMultipartUploads"' RETURN

  # split into numbered parts (needs the real file; DRYRUN without a
  # local source prints the flow — part count from the MS-5 record)
  local n
  if [ -f "${WORK}/${QCOW2_FILE}" ]; then
    rm -f "${pdir}"/part-*
    split -b "$PART_SIZE" -d -a 3 "${WORK}/${QCOW2_FILE}" "${pdir}/part-"
    local parts=("${pdir}"/part-*)
    n=${#parts[@]}
    say "uploading $n parts of $((PART_SIZE / 1048576)) MiB ..."
    local i=0
    for part in "${parts[@]}"; do
      i=$((i + 1))
      r octl storage api UploadPart --Bucket "$bucket" --Key "$QCOW2_FILE" \
        --UploadId "$upid" --PartNumber "$i" \
        --Body "$part" --ContentLength "$(stat -c%s "$part")"
      # [lesson: --Body - (stdin) fails immediately — a real file is required]
    done
  elif [ "$DRYRUN" = 1 ]; then
    n=31
    say "DRY: would split ${QCOW2_FILE} into ~$n parts of $((PART_SIZE / 1048576)) MiB and UploadPart each"
  else
    say "FATAL: ${WORK}/${QCOW2_FILE} missing after ensure_source"; return 1
  fi

  # --- complete from the SERVER-side ListParts [lesson: the authoritative
  # ETags are the server's; the raw output is the usable one — the default
  # output prints null. ETags carry embedded quotes — pass raw, do not
  # re-quote]
  local complete_flags=(--Bucket "$bucket" --Key "$QCOW2_FILE" --UploadId "$upid")
  if [ "$DRYRUN" = 1 ]; then
    say "DRY: would CompleteMultipartUpload from server-side ListParts ($n parts)"
  else
    local -a parts_json=()
    while read -r pn etag; do
      parts_json+=(--MultipartUpload.Parts."$(( ${#parts_json[@]} / 2 ))".PartNumber "$pn"
                   --MultipartUpload.Parts."$(( ${#parts_json[@]} / 2 ))".ETag "$etag")
    done < <(octl storage api ListParts --Bucket "$bucket" --Key "$QCOW2_FILE" \
             --UploadId "$upid" 2>/dev/null \
             | jq -r '.Parts[] | "\(.PartNumber) \(.ETag)"' | sort -n)
    [ "${#parts_json[@]}" -gt 0 ] \
      || { say "FATAL: ListParts returned no parts"; return 1; }
    # [lesson 2026-11-20 (x2): completing IMMEDIATELY after the last
    # part errors — retry with visible errors, never a silent set -e death]
    local cok=""
    for attempt in 1 2 3 4 5; do
      if cresp=$(octl storage api CompleteMultipartUpload "${complete_flags[@]}" "${parts_json[@]}" 2>&1); then
        cok=1; break
      fi
      say "complete attempt $attempt failed: $cresp — retrying in 60 s"
      sleep 60
    done
    [ -n "$cok" ] || { say "FATAL: CompleteMultipartUpload failed 5x"; return 1; }
    # [lesson 2026-11-20: CompleteMultipartUpload can exit 0 while the
    # upload stays open (S3-style 200-with-error — seen when completing
    # immediately after the last part; a retry 30 min later worked).
    # Assert server-side before proceeding.]
    local obj_after
    obj_after=$( (octl storage api HeadObject --Bucket "$bucket" --Key "$QCOW2_FILE" 2>/dev/null \
         | jq -r '.ContentLength // empty') || true)
    if [ -z "$obj_after" ]; then
      say "FATAL: CompleteMultipartUpload returned but the object is absent — upload $upid likely still open; re-run (the parts stay), or abort it"
      return 1
    fi
  fi
  trap - RETURN
  fi # end: object-already-present skip

  # --- presign [lesson: 900 s is enough — CreateSnapshot pulls the object
  # server-side, the window only covers the CreateSnapshot call]
  local url
  if [ "$DRYRUN" = 1 ]; then
    url="DRYRUN-PRESIGNED-URL"
    say "DRY: would presign s3 key $QCOW2_FILE (15 min, GET)"
  else
    url=$(octl storage object presign "$QCOW2_FILE" --bucket "$bucket" \
      --expires 15m --method GET 2>/dev/null \
      | grep -m1 -Eo 'https?://[^[:space:]]+') \
      || { say "FATAL: presign produced no URL"; return 1; }
  fi

  # --- snapshot [lesson: SnapshotSize = the qcow2 VIRTUAL size]
  local snap
  if [ "$DRYRUN" = 1 ]; then
    snap="DRYRUN-SNAP"
    say "DRY: would CreateSnapshot --FileLocation <presigned> --SnapshotSize $VIRTUAL_SIZE"
  else
    local resp
    resp=$(octl iaas api CreateSnapshot --profile "$profile" \
      --FileLocation "$url" --SnapshotSize "$VIRTUAL_SIZE" \
      --Description "PORCHE OMI import — FCOS ${FCOS_BUILD}") \
      || { say "FATAL: CreateSnapshot failed"; return 1; }
    snap=$(jq -r '.Snapshot.SnapshotId // empty' <<<"$resp")
    [ -n "$snap" ] || { say "FATAL: no SnapshotId in response: $resp"; return 1; }
    say "snapshot: $snap — waiting for completed (bound: ${SNAP_TIMEOUT_MIN} min)"
    local waited=0
    while :; do
      local st
      st=$(octl iaas api ReadSnapshots --profile "$profile" \
        --Filters.SnapshotIds "$snap" 2>/dev/null \
        | jq -r '.Snapshots[0].State // empty')
      [ "$st" = completed ] && break
      [ "$st" = error ] && { say "FATAL: snapshot $snap in error state"; return 1; }
      [ "$waited" -ge $((SNAP_TIMEOUT_MIN * 60)) ] \
        && { say "FATAL: snapshot $snap not completed after ${SNAP_TIMEOUT_MIN} min"; return 1; }
      sleep 30; waited=$((waited + 30))
    done
  fi

  # --- image [lesson: the flat --SnapshotId flag does not exist and the
  # --BlockDeviceMappings.0.* flag form hits "too many arguments" — use
  # --payload JSON with RootDeviceName set]
  local img
  if [ "$DRYRUN" = 1 ]; then
    img="DRYRUN-IMAGE"
    say "DRY: would CreateImage (payload JSON, root $ROOT_DEVICE)"
  else
    local payload
    payload=$(jq -n --arg name "$IMAGE_NAME" --arg desc "$DESCRIPTION" \
      --arg root "$ROOT_DEVICE" --arg snap "$snap" --argjson gib "$ROOT_SIZE_GIB" \
      '{ImageName: $name, Description: $desc, RootDeviceName: $root,
        Architecture: "x86_64",
        BlockDeviceMappings: [{DeviceName: $root,
          Bsu: {SnapshotId: $snap, VolumeType: "gp2",
                VolumeSize: $gib, DeleteOnVmDeletion: true}}]}')
    local resp
    resp=$(octl iaas api CreateImage --profile "$profile" --payload "$payload") \
      || { say "FATAL: CreateImage failed"; return 1; }
    img=$(jq -r '.Image.ImageId // empty' <<<"$resp")
    [ -n "$img" ] || { say "FATAL: no ImageId in response: $resp"; return 1; }
    say "image: $img — waiting for available"
    local waited=0
    while :; do
      local st
      st=$(octl iaas api ReadImages --profile "$profile" \
        --Filters.ImageIds "$img" 2>/dev/null \
        | jq -r '.Images[0].State // empty')
      [ "$st" = available ] && break
      [ "$st" = error ] && { say "FATAL: image $img in error state"; return 1; }
      [ "$waited" -ge 3600 ] && { say "FATAL: image $img not available after 60 min"; return 1; }
      sleep 20; waited=$((waited + 20))
    done
  fi

  # --- cleanup [lesson: the completed snapshot no longer references the
  # presigned URL — the bucket object and bucket are deletable]
  r octl storage object delete "$QCOW2_FILE" --bucket "$bucket"
  r octl storage bucket delete "$bucket"

  note "$profile OK $snap $img"
  say "$profile: OK — snapshot $snap, image $img"
}

# ------------------------------------------------------------------
# main
# ------------------------------------------------------------------
PROFILES=("$@")
[ ${#PROFILES[@]} -eq 0 ] && PROFILES=("${IN_SCOPE[@]}")

# validate every profile BEFORE creating anything — a refused call
# must leave no run directory behind
for p in "${PROFILES[@]}"; do guard_profile "$p"; done

mkdir -p "$RUN_DIR"
{
  echo "FCOS OMI build — $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "pin: FCOS stable $FCOS_BUILD ($FCOS_VARIANT), image $IMAGE_NAME"
  echo "profiles: ${PROFILES[*]} (in scope: ${IN_SCOPE[*]})"
  echo "dryrun: $DRYRUN"
  echo "work: $WORK"
} > "$RECORD"

fail=0
for p in "${PROFILES[@]}"; do
  build_region "$p" || fail=1
done

note "done (fail=$fail)"
exit "$fail"
