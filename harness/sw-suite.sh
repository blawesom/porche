#!/usr/bin/env bash
# MS-6 software-validation suite (doc 07 §7.5.1 — SOFTWARE GATES).
# Verdicts only properties of our code: teardown determinism, leak/wedge/drift,
# rollback cleanliness, idempotency, concurrency. Every timing number produced
# here is RECORDED (hardware-bound rows gate on the hardware run — E-16, v12).
#
# Runs ON node 1 as root against the live host-agent. Exit 0 = every software
# gate green; 1 = a gate failed (details in the run dir).
#
# Usage: sw-suite.sh <image-digest> <run-dir>
#   options via env:
#     SW_LOOP_CYCLES=1000      PERF-8 loop cycles (default 1000)
#     SW_ROLLBACK_CYCLES=10    deliberate-failure cycles (default 10)
#     SW_RECREATE_CYCLES=10    create/wipe/re-create cycles (default 10)
#     SW_CONCURRENCY=20        parallel creates for the smoke (default 20)
set -euo pipefail

[ "$(id -u)" -eq 0 ] || { echo "must run as root on node 1"; exit 1; }
IMG=${1:?usage: sw-suite.sh <image-digest> <run-dir>}
RD=${2:?usage: sw-suite.sh <image-digest> <run-dir>}
mkdir -p "$RD"
DATA=/var/lib/porche
AGENT="/usr/local/bin/porche host-agent --store $DATA/desired-state.sqlite3 --keys-file /etc/porche/keys.json"

fail=0
step() { printf 'sw: %-32s ' "$1"; }
pass() { echo "pass"; }
failmsg() { echo "FAIL — $1"; fail=1; }

SPEC=$(mktemp); trap 'rm -f "$SPEC"' EXIT
spec_json() { # $1=id $2=ip6 [ $3=volume-size-mib ]
  local vol=null
  [ -n "${3:-}" ] && vol="{\"size_mib\":$3}"
  cat >"$SPEC" <<EOF
{"tenant_id":"t0","id":"$1","image_digest":"$IMG","command":"sleep 600","vcpu":1,"ram_mib":128,"volume":$vol,"ip6":"$2","gw6":"fd42:1:1::ff"}
EOF
}

# --- gate 1: PERF-8 loop — wedge + drift + wipe probes -------------------------
# (loop + drift = software gates; per-cycle timing recorded as RECORD-ONLY)
LOOP_N=${SW_LOOP_CYCLES:-1000}
step "loop wedge+durability (n=$LOOP_N)"
mem_base=$(awk '/MemAvailable/{print $2}' /proc/meminfo)
: >"$RD/sw-loop.csv"
: >"$RD/sw-wedge.txt"
WEDGE=0
for i in $(seq 1 "$LOOP_N"); do
  id=$(printf 'sw%06d' $((i % 1000000)))
  spec_json "$id" "fd42:31:1::$(printf '%x' $((i % 65535 + 1)))" 64
  c0=$(date +%s%3N)
  if ! $AGENT create --spec "$SPEC" >>"$RD/sw-loop.log" 2>&1; then
    echo "cycle $i: CREATE FAILED" >>"$RD/sw-wedge.txt"; WEDGE=1; continue
  fi
  if ! $AGENT wipe "$id" >>"$RD/sw-loop.log" 2>&1; then
    echo "cycle $i: WIPE FAILED (probe)" >>"$RD/sw-wedge.txt"; WEDGE=1
  fi
  w1=$(date +%s%3N)
  echo "$i,$((w1 - c0))" >>"$RD/sw-loop.csv"
  [ $((w1 - c0)) -gt 60000 ] && { echo "cycle $i: WEDGE (>60s)" >>"$RD/sw-wedge.txt"; WEDGE=1; }
  if [ "$i" -eq 200 ] || [ "$i" -eq "$LOOP_N" ]; then
    mem=$(awk '/MemAvailable/{print $2}' /proc/meminfo)
    echo "cycle $i: MemAvailable=$mem (base $mem_base)" >>"$RD/sw-drift.txt"
    mem_last=$mem
  fi
done
[ "$WEDGE" -eq 0 ] && pass || failmsg "wedge events — see $RD/sw-wedge.txt"
[ ! -s "$RD/sw-wedge.txt" ] || failmsg "wedge log non-empty: $(wc -l <"$RD/sw-wedge.txt") events"

step "loop memory drift <5%"
drift=$(awk -v b="$mem_base" -v m="${mem_last:-$mem_base}" 'BEGIN{ printf "%.2f", (b - m) * 100 / b }')
echo "drift=${drift}% (base $mem_base → $mem_last)" >"$RD/sw-drift-summary.txt"
awk -v d="$drift" 'BEGIN{ exit (d < 5 && d > -100 ? 0 : 1) }' && pass || failmsg "drift ${drift}% out of budget — see $RD/sw-drift-summary.txt"

step "wipe probes (every loop cycle)"
# the agent exits nonzero on a failing §7.2 probe (main.rs Wipe); the loop's
# per-cycle wipe result is the probe gate
if grep -q 'WIPE FAILED' "$RD/sw-wedge.txt" 2>/dev/null; then
  failmsg "wipe probe failures recorded"
