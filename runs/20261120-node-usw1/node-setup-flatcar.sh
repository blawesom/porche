#!/usr/bin/env bash
# Flatcar node setup — the fcos-node-setup.sh adaptation (MS-5, 2026-11-20).
# Recorded differences vs the FCOS variant:
#   - NO rpm-ostree layering: Flatcar ships nft/cryptsetup/socat natively
#     (verified on-node); qemu-img is absent but the host-agent never
#     invokes it (volume regions are LUKS sparse files — volume.rs).
#   - binaries live at /var/lib/porche/bin (region volume, writable):
#     Flatcar's /usr/local is read-only. The agent gets PORCHE_CLH_BIN.
#   - brporche is created HERE (the FCOS script relied on the Rocky
#     variant's step — gap closed).
# Run ON the node as root, bundle dir as $1.
set -euo pipefail
[ "$(id -u)" -eq 0 ] || { echo "must run as root"; exit 1; }
B=${1:?usage: node-setup-flatcar.sh <node-bundle-dir>}
DATA_DIR=/var/lib/porche
IMG_DIGEST=96291392360e3448e6bdf46ce9c702de3e889efad3272dbee4e4419aeb81652a

echo "== CLH v54.0 =="
CLH_SHA256=4f4f98a3d661a40f3e5272627cc895a1b2e525f2f6bedf5f895f9cd21ba5c65b
echo "$CLH_SHA256  $B/cloud-hypervisor-static" | sha256sum -c - \
    || { echo "CLH DIGEST MISMATCH — stop"; exit 1; }

echo "== nested virt sanity =="
grep -q vmx /proc/cpuinfo || { echo "no vmx flag"; exit 1; }
[ -e /dev/kvm ] || { echo "no /dev/kvm"; exit 1; }
[ "$(cat /sys/module/kvm_intel/parameters/nested 2>/dev/null)" = "Y" ] \
    || echo "WARN: kvm_intel nested != Y"

echo "== region BSU =="
REGION_DEV=""
for link in /dev/disk/by-id/*_sdb; do
    [ -e "$link" ] || continue
    REGION_DEV=$(readlink -f "$link"); break
done
if [ -z "$REGION_DEV" ]; then
    n=$(lsblk -dn -o NAME,SIZE,TYPE | awk '$3=="disk" && $2=="200G" {print $1; exit}')
    [ -n "$n" ] && REGION_DEV="/dev/$n"
fi
[ -n "$REGION_DEV" ] || { echo "could not find the region disk"; lsblk; exit 1; }
echo "region disk: $REGION_DEV"
if ! blkid "$REGION_DEV" > /dev/null 2>&1; then
    mkfs.ext4 -q -L porche-nvme "$REGION_DEV"
fi
mkdir -p "$DATA_DIR"
if ! mountpoint -q "$DATA_DIR"; then
    mount -o discard LABEL=porche-nvme "$DATA_DIR"
    mkdir -p /etc/systemd/system
    cat > /etc/systemd/system/var-lib-porche.mount <<UNIT
[Unit]
Description=PORCHE region volume (tier-E backing)
[Mount]
What=/dev/disk/by-label/porche-nvme
Where=$DATA_DIR
Type=ext4
Options=discard
[Install]
WantedBy=multi-user.target
UNIT
    systemctl daemon-reload
    systemctl enable var-lib-porche.mount >/dev/null 2>&1 || true
fi

echo "== binaries (region volume, /usr/local is read-only on Flatcar) =="
mkdir -p "$DATA_DIR/bin"
install -m 0755 "$B/porche"              "$DATA_DIR/bin/porche"
install -m 0755 "$B/porche-web"           "$DATA_DIR/bin/porche-web"
install -m 0755 "$B/cloud-hypervisor-static" "$DATA_DIR/bin/cloud-hypervisor"
"$DATA_DIR/bin/cloud-hypervisor" --version
"$DATA_DIR/bin/porche" --help >/dev/null

echo "== platform image (digest $IMG_DIGEST) =="
mkdir -p "$DATA_DIR/images/sha256:$IMG_DIGEST" "$DATA_DIR/volumes" "$DATA_DIR/consoles" "$DATA_DIR/run"
install -m 0644 "$B/kernel"    "$DATA_DIR/images/sha256:$IMG_DIGEST/kernel"
install -m 0644 "$B/initramfs"  "$DATA_DIR/images/sha256:$IMG_DIGEST/initramfs"
install -m 0644 "$B/manifest.txt" "$DATA_DIR/images/sha256:$IMG_DIGEST/manifest.txt"

echo "== bridge + modules =="
ip link show brporche > /dev/null 2>&1 || ip link add brporche type bridge
ip link set brporche up
ip link show brporche > /dev/null || { echo "FATAL: brporche unusable"; exit 1; }
for m in bridge veth nf_tables dm_mod dm_crypt loop tun; do
    modprobe "$m" 2>/dev/null || true
done
for m in bridge veth nf_tables dm_mod dm_crypt loop tun; do
    [ -d "/sys/module/$m" ] || { echo "FATAL: kernel module $m not present"; exit 1; }
done
[ -e /dev/net/tun ] || mknod /dev/net/tun c 10 200
[ -w /dev/net/tun ] || { echo "FATAL: /dev/net/tun unusable"; exit 1; }

echo "== SEC-11 sysctl subset =="
sysctl -w kernel.io_uring_disabled=2
sysctl -w user.max_user_namespaces=0
sysctl -w kernel.core_pattern=core
sysctl -w kernel.modules_disabled=1

echo "== assert =="
bash "$B/sysctl-assert.sh" || { echo "sysctl assert failed"; exit 1; }

echo "== Flatcar node is PoC-ready =="
