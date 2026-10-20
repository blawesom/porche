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
IMG=$(ls /var/lib/porche/images/ | head -1)

# ---- hardened: the standard agent create (frozen device set) ----
HARD_SPEC=/tmp/ms4-dev-h.json
cat > "$HARD_SPEC" <<EOF
{"id":"devh","image_digest":"$IMG","command":"ls /sys/bus/virtio/devices/ > /dev/vda 2>&1; for d in /sys/bus/virtio/devices/virtio*; do cat \$d/device/vendor \$d/device/device 2>/dev/null | tr '\\n' ' '; echo; done > /dev/console; echo DEVICE-ENUM-DONE","vcpu":1,"ram_mib":256,"volume":{"size_mib":64},"ip6":"fd42:45::1","gw6":"fd42:1:1::ff"}
EOF
/usr/local/bin/porche host-agent --store /var/lib/porche/desired-state.sqlite3 --keys-file /etc/porche/keys.json create --spec "$HARD_SPEC" >"$RD/hardened.create" 2>&1 || echo CREATE-FAIL >> "$RD/hardened.create"
for _ in $(seq 1 120); do grep -q 'DEVICE-ENUM-DONE' "/var/lib/porche/consoles/devh.log" 2>/dev/null && break; sleep 0.5; done
cp /var/lib/porche/consoles/devh.log "$RD/hardened.console" 2>/dev/null || true
/usr/local/bin/porche host-agent --store /var/lib/porche/desired-state.sqlite3 --keys-file /etc/porche/keys.json wipe devh >>"$RD/hardened.wipe" 2>&1 || echo WIPE-FAIL >> "$RD/hardened.wipe"

# ---- baseline: same VM hand-driven via the CLH API with EXPANDED devices ----
# (rng + second net iface on the host bridge — the legacy surface)
SOCK=/tmp/ms4-dev-base.sock
CONSOLE=/var/lib/porche/consoles/devb-clh.log
ip link show brporche >/dev/null 2>&1 || ip link add brporche type bridge
ip link set brporche up
tap2=tap-ms4base
ip tuntap add dev $tap2 mode tap 2>/dev/null || true
ip link set $tap2 master brporche up
KERNEL=/var/lib/porche/images/$IMG/kernel
INITRD=/var/lib/porche/images/$IMG/initramfs
CMDLINE=$(cat <<EOF2
console=ttyS0 porche.ip6=fd42:45::2,fd42:1:1::ff porche.cmd=$(echo 'for d in /sys/bus/virtio/devices/virtio*; do cat $d/device/vendor $d/device/device 2>/dev/null | tr "\n" " "; echo; done > /dev/console; echo DEVICE-ENUM-DONE; poweroff -f' | base64 -w0)
EOF2
)
rm -f "$SOCK" "$CONSOLE"
cloud-hypervisor --api-socket "$SOCK" >/dev/null 2>&1 &
CLH_PID=$!
sleep 1
curl -s --unix-socket "$SOCK" -X PUT http://localhost/vm.create -H 'Content-Type: application/json' -d @- >"$RD/baseline.create" 2>&1 <<JSON
{
  "cpus": {"boot_vcpus": 1, "max_vcpus": 1},
  "memory": {"size": 268435456},
  "payload": {"kernel": "$KERNEL", "initramfs": "$INITRD", "cmdline": "$CMDLINE"},
  "disks": [],
  "rng": {"src": "/dev/urandom"},
  "net": [
    {"mac": "52:54:00:12:34:66", "tap": "$tap2"},
    {"mac": "52:54:00:12:34:67", "tap": "$tap2"}
  ],
  "serial": {"mode": "File", "file": "$CONSOLE"}
}
JSON
for _ in $(seq 1 120); do grep -q 'DEVICE-ENUM-DONE' "$CONSOLE" 2>/dev/null && break; sleep 0.5; done
sleep 1
curl -s --unix-socket "$SOCK" -X PUT http://localhost/vm.power-button >/dev/null 2>&1 || true
sleep 2
kill $CLH_PID 2>/dev/null || true
cp "$CONSOLE" "$RD/baseline.console" 2>/dev/null || true
ip link del $tap2 2>/dev/null || true

# ---- delta analysis ----
python3 - "$RD" <<'EOF' | tee "$RD/verdict.txt"
import re, sys, pathlib
rd = pathlib.Path(sys.argv[1])
def devices(label):
    log = (rd / f"{label}.console").read_text(errors="replace") if (rd / f"{label}.console").exists() else ""
    ids = re.findall(r"0x([0-9a-f]{4}) 0x([0-9a-f]{4})", log)
    # virtio vendor 0x1af4; device ids: 0x1041 block, 0x1042 net, 0x1004? rng=0x1044, console 0x1043
    return set(ids)
h = devices("hardened"); b = devices("baseline")
print(f"hardened devices:  {sorted(h)}")
print(f"baseline devices:  {sorted(b)}")
extra = b - h
if extra:
    print(f"DELTA: present — baseline exposes extra virtio devices: {sorted(extra)}")
    print("verdict: the frozen allow-list denies the expanded surface (E-18 delta; no absolute claim)")
else:
    print("DELTA: none (SEC-31 note: same device surface both sides)")
EOF
echo done
