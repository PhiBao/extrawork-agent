#!/usr/bin/env bash
# Verify the Zetaris Gateway client over real HTTP against the local contract
# mock, then restore the demo. The mock is not Zetaris.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BASE="${EXTRAWORK_URL:-http://127.0.0.1:8081}"
PORT=9099
APP_PORT=8082
MOCK_PID=""
APP_PID=""
cleanup() {
  [ -n "$APP_PID" ] && kill "$APP_PID" 2>/dev/null || true
  [ -n "$MOCK_PID" ] && kill "$MOCK_PID" 2>/dev/null || true
  curl -sf -X POST "$BASE/api/cases/harbor-annex/reset" >/dev/null 2>&1 || true
}
trap cleanup EXIT

curl -sf "$BASE/api/health" >/dev/null || { echo "start the app first: ./scripts/demo.sh"; exit 1; }
for p in "$PORT" "$APP_PORT"; do
  pid=$(lsof -ti "tcp:$p" 2>/dev/null || true)
  [ -n "$pid" ] && kill "$pid" 2>/dev/null || true
done
sleep 1

python3 "$ROOT/scripts/mock_zetaris.py" "$PORT" gateway > /tmp/extrawork-mock.log 2>&1 &
MOCK_PID=$!
sleep 1

DATABASE_URL="${DATABASE_URL:-postgres://extrawork:extrawork@127.0.0.1:54329/extrawork}" \
EXTRAWORK_ROOT="$ROOT" EXTRAWORK_BIND="127.0.0.1:$APP_PORT" \
ZETARIS_API_STYLE=gateway ZETARIS_BASE_URL="http://127.0.0.1:$PORT" \
ZETARIS_API_KEY=test-key ZETARIS_ORG_ID=123 \
"$ROOT/server/target/debug/extrawork" > /tmp/extrawork-gateway.log 2>&1 &
APP_PID=$!
sleep 2

health=$(curl -sf "http://127.0.0.1:$APP_PORT/api/health")
python3 - "$health" <<'PY'
import json, sys
z = json.loads(sys.argv[1])["zetaris"]
assert z["live"] is True, z
assert z["style"] == "gateway", z
print("ok   gateway health", z["detail"])
PY

case_json=$(curl -sf -X POST "http://127.0.0.1:$APP_PORT/api/cases/harbor-annex/reset")
python3 - "$case_json" <<'PY'
import json, sys
z = json.loads(sys.argv[1])["proof"]["zetaris"]
assert z["live"] is True, z
assert z["style"] == "gateway", z
assert z["totalCount"] == 2, z
assert z["queryClosed"] is True, z
assert len(z["recordSample"]) == 2, z
print("ok   gateway join", z["totalCount"], "rows; query closed")
PY

grep -q 'POST /api/proxy/sql-editor/sqls/run-query' /tmp/extrawork-mock.log || { echo "FAIL query path"; exit 1; }
echo "ZETARIS GATEWAY VERIFIED AGAINST CONTRACT MOCK"
