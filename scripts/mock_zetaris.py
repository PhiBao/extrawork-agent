#!/usr/bin/env python3
"""Local contract mock for Zetaris surfaces. This is NOT Zetaris.

Implements enough of the published shapes to exercise ExtraWork's client over
real HTTP:
  gateway: /api/auth/login, /api/proxy/lightning-database/databases,
           /api/proxy/sql-editor/sqls/run-query
  rest:    /api/v1.0/datasource/datasources, /api/v1.0/sql-editor/sqls/run
  saas:    /api/v1.0/datamart, /api/v1.0/query/sql/start, /query/sql/close/{token}
"""
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import sys

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 9099
MODE = sys.argv[2] if len(sys.argv) > 2 else "gateway"
TOKEN = "test-key"
LOGIN_TOKEN = "jwt.test.token"
ORG = "123"

ROWS = [
    {"line_id": "L1", "amount_cents": 240000, "document": "field-ticket.txt", "excerpt": "16 overtime hours"},
    {"line_id": "L4", "amount_cents": 100000, "document": "invoice-902.txt", "excerpt": "Scissor lift, one day, $1,000.00."},
]


class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        sys.stderr.write("%s %s\n" % (self.command, self.path))

    def body(self):
        length = int(self.headers.get("content-length", "0"))
        raw = self.rfile.read(length) if length else b"{}"
        try:
            return json.loads(raw or b"{}")
        except json.JSONDecodeError:
            return {}

    def token(self):
        auth = self.headers.get("Authorization", "")
        return auth.removeprefix("Bearer ").strip()

    def authorized(self):
        if not self.headers.get("X-Request-ID"):
            return False
        if self.token() == LOGIN_TOKEN:
            return True
        if self.token() != TOKEN:
            return False
        if MODE in ("rest", "saas") and self.headers.get("X-Org-ID") != ORG:
            return False
        return True

    def send_json(self, status, payload):
        body = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self):
        if self.path == "/api/auth/login" and MODE == "gateway":
            if self.body().get("password") == "test-password":
                self.send_json(200, {"idToken": LOGIN_TOKEN})
            else:
                self.send_json(401, {"message": "bad login"})
            return
        if not self.authorized():
            self.send_json(401, {"message": "bad credentials"})
            return
        body = self.body()
        if self.path == "/api/proxy/sql-editor/sqls/run-query":
            if "sql" not in body or "limit" not in body:
                self.send_json(400, {"message": "sql and limit are required"})
                return
            self.send_json(200, {"data": ROWS})
            return
        if self.path == "/api/v1.0/sql-editor/sqls/run":
            if not {"queryId", "sql", "source", "limit"} <= set(body):
                self.send_json(400, {"message": "documented body required"})
                return
            self.send_json(200, {"data": ROWS})
            return
        if self.path == "/api/v1.0/query/sql/start":
            if "select" not in body or "pageLimit" not in body:
                self.send_json(400, {"message": "select and pageLimit are required"})
                return
            self.send_json(200, {
                "records": ROWS,
                "pageNumber": 1,
                "pageLimit": body["pageLimit"],
                "totalCount": len(ROWS),
                "totalPages": 1,
                "queryToken": "qt-123",
            })
            return
        self.send_json(404, {"message": "not found"})

    def do_GET(self):
        if not self.authorized():
            self.send_json(401, {"message": "bad credentials"})
            return
        if self.path == "/api/proxy/lightning-database/databases":
            self.send_json(200, ["harbor_annex"])
            return
        if self.path == "/api/v1.0/datasource/datasources":
            self.send_json(200, [{"name": "harbor_annex"}])
            return
        if self.path == "/api/v1.0/datamart":
            self.send_json(200, [{"id": "dm-1", "name": "Harbor Annex"}])
            return
        self.send_json(404, {"message": "not found"})

    def do_DELETE(self):
        if not self.authorized():
            self.send_json(401, {"message": "bad credentials"})
            return
        if self.path == "/api/v1.0/query/sql/close/qt-123":
            self.send_json(200, {})
            return
        self.send_json(404, {"message": "not found"})


if __name__ == "__main__":
    print(f"mock zetaris ({MODE}) on 127.0.0.1:{PORT}", flush=True)
    HTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
