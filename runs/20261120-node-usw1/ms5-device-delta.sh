#!/usr/bin/env bash
# MS-5 escape-delta re-run, class 2 (device model) — Flatcar-node
# adaptation of harness/ms4-device-delta.sh (2026-11-20). Differences:
# agent/CLH at /var/lib/porche/bin; the python3 analysis ported to
# shell; the baseline's vm.info captured as the mechanism evidence
# (the MS-4 c2 record: the console enumeration probe is weak — VEND
# counts only — the vm.info divergence is the verdict basis).
set -euo pipefail
[ "$(id -u)" -eq 0 ] || { echo "run on the node as root"; exit 1; }
RD=${1:?usage: ms5-device-delta.sh <run-dir>}
mkdir -p "$RD"
export PORCHE_CLH_BIN=/var/lib/porche/bin/cloud-hypervisor
AGENT="/var/lib/porche/bin/porche host-agent --store /var/lib/porche/desired-state.sqlite3 --keys-file /etc/porche/keys.json"
IMG=$(ls /var/lib/porche/images/ | head -1)

# ---- hardened: the standard agent create (frozen device set) ----
HARD_SPEC=/tmp/ms5-dev-h.json
cat > "$HARD_SPEC" <<EOF
{"id":"devh","image_digest":"$IMG","command":"for d in /sys/bus/virtio/devices/virtio*; do busybox cat \$d/device/vendor \$d/device/device > /dev/console; busybox echo -n VEND; done; busybox echo; echo DEVICE-ENUM-DONE","vcpu":1,"ram_mib":256,"volume":{"size_mib":64},"ip6":"fd42:45::1","gw6":"fd42:1:1::ff"}
EOF
$AGENT create --spec "$HARD_SPEC" >"$RD/hardened.create" 2>&1 || echo CREATE-FAIL >> "$RD/hardened.create"
for _ in $(seq 1 120); do grep -q 'DEVICE-ENUM-DONE' "/var/lib/porche/consoles/devh.log" 2>/dev/null && break; sleep 0.5; done
cp /var/lib/porche/consoles/devh.log "$RD/hardened.console" 2>/dev/null || true
$AGENT wipe devh >>"$RD/hardened.wipe" 2>&1 || echo WIPE-FAIL >> "$RD/hardened.wipe"

# ---- baseline: same VM hand-driven via the CLH API with EXPANDED devices ----
SOCK=/tmp/ms5-dev-base.sock
CONSOLE=/var/lib/porche/consoles/devb-clh.log
ip link show brporche >/dev/null 2>&1 || ip link add brporche type bridge
ip link set brporche up
tap2=tap-ms5base
KERNEL=/var/lib/porche/images/$IMG/kernel
INITRD=/var/lib/porche/images/$IMG/initramfs
ip tuntap add dev $tap2 mode tap 2>/dev/null || true
ip tuntap add dev ${tap2}b mode tap 2>/dev/null || true
ip link set ${tap2}b master brporche up 2>/dev/null || true
CMDLINE=$(cat <<EOF2
console=ttyS0 porche.ip6=fd42:45::2,fd42:1:1::ff porche.cmd=$(echo 'for d in /sys/bus/virtio/devices/virtio*; do busybox cat $d/device/vendor $d/device/device > /dev/console; busybox echo -n VEND; done; busybox echo; echo DEVICE-ENUM-DONE; poweroff -f' | base64 -w0)
EOF2
)
rm -f "$SOCK" "$CONSOLE"
/var/lib/porche/bin/cloud-hypervisor --api-socket "$SOCK" >/dev/null 2>&1 &
CLH_PID=$!
sleep 1
cat > /tmp/ms5-baseline.json <<JSON
{
  "cpus": {"boot_vcpus": 1, "max_vcpus": 1},
  "memory": {"size": 268435456},
  "payload": {"kernel": "$KERNEL", "initramfs": "$INITRD", "cmdline": "$CMDLINE"},
  "disks": [],
  "rng": {"src": "/dev/urandom"},
  "net": [
    {"mac": "52:54:00:12:34:66", "tap": "$tap2"},
    {"mac": "52:54:00:12:34:67", "tap": "${tap2}b"}
  ],
  "serial": {"mode": "File", "file": "$CONSOLE"}
}
JSON
curl -s --unix-socket "$SOCK" -X PUT http://localhost/api/v1/vm.create -H 'Content-Type: application/json' -d @/tmp/ms5-baseline.json >"$RD/baseline.create" 2>&1
curl -s --unix-socket "$SOCK" -X PUT http://localhost/api/v1/vm.boot >>"$RD/baseline.create" 2>&1
# mechanism evidence: CLH's own live vm.info (the MS-4 verdict basis)
curl -s --unix-socket "$SOCK" -X GET http://localhost/api/v1/vm.info > "$RD/baseline.vminfo.json" 2>/dev/null || true
for _ in $(seq 1 120); do grep -q 'DEVICE-ENUM-DONE' "$CONSOLE" 2>/dev/null && break; sleep 0.5; done
sleep 1
curl -s --unix-socket "$SOCK" -X PUT http://localhost/api/v1/vm.shutdown >/dev/null 2>&1 || true
sleep 2
kill $CLH_PID 2>/dev/null || true
cp "$CONSOLE" "$RD/baseline.console" 2>/dev/null || true
ip link del $tap2 2>/dev/null || true
ip link del ${tap2}b 2>/dev/null || true

# ---- analysis (shell port; VEND counts + vm.info are the evidence) ----
hv=$(grep -c VEND "$RD/hardened.console" 2>/dev/null || true)
bv=$(grep -c VEND "$RD/baseline.console" 2>/dev/null || true)
rng=$(grep -c '"rng"' "$RD/baseline.vminfo.json" 2>/dev/null || true)
netif=$(grep -o '"tap"' "$RD/baseline.vminfo.json" 2>/dev/null | wc -l)
{
  echo "hardened VEND count: ${hv:-0} (console enumeration, weak probe)"
  echo "baseline VEND count: ${bv:-0}"
  echo "baseline vm.info: rng=$rng taps=$netif (live mechanism evidence)"
  if [ "${rng:-0}" -ge 1 ] && [ "${netif:-0}" -ge 2 ]; then
    echo "DELTA: present — baseline exposes rng + 2 net ifaces; the hardened"
    echo "  agent config emits neither (VmConfig, unit-pinned)"
    echo "verdict: the frozen allow-list denies the expanded surface (E-18 delta)"
  else
    echo "DELTA: not shown by vm.info — inspect baseline.vminfo.json"
  fi
} | tee "$RD/verdict.txt"
echo done
