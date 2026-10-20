#!/usr/bin/env bash
# MS-1.M1 node bring-up (doc 07 §8.2 WS-A, E-29 re-entry) — runs ON node 1 as root.
# Installs: CLH v54.0 (digest-pinned), the porche binary and the guest platform image
# when present in the bundle; applies the SEC-11 sysctl subset (modules_disabled LAST —
# irreversible until reboot); formats + mounts the region BSU (the tier-E volume
# backing) as /var/lib/porche. Later milestones run this recipe instead of rebuilding.
#
# Bundle layout (scp this dir up):
#   node-bundle/porche                    static musl binary
#   node-bundle/kernel                    guest kernel            (M2 onwards)
#   node-bundle/initramfs                 guest initramfs         (M2 onwards)
#   node-bundle/manifest.txt              guest digests (§7.1)    (M2 onwards)
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

echo "== porche binary + platform image =="
if [ -f "$B/porche" ]; then
    install -m 0755 "$B/porche" /usr/local/bin/porche
fi
if [ -f "$B/kernel" ] && [ -f "$B/initramfs" ] && [ -f "$B/image-digest" ]; then
    DIGEST=$(cat "$B/image-digest")
    mkdir -p "$DATA_DIR/images/$DIGEST" "$DATA_DIR/volumes" "$DATA_DIR/consoles" "$DATA_DIR/run"
    install -m 0644 "$B/kernel"    "$DATA_DIR/images/$DIGEST/kernel"
    install -m 0644 "$B/initramfs" "$DATA_DIR/images/$DIGEST/initramfs"
    [ -f "$B/manifest.txt" ] && install -m 0644 "$B/manifest.txt" "$DATA_DIR/images/$DIGEST/manifest.txt"
    echo "guest image installed at $DATA_DIR/images/$DIGEST"
else
    echo "NOTE: no guest image in bundle — host prep only (image lands in M2)"
fi

echo "== region BSU (tier-E volume backing) =="
# Platform device naming is mapped: the linked BSU can show under a different
# name than its SCSI identity (2026-10-20: region volume linked as /dev/sdb
# surfaces as /dev/sda). Detect by SERIAL via /dev/disk/by-id — never by name —
# with a 200G size fallback for hosts without by-id.
REGION_DEV=""
for link in /dev/disk/by-id/*_sdb; do
    [ -e "$link" ] || continue
    REGION_DEV=$(readlink -f "$link"); break
done
if [ -z "$REGION_DEV" ]; then
    # fallback: the non-root ~200G disk
    n=$(lsblk -dn -o NAME,SIZE,TYPE | awk '$3=="disk" && $2=="200G" {print $1; exit}')
    [ -n "$n" ] && REGION_DEV="/dev/$n"
fi
[ -n "$REGION_DEV" ] || { echo "could not find the region disk"; lsblk; exit 1; }
echo "region disk: $REGION_DEV ($(lsblk -dn -o SIZE "$REGION_DEV"))"
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
mkdir -p "$DATA_DIR/volumes" "$DATA_DIR/consoles" "$DATA_DIR/run"

echo "== packages + module warm-up (before modules_disabled=1) =="
# the minimal OMI ships no nft/cryptsetup userspace; the seal is irreversible,
# so every module the agent needs is loaded AND VERIFIED here (modprobe with
# multiple names silently loads only the first — observed 2026-10-19; /sys/module
# catches built-ins too, unlike lsmod)
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
# -e: unknown keys tolerated (interim kernels may lack io_uring_disabled —
# the assert records the gap; FCOS (recorded run) has all four)
sysctl -e -w kernel.io_uring_disabled=2 || true
sysctl -w user.max_user_namespaces=0
sysctl -w kernel.core_pattern=core
# modules_disabled is IRREVERSIBLE until reboot — last, deliberately
sysctl -w kernel.modules_disabled=1

echo "== assert =="
"$B/sysctl-assert.sh" || { echo "FATAL: sysctl assert failed — host out of the pool (doc 01 §8)"; exit 1; }

if [ -x /usr/local/bin/porche ]; then
    echo "== host-agent version =="
    /usr/local/bin/porche --version || /usr/local/bin/porche --help | head -3 || true
fi

echo "== done — node is PoC-ready =="