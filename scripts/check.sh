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
echo "== HTTP smoke test (requires a running server on 127.0.0.1:8081)"
curl -sf http://127.0.0.1:8081/api/health >/dev/null && echo "health ok"
curl -sf http://127.0.0.1:8081/api/cases/harbor-annex | grep -q '"proposalCents":580000' && echo "golden case ok"
echo "ALL CHECKS PASSED"
