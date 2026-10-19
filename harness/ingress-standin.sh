#!/bin/bash
# Ingress stand-in — PoC ONLY. A substitute for the P1 ingress path (doc 14 §5),
# recorded in doc 07 §7. Each piece stands in for one design element:
#
#   | stand-in                                       | design element (P1)                |
#   |------------------------------------------------|-------------------------------------|
#   | host address fd42:1:1::253/64 on brporche      | fabric reachability to the /128     |
#   | netns v6 forwarding + default via the host     | the routed /128 fabric uplink       |
#   | socat TCP4-LISTEN,range=<cidr>                 | ingress-proxy listener +            |
#   |                                                | IngressRule.source_ranges (I-14/E-2)|
#
# NOT here: the SG rule (platform perimeter — applied via octl), the workload
# itself (porche create). Run AFTER `porche create` (needs the netns to exist).
#
# Usage: ingress-standin.sh <workload-id> <guest-ip6> <public-port> <source-cidr>
# Example: ingress-standin.sh live1 fd42:1:1::2 30080 171.33.93.229/32
set -euo pipefail

ID=$1; GUEST_IP6=$2; PORT=$3; SOURCE=$4
HOST_FABRIC=fd42:1:1::253   # the host's stand-in presence on the workload fabric /64
GUEST_PORT=8080
NS=w-$ID
NSBR=br-$ID

# 1. host bridge + fabric address (idempotent)
ip link show brporche >/dev/null 2>&1 || ip link add brporche type bridge
ip link set brporche up
ip addr add "${HOST_FABRIC}/64" dev brporche 2>/dev/null || true

# 2. netns egress: forwarding + default route via the host (the fabric stand-in —
#    replies from the guest ride this; without it the netns drops off-/64 traffic)
ip netns exec "$NS" sysctl -qw net.ipv6.conf.all.forwarding=1
ip netns exec "$NS" ip route replace default via "$HOST_FABRIC" dev "$NSBR"

# 3. the listener: public port → workload /128, source-filtered (source_ranges)
pkill -f "socat TCP4-LISTEN:${PORT}," 2>/dev/null || true
nohup socat "TCP4-LISTEN:${PORT},fork,reuseaddr,range=${SOURCE}" \
    "TCP6:[${GUEST_IP6}]:${GUEST_PORT}" >"/var/log/porche-ingress-${ID}.log" 2>&1 &

echo "ingress stand-in: :${PORT} -> [${GUEST_IP6}]:${GUEST_PORT} (source ${SOURCE} only)"
echo "teardown: pkill -f 'socat TCP4-LISTEN:${PORT},' ; ip addr del ${HOST_FABRIC}/64 dev brporche"
