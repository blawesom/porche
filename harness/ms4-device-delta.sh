#!/usr/bin/env bash
# MS-4 escape-delta class 2: CLH device-model surface (SEC-3/4, E-18).
# Hardened = the PoC's frozen device set (virtio-block disk + one virtio-net
#   on the tap + file serial — exactly what VmConfig emits; nothing else).
# Baseline = expanded device set: adds a virtio-rng and a second net iface
#   on a shared (non-netns) bridge — device-model surface the hardening
#   exists to deny.
# Probe: from inside the guest, enumerate the virtio device tree
#   (/sys/bus/virtio/devices) — the delta is the visible device count/types.
# SAFETY (§7.4): everything runs inside our own guest VM.
set -euo pipefail
[ "$(id -u)" -eq 0 ] || { echo "run on node 1 as root"; exit 1; }
RD=${1:?usage: ms4-device-delta.sh <run-dir>}
mkdir -p "$RD"
# Tool root: override with PORCHE_BIN_DIR (Flatcar: /var/lib/porche/bin)
BIN="${PORCHE_BIN_DIR:-/usr/local/bin}"
AGENT="$BIN/porche host-agent --store /var/lib/porche/desired-state.sqlite3 --keys-file /etc/porche/keys.json"
export PORCHE_CLH_BIN="$BIN/cloud-hypervisor"
IMG=$(ls /var/lib/porche/images/ | head -1)

# ---- hardened: the standard agent create (frozen device set) ----
HARD_SPEC=/tmp/ms4-dev-h.json
cat > "$HARD_SPEC" <<EOF
{"id":"devh","image_digest":"$IMG","command":"for d in /sys/bus/virtio/devices/virtio*; do busybox cat \$d/device/vendor \$d/device/device > /dev/console; busybox echo -n VEND; done; busybox echo; echo DEVICE-ENUM-DONE","vcpu":1,"ram_mib":256,"volume":{"size_mib":64},"ip6":"fd42:45::1","gw6":"fd42:1:1::ff"}
EOF
$AGENT create --spec "$HARD_SPEC" >"$RD/hardened.create" 2>&1 || echo CREATE-FAIL >> "$RD/hardened.create"
for _ in $(seq 1 120); do grep -q 'DEVICE-ENUM-DONE' "/var/lib/porche/consoles/devh.log" 2>/dev/null && break; sleep 0.5; done
# mechanism evidence, hardened side: the agent's OWN live vm.info (the
# 2026-11-21 fix: the console-enumeration regex matched nothing on either
# leg — a silent non-probe; vm.info is the stable signal on both sides)
curl -s --unix-socket /var/lib/porche/run/devh.sock -X GET http://localhost/api/v1/vm.info > "$RD/hardened.vminfo.json" 2>/dev/null || true
cp /var/lib/porche/consoles/devh.log "$RD/hardened.console" 2>/dev/null || true
$AGENT wipe devh >>"$RD/hardened.wipe" 2>&1 || echo WIPE-FAIL >> "$RD/hardened.wipe"

# ---- baseline: same VM hand-driven via the CLH API with EXPANDED devices ----
# (rng + second net iface on the host bridge — the legacy surface)
SOCK=/tmp/ms4-dev-base.sock
CONSOLE=/var/lib/porche/consoles/devb-clh.log
ip link show brporche >/dev/null 2>&1 || ip link add brporche type bridge
ip link set brporche up
tap2=tap-ms4base
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
"$BIN/cloud-hypervisor" --api-socket "$SOCK" >/dev/null 2>&1 &
CLH_PID=$!
sleep 1
cat > /tmp/ms4-baseline.json <<JSON
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
curl -s --unix-socket "$SOCK" -X PUT http://localhost/api/v1/vm.create -H 'Content-Type: application/json' -d @/tmp/ms4-baseline.json >"$RD/baseline.create" 2>&1
curl -s --unix-socket "$SOCK" -X PUT http://localhost/api/v1/vm.boot >>"$RD/baseline.create" 2>&1
# mechanism evidence, baseline side: live vm.info (rng + net count)
curl -s --unix-socket "$SOCK" -X GET http://localhost/api/v1/vm.info > "$RD/baseline.vminfo.json" 2>/dev/null || true
for _ in $(seq 1 120); do grep -q 'DEVICE-ENUM-DONE' "$CONSOLE" 2>/dev/null && break; sleep 0.5; done
sleep 1
curl -s --unix-socket "$SOCK" -X PUT http://localhost/api/v1/vm.shutdown >/dev/null 2>&1 || true
sleep 2
kill $CLH_PID 2>/dev/null || true
cp "$CONSOLE" "$RD/baseline.console" 2>/dev/null || true
ip link del $tap2 2>/dev/null || true

# ---- delta analysis (shell — no python3; the 2026-11-21 fix: the
# console-enumeration regex above matched nothing on either leg (sysfs
# cat renders as separate lines), a silent non-probe. The verdict basis
# is now BOTH sides' live vm.info: net iface count + rng presence) ----
count_taps() { grep -o '"tap"'  "$RD/$1.vminfo.json" 2>/dev/null | wc -l; }
count_rng()  { grep -c  '"rng"' "$RD/$1.vminfo.json" 2>/dev/null || true; }
hv=$(count_taps hardened); hr=$(count_rng hardened)
bv=$(count_taps baseline);  br=$(count_rng baseline)
{
  echo "hardened vm.info: taps=$hv rng=$hr (the agent's frozen VmConfig)"
  echo "baseline vm.info: taps=$bv rng=$br (hand-driven expanded set)"
  echo "(console VEND counts, weak probe only: hardened=$(grep -c VEND "$RD/hardened.console" 2>/dev/null || true) baseline=$(grep -c VEND "$RD/baseline.console" 2>/dev/null || true))"
  if [ "${br:-0}" -gt "${hr:-0}" ] || [ "${bv:-0}" -gt "${hv:-0}" ]; then
    echo "DELTA: present — baseline exposes extra devices (rng and/or net iface)"
    echo "verdict: the frozen allow-list denies the expanded surface (E-18 delta; no absolute claim)"
  else
    echo "DELTA: none (SEC-31 note: same device surface both sides)"
  fi
} | tee "$RD/verdict.txt"
echo done
