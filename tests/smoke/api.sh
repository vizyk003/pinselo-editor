#!/usr/bin/env bash
# End-to-end API smoke test: accounts, teams, reports, privacy export, security headers.
set -euo pipefail
A=${1:-http://localhost:4000}
J=$(mktemp)
E="smoke$RANDOM$RANDOM@example.com"
fail() { echo "FAIL: $*" >&2; exit 1; }
post() { curl -s -b "$J" -c "$J" -H 'content-type: application/json' -X POST "$A$1" -d "$2"; }
get() { curl -s -b "$J" -c "$J" "$A$1"; }

r=$(post /api/auth/signup "{\"email\":\"$E\",\"password\":\"correct horse battery\",\"name\":\"Smoke\",\"birthYear\":1990,\"acceptTerms\":true}")
[[ $r == *'"user"'* ]] || fail "signup: $r"
r=$(post /api/auth/signup "{\"email\":\"kid$E\",\"password\":\"correct horse battery\",\"name\":\"Kid\",\"birthYear\":$(( $(date +%Y) - 8 )),\"acceptTerms\":true}")
[[ $r == *'"age"'* ]] || fail "age gate: $r"
post /api/auth/logout '{}' > /dev/null
r=$(post /api/auth/login "{\"email\":\"$E\",\"password\":\"wrong password!\"}")
[[ $r == *'"invalid"'* ]] || fail "bad password accepted: $r"
r=$(post /api/auth/login "{\"email\":\"$E\",\"password\":\"correct horse battery\"}")
[[ $r == *'"user"'* ]] || fail "login: $r"
r=$(post /api/workspaces '{"name":"Smoke Team"}')
T=$(echo "$r" | sed -E 's/.*"id":"([^"]+)".*/\1/')
r=$(post "/api/workspaces/$T/invites" '{"emails":["friend@example.com"],"role":"designer"}')
[[ $r == *'"sent":1'* ]] || fail "invite: $r"
r=$(post /api/reports '{"targetType":"design","targetId":"x","reason":"copyright"}')
[[ $r == *'incomplete_notice'* ]] || fail "dmca validation: $r"
r=$(get /api/admin/overview)
[[ $r == *'forbidden'* ]] || fail "admin not gated: $r"
r=$(post /api/account/export '{}')
[[ $r == *'jobId'* ]] || fail "export: $r"
h=$(curl -s -D - -o /dev/null -b "$J" "$A/api/auth/me")
grep -qi '^x-content-type-options: nosniff' <<<"$h" || fail "missing nosniff"
grep -qi '^x-frame-options: DENY' <<<"$h" || fail "missing frame options"
echo "API smoke test passed"
