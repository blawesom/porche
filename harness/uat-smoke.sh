#!/usr/bin/env bash
# MS-1 UAT smoke backstop (doc 07 §8.6; runs beside — never instead of — the recorded
# user test). Hits the console-host container edges with the two tokens and the
# host-agent's create→done→delete cycle. It crashes on state drift; its stdout appends
# to user-test.md as machine trace.
#
# usage: uat-smoke.sh <user-token> <admin-token> [user-url] [admin-url]
# defaults: http://127.0.0.1:18081 (user) http://127.0.0.1:18082 (admin)
set -euo pipefail

UT=${1:?usage: uat-smoke.sh <user-token> <admin-token> [user-url] [admin-url]}
AT=${2:?need admin token}
WU=${3:-http://127.0.0.1:18081}
AU=${4:-http://127.0.0.1:18082}
fail=0
step() { printf 'smoke: %-28s ' "$1"; }
pass() { echo "pass"; }
failmsg() { echo "FAIL — $1"; fail=1; }

step "user login 200"
c=$(curl -s -o /dev/null -w '%{http_code}' -H "Cookie: porche_token=$UT" "$WU/w")
[ "$c" = 200 ] && pass || failmsg "want 200 got $c"

step "admin login 200"
c=$(curl -s -o /dev/null -w '%{http_code}' -H "Cookie: porche_token=$AT" "$AU/a")
[ "$c" = 200 ] && pass || failmsg "want 200 got $c"

step "cross-domain user on /a = 401"
c=$(curl -s -o /dev/null -w '%{http_code}' -H "Cookie: porche_token=$UT" "$AU/a")
[ "$c" = 401 ] && pass || failmsg "want 401 got $c"

step "cross-domain admin on /w = 401"
c=$(curl -s -o /dev/null -w '%{http_code}' -H "Cookie: porche_token=$AT" "$WU/w")
[ "$c" = 401 ] && pass || failmsg "want 401 got $c"

step "unauthenticated = 401"
c=$(curl -s -o /dev/null -w '%{http_code}' "$WU/w")
[ "$c" = 401 ] && pass || failmsg "want 401 got $c"

step "create form 200"
c=$(curl -s -o /dev/null -w '%{http_code}' -H "Cookie: porche_token=$UT" "$WU/w/new")
[ "$c" = 200 ] && pass || failmsg "want 200 got $c"

step "create→done probe (live path)"
id="sm$(date +%s | tail -c 7)"   # ≤10 chars: it names netns/tap devices (spec contract)
img=$(curl -s -H "Cookie: porche_token=$UT" "$WU/w/new" \
  | grep -oE 'sha256:[0-9a-f]{64}' | head -1 || true)
[ -n "$img" ] || { failmsg "no image in create form"; }
curl -s -o /dev/null -H "Cookie: porche_token=$UT" \
  --data-urlencode "id=$id" \
  --data-urlencode "command=printf smoke" \
  --data-urlencode "image_digest=$img" \
  --data-urlencode "vcpu=1" \
  --data-urlencode "ram_mib=128" \
  --data-urlencode "volume_mib=64" \
  --data-urlencode "ip6=fd42:1:1::99" \
  --data-urlencode "gw6=fd42:1:1::ff" \
  "$WU/w/new"
# poll the detail until done (rc) or timeout — the create is converged live
state=""
for _ in $(seq 1 30); do
  sleep 1
  state=$(curl -s -H "Cookie: porche_token=$UT" "$WU/w/$id" \
    | grep -oE 'done \(rc=[0-9]+\)|running|pending|failed[^<]*' | head -1)
  case "$state" in done*) break ;; esac
done
case "$state" in
  'done (rc=0)') pass ;;
  *) failmsg "expected done (rc=0), got '$state'" ;;
esac

step "delete→wipe report renders"
body=$(curl -s -H "Cookie: porche_token=$UT" -X POST "$WU/w/$id/delete")
echo "$body" | grep -q 'wipe report' && pass || failmsg "wipe report page missing"

# --- MS-2 tenancy backstops (E-31.2) ------------------------------------
# usage: also export SMOKE_T1_TOKEN (a second tenant's token) to exercise
# the isolation checks; without it they are skipped.
if [ -n "${SMOKE_T1_TOKEN:-}" ]; then
  T1=$SMOKE_T1_TOKEN

  step "t1 login 200 (own view)"
  c=$(curl -s -o /dev/null -w '%{http_code}' -H "Cookie: porche_token=$T1" "$WU/w")
  [ "$c" = 200 ] && pass || failmsg "want 200 got $c"

  step "t1 sees no t0 rows (isolation)"
  body=$(curl -s -H "Cookie: porche_token=$T1" "$WU/w")
  if echo "$body" | grep -q 'no workloads' || ! echo "$body" | grep -q "$id"; then
    pass
  else
    failmsg "t1 list leaks t0 rows"
  fi

  step "t1 detail of t0 workload = 404"
  c=$(curl -s -o /dev/null -w '%{http_code}' -H "Cookie: porche_token=$T1" "$WU/w/$id")
  [ "$c" = 404 ] && pass || failmsg "want 404 got $c (cross-tenant leak)"

  step "t1 token on /a = 401 (domain split)"
  c=$(curl -s -o /dev/null -w '%{http_code}' -H "Cookie: porche_token=$T1" "$AU/a")
  [ "$c" = 401 ] && pass || failmsg "want 401 got $c"
fi

echo "smoke: done (fail=$fail)"
exit $fail