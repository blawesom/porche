#!/usr/bin/env bash
# Deploy the FCOS node on the ONE stock cell found: gen-5 @ us-west-1b
# (group porche-usw1-g5-1b, ded-d326ad5a). Image comes from the
# 2026-11-20 OMI build (runs/<ts>-omi-build/record.txt).
set -euo pipefail
cd "$(dirname "$0")"
# [sanitized pre-commit 2026-11-21: credentials live only in
# ~/.osc/config.json (the repo rule — never stored in the repo)]
export OSC_ACCESS_KEY OSC_SECRET_KEY OSC_REGION
OSC_ACCESS_KEY="$(jq -r .usw1.access_key ~/.osc/config.json)"
OSC_SECRET_KEY="$(jq -r .usw1.secret_key ~/.osc/config.json)"
OSC_REGION="$(jq -r .usw1.region ~/.osc/config.json)"

IMG=$(grep -h '^usw1 FLATCAR OK ' "$(dirname "$0")"/flatcar-import-record.txt | tail -1 | awk '{print $5}')
[ -n "$IMG" ] && [ "$IMG" != null ] || { echo "no usw1 image in the build record yet"; exit 1; }
echo "image: $IMG"

out=$(octl iaas api CreateVms --profile usw1 --ImageId "$IMG" \
  --VmType tinav5.c8r16p1 --NestedVirtualization \
  --Placement.Tenancy ded-d326ad5a --Placement.SubregionName us-west-1b \
  --SecurityGroupIds sg-6e78844f --UserData userdata.b64 2>&1)
json=$(tail -n +2 <<<"$out"); [ "$(head -c1 <<<"$out")" = "{" ] && json="$out"
VM=$(jq -r '.Vms[0].VmId // empty' <<<"$json")
[ -n "$VM" ] || { echo "FATAL: no VmId: $out"; exit 1; }
echo "$VM" > vm-id.txt; echo "vm: $VM"

ip=""
if [ -f eip-id.txt ] && [ -s eip-id.txt ]; then
  ip=$(cat eip-id.txt)
  # validate it still exists (a prior deploy's EIP, now unlinked)
  ok=$(octl iaas api ReadPublicIps --profile usw1 --Filters.PublicIpIds "$ip" 2>/dev/null \
       | jq -r '.PublicIps[0].PublicIpId // empty')
  [ -n "$ok" ] || ip=""
fi
if [ -z "$ip" ]; then
  ip=$(octl iaas api CreatePublicIp --profile usw1 2>/dev/null | jq -r '.PublicIp.PublicIpId')
fi
echo "$ip" > eip-id.txt
octl iaas api LinkPublicIp --profile usw1 --PublicIpId "$ip" --VmId "$VM" >/dev/null
addr=$(octl iaas api ReadPublicIps --profile usw1 --Filters.PublicIpIds "$ip" 2>/dev/null \
  | jq -r '.PublicIps[0].PublicIp')
echo "$addr" > node-ip.txt; echo "public ip: $addr"

echo "waiting for running..."
for i in $(seq 1 60); do
  st=$(octl iaas api ReadVms --profile usw1 --Filters.VmIds "$VM" 2>/dev/null \
    | jq -r '.Vms[0].State')
  [ "$st" = running ] && { echo "RUNNING: core@$addr"; exit 0; }
  sleep 10
done
echo "TIMEOUT waiting for $VM (last state: $st)"; exit 1
