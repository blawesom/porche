#!/usr/bin/env bash
# MS-5 node setup — FCOS variant (doc 07 §8.2 WS-A re-entry, E-5: the recorded
# PoC run lands on FCOS). Adapts harness/node-setup.sh to an immutable OS:
# packages via rpm-ostree layering (one reboot), everything else identical.
# Run ON the FCOS node as root (sudo -i via the core user).
set -euo pipefail
[ "$(id -u)" -eq 0 ] || { echo "must run as root"; exit 1; }
B=${1:?usage: fcos-node-setup.sh <node-bundle-dir>}
DATA_DIR=/var/lib/porche

echo "== packages (rpm-ostree layering) =="
# layer the PoC userspace; nftables-libs pulls the CLI
if ! command -v cryptsetup >/dev/null 2>&1; then
  rpm-ostree install --allow-inactive --idempotent \
    cryptsetup nftables socat qemu-img 2>&1 | tail -2
  echo "REBOOT REQUIRED after layering" && exit 3
fi

echo "== CLH v54.0 =="
CLH_SHA256=4f4f98a3d661a40f3e5272627cc895a1b2e525f2f6bedf5f895f9cd21ba5c65b
echo "$CLH_SHA256  $B/cloud-hypervisor-static" | sha256sum -c - \
    || { echo "CLH DIGEST MISMATCH — stop"; exit 1; }
install -m 0755 "$B/cloud-hypervisor-static" /usr/local/bin/cloud-hypervisor
/usr/local/bin/cloud-hypervisor --version

echo "== nested virt sanity =="
grep -q vmx /proc/cpuinfo || { echo "no vmx flag"; exit 1; }
[ -e /dev/kvm ] || { echo "no /dev/kvm"; exit 1; }
[ "$(cat /sys/module/kvm_intel/parameters/nested 2>/dev/null)" = "Y" ] \
    || echo "WARN: kvm_intel nested != Y"

echo "== porche binaries + platform image =="
install -m 0755 "$B/porche" /usr/local/bin/porche
install -m 0755 "$B/porche-web" /usr/local/bin/porche-web
DIGEST=$(cat "$B/image-digest")
mkdir -p "$DATA_DIR/images/$DIGEST" "$DATA_DIR/volumes" "$DATA_DIR/consoles" "$DATA_DIR/run"
install -m 0644 "$B/kernel"    "$DATA_DIR/images/$DIGEST/kernel"
install -m 0644 "$B/initramfs" "$DATA_DIR/images/$DIGEST/initramfs"
install -m 0644 "$B/manifest.txt" "$DATA_DIR/images/$DIGEST/manifest.txt"

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
    # FCOS has an immutable root (no /etc/fstab) — persist via a systemd unit
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

echo "== FCOS node is PoC-ready =="
