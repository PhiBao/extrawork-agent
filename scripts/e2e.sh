#!/usr/bin/env bash
# End-to-end API verification against a running ExtraWork instance.
set -euo pipefail
BASE="${EXTRAWORK_URL:-http://127.0.0.1:8081}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
pass() { printf 'ok   %s\n' "$1"; }
fail() { printf 'FAIL %s\n' "$1"; exit 1; }

json() { python3 -c 'import json,sys; d=json.load(sys.stdin); print(eval(sys.argv[1]))' "$1"; }

curl -sf -X POST "$BASE/api/cases/harbor-annex/reset" >/dev/null
pass "reset"

case_json=$(curl -sf "$BASE/api/cases/harbor-annex")
[ "$(echo "$case_json" | json 'd["submittedCents"]')" = "680000" ] || fail "submitted total"
[ "$(echo "$case_json" | json 'd["proposalCents"]')" = "580000" ] || fail "proposal total"
[ "$(echo "$case_json" | json 'd["workflow"]')" = "needs_review" ] || fail "workflow"
[ "$(echo "$case_json" | json 'd["billingPosture"]')" = "not_authorized" ] || fail "posture"
[ "$(echo "$case_json" | json '[o["status"] for o in d["objections"]]')" = "['repairable', 'unresolved', 'answered']" ] || fail "objection states"
pass "golden case"

revision=$(echo "$case_json" | json 'd["revision"]')
key=$(echo "$case_json" | json '[f["key"] for f in d["findings"] if f["kind"]=="duplicate_charge"][0]')
decided=$(curl -sf -X POST -H 'content-type: application/json' \
  -d "{\"findingKey\":\"$key\",\"decision\":\"accept\",\"expectedRevision\":$revision}" \
  "$BASE/api/cases/harbor-annex/decisions")
[ "$(echo "$decided" | json 'd["supportedCents"]')" = "580000" ] || fail "accepted duplicate total"
[ "$(echo "$decided" | json 'd["billingPosture"]')" = "not_authorized" ] || fail "duplicate changed authority"
[ "$(echo "$decided" | json 'len([l for l in d["lines"] if l["removed"]])')" = "1" ] || fail "one removed line"
pass "accept duplicate"

uploaded=$(curl -sf -F "file=@$ROOT/fixtures/harbor-annex/rate-approval.eml" "$BASE/api/cases/harbor-annex/sources")
[ "$(echo "$uploaded" | json '[o["status"] for o in d["objections"] if o["topic"]=="rate"][0]')" = "answered" ] || fail "rate approval did not answer"
[ "$(echo "$uploaded" | json 'd["supportedCents"]')" = "580000" ] || fail "upload changed money"
pass "new evidence changes one finding"

packet=$(curl -sf "$BASE/api/cases/harbor-annex/export")
echo "$packet" | grep -q '\$5,800.00' || fail "packet total"
echo "$packet" | grep -qi 'not a billing approval' || fail "packet disclaimer"
pass "review packet"

curl -sf "$BASE/api/cases/harbor-annex/export.zip" -o /tmp/extrawork-e2e.zip
unzip -l /tmp/extrawork-e2e.zip | grep -q 'packet.html' || fail "zip packet"
unzip -l /tmp/extrawork-e2e.zip | grep -q 'manifest.json' || fail "zip manifest"
pass "record zip"

code=$(curl -s -o /dev/null -w '%{http_code}' -X POST -H 'content-type: application/json' \
  -d '{"findingKey":"anything","decision":"accept","expectedRevision":999}' \
  "$BASE/api/cases/harbor-annex/decisions")
[ "$code" = "409" ] || fail "stale revision expected 409, got $code"
pass "stale decision rejected"

printf '%%PDF-1.4 /Filter /FlateDecode' > /tmp/extrawork-bad.pdf
code=$(curl -s -o /tmp/extrawork-bad-response -w '%{http_code}' -F "file=@/tmp/extrawork-bad.pdf" "$BASE/api/cases/harbor-annex/sources")
[ "$code" = "400" ] || fail "compressed pdf expected 400, got $code"
grep -q 'compressed' /tmp/extrawork-bad-response || fail "pdf error message"
pass "unsupported pdf rejected with a reason"

curl -sf -X POST "$BASE/api/cases/harbor-annex/reset" >/dev/null
[ "$(curl -sf "$BASE/api/cases/harbor-annex" | json 'd["revision"]')" = "1" ] || fail "reset revision"
pass "reset restores the example"

echo "E2E PASSED"
