#!/usr/bin/env bash
# §7.1 fixture record — Flatcar node variant (2026-11-20). The harness
# fixtures-capture.sh with /usr/local/bin/{cloud-hypervisor,porche,porche-web}
# replaced by /var/lib/porche/bin (read-only /usr/local on Flatcar).
set -euo pipefail
[ "$(id -u)" -eq 0 ] || { echo "must run as root on the node"; exit 1; }
RD=${1:?usage: fixtures-flatcar.sh <run-dir>}
COMMIT=${2:-unknown}
mkdir -p "$RD"

{
echo "== PORCHE §7.1 fixture record (Flatcar node variant) =="
echo "captured: $(date -u '+%Y-%m-%dT%H:%M:%SZ')  (host: $(hostname))"
echo
echo "-- node OS + kernel --"
. /etc/os-release; echo "$PRETTY_NAME"
echo "uname -r: $(uname -r)"
echo
echo "-- CLH --"
echo "version: $(/var/lib/porche/bin/cloud-hypervisor --version 2>&1 | head -1)"
echo "sha256: $(sha256sum /var/lib/porche/bin/cloud-hypervisor | cut -d' ' -f1)"
echo
echo "-- guest image (platform image dirs) --"
for d in /var/lib/porche/images/*/; do
  [ -d "$d" ] || continue
  echo "dir: $d"
  cat "$d/manifest.txt" 2>/dev/null || echo "  (no manifest.txt)"
  sha256sum "$d/kernel" "$d/initramfs" 2>/dev/null || true
done
echo
echo "-- host-agent --"
echo "commit: $COMMIT"
echo "sha256: $(sha256sum /var/lib/porche/bin/porche | cut -d' ' -f1)"
echo "porche-web sha256: $(sha256sum /var/lib/porche/bin/porche-web 2>/dev/null | cut -d' ' -f1 || echo missing)"
echo
echo "-- hardening state (SEC-11 sysctls, §7.2) --"
for k in kernel.io_uring_disabled user.max_user_namespaces kernel.core_pattern kernel.modules_disabled; do
  echo "$k = $(sysctl -n "$k" 2>/dev/null || echo unreadable)"
done
echo "sysctl-assert: $(bash /tmp/bundle/sysctl-assert.sh >/dev/null 2>&1 && echo PASS || echo FAIL)"
} | tee "$RD/fixtures.txt"
