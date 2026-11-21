#!/usr/bin/env bash
# MS-5 recorded-run suite — the ms3-harness.sh adaptation for the
# Flatcar node (2026-11-20). Only differences from the MS-3 harness:
# the agent and CLH live at /var/lib/porche/bin (read-only /usr/local
# on Flatcar) and PORCHE_CLH_BIN is exported. Everything else —
# PERF-1 (100), PERF-5 (20 VMs / 10 min RSS), PERF-8 (1,000-cycle
# create/wipe gate + drift) — is verbatim MS-3.
# Runs ON the node as root: ms5-suite.sh <image-digest> <run-dir>
set -euo pipefail
[ "$(id -u)" -eq 0 ] || { echo "must run as root on the node"; exit 1; }
IMG=${1:?usage: ms5-suite.sh <image-digest> <run-dir>}
RD=${2:?usage: ms5-suite.sh <image-digest> <run-dir>}
mkdir -p "$RD"
export PORCHE_CLH_BIN=/var/lib/porche/bin/cloud-hypervisor
AGENT="/var/lib/porche/bin/porche host-agent --store /var/lib/porche/desired-state.sqlite3 --keys-file /etc/porche/keys.json"
STORE=/var/lib/porche/desired-state.sqlite3

create_ready_ms() {
  local id=$1 ip=$2
  local t0=$(date +%s%3N)
  $AGENT create --spec <(cat <<EOF
{"id":"$id","image_digest":"$IMG","command":"sleep 600","vcpu":1,"ram_mib":128,"volume":null,"ip6":"$ip","gw6":"fd42:1:1::ff"}
EOF
) >/dev/null 2>&1 || { echo "ERR"; return; }
  local log="/var/lib/porche/consoles/$id.log"
  for _ in $(seq 1 120); do
    grep -q 'porche.ready' "$log" 2>/dev/null && break
    sleep 0.05
  done
  grep -q 'porche.ready' "$log" 2>/dev/null || { echo "ERR"; return; }
  local tnow=$(date +%s%3N)
  echo $((tnow - t0))
}

echo "== PERF-1: create->ready, 10 warmups + 100 samples ==" | tee "$RD/perf1.txt"
for i in $(seq 1 10); do
  id="w$(printf '%x' $((i + 1000)))"
  create_ready_ms "$id" "fd42:31::$(printf '%x' $i)" >/dev/null 2>&1 || true
  $AGENT wipe "$id" >/dev/null 2>&1 || true
done
: > "$RD/perf1.csv"
for i in $(seq 1 100); do
  id="p1$(printf '%03d' $i)"
  ms=$(create_ready_ms "$id" "fd42:31:1::$(printf '%x' $i)")
  echo "$i,$ms" >> "$RD/perf1.csv"
  $AGENT wipe "$id" >/dev/null 2>&1 || true
done
awk -F, '{print $2}' "$RD/perf1.csv" | sort -n > "$RD/perf1.sorted"
n=$(wc -l < "$RD/perf1.sorted")
p50=$(sed -n "$((n/2))p" "$RD/perf1.sorted")
p99_idx=$(( (99*n + 50) / 100 )); [ $p99_idx -gt $n ] && p99_idx=$n
p99=$(sed -n "${p99_idx}p" "$RD/perf1.sorted")
echo "n=$n P50=${p50}ms P99=${p99}ms (thresholds: P50<1500 P99<5000)" | tee -a "$RD/perf1.txt"

echo "== PERF-5: 20 parallel VMs, RSS sampler, 10 min steady ==" | tee "$RD/perf5.txt"
for i in $(seq 1 20); do
  id="d5$(printf '%02d' $i)"
  $AGENT create --spec <(cat <<EOF
{"id":"$id","image_digest":"$IMG","command":"sleep 3600","vcpu":1,"ram_mib":128,"volume":null,"ip6":"fd42:32::$(printf '%x' $i)","gw6":"fd42:1:1::ff"}
EOF
) >/dev/null 2>&1 || echo "create $id failed" | tee -a "$RD/perf5.txt" &
done
wait
sleep 60
: > "$RD/perf5.csv"
for _ in $(seq 1 600); do
  ts=$(date +%s)
  for pid in $(pgrep -f 'cloud-hypervisor.*--api-socket'); do
    rss=$(awk '/VmRSS/{print $2}' /proc/$pid/status 2>/dev/null || echo 0)
    echo "$ts,$pid,$rss" >> "$RD/perf5.csv"
  done
  sleep 1
