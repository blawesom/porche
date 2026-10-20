#!/usr/bin/env bash
# SEC-11 boot assert (doc 07 §7.2): every item, every boot. A failure here means the
# host is out of the pool (doc 01 §8 criterion 4) — the harness refuses to continue.
# MS-1 variant (E-29 re-entry, 2026-10-20): same four checks as the pre-rollback script.
set -u

fail=0

check() {
    local name=$1 want=$2 got
    got=$(sysctl -n "$name" 2>/dev/null) || got="MISSING"
    if [ "$got" != "$want" ]; then
        echo "FAIL $name: want=$want got=$got"
        fail=1
    else
        echo "ok   $name=$got"
    fi
}

# the four §7.2 items
check kernel.io_uring_disabled 2
check user.max_user_namespaces 0
check kernel.modules_disabled 1

# core_pattern: locked = never a pipe to a helper (helper = post-boot code exec)
cp=$(sysctl -n kernel.core_pattern 2>/dev/null) || cp="MISSING"
case "$cp" in
    \|*) echo "FAIL kernel.core_pattern pipes to helper: $cp"; fail=1 ;;
    "")  echo "FAIL kernel.core_pattern: empty"; fail=1 ;;
    *)   echo "ok   kernel.core_pattern=$cp" ;;
esac

exit $fail