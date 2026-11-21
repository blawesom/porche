#!/usr/bin/env bash
# The pending validation ("quick coverage test", the 2026-11-19 ask):
# 1. nested virt hardware: /dev/kvm, vmx, kvm_intel nested=Y (doc 13 §4.3)
# 2. CLH v54.0 smoke: boot the pinned guest kernel under KVM,
#    serial-to-file, success = guest kernel banner in the log
#    (mirrors porche-clh's config: kernel+initramfs+console=ttyS0)
set -euo pipefail
cd "$(dirname "$0")"
ADDR=$(cat node-ip.txt)
KEY=~/.ssh/porche_integ
SSH="ssh -i $KEY -o StrictHostKeyChecking=accept-new -o ConnectTimeout=10 core@$ADDR"

echo "== upload bundle =="
scp -i "$KEY" -o StrictHostKeyChecking=accept-new -q \
    cloud-hypervisor-static kernel initramfs.cpio.gz manifest.txt core@$ADDR:/tmp/

echo "== 1. nested virt hardware =="
$SSH 'sudo sh -c "
  echo vmx-flag: \$(grep -c vmx /proc/cpuinfo) cpus
  ls -l /dev/kvm
  echo nested: \$(cat /sys/module/kvm_intel/parameters/nested)
  echo kernel: \$(uname -r)
"'

echo "== 2. CLH v54.0 guest boot smoke =="
$SSH 'sudo sh -c "
  chmod 0755 /tmp/cloud-hypervisor-static
  /tmp/cloud-hypervisor-static --version
  rm -f /tmp/console.log
  timeout 25 /tmp/cloud-hypervisor-static \
    --kernel /tmp/kernel --initramfs /tmp/initramfs.cpio.gz \
    --cmdline \"console=ttyS0\" --memory size=512M --cpus boot=1 \
    --serial file=/tmp/console.log --console off &
  CLHPID=\$!
  sleep 20
  kill \$CLHPID 2>/dev/null || true
  echo \"--- serial log (first 15 lines) ---\"
  head -15 /tmp/console.log
  echo \"--- verdict ---\"
  grep -q \"Linux version\" /tmp/console.log \
    && echo GUEST-BOOT-OK \
    || echo GUEST-BOOT-FAILED
"'
