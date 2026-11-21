#!/usr/bin/env bash
# 2026-11-20 full-grid recheck — deployment option space, both in-scope regions.
# Grid: gen {4,5,6,7} x AZ {a,b} x perf {p1,p2,p3} x tenancy {group, plain dedicated}.
# Rules encoded (doc 13 §4.2 / MS-5 grid): dedicated groups accept ONLY p1
# (p2/p3 -> 3002 — one verification cell per region); plain "dedicated"
# tenancy accepts p1/p2/p3 and the NestedVirtualization flag.
# First success wins and stops the sweep. use2 carries the real node payload
# (FCOS + Ignition); usw1 carries a probe payload (Rocky public OMI) — a
# usw1 success is DELETED immediately (wrong image; stock proof only).
set -u
cd "$(dirname "$0")"
LOG=grid.log
: > "$LOG"

use2_env() {
  export OSC_ACCESS_KEY=$(jq -r .use2.access_key ~/.osc/config.json)
  export OSC_SECRET_KEY=$(jq -r .use2.secret_key ~/.osc/config.json)
  export OSC_REGION=$(jq -r .use2.region ~/.osc/config.json)
}
usw1_env() {
  # [sanitized pre-commit 2026-11-21: credentials live only in
  # ~/.osc/config.json (the repo rule — never stored in the repo)]
  export OSC_ACCESS_KEY OSC_SECRET_KEY OSC_REGION
  OSC_ACCESS_KEY="$(jq -r .usw1.access_key ~/.osc/config.json)"
  OSC_SECRET_KEY="$(jq -r .usw1.secret_key ~/.osc/config.json)"
  OSC_REGION="$(jq -r .usw1.region ~/.osc/config.json)"
}

# attempt <region> <mode> <gen> <az-letter> <perf> <tenancy> <image> [userdata]
attempt() {
  local region=$1 mode=$2 gen=$3 az=$4 perf=$5 ten=$6 img=$7 ud=$8
  local vmt="tinav${gen}.c8r16${perf}"
  local sub out vmid code typ
  case $region in
    use2) sub="us-east-2${az}" ;;
    usw1) sub="us-west-1${az}" ;;
  esac
  local udflag=()
  [ -n "$ud" ] && udflag=(--UserData "$ud")
  out=$(octl iaas api CreateVms --profile "$region" --ImageId "$img" \
    --VmType "$vmt" --NestedVirtualization \
    --Placement.Tenancy "$ten" --Placement.SubregionName "$sub" \
    "${udflag[@]}" 2>&1)
  # response may carry a one-line human prefix — find the JSON
  local json
  if [ "$(head -c1 <<<"$out")" = "{" ]; then json="$out"; else json=$(tail -n +2 <<<"$out"); fi
  vmid=$(jq -r '.Vms[0].VmId // empty' <<<"$json" 2>/dev/null)
  code=$(jq -r '.Errors[0].Code // empty' <<<"$json" 2>/dev/null)
  typ=$(jq -r '.Errors[0].Type // empty' <<<"$json" 2>/dev/null)
  if [ -n "$vmid" ]; then
    echo "$(date -u +%H:%M:%S) $region $mode g$gen-$az $perf => SUCCESS $vmid" | tee -a "$LOG"
    echo "$region $mode g$gen-$az $perf $vmt $ten $sub $vmid" > success.txt
    return 0
  fi
  echo "$(date -u +%H:%M:%S) $region $mode g$gen-$az $perf => ${code:-?} ${typ:-?}" | tee -a "$LOG"
  return 1
}

# ---------------- use2: the real node payload ----------------
use2_env
declare -A U2G=( [4a]=ded-8b6fe861 [4b]=ded-9559aac7
                  [5a]=ded-59d7fbe7 [5b]=ded-22ca7d87
                  [6a]=ded-ff33bd58 [6b]=ded-467a6840
                  [7a]=ded-470f1199 [7b]=ded-bc88ffa1 )
echo "== use2: groups, p1 (node shape) ==" | tee -a "$LOG"
for az in a b; do for gen in 7 6 5 4; do
  attempt use2 group "$gen" "$az" p1 "${U2G[${gen}${az}]}" ami-c804ee38 userdata.b64 && exit 0
done; done
echo "== use2: rule re-verify (p2 in a g7 group -> expect 3002) ==" | tee -a "$LOG"
attempt use2 group 7 a p2 "${U2G[7a]}" ami-c804ee38 userdata.b64 || true
echo "== use2: plain dedicated, p1/p2/p3 ==" | tee -a "$LOG"
for perf in p1 p2 p3; do for az in a b; do for gen in 7 6 5 4; do
  attempt use2 dedicated "$gen" "$az" "$perf" dedicated ami-c804ee38 userdata.b64 && exit 0
done; done; done

# ---------------- usw1: probe payload ----------------
usw1_env
declare -A W1G=( [4a]=ded-97adc6b0 [4b]=ded-892cf0a1
                 [5a]=ded-ccad54cd [5b]=ded-d326ad5a
                 [6a]=ded-8428ed0b [6b]=ded-4cba0d76
                 [7a]=ded-17bf03d6 [7b]=ded-31667ea0 )
echo "== usw1: groups, p1 (node shape) ==" | tee -a "$LOG"
for az in a b; do for gen in 7 6 5 4; do
  attempt usw1 group "$gen" "$az" p1 "${W1G[${gen}${az}]}" ami-4a4837a3 "" && exit 0
done; done
echo "== usw1: rule re-verify (p2 in a g7 group -> expect 3002) ==" | tee -a "$LOG"
attempt usw1 group 7 a p2 "${W1G[7a]}" ami-4a4837a3 "" || true
echo "== usw1: plain dedicated, p1/p2/p3 ==" | tee -a "$LOG"
for perf in p1 p2 p3; do for az in a b; do for gen in 7 6 5 4; do
  attempt usw1 dedicated "$gen" "$az" "$perf" dedicated ami-4a4837a3 "" && exit 0
done; done; done

echo "ALL CELLS FAIL" | tee -a "$LOG"
exit 1