else
  pass
fi

# --- gate 2: rollback cleanliness ---------------------------------------------
# A create whose platform image is missing MUST end Failed with a FULL teardown:
# no netns, no root veth, no mapper, no keyfile entry, no region leftovers.
RB_N=${SW_ROLLBACK_CYCLES:-10}
step "rollback cleanliness (n=$RB_N)"
BADIMG="sha256:$(printf 'f%.0s' $(seq 64))"   # valid digest, no image dir
for i in $(seq 1 "$RB_N"); do
  id=$(printf 'rb%06d' "$i")
  spec_json "$id" "fd42:31:2::$(printf '%x' $i)" 64
  # rewrite with the deliberately missing digest
  sed -i "s|\"image_digest\":\"[^\"]*\"|\"image_digest\":\"$BADIMG\"|" "$SPEC"
  $AGENT create --spec "$SPEC" >>"$RD/sw-rollback.log" 2>&1 || true
  state=$($AGENT status "$id" 2>/dev/null | grep -c failed || true)
  [ "$state" -ge 1 ] || failmsg "cycle $i: row not Failed after failed create"
  # residue sweep
  ip netns list | grep -q "^w-$id " && failmsg "cycle $i: netns w-$id residue"
  ip link show "vr-$id" >/dev/null 2>&1 && failmsg "cycle $i: veth vr-$id residue"
  [ -e "/dev/mapper/porche-$id" ] && failmsg "cycle $i: mapper residue"
  grep -q "\"$id\"" /etc/porche/keys.json 2>/dev/null && failmsg "cycle $i: keys.json entry residue"
  [ -e "$DATA/volumes/$id.img" ] && failmsg "cycle $i: volume region residue"
  $AGENT wipe "$id" >/dev/null 2>&1 || true   # clear the Failed row
done
pass # individual failures already flagged

# --- gate 3: re-create idempotency --------------------------------------------
# create → wipe → same id again. Every cycle must converge; the netns-veth
# defect class (MS-1: netns del leaves the root veth) fails here loudly.
RC_N=${SW_RECREATE_CYCLES:-10}
step "re-create idempotency (n=$RC_N)"
for i in $(seq 1 "$RC_N"); do
  id=$(printf 'rc%02d' "$i")
  for round in 1 2; do
    spec_json "$id" "fd42:31:3::$(printf '%x' $i)"
    if ! $AGENT create --spec "$SPEC" >>"$RD/sw-recreate.log" 2>&1; then
      failmsg "cycle $i round $round: create FAILED (idempotency)"; break
    fi
    $AGENT wipe "$id" >>"$RD/sw-recreate.log" 2>&1 || failmsg "cycle $i round $round: wipe FAILED"
  done
done
grep -q 'File exists' "$RD/sw-recreate.log" 2>/dev/null && failmsg "RTNETLINK 'File exists' in recreate log"
pass

# --- gate 4: concurrency smoke -----------------------------------------------
# Parallel creates + concurrent reads: verdict = 0 errors, 0 lock timeouts.
# Durations land in sw-concurrency.csv as RECORD-ONLY (hardware-bound).
CN=${SW_CONCURRENCY:-20}
step "concurrency smoke (n=$CN)"
: >"$RD/sw-concurrency.csv"
pids=()
for i in $(seq 1 "$CN"); do
  id=$(printf 'cc%02d' "$i")
  spec_json "$id" "fd42:31:4::$(printf '%x' $i)"
  # per-id spec copy: $SPEC is rewritten by the next loop iteration before the
  # background create has necessarily read it
  s="$RD/spec-$id.json"; cp "$SPEC" "$s"
  ( t0=$(date +%s%3N)
    if $AGENT create --spec "$s" >>"$RD/sw-concurrency.log" 2>&1; then
      echo "$i,$(($(date +%s%3N) - t0))" >>"$RD/sw-concurrency.csv"
    else
      echo "concurrent create $id FAILED" >>"$RD/sw-concurrency.err"
    fi
    rm -f "$s" ) &
  pids+=($!)
  # concurrent reader against the same store
  ( $AGENT status >/dev/null 2>&1 || echo "concurrent status FAILED" >>"$RD/sw-concurrency.err" ) &
  pids+=($!)
done
for p in "${pids[@]}"; do wait "$p" || true; done
[ -s "$RD/sw-concurrency.err" ] && failmsg "$(wc -l <"$RD/sw-concurrency.err") concurrency errors" || pass
grep -qi 'database is locked' "$RD/sw-concurrency.log" 2>/dev/null && failmsg "sqlite lock timeout"

# cleanup the surviving concurrency workloads
for i in $(seq 1 "$CN"); do $AGENT wipe "$(printf 'cc%02d' "$i")" >/dev/null 2>&1 || true; done

# --- record-only banner -------------------------------------------------------
cat >"$RD/sw-record-only.txt" <<EOF
RECORD-ONLY (hardware-bound — gate on the hardware run, E-16/v12):
  loop per-cycle timing   $RD/sw-loop.csv
  concurrency durations   $RD/sw-concurrency.csv
EOF

echo "sw: done (fail=$fail) — verdict class: software gate (doc 07 §7.5.1)"
exit $fail