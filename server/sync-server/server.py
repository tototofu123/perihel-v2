#!/usr/bin/env python3
"""Perihel sync server — thin relay for captures, runs on the free-tier e2-micro.

Local SQLite stays the source of truth on each device; this service is the
meeting point. Protocol (all JSON, bearer-token auth, TLS via self-signed cert):

  GET  /v1/health                      -> {"ok": true, "server_time": ...}
  GET  /v1/captures?since=<rfc3339>    -> rows with updated_at > since (tombstones included)
  POST /v1/captures {"captures": [...]}-> last-write-wins upsert by updated_at

Standard library only — no pip, nothing to break on a 1 GB Debian box.
"""
import json
import os
import sqlite3
import ssl
import threading
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse, parse_qs

DATA_DIR = os.environ.get("PERIHEL_SYNC_DIR", "/opt/perihel-sync")
DB_PATH = os.path.join(DATA_DIR, "sync.db")
TOKEN_PATH = os.path.join(DATA_DIR, "token")
CERT_PATH = os.environ.get("PERIHEL_SYNC_CERT", os.path.join(DATA_DIR, "cert.pem"))
KEY_PATH = os.environ.get("PERIHEL_SYNC_KEY", os.path.join(DATA_DIR, "key.pem"))
PORT = int(os.environ.get("PERIHEL_SYNC_PORT", "8443"))

with open(TOKEN_PATH) as f:
    AUTH_TOKEN = f.read().strip()

_db_lock = threading.Lock()

SCHEMA = """
CREATE TABLE IF NOT EXISTS captures (
    uuid TEXT PRIMARY KEY,
    content TEXT NOT NULL,
    content_type TEXT NOT NULL DEFAULT 'text',
    status TEXT NOT NULL DEFAULT 'pending',
    created_at TEXT NOT NULL,
    scheduled_at TEXT,
    ai_summary TEXT,
    agent_used TEXT,
    updated_at TEXT NOT NULL,
    deleted INTEGER NOT NULL DEFAULT 0,
    notified INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_captures_updated ON captures (updated_at);
CREATE INDEX IF NOT EXISTS idx_captures_due ON captures (status, scheduled_at);
"""

FIELDS = ["content", "content_type", "status", "created_at",
          "scheduled_at", "ai_summary", "agent_used"]


def now_utc() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds")


def db() -> sqlite3.Connection:
    conn = sqlite3.connect(DB_PATH, timeout=10)
    conn.row_factory = sqlite3.Row
    return conn


def row_to_capture(row) -> dict:
    return {
        "uuid": row["uuid"],
        "content": row["content"],
        "content_type": row["content_type"],
        "status": row["status"],
        "created_at": row["created_at"],
        "scheduled_at": row["scheduled_at"],
        "ai_summary": row["ai_summary"],
        "agent_used": row["agent_used"],
        "updated_at": row["updated_at"],
        "deleted": bool(row["deleted"]),
    }


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):
        print(f"[{now_utc()}] {self.address_string()} {fmt % args}", flush=True)

    def _auth(self) -> bool:
        return self.headers.get("Authorization", "") == f"Bearer {AUTH_TOKEN}"

    def _json(self, code: int, body: dict):
        data = json.dumps(body).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        if not self._auth():
            return self._json(401, {"error": "unauthorized"})
        url = urlparse(self.path)
        if url.path == "/v1/health":
            return self._json(200, {"ok": True, "server_time": now_utc()})
        if url.path == "/v1/captures":
            since = parse_qs(url.query).get("since", [None])[0]
            with _db_lock, db() as conn:
                if since:
                    rows = conn.execute(
                        "SELECT * FROM captures WHERE updated_at > ? ORDER BY updated_at ASC",
                        (since,)).fetchall()
                else:
                    rows = conn.execute(
                        "SELECT * FROM captures ORDER BY updated_at ASC").fetchall()
            return self._json(200, {"server_time": now_utc(),
                                    "captures": [row_to_capture(r) for r in rows]})
        return self._json(404, {"error": "not found"})

    def do_POST(self):
        if not self._auth():
            return self._json(401, {"error": "unauthorized"})
        url = urlparse(self.path)
        if url.path != "/v1/captures":
            return self._json(404, {"error": "not found"})
        try:
            length = int(self.headers.get("Content-Length", "0"))
            body = json.loads(self.rfile.read(length))
            incoming = body.get("captures", [])
        except (ValueError, json.JSONDecodeError):
            return self._json(400, {"error": "bad json"})

        accepted = []
        with _db_lock, db() as conn:
            for cap in incoming:
                uuid = str(cap.get("uuid", "")).strip()
                if not uuid:
                    continue
                updated = str(cap.get("updated_at") or cap.get("created_at") or now_utc())
                existing = conn.execute(
                    "SELECT updated_at FROM captures WHERE uuid = ?", (uuid,)).fetchone()
                if existing and existing["updated_at"] >= updated:
                    continue  # server copy is newer — last write wins
                conn.execute(
                    """INSERT INTO captures
                       (uuid, content, content_type, status, created_at, scheduled_at,
                        ai_summary, agent_used, updated_at, deleted)
                       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                       ON CONFLICT(uuid) DO UPDATE SET
                         content=excluded.content, content_type=excluded.content_type,
                         status=excluded.status, created_at=excluded.created_at,
                         scheduled_at=excluded.scheduled_at, ai_summary=excluded.ai_summary,
                         agent_used=excluded.agent_used, updated_at=excluded.updated_at,
                         deleted=excluded.deleted""",
                    (uuid,
                     str(cap.get("content", "")),
                     str(cap.get("content_type", "text")),
                     str(cap.get("status", "pending")),
                     str(cap.get("created_at") or now_utc()),
                     cap.get("scheduled_at"),
                     cap.get("ai_summary"),
                     cap.get("agent_used"),
                     updated,
                     1 if cap.get("deleted") else 0))
                accepted.append(uuid)
        return self._json(200, {"server_time": now_utc(), "accepted": accepted})


def main():
    os.makedirs(DATA_DIR, exist_ok=True)
    with _db_lock, db() as conn:
        conn.executescript(SCHEMA)

    httpd = ThreadingHTTPServer(("0.0.0.0", PORT), Handler)
    use_tls = os.environ.get("PERIHEL_SYNC_NO_TLS") != "1"
    if use_tls:
        ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        ctx.load_cert_chain(CERT_PATH, KEY_PATH)
        httpd.socket = ctx.wrap_socket(httpd.socket, server_side=True)
    mode = "TLS" if use_tls else "plain HTTP (behind reverse proxy)"
    print(f"perihel-sync listening on :{PORT} ({mode}, token auth)", flush=True)
    httpd.serve_forever()


if __name__ == "__main__":
    main()
