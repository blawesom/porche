#!/usr/bin/env bash
# MS-4 escape-delta class 1: virtio-net offload attack (PwnEscape class, E-18).
# Runs a known offload-driven packet-processing stress INSIDE a guest, twice:
#   hardened  — offloads off (SEC-7 control)
#   baseline  — offloads on  (PORCHE_ESCAPE_BASELINE=1)
# The delta verdict: behavior/crash/error difference between the two runs.
# SAFETY (§7.4): all material runs inside our own guest VMs only.
set -euo pipefail
[ "$(id -u)" -eq 0 ] || { echo "run on node 1 as root"; exit 1; }
RD=${1:?usage: ms4-offload-delta.sh <run-dir>}
mkdir -p "$RD"
# Tool root: override with PORCHE_BIN_DIR (Flatcar: /var/lib/porche/bin)
BIN="${PORCHE_BIN_DIR:-/usr/local/bin}"
AGENT="$BIN/porche host-agent --store /var/lib/porche/desired-state.sqlite3 --keys-file /etc/porche/keys.json"
export PORCHE_CLH_BIN="$BIN/cloud-hypervisor"
IMG=$(ls /var/lib/porche/images/ | head -1)

run_case() {
  local label=$1 id=$2 env_baseline=$3
  local spec=/tmp/ms4-$id.json
  cat > "$spec" <<EOF
{"id":"$id","image_digest":"$IMG","command":"for i in 1 2 3 4 5 6 7 8; do busybox ping -c 60 -i 0.1 -s 65000 -W 1 fd42:1:1::ff 2>&1 | busybox tail -1; done; echo OFFLOAD-STRESS-DONE","vcpu":1,"ram_mib":256,"volume":null,"ip6":"fd42:1:1::54","gw6":"fd42:1:1::ff"}
EOF
  local t0=$(date +%s%3N)
  if [ "$env_baseline" = 1 ]; then
    PORCHE_ESCAPE_BASELINE=1 $AGENT create --spec "$spec" >"$RD/$label.create" 2>&1 || echo "CREATE-FAIL" >> "$RD/$label.create"
  else
    $AGENT create --spec "$spec" >"$RD/$label.create" 2>&1 || echo "CREATE-FAIL" >> "$RD/$label.create"
  fi
  # wait for the done marker (the guest powers off after the command)
  local log="/var/lib/porche/consoles/$id.log"
  for _ in $(seq 1 240); do
    grep -q 'OFFLOAD-STRESS-DONE\|porche.done' "$log" 2>/dev/null && break
    grep -qi 'panic\|Oops\|BUG:' "$log" 2>/dev/null && break
    sleep 0.5
  done
  sleep 1
  cp "$log" "$RD/$label.console" 2>/dev/null || true
  # CLH process state at the end: alive/exit code tell the crash story
  local t1=$(date +%s%3N)
  echo "elapsed_ms=$((t1 - t0))" >> "$RD/$label.create"
  # kernel-side offload state of the tap (the delta's mechanism evidence)
  nsenter --net=/var/run/netns/w-$id ethtool -k tap-$id 2>/dev/null | grep -E 'tcp-segmentation|rx-checksumming|generic-segmentation' > "$RD/$label.tap-offloads" || \
    ip netns exec w-$id ethtool -k tap-$id 2>/dev/null | grep -E 'tcp-segmentation|rx-checksumming|generic-segmentation' > "$RD/$label.tap-offloads" || true
  $AGENT wipe "$id" >>"$RD/$label.wipe" 2>&1 || echo "WIPE-FAIL" >> "$RD/$label.wipe"
}

echo "== hardened (offloads off) ==" | tee "$RD/verdict.txt"
run_case hardened hs1 0
echo "== baseline (offloads on) ==" | tee -a "$RD/verdict.txt"
run_case baseline bs1 1

# ---- delta analysis (shell — no python3 dependency; 2026-11-21 fix: the
# python predecessor compared dicts that included the leg LABEL, so
# "DELTA: present" was structurally always printed — the MS-4 c1 verdict
# bug. Compare the outcome tuple only; the mechanism evidence is the tap
# offload state in {hardened,baseline}.tap-offloads) ----
leg_summary() {
  local label=$1 done_ok=no panic=no netup=no wipe_ok=no c
  grep -q 'OFFLOAD-STRESS-DONE' "$RD/$label.console" 2>/dev/null && done_ok=yes
  grep -qiE 'panic|Oops|BUG:' "$RD/$label.console" 2>/dev/null && panic=yes
  c=$(grep -c 'packet loss' "$RD/$label.console" 2>/dev/null || true); [ "${c:-0}" -gt 0 ] 2>/dev/null && netup=yes
  [ -f "$RD/$label.wipe" ] && ! grep -q WIPE-FAIL "$RD/$label.wipe" && wipe_ok=yes
  echo "stress_completed=$done_ok kernel_panic=$panic network_up=$netup wipe_ok=$wipe_ok"
}
H=$(leg_summary hardened); B=$(leg_summary baseline)
{
  echo "hardened: $H"
  echo "baseline: $B"
  if [ "$H" = "$B" ]; then
    echo "DELTA: none — same outcome both sides (version floor only, SEC-31 note)"
    echo "mechanism evidence: tap offload state, $RD/{hardened,baseline}.tap-offloads"
  else
    echo "DELTA: present — outcome differs between hardened and baseline"
    echo "verdict: delta demonstrated (E-18; no absolute-security claim)"
  fi
} | tee -a "$RD/verdict.txt"
echo done
