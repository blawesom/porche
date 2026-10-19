#!/usr/bin/env bash
# PERF-8 (doc 07 §7.2) — THE GATE: 1,000 create/wipe cycles with no wedge, no host
# memory growth (< 5 % drift between cycles 200 and 1,000), plus per-cycle teardown
# timing (P99 < 2 s — smoke) and the wipe probe on every cycle (markers written by
# the guest into the volume, scanned by the agent at wipe).
#
# PoC CLI note: §7.2's "host-agent cgroup RSS" applies to the P0 long-running
# agent; with the PoC CLI-per-verb model the equivalent leak signals are residue
# (CLH processes, netns, mapper devices left behind after wipe) + MemAvailable.
set -euo pipefail

CYCLES=${CYCLES:-1000}
DRIFT_CHECKPOINT_A=${DRIFT_CHECKPOINT_A:-200}
CREATE_WEDGE_S=${CREATE_WEDGE_S:-60}     # §7.2: a create > 60 s ⇒ wedge ⇒ fail
LIVENESS_S=${LIVENESS_S:-30}             # §7.2: liveness timeout 30 s

PORCHE_BIN=${PORCHE_BIN:-porche}
DATA_DIR=${DATA_DIR:-/var/lib/porche}
STORE=${STORE:-$DATA_DIR/desired-state.sqlite3}
SPEC=${SPEC:?set SPEC to a volume-carrying workload spec json (e.g. harness/spec-loop.json)}
ID=$(jq -r .id "$SPEC")

RUN_DIR=${RUN_DIR:-runs/$(date +%Y%m%d-%H%M%S)-perf8}
mkdir -p "$RUN_DIR"
source "$(dirname "$0")/lib/fixtures.sh"
fixtures_record "$RUN_DIR"

mem_available_kb() { awk '/^MemAvailable:/{print $2}' /proc/meminfo; }

residue_check() { # after wipe: nothing left
    local bad=0
    if pgrep -x cloud-hypervisor > /dev/null 2>&1; then
        echo "residue: cloud-hypervisor process alive after wipe" >&2; bad=1; fi
    if ip netns list | grep -q "^w-$ID"; then
        echo "residue: netns w-$ID alive after wipe" >&2; bad=1; fi
    if [ -e "/dev/mapper/porche-$ID" ]; then
        echo "residue: mapper porche-$ID present after wipe" >&2; bad=1; fi
    return $bad
}

now_ns() { date +%s%N; }

: > "$RUN_DIR/perf8-teardown-ms.csv"
: > "$RUN_DIR/perf8-memory.csv"
echo "cycle,mem_available_kb" >> "$RUN_DIR/perf8-memory.csv"

for c in $(seq 1 "$CYCLES"); do
    # unique marker per cycle: guest writes it into the volume; wipe scans for it
    marker_b64=$(openssl rand 32 | base64 -w0)
    marker_hex=$(echo "$marker_b64" | base64 -d | od -An -tx1 | tr -d ' \n')
    echo "$marker_hex" > "$RUN_DIR/markers.txt"
    cmd="echo $marker_b64 | base64 -d | dd of=/dev/vda bs=64k conv=notrunc"
    jq --arg cmd "$cmd" --arg id "$ID" '.command = $cmd | .id = $id' \
        "$SPEC" > "$RUN_DIR/spec.json"

    t0=$(now_ns)
    timeout "$CREATE_WEDGE_S" "$PORCHE_BIN" host-agent --data-dir "$DATA_DIR" \
        --store "$STORE" create --spec "$RUN_DIR/spec.json" > /dev/null \
        || { echo "FAIL cycle $c: create wedged (> ${CREATE_WEDGE_S}s)" >&2; exit 1; }
    t1=$(now_ns)

    # wait for the guest to finish writing the marker (porche.done on the console)
    log="$DATA_DIR/consoles/$ID.log"
    deadline=$(( CREATE_WEDGE_S * 100 ))
    for _ in $(seq 1 "$deadline"); do
        grep -q '^porche.done' "$log" 2>/dev/null && break
        sleep 0.01
    done
    grep -q '^porche.done' "$log" 2>/dev/null \
        || { echo "FAIL cycle $c: guest did not reach porche.done" >&2; exit 1; }

    # liveness probe (§7.2 wedge detection)
    timeout "$LIVENESS_S" "$PORCHE_BIN" host-agent --data-dir "$DATA_DIR" \
        --store "$STORE" status > /dev/null \
        || { echo "FAIL cycle $c: liveness probe timed out (${LIVENESS_S}s)" >&2; exit 1; }

    t2=$(now_ns)
    report=$("$PORCHE_BIN" host-agent --data-dir "$DATA_DIR" --store "$STORE" \
        wipe "$ID" --markers "$RUN_DIR/markers.txt") \
        || { echo "FAIL cycle $c: wipe probe failed" >&2
             echo "$report" >&2; exit 1; }
    t3=$(now_ns)

    echo $(( (t3 - t2) / 1000000 )) >> "$RUN_DIR/perf8-teardown-ms.csv"
    echo "$c,$(mem_available_kb)" >> "$RUN_DIR/perf8-memory.csv"
    residue_check || { echo "FAIL cycle $c: teardown residue" >&2; exit 1; }

    if [ "$c" -eq "$DRIFT_CHECKPOINT_A" ]; then
        MEM_A=$(mem_available_kb)
    fi
    printf '\rcycle %d/%d  create=%dms teardown=%dms' \
        "$c" "$CYCLES" "$(( (t1 - t0) / 1000000 ))" "$(( (t3 - t2) / 1000000 ))" >&2
done
echo >&2

# --- verdicts -------------------------------------------------------------------
python3 - "$RUN_DIR" "$DRIFT_CHECKPOINT_A" "$CYCLES" <<'EOF'
import sys
run_dir, a, n = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])

td = sorted(int(l) for l in open(f"{run_dir}/perf8-teardown-ms.csv") if l.strip())
def pct(xs, p):
    return xs[min(len(xs) - 1, round(p / 100 * (len(xs) - 0.5)))]

mem = {}
for line in open(f"{run_dir}/perf8-memory.csv"):
    if line.startswith("cycle"): continue
    c, m = line.strip().split(",")
    mem[int(c)] = int(m)
mem_a, mem_b = mem.get(a), mem.get(n)
if mem_a is None or mem_b is None:
    print("PERF-8 drift: CHECKPOINT MISSING")
    drift = None
else:
    drift = 100 * (mem_a - mem_b) / mem_a  # positive = MemAvailable shrank
    print(f"MemAvailable: cycle {a}={mem_a}kB  cycle {n}={mem_b}kB  drift={drift:+.2f}%")

print(f"teardown: n={len(td)} P50={pct(td,50)}ms P99={pct(td,99)}ms max={td[-1]}ms")
gate = drift is not None and abs(drift) < 5
print(f"PERF-8 LOOP (GATE — no wedge, <5% drift): {'PASS' if gate else 'FAIL'}")
print(f"PERF-8 timing (SMOKE — P99 < 2000ms): {'PASS' if pct(td,99) < 2000 else 'FAIL'}")
EOF
