#!/usr/bin/env bash
# 2026-11-19 redeploy sweep — dedicated host, any gen, any AZ, P1 only.
# Real CreateVms calls; first success wins; every attempt logged (E-31.1).
set -u
cd "$(dirname "$0")"
export OSC_ACCESS_KEY=$(jq -r .use2.access_key ~/.osc/config.json)
export OSC_SECRET_KEY=$(jq -r .use2.secret_key ~/.osc/config.json)
export OSC_REGION=$(jq -r .use2.region ~/.osc/config.json)

IMG=ami-c804ee38        # porche-fcos-44.20260913.3.2 (MS-5 pin, re-verified available)
SG=sg-0fca547a          # porche-ms5-sg (staged 2026-10-30)
UD=userdata.b64         # Ignition 3.4: user core + porche_integ key (doc 13 §4.12)
LOG=sweep.log
mkdir -p raw

# name | vmttype | group | subregion
attempts=(
  "t01|tinav7.c8r16p1|ded-470f1199|us-east-2a"
  "t02|tinav7.c8r16p1|ded-bc88ffa1|us-east-2b"
  "t03|tinav6.c8r16p1|ded-ff33bd58|us-east-2a"
  "t04|tinav6.c8r16p1|ded-467a6840|us-east-2b"
  "t05|tinav5.c8r16p1|ded-59d7fbe7|us-east-2a"
  "t06|tinav5.c8r16p1|ded-22ca7d87|us-east-2b"
)

: > "$LOG"
for row in "${attempts[@]}"; do
  IFS='|' read -r name vmt group sub <<< "$row"
  ts=$(date -u +%H:%M:%S)
  out=$(octl iaas api CreateVms --profile use2 \
    --ImageId "$IMG" --VmType "$vmt" --NestedVirtualization \
    --SecurityGroupIds "$SG" \
    --Placement.Tenancy "$group" --Placement.SubregionName "$sub" \
    --UserData "$UD" 2>&1)
  echo "$out" > "raw/$name.json"
  vmid=$(echo "$out" | jq -r '.Vms[0].VmId // empty' 2>/dev/null)
  code=$(echo "$out" | jq -r '.Errors[0].Code // empty' 2>/dev/null)
  msg=$(echo "$out" | jq -r '.Errors[0].Details // .Errors[0].Type // empty' 2>/dev/null)
  if [ -n "$vmid" ]; then
    echo "$ts $name $vmt $group $sub => SUCCESS $vmid" | tee -a "$LOG"
    echo "$vmid" > vm-id.txt
    echo "$vmt" > vm-type.txt
    echo "$group" > vm-group.txt
    echo "$sub" > vm-subregion.txt
    exit 0
  fi
  echo "$ts $name $vmt $group $sub => FAIL ${code:-?} ${msg:0:60}" | tee -a "$LOG"
done
echo "ALL FAIL" | tee -a "$LOG"
exit 1
