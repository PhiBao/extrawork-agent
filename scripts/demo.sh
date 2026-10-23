#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
if ! docker ps --format '{{.Names}}' | grep -qx extrawork-pg; then
  docker start extrawork-pg >/dev/null 2>&1 || docker run -d --name extrawork-pg -e POSTGRES_PASSWORD=extrawork -e POSTGRES_USER=extrawork -e POSTGRES_DB=extrawork -p 127.0.0.1:54329:5432 postgres:16-alpine
fi
for _ in $(seq 1 30); do
  docker exec extrawork-pg pg_isready -U extrawork >/dev/null 2>&1 && break
  sleep 1
done
if [ ! -d meterless-world/node_modules ]; then
  (cd meterless-world && pnpm install)
fi
if [ ! -d web/node_modules ]; then
  (cd web && pnpm install)
fi
(cd web && pnpm build)
if ! curl -sf http://127.0.0.1:8091/health >/dev/null; then
  (cd meterless-world && pnpm start) &
  for _ in $(seq 1 30); do
    curl -sf http://127.0.0.1:8091/health >/dev/null && break
    sleep 0.2
  done
fi
export DATABASE_URL="${DATABASE_URL:-postgres://extrawork:extrawork@127.0.0.1:54329/extrawork}"
export EXTRAWORK_ROOT="$PWD"
exec cargo run --manifest-path server/Cargo.toml
