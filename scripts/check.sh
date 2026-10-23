#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
echo "== Rust domain and client tests"
(cd server && cargo test --lib)
echo "== Meterless World Model feature test"
(cd meterless-world && pnpm test)
echo "== Meterless conformance suite"
(cd meterless-world && pnpm conformance)
echo "== Frontend typecheck and build"
(cd web && pnpm build)
echo "== HTTP end-to-end (requires a running server on 127.0.0.1:8081)"
if curl -sf http://127.0.0.1:8081/api/health >/dev/null; then
  ./scripts/e2e.sh
else
  echo "SKIP: server not running; start it with ./scripts/demo.sh"
fi
echo "ALL CHECKS PASSED"
