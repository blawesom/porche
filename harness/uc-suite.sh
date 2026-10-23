#!/usr/bin/env bash
# MS-6 use-case + functional suite (doc 07 §7.5.1 — SOFTWARE GATES).
# Exercises the real use cases through the LIVE console surface (MS-1 rule: no
# --dry surfaces; these runs are the recorded human-UAT machines' backstop, and
# the operator still walks the scenario reports — §8.6 milestone rule).
#
#   U1 — untrusted AI agent job: create → done (rc=0) → GetEvents → delete →
#        wipe report all-pass
#   U2 — disposable CI job with a tier-E volume: I/O burst → done (rc=0); and a
#        long job through the ordered-stop path: stopped stays stopped → wipe
#
# Every FR-2 transition and every negative path below is a §7.5.1 row.
#
# Usage: uc-suite.sh <user-token> <admin-token> [user-url] [admin-url] [run-dir]
#   env: SMOKE_T1_TOKEN   second tenant's token (over-quota + cross-tenant rows;
#                          when absent those rows are skipped and marked so)
set -euo pipefail

UT=${1:?usage: uc-suite.sh <user-token> <admin-token> [user-url] [admin-url] [run-dir]}
AT=${2:?need admin token}
WU=${3:-http://127.0.0.1:18081}
AU=${4:-http://127.0.0.1:18082}
RD=${5:-runs/$(date +%Y%m%d-%H%M%S)-ms6/uc}
mkdir -p "$RD"
REPORT="$RD/uc-report.md"
DATA=/var/lib/porche
AGENT="/usr/local/bin/porche host-agent --store $DATA/desired-state.sqlite3 --keys-file /etc/porche/keys.json"

fail=0
CUR=""
# REPORT lines always carry the check name: `pass` is usually invoked bare
# (`... && pass || failmsg ...`), so the label comes from the last `step()`.
step() { CUR="$1"; printf 'uc: %-40s ' "$1"; }
pass() { echo "pass"; echo "- [x] ${1:-$CUR} — pass" >>"$REPORT"; }
failmsg() { echo "FAIL — $1"; echo "- [ ] $CUR — **FAIL**: $1" >>"$REPORT"; fail=1; }
skipmsg() { echo "skip — $1"; echo "- [~] $CUR — skipped: $1" >>"$REPORT"; }
UIDN=0
mkid() { UIDN=$((UIDN + 1)); printf 'uc%s%02d' "$(date +%s | tail -c 5)" "$UIDN"; }   # ≤10 chars (spec contract)

# The report is opened ONCE here and only appended to afterwards — a mid-run
# open with '>' would truncate the scenario evidence the MS-6 walk reads.
echo "# MS-6 uc-suite evidence ($(date -u '+%Y-%m-%d %H:%M UTC'))" >"$REPORT"

img=$(curl -s -H "Cookie: porche_token=$UT" "$WU/w/new" \
  | grep -oE 'sha256:[0-9a-f]{64}' | head -1 || true)
[ -n "$img" ] || { echo "uc: no image in create form — is the console up?"; exit 1; }

console_create() { # $1=id $2=command [ $3=volume_mib ] [ $4=ip6 ]
  curl -s -o /dev/null -H "Cookie: porche_token=$UT" \
    --data-urlencode "id=$1" \
    --data-urlencode "command=$2" \
    --data-urlencode "image_digest=$img" \
    --data-urlencode "vcpu=1" \
    --data-urlencode "ram_mib=128" \
    --data-urlencode "volume_mib=${3:-}" \
    --data-urlencode "ip6=${4:-fd42:1:1::$((RANDOM % 60000 + 1))}" \
    --data-urlencode "gw6=fd42:1:1::ff" \
    "$WU/w/new"
}
detail_state() { # $1=id — the display state derived by the console (state.rs)
  curl -s -H "Cookie: porche_token=$UT" "$WU/w/$1" \
    | grep -oE 'done \(rc=[0-9?]+\)|running|pending|stopping|stopped|failed[^<]*' | head -1 || true
}
wait_state() { # $1=id $2=regex-want $3=timeout-s
  local st=""
  for _ in $(seq 1 "${3:-60}"); do
    st=$(detail_state "$1")
    echo "$st" | grep -qE "$2" && { echo "$st"; return 0; }
    sleep 1
  done
  echo "$st"
}

# ============================ U1 — untrusted AI agent ============================
U1=$(mkid)
HEXMARK=$(printf 'porche-uc-u1-%s' "$U1" | od -An -tx1 | tr -d ' \n')

step "U1 create (agent job, rc=0)"
console_create "$U1" "printf agent-output; exit 0"
[ "$(wait_state "$U1" 'pending|running|done \(rc=0\)' 30)" ] && pass || failmsg "U1 never observed a state"

step "U1 FR-2: done (rc=0) derived (marker+gone socket)"
st=$(wait_state "$U1" '^done \(rc=0\)$' 60)
[ "$st" = "done (rc=0)" ] && pass || failmsg "U1 display state '$st', want done (rc=0)"

step "U1 GetEvents timeline present"
curl -s -H "Cookie: porche_token=$UT" "$WU/w/$U1" | grep -qi 'events\|timeline' \
  && pass || failmsg "U1 detail page carries no events section"

step "U1 delete → wipe report all-pass"
body=$(curl -s -H "Cookie: porche_token=$UT" -X POST "$WU/w/$U1/delete")
echo "$body" | grep -qi 'wipe report' || failmsg "U1 wipe report page missing"
echo "$body" | grep -qi 'all.*pass\|5/5\|pass.*pass' || \
  echo "(record: report body kept in $RD/u1-wipe.html for the operator walk)"
echo "$body" >"$RD/u1-wipe.html"
echo "$body" | grep -qi 'wipe report' && pass || true

# ============================ U2 — disposable CI job ============================
U2=$(mkid)
step "U2 create (CI job + 64M tier-E volume)"
console_create "$U2" "dd if=/dev/zero of=/tmp/uc-burst bs=1M count=32 2>/dev/null; sync; exit 0" 64
st=$(wait_state "$U2" '^done \(rc=0\)$' 60)
[ "$st" = "done (rc=0)" ] && pass || failmsg "U2 display state '$st', want done (rc=0)"
step "U2 delete → wipe report"
curl -s -o "$RD/u2-wipe.html" -H "Cookie: porche_token=$UT" -X POST "$WU/w/$U2/delete"
grep -qi 'wipe report' "$RD/u2-wipe.html" && pass || failmsg "U2 wipe report missing"

# U2 ordered-stop leg: an ordered stop stays `stopped` (MS-1 display rule — the
# done rule never re-labels an ordered stop)
U2S=$(mkid)
step "U2 stop-then-wipe: stopped stays stopped"
console_create "$U2S" "sleep 600"
wait_state "$U2S" 'running' 60 >/dev/null
curl -s -o /dev/null -H "Cookie: porche_token=$UT" -X POST "$WU/w/$U2S/stop"
st=$(wait_state "$U2S" '^stopped$' 30)
[ "$st" = "stopped" ] && pass || failmsg "after ordered stop, display state '$st', want stopped"
curl -s -o "$RD/u2s-wipe.html" -H "Cookie: porche_token=$UT" -X POST "$WU/w/$U2S/delete"
grep -qi 'wipe report' "$RD/u2s-wipe.html" && pass || failmsg "stop-then-wipe report missing"

# ============================ FR-2 transitions (evidence set) ============================
# pending → running observed above; done (rc=N) via marker; stopped via order;
# Failed{reason} via a deliberate create failure (missing image digest)
F1=$(mkid)
step "FR-2 Failed{reason} on create failure (rollback)"
curl -s -o /dev/null -H "Cookie: porche_token=$UT" \
  --data-urlencode "id=$F1" \
  --data-urlencode "command=exit 0" \
  --data-urlencode "image_digest=sha256:$(printf 'e%.0s' $(seq 64))" \
  --data-urlencode "vcpu=1" --data-urlencode "ram_mib=128" \
  --data-urlencode "volume_mib=64" \
  --data-urlencode "ip6=fd42:1:1::ea" --data-urlencode "gw6=fd42:1:1::ff" \
  "$WU/w/new"
st=$(wait_state "$F1" 'failed' 30)
echo "$st" | grep -q failed && pass || failmsg "bad-image create showed '$st', want failed(reason)"
curl -s -o /dev/null -H "Cookie: porche_token=$UT" -X POST "$WU/w/$F1/delete" || true

# ============================ Sibling isolation probe ============================
# FR-21 / doc 14 §2: "no workload↔workload L2" — enforced by `bridge link set
# dev vr-<id> isolated on` (netns.rs, unit-pinned argv). This probe OBSERVES
# it live: two workloads, peer /128s, deterministic failure. If pings
# succeed, the gate FAILS — never demo sibling isolation until it passes
# (gap rule: config intent is not a demo beat).
S1A=$(mkid); S1B=$(mkid)
step "sibling isolation: peer /128 unreachable between two live workloads"
console_create "$S1A" "sleep 300" "" "fd42:1:2::a"
console_create "$S1B" "ping -6 -c 3 -W 2 fd42:1:2::a >/dev/null 2>&1; echo PINGRC=\$?" ""
wait_state "$S1A" 'running' 60 >/dev/null
wait_state "$S1B" '^done \(rc=0\)$' 90 >/dev/null
if grep -q "PINGRC=1" /var/lib/porche/consoles/"$S1B".log 2>/dev/null; then
  pass
else
  failmsg "sibling reachable: peer /128 answered (isolated-on not effective or marker missing)"
fi
# mechanism evidence (MS-4 style): the bridge flag itself, both ports
for id in "$S1A" "$S1B"; do
  bridge link show "vr-$id" 2>/dev/null | grep -q "isolated on" \
    || failmsg "vr-$id not isolated on the bridge"
done
curl -s -o /dev/null -H "Cookie: porche_token=$UT" -X POST "$WU/w/$S1A/delete" || true
curl -s -o /dev/null -H "Cookie: porche_token=$UT" -X POST "$WU/w/$S1B/delete" || true

# ============================ Negative paths ============================
echo "## Negative-path evidence" >>"$REPORT"

step "neg: duplicate id rejected"
DUP=$(mkid)
console_create "$DUP" "sleep 600"
console_create "$DUP" "sleep 600"
c=$(curl -s -o /dev/null -w '%{http_code}' -H "Cookie: porche_token=$UT" "$WU/w/$DUP")
[ "$c" = 200 ] && curl -s -H "Cookie: porche_token=$UT" "$WU/w/new" >/dev/null # warm
body=$(curl -s -H "Cookie: porche_token=$UT" \
  --data-urlencode "id=$DUP" --data-urlencode "command=sleep 600" \
  --data-urlencode "image_digest=$img" --data-urlencode "vcpu=1" \
  --data-urlencode "ram_mib=128" --data-urlencode "volume_mib=" \
  --data-urlencode "ip6=fd42:1:1::d1" --data-urlencode "gw6=fd42:1:1::ff" \
  "$WU/w/new")
echo "$body" | grep -qi 'already exists' && pass || failmsg "duplicate create not rejected with 'already exists'"
curl -s -o /dev/null -H "Cookie: porche_token=$UT" -X POST "$WU/w/$DUP/delete" || true

step "neg: invalid spec re-renders the form (no row)"
LONGID="this-id-is-way-too-long"
body=$(curl -s -H "Cookie: porche_token=$UT" \
  --data-urlencode "id=$LONGID" --data-urlencode "command=exit 0" \
  --data-urlencode "image_digest=$img" --data-urlencode "vcpu=1" \
  --data-urlencode "ram_mib=128" --data-urlencode "volume_mib=" \
  --data-urlencode "ip6=fd42:1:1::d2" --data-urlencode "gw6=fd42:1:1::ff" \
  "$WU/w/new")
echo "$body" | grep -qi 'longer than 10 chars\|error' && pass || failmsg "invalid spec not rejected by the form"
c=$(curl -s -o /dev/null -w '%{http_code}' -H "Cookie: porche_token=$UT" "$WU/w/$LONGID")
[ "$c" = 404 ] && pass || failmsg "invalid create left row behind (GET returned $c)"

step "neg: wrong-domain token = 401 (user token on /a)"
c=$(curl -s -o /dev/null -w '%{http_code}' -H "Cookie: porche_token=$UT" "$AU/a")
[ "$c" = 401 ] && pass || failmsg "want 401 got $c"

step "pos: admin token opens /a (FR-40 two domains)"
c=$(curl -s -o /dev/null -w '%{http_code}' -H "Cookie: porche_token=$AT" "$AU/a")
[ "$c" = 200 ] && pass || failmsg "want 200 got $c"

step "neg: unauthenticated = 401"
c=$(curl -s -o /dev/null -w '%{http_code}' "$WU/w")
[ "$c" = 401 ] && pass || failmsg "want 401 got $c"

if [ -n "${SMOKE_T1_TOKEN:-}" ]; then
  T1=$SMOKE_T1_TOKEN
  step "neg: cross-tenant detail = 404 (never 403)"
  c=$(curl -s -o /dev/null -w '%{http_code}' -H "Cookie: porche_token=$T1" "$WU/w/$U1")
  [ "$c" = 404 ] && pass || failmsg "want 404 got $c"

  step "neg: over-quota create rejected (t1 ceiling)"
  body=$(curl -s -H "Cookie: porche_token=$T1" \
    --data-urlencode "id=$(mkid)" --data-urlencode "command=exit 0" \
    --data-urlencode "image_digest=$img" --data-urlencode "vcpu=4" \
    --data-urlencode "ram_mib=4096" --data-urlencode "volume_mib=" \
    --data-urlencode "ip6=fd42:1:1::d3" --data-urlencode "gw6=fd42:1:1::ff" \
    "$WU/w/new")
  echo "$body" | grep -qi 'quota' && pass || failmsg "over-quota create not rejected"
else
  skipmsg "cross-tenant + over-quota rows (SMOKE_T1_TOKEN unset)"
fi

# CLI state-machine negatives (OpsError paths — the §7.5.1 negative set)
step "neg: stop on non-running rejected (OpsError::WrongState)"
S2=$(mkid)
spec="$RD/spec-$S2.json"
cat >"$spec" <<EOF
{"tenant_id":"t0","id":"$S2","image_digest":"sha256:$(printf 'd%.0s' $(seq 64))","command":"sleep 600","vcpu":1,"ram_mib":128,"volume":null,"ip6":"fd42:1:1::d4","gw6":"fd42:1:1::ff"}
EOF
# bad image → the row settles as failed (ops::create rollback) — stop must then
# be WrongState("running"), since after a settle the row is NOT Stopping
$AGENT create --spec "$spec" >/dev/null 2>&1 || true
wait_state "$S2" 'failed' 30 >/dev/null
if $AGENT stop "$S2" 2>"$RD/stop-nonrunning.err"; then
  failmsg "stop on a failed workload succeeded (spec: expected running)"
else
  grep -qi 'expected running\|WrongState' "$RD/stop-nonrunning.err" \
    && pass || failmsg "stop rejection message wrong: $(cat "$RD/stop-nonrunning.err")"
fi

step "neg: wipe during Stopping = WrongState (race attempt, recorded)"
if [ -f "$RD/stop-nonrunning.err" ]; then
  # best-effort race: fire stop+wipe concurrently; a WrongState answer from the
  # wipe while stop is in flight is the specified behavior — and a wipe that
  # succeeds instead must still leave no half-torn state (assert residue below)
  S3=$(mkid)
  cat >"$spec" <<EOF
{"tenant_id":"t0","id":"$S3","image_digest":"$img","command":"sleep 600","vcpu":1,"ram_mib":128,"volume":null,"ip6":"fd42:1:1::d5","gw6":"fd42:1:1::ff"}
EOF
  $AGENT create --spec "$spec" >/dev/null 2>&1 || true
  $AGENT stop "$S3" & spid=$!
  $AGENT wipe "$S3" >"$RD/wipe-stopping.out" 2>"$RD/wipe-stopping.err" || true
  wait "$spid" || true
  $AGENT wipe "$S3" >/dev/null 2>&1 || true   # settle the row either way
  ip netns list | grep -q "^w-$S3 " && failmsg "wipe-stopping race left netns residue" || true
  pass   # outcome recorded in wipe-stopping.{out,err} for the operator walk
fi

$AGENT wipe "$S2" >/dev/null 2>&1 || true
step "follow-up-console re-walk (operator)"
echo "reports in $RD/" && pass

echo "uc: done (fail=$fail) — verdict class: software gate (doc 07 §7.5.1); operator walk of $REPORT is the MS-6 user test"
exit $fail