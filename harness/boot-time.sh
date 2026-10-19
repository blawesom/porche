#!/usr/bin/env bash
# PERF-1 (doc 07 §7.2): create → guest running, measured T0 (harness, just before
# create) to T1 (porche.ready line on the serial console). 100 runs after 10
# warmups. P50 < 1.5 s — a SMOKE signal in the integration environment (E-16);
# the gate is the metal validation run.
set -euo pipefail

RUNS=${RUNS:-100}
WARMUPS=${WARMUPS:-10}
TIMEOUT_READY=${TIMEOUT_READY:-60}   # s — gross pathology ⇒ investigate early (E-16)

PORCHE_BIN=${PORCHE_BIN:-porche}
DATA_DIR=${DATA_DIR:-/var/lib/porche}
STORE=${STORE:-$DATA_DIR/desired-state.sqlite3}
SPEC=${SPEC:?set SPEC to a workload spec json (e.g. harness/spec-perf1.json)}

RUN_DIR=${RUN_DIR:-runs/$(date +%Y%m%d-%H%M%S)-perf1}
mkdir -p "$RUN_DIR"
source "$(dirname "$0")/lib/fixtures.sh"
fixtures_record "$RUN_DIR"

ID=$(jq -r .id "$SPEC")
LOG="$DATA_DIR/consoles/$ID.log"

now_ns() { date +%s%N; }

# one measured cycle: create → wait for the ready marker → wipe
cycle() {
    local t0 t1 elapsed_ms
    rm -f "$LOG"
    t0=$(now_ns)
    "$PORCHE_BIN" host-agent --data-dir "$DATA_DIR" --store "$STORE" \
        create --spec "$SPEC" > /dev/null

    local deadline=$(( TIMEOUT_READY * 100 ))
    for _ in $(seq 1 "$deadline"); do
        grep -q 'porche.ready' "$LOG" 2>/dev/null && break
        sleep 0.01
    done
    if ! grep -q 'porche.ready' "$LOG" 2>/dev/null; then
        echo "wedge: porche.ready not seen within ${TIMEOUT_READY}s" >&2
        return 1
    fi
    t1=$(now_ns)
    elapsed_ms=$(( (t1 - t0) / 1000000 ))

    "$PORCHE_BIN" host-agent --data-dir "$DATA_DIR" --store "$STORE" wipe "$ID" > /dev/null
    echo "$elapsed_ms"
}

echo "warmups: $WARMUPS"
for i in $(seq 1 "$WARMUPS"); do cycle > /dev/null; done

echo "runs: $RUNS"
: > "$RUN_DIR/perf1.csv"
for i in $(seq 1 "$RUNS"); do
    ms=$(cycle) || { echo "FAIL at run $i" >&2; exit 1; }
    echo "$ms" >> "$RUN_DIR/perf1.csv"
    printf '\r%d/%d  last=%sms' "$i" "$RUNS" "$ms" >&2
done
echo >&2

python3 - "$RUN_DIR/perf1.csv" <<'EOF'
import sys
xs = sorted(int(l) for l in open(sys.argv[1]) if l.strip())
def pct(p):
    i = min(len(xs) - 1, round(p / 100 * (len(xs) - 0.5)))
    return xs[i]
p50 = pct(50)
print(f"n={len(xs)}  P50={p50}ms  P99={pct(99)}ms  min={xs[0]}ms  max={xs[-1]}ms")
verd = "PASS (smoke)" if p50 < 1500 else "FAIL"
print(f"PERF-1 P50 < 1500ms (SMOKE — metal is the gate): {verd}")
EOF
