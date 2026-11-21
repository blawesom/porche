#!/usr/bin/env bash
# Flatcar Outscale image import — us-west-1, 2026-11-20.
#
# WHY (recorded substitution, E-31.1 + E-5 fallback): FCOS cannot
# consume Outscale user-data — Ignition's Outscale provider landed in
# 2.28.0 (2026-10-26) and FCOS stable 45.20261030.3.1 still ships
# 2.27.0 (verified in its meta.json pkgdiff). The FCOS node
# (i-88b1e2b4, runs/20261120-node-usw1/) boots but Ignition 404s on
# the metadata user-data path -> no key -> inaccessible. Flatcar is
# the documented fallback host OS (doc 08 E-5); the Outscale variant
# with patched Ignition+Afterburn was verified working on Outscale by
# Kenshir0 (flatcar/Flatcar#2334, 2026-11-12: SSH key, hostname,
# Ignition provisioning). REVERT TRIGGER: FCOS stable ships
# Ignition >= 2.28 -> rebuild with build-fcos-omi.sh.
#
# Image: flatcar 4881.0.0+nightly-20261111-2100 outscale variant
#   (kernel 6.18.58 — the ACPI power-button regression in .57+ is
#   irrelevant here: doc 13 §4.13 — the node is never stopped
#   mid-burst; teardown is terminate).
#   qcow2.gz sha256 448262a56defaacfcdfd838bb97fa2bb72bb1c2353fbce4213545aea82efda0c
#   qcow2    sha256 686cf7c6c404629d73f420a3da46e876d01e664a622d1d935bf8aeccdbbd40a6
#   virtual size 13212057600 (qemu-img info; SnapshotSize must be this)
set -euo pipefail
cd "$(dirname "$0")"
# [sanitized pre-commit 2026-11-21: credentials live only in
# ~/.osc/config.json (the repo rule — never stored in the repo)]
export OSC_ACCESS_KEY OSC_SECRET_KEY OSC_REGION
OSC_ACCESS_KEY="$(jq -r .usw1.access_key ~/.osc/config.json)"
OSC_SECRET_KEY="$(jq -r .usw1.secret_key ~/.osc/config.json)"
OSC_REGION="$(jq -r .usw1.region ~/.osc/config.json)"

IMG_FILE=/tmp/opencode/flatcar/flatcar_production_outscale_image.qcow2
KEY=flatcar_production_outscale_image.qcow2
BUCKET=porche-usw1-images
IMAGE_NAME="porche-flatcar-4881.0.0-nightly20261111"
VIRTUAL_SIZE=13212057600
ROOT_DEVICE=/dev/sda1
PART_SIZE=$((64 * 1024 * 1024))

say() { printf '%s %s\n' "$(date -u +%H:%M:%S)" "$*"; }

# 0) local source checks
echo "686cf7c6c404629d73f420a3da46e876d01e664a622d1d935bf8aeccdbbd40a6  $IMG_FILE" \
  | sha256sum -c - || { echo "FATAL: qcow2 digest mismatch"; exit 1; }

# 1) multipart upload (skip if the object is already there)
# [lesson: HeadObject exits 1 on a missing object — under
# set -euo pipefail that kills the script; tolerate it]
have=$( (octl storage api HeadObject --Bucket "$BUCKET" --Key "$KEY" 2>/dev/null \
       | jq -r '.ContentLength // empty') || true)
if [ -n "$have" ]; then
  say "object already present ($have bytes) — skipping upload"
