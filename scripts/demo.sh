#!/usr/bin/env bash
# One-command demo start: PostgreSQL, local Zetaris, Meterless, web build, app,
# then reset both cases to revision 1.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$PWD"
ZETARIS_DIR="${ZETARIS_LOCAL_DIR:-$HOME/zetaris-local/zetaris-platform}"
APP_PID=""
METERLESS_PID=""

cleanup() {
  [ -n "$APP_PID" ] && kill "$APP_PID" 2>/dev/null || true
  [ -n "$METERLESS_PID" ] && kill "$METERLESS_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

echo "== PostgreSQL =="
if ! docker ps --format '{{.Names}}' | grep -qx extrawork-pg; then
  docker start extrawork-pg >/dev/null 2>&1 || docker run -d --name extrawork-pg \
    -e POSTGRES_PASSWORD=extrawork -e POSTGRES_USER=extrawork -e POSTGRES_DB=extrawork \
    -p 127.0.0.1:54329:5432 postgres:16-alpine
fi
for _ in $(seq 1 30); do
  docker exec extrawork-pg pg_isready -U extrawork >/dev/null 2>&1 && break
  sleep 1
done

echo "== Local Zetaris =="
if [ -f "$ZETARIS_DIR/docker-compose.yml" ]; then
  (cd "$ZETARIS_DIR" && docker compose up -d)
  for _ in $(seq 1 90); do
    curl -sf -o /dev/null http://127.0.0.1:3010/api/config && break
    sleep 2
  done
  if curl -sf -o /dev/null http://127.0.0.1:3010/api/config; then
    echo "Zetaris Gateway ready on :3010"
  else
    echo "WARNING: Zetaris Gateway did not answer on :3010; the app will use the local fallback."
  fi
else
  echo "WARNING: no local Zetaris bundle at $ZETARIS_DIR; the app will use the local fallback."
fi

echo "== Meterless =="
if ! curl -sf http://127.0.0.1:8091/health >/dev/null; then
  (cd meterless-world && pnpm start) > /tmp/extrawork-meterless.log 2>&1 &
  METERLESS_PID=$!
  for _ in $(seq 1 40); do
    curl -sf http://127.0.0.1:8091/health >/dev/null && break
    sleep 0.5
  done
fi
curl -sf http://127.0.0.1:8091/health >/dev/null && echo "World Model ready on :8091" || echo "WARNING: Meterless did not answer."

echo "== Build =="
[ -d web/node_modules ] || (cd web && pnpm install)
(cd web && pnpm build >/dev/null)
cargo build --quiet --manifest-path server/Cargo.toml --bin extrawork

echo "== App =="
pid=$(lsof -ti tcp:8081 2>/dev/null || true)
if [ -n "$pid" ]; then
  kill "$pid" 2>/dev/null || true
  sleep 1
fi
export EXTRAWORK_ROOT="$ROOT"
export DATABASE_URL="${DATABASE_URL:-postgres://extrawork:extrawork@127.0.0.1:54329/extrawork}"
./server/target/debug/extrawork > /tmp/extrawork.log 2>&1 &
APP_PID=$!
for _ in $(seq 1 50); do
  curl -sf -o /dev/null http://127.0.0.1:8081/api/health && break
  sleep 0.5
done
if ! curl -sf -o /dev/null http://127.0.0.1:8081/api/health; then
  echo "The app did not start. See /tmp/extrawork.log"
  exit 1
fi

echo "== Reset both cases =="
curl -sf -X POST http://127.0.0.1:8081/api/cases/harbor-annex/reset >/dev/null
curl -sf -X POST http://127.0.0.1:8081/api/cases/northgate-mechanical/reset >/dev/null

curl -s http://127.0.0.1:8081/api/health | python3 -c 'import json,sys;d=json.load(sys.stdin);print("health:",d["ok"],"| zetaris:",d["zetaris"]["state"],"| meterless:",d["meterless"]["state"],"| nemotron:",d["nemotron"]["state"])' || true

cat <<'EOF'

ExtraWork is ready:
  http://127.0.0.1:8081/          app
  http://127.0.0.1:3010/          local Zetaris
  /tmp/extrawork.log              app log

Start with COR-184 (duplicate + rate approval), then switch to CHG-207
(shortfall + missing ticket). Ctrl-C stops the app.

EOF

wait "$APP_PID"
