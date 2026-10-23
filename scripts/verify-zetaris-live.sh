#!/usr/bin/env bash
# Verify a live Zetaris Gateway with a real SELECT 1.
# Reads ZETARIS_* from .env. Never prints the key, password, or token.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
python3 - <<'PY'
import json
import os
import urllib.error
import urllib.request
import uuid
from pathlib import Path

env = {}
for line in Path(".env").read_text().splitlines() if Path(".env").exists() else []:
    line = line.strip()
    if not line or line.startswith("#") or "=" not in line:
        continue
    key, value = line.split("=", 1)
    env[key.strip()] = value.strip().strip('"').strip("'")

base = env.get("ZETARIS_BASE_URL", "").rstrip("/")
style = env.get("ZETARIS_API_STYLE", "gateway")
if not base:
    raise SystemExit("ZETARIS_BASE_URL is not set in .env")

def call(path, method="GET", body=None, token=None):
    headers = {"X-Request-ID": str(uuid.uuid4())}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    if env.get("ZETARIS_ORG_ID"):
        headers["X-Org-ID"] = env["ZETARIS_ORG_ID"]
    data = None
    if body is not None:
        headers["Content-Type"] = "application/json"
        data = json.dumps(body).encode()
    request = urllib.request.Request(base + path, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return response.status, json.loads(response.read().decode() or "null")
    except urllib.error.HTTPError as error:
        return error.code, error.read(300).decode(errors="replace")
    except Exception as error:
        raise SystemExit(f"Cannot reach {base}: {error}")

token = env.get("ZETARIS_API_KEY")
if not token:
    username = env.get("ZETARIS_USERNAME")
    password = env.get("ZETARIS_PASSWORD")
    if not username or not password:
        raise SystemExit("Set ZETARIS_API_KEY or ZETARIS_USERNAME/ZETARIS_PASSWORD in .env")
    status, body = call("/api/auth/login", "POST", {"username": username, "password": password})
    if status != 200 or not isinstance(body, dict):
        raise SystemExit(f"Gateway login failed: HTTP {status}")
    token = body.get("idToken") or body.get("token")
    if not token:
        raise SystemExit("Gateway login returned no idToken")

if style == "saas":
    status, body = call("/api/v1.0/datamart", token=token)
    print(f"health HTTP {status}: {len(body) if isinstance(body, list) else 'non-list'}")
    status, body = call("/api/v1.0/query/sql/start", "POST", {"select": "SELECT 1 AS one", "pageLimit": 1}, token)
    count = body.get("totalCount") if isinstance(body, dict) else None
    print(f"query HTTP {status}: totalCount={count}")
elif style == "rest":
    status, body = call("/datasource/datasources", token=token)
    print(f"health HTTP {status}")
    status, body = call("/sql-editor/sqls/run", "POST", {"queryId": str(uuid.uuid4()), "sql": "SELECT 1 AS one", "source": "SqlEditor", "limit": 1}, token)
    print(f"query HTTP {status}")
else:
    status, body = call("/api/proxy/lightning-database/databases", token=token)
    print(f"health HTTP {status}: {len(body) if isinstance(body, list) else 'non-list'}")
    status, body = call("/api/proxy/sql-editor/sqls/run-query", "POST", {"sql": "SELECT 1 AS one", "limit": 1}, token)
    rows = None
    if isinstance(body, dict):
        rows = body.get("data") or body.get("rows")
    print(f"query HTTP {status}: rows={len(rows) if isinstance(rows, list) else 'unknown'}")
    if status != 200 or not isinstance(rows, list) or len(rows) != 1:
        raise SystemExit(f"SELECT 1 did not return one row: {body}")
print("LIVE ZETARIS QUERY OK")
PY
