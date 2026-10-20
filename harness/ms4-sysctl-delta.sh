#!/usr/bin/env bash
# MS-4 escape-delta class 3: 2026 page-cache/userns kernel cluster (SEC-11).
# Two legs around a node reboot (the seal is irreversible until reboot):
#   leg hardened  — run on the SEALED node (current state): io_uring_disabled=2,
#                   max_user_namespaces=0, core_pattern locked, modules_disabled=1
#   leg baseline  — run after `sysctl -w` to stock defaults (io_uring_disabled=0,
#                   max_user_namespaces=large, modules loadable): the unhardened state
# Probe (in-guest + host, both target the same kernel surface the cluster guards):
#   host-side: unprivileged io_uring probe + user namespace creation probe —
#              the two 2026 attack primitives the cluster closes
# Usage: ms4-sysctl-delta.sh <run-dir> hardened|baseline
set -euo pipefail
[ "$(id -u)" -eq 0 ] || { echo "run on node 1 as root"; exit 1; }
RD=${1:?usage: ms4-sysctl-delta.sh <run-dir> hardened|baseline}
LEG=${2:?leg: hardened|baseline}
mkdir -p "$RD"

# --- leg precondition: state the sysctls actually in force (the toggle proof) ---
{
  echo "== leg: $LEG =="
  sysctl kernel.io_uring_disabled user.max_user_namespaces kernel.core_pattern kernel.modules_disabled
} > "$RD/$LEG.sysctls" 2>&1

if [ "$LEG" = baseline ]; then
  # flip to stock defaults for this leg's probes (recording we did)
  sysctl -w kernel.io_uring_disabled=0 >> "$RD/$LEG.sysctls" 2>&1 || true
  sysctl -w user.max_user_namespaces=63183 >> "$RD/$LEG.sysctls" 2>&1 || true
  sysctl -w kernel.core_pattern='core' >> "$RD/$LEG.sysctls" 2>&1 || true
  # modules_disabled cannot be un-set without reboot — recorded as still-sealed
  echo "NOTE: modules_disabled stays 1 until reboot (irreversible)" >> "$RD/$LEG.sysctls"
fi

# --- probe 1: unprivileged io_uring (the 2026 io_uring attack surface) ---
# a plain io_uring_setup syscall from an unprivileged context
cat > /tmp/ms4_iouring.c <<'EOF'
#include <stdio.h>
#include <unistd.h>
#include <sys/syscall.h>
#include <errno.h>
#include <string.h>
int main(void) {
    long r = syscall(425 /* io_uring_setup */, 4, 0);
    if (r < 0) printf("io_uring_setup: DENIED (%s)\n", strerror(errno));
    else { printf("io_uring_setup: ALLOWED (fd %ld)\n", r); close((int)r); }
    return 0;
}
EOF
gcc -o /tmp/ms4_iouring /tmp/ms4_iouring.c 2>>"$RD/$LEG.sysctls" || {
  echo "gcc missing — using python ctypes" >> "$RD/$LEG.sysctls"
  python3 - > "$RD/$LEG.iouring" <<'PY'
import ctypes
libc = ctypes.CDLL(None, use_errno=True)
r = libc.syscall(425, 4, 0)
if r < 0:
    import errno
    e = ctypes.get_errno()
    print(f"io_uring_setup: DENIED ({errno.errorcode.get(e, e)})")
else:
    print(f"io_uring_setup: ALLOWED (fd {r})")
PY
}
[ -f /tmp/ms4_iouring ] && su nobody -s /bin/sh -c /tmp/ms4_iouring > "$RD/$LEG.iouring" 2>&1 || true

# --- probe 2: unprivileged user namespace creation (the userns primitive) ---
su nobody -s /bin/sh -c 'unshare -Ur echo "userns: ALLOWED (mapped root)"' > "$RD/$LEG.userns" 2>&1 \
  || echo "userns: DENIED" >> "$RD/$LEG.userns"

# --- verdict for this leg ---
{
  echo "io_uring: $(cat "$RD/$LEG.iouring" 2>/dev/null | head -1)"
  echo "userns:   $(cat "$RD/$LEG.userns" 2>/dev/null | head -1)"
} | tee "$RD/$LEG.verdict"
