#!/usr/bin/env bash
# §7.1 fixture record (doc 07 §7.1) — runs ON node 1 as root, alongside the
# suite. Emits <run-dir>/fixtures.txt: node OS+kernel, CLH version+digest,
# guest image digests, host-agent digest+commit, hardening state.
#
# Usage: fixtures-capture.sh <run-dir> [host-agent-commit]
#   host-agent-commit: the repo commit the node's `porche` was built from
#   (also via $PORCHE_COMMIT). "unknown" if absent.
set -euo pipefail
[ "$(id -u)" -eq 0 ] || { echo "must run as root on node 1"; exit 1; }
RD=${1:?usage: fixtures-capture.sh <run-dir> [host-agent-commit]}
COMMIT=${2:-${PORCHE_COMMIT:-unknown}}
mkdir -p "$RD"
# Tool root: override with PORCHE_BIN_DIR (Flatcar: /var/lib/porche/bin)
BIN="${PORCHE_BIN_DIR:-/usr/local/bin}"

{
echo "== PORCHE §7.1 fixture record =="
echo "captured: $(date -u '+%Y-%m-%dT%H:%M:%SZ')  (host: $(hostname))"
echo
echo "-- node OS + kernel --"
if [ -r /etc/os-release ]; then . /etc/os-release; echo "$PRETTY_NAME"; fi
echo "uname -r: $(uname -r)"
command -v rpm-ostree >/dev/null 2>&1 && rpm-ostree status 2>/dev/null | sed -n '1,6p' || true
echo
echo "-- CLH --"
if [ -x "$BIN/cloud-hypervisor" ]; then
  echo "version: $($BIN/cloud-hypervisor --version 2>&1 | head -1)"
  echo "sha256: $(sha256sum "$BIN/cloud-hypervisor" | cut -d' ' -f1)"
else
  echo "CLH binary MISSING"
fi
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
echo "sha256: $(sha256sum "$BIN/porche" | cut -d' ' -f1)"
echo "porche-web sha256: $(sha256sum "$BIN/porche-web" 2>/dev/null | cut -d' ' -f1 || echo missing)"
echo
echo "-- hardening state (SEC-11 sysctls, §7.2) --"
for k in kernel.io_uring_disabled user.max_user_namespaces kernel.core_pattern kernel.modules_disabled; do
  echo "$k = $(sysctl -n "$k" 2>/dev/null || echo unreadable)"
done
echo "sysctl-assert: $(bash "$(dirname "$0")/sysctl-assert.sh" >/dev/null 2>&1 && echo PASS || echo FAIL)"
echo "offload/device policy: frozen allow-list per SEC-3/SEC-7 — exercised"
echo "  and recorded by the §7.3 delta toggles in this run's ms4-* artifacts"
} | tee "$RD/fixtures.txt"
