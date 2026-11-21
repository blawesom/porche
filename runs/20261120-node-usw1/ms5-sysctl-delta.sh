#!/usr/bin/env bash
# MS-5 escape-delta re-run, class 3 (SEC-11 sysctl cluster) — Flatcar-node
# adaptation of harness/ms4-sysctl-delta.sh (2026-11-20). Differences:
# the io_uring probe uses a STATIC binary uploaded from the build box
# (Flatcar ships no gcc/python3); both legs on the same boot (only
# modules_disabled is one-way, recorded as still-sealed in baseline).
# Usage: ms5-sysctl-delta.sh <run-dir> hardened|baseline
set -euo pipefail
[ "$(id -u)" -eq 0 ] || { echo "run on the node as root"; exit 1; }
RD=${1:?usage: ms5-sysctl-delta.sh <run-dir>}
LEG=${2:?leg: hardened|baseline}
PROBE=/tmp/bundle/ms4_iouring
[ -x "$PROBE" ] || { echo "FATAL: $PROBE missing (upload the static probe)"; exit 1; }
mkdir -p "$RD"

{
  echo "== leg: $LEG =="
  sysctl kernel.io_uring_disabled user.max_user_namespaces kernel.core_pattern kernel.modules_disabled
} > "$RD/$LEG.sysctls" 2>&1

if [ "$LEG" = baseline ]; then
  sysctl -w kernel.io_uring_disabled=0 >> "$RD/$LEG.sysctls" 2>&1 || true
  sysctl -w user.max_user_namespaces=63183 >> "$RD/$LEG.sysctls" 2>&1 || true
  sysctl -w kernel.core_pattern='core' >> "$RD/$LEG.sysctls" 2>&1 || true
  echo "NOTE: modules_disabled stays 1 until reboot (irreversible)" >> "$RD/$LEG.sysctls"
fi

# probe 1: unprivileged io_uring_setup (the static binary)
su nobody -s /bin/sh -c "$PROBE" > "$RD/$LEG.iouring" 2>&1 || true

# probe 2: unprivileged user namespace creation
su nobody -s /bin/sh -c 'unshare -Ur echo "userns: ALLOWED (mapped root)"' > "$RD/$LEG.userns" 2>&1 \
  || echo "userns: DENIED" >> "$RD/$LEG.userns"

{
  echo "io_uring: $(head -1 "$RD/$LEG.iouring" 2>/dev/null)"
  echo "userns:   $(head -1 "$RD/$LEG.userns" 2>/dev/null)"
} | tee "$RD/$LEG.verdict"
