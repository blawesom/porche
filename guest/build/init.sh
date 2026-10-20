#!/bin/sh
# platform init (E-25) — the guest never configures itself (doc 14 §2 principle 2):
# address + gateway arrive on the kernel command line we built; this init applies
# them, prints the PERF-1 marker (doc 07 §7.2), runs the workload command, powers
# off. POSIX sh on BusyBox — deliberately boring (doc 07 §8.4).
set -eu

mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs dev /dev

# --- porche.ip6=<addr>,<gw> ----------------------------------------------------
ip6=""
gw=""
for w in $(cat /proc/cmdline); do
    case "$w" in
        porche.ip6=*)
            pair=${w#porche.ip6=}
            ip6=${pair%,*}
            gw=${pair#*,}
            ;;
    esac
done

if [ -z "$ip6" ] || [ -z "$gw" ] || [ "$ip6" = "$pair" ]; then
    echo "porche.error missing-or-malformed porche.ip6 on cmdline" > /dev/console
    exec poweroff -f
fi

ip link set lo up
ip link set eth0 up
# /128 on the interface; the gateway is made on-link by an explicit /128 route —
# no DHCP, no SLAAC/RA (E-25: no guest-side family negotiation)
ip addr add "$ip6/128" dev eth0
ip route add "$gw/128" dev eth0
ip route add default via "$gw" dev eth0

# --- the PERF-1 T1 marker -------------------------------------------------------
echo "porche.ready" > /dev/console

# --- workload command: porche.cmd=<base64 shell string> --------------------------
# (base64: the kernel splits cmdline on spaces — one space-free token)
cmd=""
for w in $(cat /proc/cmdline); do
    case "$w" in
        porche.cmd=*) cmdraw=${w#porche.cmd=}
                      cmd=$(echo "$cmdraw" | base64 -d) ;;
    esac
done

rc=0
if [ -n "$cmd" ]; then
    # `sh -c` exit code must reach the marker even when nonzero — never let
    # `set -e` kill init before `porche.done` prints (observed 2026-10-20:
    # a non-executable command exited init early and the kernel panicked).
    /bin/sh -c "$cmd" || rc=$?
fi
echo "porche.done rc=$rc" > /dev/console

# deterministic exit: the loop tests rely on the VM dying here, not idling
poweroff -f