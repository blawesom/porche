#!/usr/bin/env bash
# WS-A node bring-up (doc 07 §8.2) — runs ON node 1 as root.
# Installs: CLH v54.0 (digest-pinned), the porche static binary, the guest
# platform image; applies the SEC-11 sysctl subset (modules_disabled LAST —
# irreversible until reboot); formats + mounts /dev/vdb (the "NVMe region" BSU)
# as the agent data dir.
#
# Bundle layout (scp this dir up):
#   node-bundle/porche                    static musl binary
#   node-bundle/kernel                    guest kernel
#   node-bundle/initramfs                 guest initramfs
#   node-bundle/manifest.txt              guest digests (§7.1)
#   node-bundle/cloud-hypervisor-static   CLH v54.0
#   node-bundle/image-digest              the platform image digest (dir name)
#   node-bundle/sysctl-assert.sh          harness assert
set -euo pipefail

[ "$(id -u)" -eq 0 ] || { echo "must run as root (netns, cryptsetup, sysctl)"; exit 1; }
B=${1:?usage: node-setup.sh <node-bundle-dir>}

DATA_DIR=/var/lib/porche
CLH_SHA256=4f4f98a3d661a40f3e5272627cc895a1b2e525f2f6bedf5f895f9cd21ba5c65b

echo "== CLH v54.0 =="
echo "$CLH_SHA256  $B/cloud-hypervisor-static" | sha256sum -c - \
    || { echo "CLH DIGEST MISMATCH — stop"; exit 1; }
install -m 0755 "$B/cloud-hypervisor-static" /usr/local/bin/cloud-hypervisor
/usr/local/bin/cloud-hypervisor --version

echo "== nested virt sanity =="
grep -q vmx /proc/cpuinfo || { echo "no vmx flag"; exit 1; }
[ -e /dev/kvm ] || { echo "no /dev/kvm"; exit 1; }
[ "$(cat /sys/module/kvm_intel/parameters/nested 2>/dev/null)" = "Y" ] \
    || echo "WARN: kvm_intel nested != Y (migration spike needs it; PoC single-host does not)"

echo "== porche + platform image =="
install -m 0755 "$B/porche" /usr/local/bin/porche
DIGEST=$(cat "$B/image-digest")
mkdir -p "$DATA_DIR/images/$DIGEST" "$DATA_DIR/volumes" "$DATA_DIR/consoles" "$DATA_DIR/run"
install -m 0644 "$B/kernel"    "$DATA_DIR/images/$DIGEST/kernel"
install -m 0644 "$B/initramfs" "$DATA_DIR/images/$DIGEST/initramfs"

echo "== NVMe region (tier-E volume backing): the second BSU =="
# Outscale maps the linked BSU to a SCSI device (observed: /dev/sdb link → /dev/sda
# inside the VM; device naming is platform-mapped, so find the non-root 200G disk)
REGION_DEV=$(lsblk -dn -o NAME,SIZE,TYPE | awk '$3=="disk" && $2=="200G" {print $1; exit}')
[ -n "$REGION_DEV" ] || { echo "could not find the 200G region disk"; lsblk; exit 1; }
REGION_DEV="/dev/$REGION_DEV"
echo "region disk: $REGION_DEV"
if ! blkid "$REGION_DEV" > /dev/null 2>&1; then
    mkfs.ext4 -q -L porche-nvme "$REGION_DEV"
fi
mkdir -p "$DATA_DIR"
if ! mountpoint -q "$DATA_DIR"; then
    # discard: fallocate --dig → ext4 freed extents → TRIM to the BSU (E-6)
    mount -o discard LABEL=porche-nvme "$DATA_DIR"
    grep -q "porche-nvme" /etc/fstab || \
        echo "LABEL=porche-nvme $DATA_DIR ext4 discard 0 2" >> /etc/fstab
fi

echo "== packages + module warm-up (before modules_disabled=1) =="
# the minimal Rocky OMI ships no nft/cryptsetup userspace; the seal is
# irreversible, so every module the agent needs is loaded AND VERIFIED here
# (modprobe with multiple names silently loads only the first — observed
# 2026-10-19; /sys/module catches built-ins too, unlike lsmod)
command -v nft > /dev/null 2>&1 || dnf install -y -q nftables
command -v cryptsetup > /dev/null 2>&1 || dnf install -y -q cryptsetup
command -v socat > /dev/null 2>&1 || dnf install -y -q socat
command -v nft > /dev/null 2>&1 || { echo "FATAL: nft unusable"; exit 1; }
command -v cryptsetup > /dev/null 2>&1 || { echo "FATAL: cryptsetup unusable"; exit 1; }
for m in bridge veth nf_tables dm_mod dm_crypt loop tun; do
    modprobe "$m" 2>/dev/null || true
done
for m in bridge veth nf_tables dm_mod dm_crypt loop tun; do
    [ -d "/sys/module/$m" ] || { echo "FATAL: kernel module $m not present — refusing to seal"; exit 1; }
done
# the TUN/TAP device node (CLH attaches to taps via /dev/net/tun)
[ -e /dev/net/tun ] || mknod /dev/net/tun c 10 200
[ -w /dev/net/tun ] || { echo "FATAL: /dev/net/tun unusable"; exit 1; }
# exercise them for real: veth pair + nft table + the workload bridge
ip link add porche-warmup type veth peer name porche-warmup2 2>/dev/null || true
ip link del porche-warmup 2>/dev/null || true
nft add table inet porche_warmup || { echo "FATAL: nft unusable — refusing to seal"; exit 1; }
nft delete table inet porche_warmup 2>/dev/null || true
ip link show brporche > /dev/null 2>&1 || ip link add brporche type bridge
ip link set brporche up
ip link show brporche > /dev/null || { echo "FATAL: brporche unusable"; exit 1; }

echo "== SEC-11 sysctl subset (doc 07 §7.2) =="
# -e: unknown keys tolerated (interim Rocky 5.14 may lack io_uring_disabled —
# the assert records the gap; FCOS (recorded run) has all four)
sysctl -e -w kernel.io_uring_disabled=2 || true
sysctl -w user.max_user_namespaces=0
sysctl -w kernel.core_pattern=core
# modules_disabled is IRREVERSIBLE until reboot — last, deliberately
sysctl -w kernel.modules_disabled=1

echo "== assert =="
"$B/sysctl-assert.sh" || echo "NOTE: assert gaps above (interim host — see doc 07 §7.1)"

echo "== spec files =="
mkdir -p /root/poc
IP6=fd42:1:1::1
GW=fd42:1:1::ff
for s in loop density perf1; do
    case $s in
        loop)    ID=loop1; VOL=256;;
        density) ID=d0;    VOL=128;;
        perf1)   ID=p1;    VOL=128;;
    esac
    jq -n --arg id "$ID" --arg d "$DIGEST" --argjson v "$VOL" \
       '{id:$id, image_digest:$d, command:"echo alive > /dev/console",
         vcpu:1, ram_mib:128, volume:{size_mib:$v}, ip6:"'$IP6'", gw6:"'$GW'"}' \
       > /root/poc/spec-$s.json
done
echo "specs in /root/poc/ (digest $DIGEST)"

echo "== done — node is PoC-ready =="
