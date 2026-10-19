#!/usr/bin/env bash
# PERF-5 (doc 07 §7.2): 20 parallel microVMs, steady state 60 s after boot, RSS
# sampled at 1 Hz for 10 min. < 200 MB per CLH process, zero anomalies.
set -euo pipefail

VMS=${VMS:-20}
MINUTES=${MINUTES:-10}
RSS_LIMIT_MB=${RSS_LIMIT_MB:-200}

PORCHE_BIN=${PORCHE_BIN:-porche}
DATA_DIR=${DATA_DIR:-/var/lib/porche}
STORE=${STORE:-$DATA_DIR/desired-state.sqlite3}
SPEC=${SPEC:?set SPEC to a workload spec json (e.g. harness/spec-density.json)}

RUN_DIR=${RUN_DIR:-runs/$(date +%Y%m%d-%H%M%S)-perf5}
mkdir -p "$RUN_DIR"
source "$(dirname "$0")/lib/fixtures.sh"
fixtures_record "$RUN_DIR"

# N specs with distinct ids (id ≤ 10 chars, [a-z0-9-])
for i in $(seq 1 "$VMS"); do
    jq --arg id "d$i" '.id = $id' "$SPEC" > "$RUN_DIR/spec-$i.json"
done

echo "creating $VMS VMs"
for i in $(seq 1 "$VMS"); do
    "$PORCHE_BIN" host-agent --data-dir "$DATA_DIR" --store "$STORE" \
        create --spec "$RUN_DIR/spec-$i.json" > /dev/null &
done
wait

echo "steady state: 60 s"
sleep 60

sample_rss_kb() { # pid → VmRSS in kB (0 if gone)
    local pid=$1
    [ -r "/proc/$pid/status" ] \
        && awk '/^VmRSS:/{print $2}' "/proc/$pid/status" 2>/dev/null || echo 0
}

pids=()
for i in $(seq 1 "$VMS"); do
    pids+=("$(cat "$DATA_DIR/run/d$i.pid")")
done

echo "sampling RSS at 1 Hz for ${MINUTES} min"
SAMPLES=$(( MINUTES * 60 ))
: > "$RUN_DIR/perf5.csv"
echo "sample,timestamp,vm,pid,rss_kb" >> "$RUN_DIR/perf5.csv"
for s in $(seq 1 "$SAMPLES"); do
    ts=$(date +%s)
    alive=0
    for i in $(seq 1 "$VMS"); do
        pid=${pids[$((i - 1))]}
        rss=$(sample_rss_kb "$pid")
        [ "$rss" -gt 0 ] && alive=$((alive + 1))
        echo "$s,$ts,d$i,$pid,$rss" >> "$RUN_DIR/perf5.csv"
    done
    if [ "$alive" -ne "$VMS" ]; then
        echo "ANOMALY: only $alive/$VMS CLH processes alive at sample $s" >&2
        break
    fi
    sleep 1
done

python3 - "$RUN_DIR/perf5.csv" "$RSS_LIMIT_MB" "$VMS" <<'EOF'
import sys, collections
path, limit, vms = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
per_vm = collections.defaultdict(list)
for line in open(path):
    p = line.strip().split(",")
    if p[0] == "sample": continue
    per_vm[p[2]].append(int(p[4]))
worst = max(max(v) for v in per_vm.values()) / 1024
print(f"vms={len(per_vm)} samples/vm={min(len(v) for v in per_vm.values())}")
for vm in sorted(per_vm):
    xs = per_vm[vm]
    print(f"  {vm}: min={min(xs)/1024:.0f}MB avg={sum(xs)/len(xs)/1024:.0f}MB max={max(xs)/1024:.0f}MB")
verd = "PASS (smoke)" if worst < limit and len(per_vm) == vms else "FAIL"
print(f"PERF-5 < {limit}MB each, {vms} stable: {verd} (worst {worst:.0f}MB)")
EOF

echo "teardown"
for i in $(seq 1 "$VMS"); do
    "$PORCHE_BIN" host-agent --data-dir "$DATA_DIR" --store "$STORE" \
        wipe "d$i" > /dev/null &
done
wait