else
  upid=$(octl storage api CreateMultipartUpload --Bucket "$BUCKET" --Key "$KEY" 2>/dev/null \
         | jq -r '.UploadId')
  [ -n "$upid" ] || { echo "FATAL: no UploadId"; exit 1; }
  say "upload id: $upid"
  # parts go to a TEMP dir — never the script's CWD (the first run left
  # 596 MB of part-* in the run dir; caught pre-push 2026-11-21)
  PDIR=$(mktemp -d)
  split -b "$PART_SIZE" -d -a 3 "$IMG_FILE" "$PDIR/part-"
  i=0
  for part in "$PDIR"/part-*; do
    i=$((i + 1))
    octl storage api UploadPart --Bucket "$BUCKET" --Key "$KEY" --UploadId "$upid" \
      --PartNumber "$i" --Body "$part" --ContentLength "$(stat -c%s "$part")"
  done
  say "uploaded $i parts — completing from server-side ListParts"
  rm -rf "$PDIR"
  local_flags=()
  while read -r pn etag; do
    local_flags+=(--MultipartUpload.Parts."$(( ${#local_flags[@]} / 2 ))".PartNumber "$pn"
                  --MultipartUpload.Parts."$(( ${#local_flags[@]} / 2 ))".ETag "$etag")
  done < <(octl storage api ListParts --Bucket "$BUCKET" --Key "$KEY" --UploadId "$upid" 2>/dev/null \
           | jq -r '.Parts[] | "\(.PartNumber) \(.ETag)"' | sort -n)
  [ "${#local_flags[@]}" -gt 0 ] || { echo "FATAL: ListParts returned no parts"; exit 1; }
  # [lesson, seen twice now: completing IMMEDIATELY after the last part
  # errors (octl exits non-zero) — a retry ~minutes later succeeds.
  # Retry with visible errors; never let set -e swallow it silently.]
  ok=""
  for attempt in 1 2 3 4 5; do
    if cresp=$(octl storage api CompleteMultipartUpload --Bucket "$BUCKET" --Key "$KEY" \
               --UploadId "$upid" "${local_flags[@]}" 2>&1); then
      ok=1; break
    fi
    say "complete attempt $attempt failed: $cresp — retrying in 60 s"
    sleep 60
  done
  [ -n "$ok" ] || { echo "FATAL: CompleteMultipartUpload failed 5x"; exit 1; }
  obj_after=$(octl storage api HeadObject --Bucket "$BUCKET" --Key "$KEY" 2>/dev/null \
              | jq -r '.ContentLength // empty')
  [ -n "$obj_after" ] || { echo "FATAL: object absent after complete (retry later or abort $upid)"; exit 1; }
  say "object complete: $obj_after bytes"
fi

# 2) presign + snapshot
url=$(octl storage object presign "$KEY" --bucket "$BUCKET" --expires 15m --method GET 2>/dev/null \
      | grep -m1 -Eo 'https?://[^[:space:]]+')
[ -n "$url" ] || { echo "FATAL: presign failed"; exit 1; }
resp=$(octl iaas api CreateSnapshot --profile usw1 --FileLocation "$url" \
  --SnapshotSize "$VIRTUAL_SIZE" --Description "PORCHE Flatcar outscale 4881.0.0 nightly import")
snap=$(jq -r '.Snapshot.SnapshotId // empty' <<<"$resp")
[ -n "$snap" ] || { echo "FATAL: no SnapshotId: $resp"; exit 1; }
say "snapshot: $snap — waiting"
while :; do
  st=$(octl iaas api ReadSnapshots --profile usw1 --Filters.SnapshotIds "$snap" 2>/dev/null \
       | jq -r '.Snapshots[0].State // empty')
  [ "$st" = completed ] && break
  [ "$st" = error ] && { echo "FATAL: snapshot $snap error"; exit 1; }
  sleep 30
done

# 3) image
payload=$(jq -n --arg name "$IMAGE_NAME" --arg root "$ROOT_DEVICE" --arg snap "$snap" \
  '{ImageName: $name, Description: "PORCHE node OMI - Flatcar outscale 4881.0.0+nightly-20261111-2100",
    RootDeviceName: $root, Architecture: "x86_64",
    BlockDeviceMappings: [{DeviceName: $root,
      Bsu: {SnapshotId: $snap, VolumeType: "gp2", VolumeSize: 13, DeleteOnVmDeletion: true}}]}')
resp=$(octl iaas api CreateImage --profile usw1 --payload "$payload")
img=$(jq -r '.Image.ImageId // empty' <<<"$resp")
[ -n "$img" ] || { echo "FATAL: no ImageId: $resp"; exit 1; }
say "image: $img — waiting for available"
while :; do
  st=$(octl iaas api ReadImages --profile usw1 --Filters.ImageIds "$img" 2>/dev/null \
       | jq -r '.Images[0].State // empty')
  [ "$st" = available ] && break
  [ "$st" = error ] && { echo "FATAL: image $img error"; exit 1; }
  sleep 20
done

# 4) cleanup + record
octl storage object delete "$KEY" --bucket "$BUCKET" >/dev/null 2>&1 || true
echo "usw1 FLATCAR OK $snap $img $IMAGE_NAME" | tee -a flatcar-import-record.txt
say "DONE: snapshot $snap image $img"
