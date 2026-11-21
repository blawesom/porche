#!/usr/bin/env bash
# 2026-11-19 discriminator — P1 only.
#  A) small P1 dedicated canary: does size (c2r2p1 vs c8r16p1) matter?
#  B) P1 default-tenancy probe (no dedicated group, no nested virt):
#     does the region have ANY P1 capacity, or is the block dedicated-specific?
#     Probe is transient: create, read state once, delete immediately.
set -u
cd "$(dirname "$0")"
export OSC_ACCESS_KEY=$(jq -r .use2.access_key ~/.osc/config.json)
export OSC_SECRET_KEY=$(jq -r .use2.secret_key ~/.osc/config.json)
export OSC_REGION=$(jq -r .use2.region ~/.osc/config.json)
IMG=ami-c804ee38
SG=sg-0fca547a
UD=userdata.b64
LOG=disc.log
: > "$LOG"

try() { # name vmt group sub [default]
  local name=$1 vmt=$2 group=$3 sub=$4 mode=${5:-dedicated}
  local ts=$(date -u +%H:%M:%S) nv="" ten=""
  [ "$mode" = default ] && ten=--Placement.Tenancy default
  out=$(octl iaas api CreateVms --profile use2 \
    --ImageId "$IMG" --VmType "$vmt" $nv \
    --SecurityGroupIds "$SG" \
    ${group:+--Placement.Tenancy "$group"} --Placement.SubregionName "$sub" \
    $ten --UserData "$UD" 2>&1)
  echo "$out" > "raw/$name.json"
  vmid=$(echo "$out" | jq -r '.Vms[0].VmId // empty' 2>/dev/null)
  code=$(echo "$out" | jq -r '.Errors[0].Code // empty' 2>/dev/null)
  typ=$(echo "$out" | jq -r '.Errors[0].Type // empty' 2>/dev/null)
  if [ -n "$vmid" ]; then
    echo "$ts $name $vmt $group $sub ($mode) => CREATED $vmid" | tee -a "$LOG"
    if [ "$mode" = default ]; then
      # transient discriminator: capture state, then delete
      st=$(octl iaas api ReadVms --profile use2 --VmIds "$vmid" 2>/dev/null \
           | jq -r '.Vms[0].State // empty')
      octl iaas api DeleteVms --profile use2 --VmIds "$vmid" >/dev/null 2>&1
      echo "$ts $name probe state=$st — DELETED (discriminator only)" | tee -a "$LOG"
    else
      echo "$vmid" > vm-id.txt; echo "$vmt" > vm-type.txt
      echo "$group" > vm-group.txt; echo "$sub" > vm-subregion.txt
      echo "$ts DEDICATED CANARY SUCCESS — kept" | tee -a "$LOG"
      exit 0
    fi
  else
    echo "$ts $name $vmt $group $sub ($mode) => FAIL ${code:-?} ${typ}" | tee -a "$LOG"
  fi
}

# A) small P1 dedicated canary across gens/AZs
try a1 tinav7.c2r2p1 ded-470f1199 us-east-2a
try a2 tinav7.c2r2p1 ded-bc88ffa1 us-east-2b
try a3 tinav6.c2r2p1 ded-ff33bd58 us-east-2a
try a4 tinav5.c2r2p1 ded-59d7fbe7 us-east-2a
# B) P1 default-tenancy discriminator (transient)
try b1 tinav7.c2r2p1 "" us-east-2a default
try b2 tinav6.c4r8p1 "" us-east-2a default