done
awk -F, '{if ($3 > m[$2]) m[$2] = $3} END {max = 0; for (p in m) if (m[p] > max) max = m[p]; print "max per-VM RSS:", max, "kB (threshold < 204800kB)"}' "$RD/perf5.csv" | tee -a "$RD/perf5.txt"
for i in $(seq 1 20); do $AGENT wipe "d5$(printf '%02d' $i)" >/dev/null 2>&1 || true; done

echo "== PERF-8: 1,000-cycle create/wipe loop (the GATE) ==" | tee "$RD/perf8.txt"
mem_base=$(awk '/MemAvailable/{print $2}' /proc/meminfo)
echo "MemAvailable base: $mem_base kB" | tee -a "$RD/perf8.txt"
: > "$RD/perf8.csv"
: > "$RD/wedge.txt"
WEDGE=0
for i in $(seq 1 1000); do
  id="g$(printf '%04d' $i)"
  [ ${#id} -gt 10 ] && id="g$(printf '%03d' $i)"
  c0=$(date +%s%3N)
  $AGENT create --spec <(cat <<EOF
{"id":"$id","image_digest":"$IMG","command":"true","vcpu":1,"ram_mib":128,"volume":{"size_mib":64},"ip6":"fd42:33::$(printf '%x' $((i % 60000 + 16)))","gw6":"fd42:1:1::ff"}
EOF
) >/dev/null 2>&1 || { echo "cycle $i: CREATE FAILED" | tee -a "$RD/wedge.txt"; WEDGE=1; }
  w0=$(date +%s%3N)
  $AGENT wipe "$id" >/dev/null 2>&1 || { echo "cycle $i: WIPE FAILED" | tee -a "$RD/wedge.txt"; WEDGE=1; }
  w1=$(date +%s%3N)
  echo "$i,$((w1 - w0))" >> "$RD/perf8.csv"
  c1=$(date +%s%3N)
  [ $((c1 - c0)) -gt 60000 ] && { echo "cycle $i: WEDGE (>60s)" | tee -a "$RD/wedge.txt"; WEDGE=1; }
  if [ $i -eq 200 ] || [ $i -eq 1000 ]; then
    mem=$(awk '/MemAvailable/{print $2}' /proc/meminfo)
    ag=$(awk '/VmRSS/{print $2}' /proc/$$/status 2>/dev/null || echo 0)
    echo "cycle $i: MemAvailable=$mem kB (base $mem_base), agent RSS=$ag kB" | tee -a "$RD/perf8.txt"
  fi
done
awk -F, '{print $2}' "$RD/perf8.csv" | sort -n > "$RD/perf8.sorted"
n=$(wc -l < "$RD/perf8.sorted")
p50=$(sed -n "$((n/2))p" "$RD/perf8.sorted")
p99_idx=$(( (99*n + 50) / 100 )); [ $p99_idx -gt $n ] && p99_idx=$n
p99=$(sed -n "${p99_idx}p" "$RD/perf8.sorted")
echo "teardown: n=$n P50=${p50}ms P99=${p99}ms (threshold P99<2000)" | tee -a "$RD/perf8.txt"
echo "wedge events: $(grep -c . "$RD/wedge.txt" || true)" | tee -a "$RD/perf8.txt"
[ $WEDGE -eq 0 ] && echo "GATE: PASS (no wedge)" | tee -a "$RD/perf8.txt" || echo "GATE: FAIL" | tee -a "$RD/perf8.txt"
echo "== harness done ==" | tee -a "$RD/perf8.txt"
