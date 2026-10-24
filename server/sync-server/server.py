#!/usr/bin/env python3
"""Perihel sync server — thin relay + AI chat gateway, free-tier e2-micro VM.

All inference is remote (NVIDIA NIM); this process only orchestrates.

  GET  /v1/health
  GET  /v1/captures?since=<rfc3339>     -> rows (tombstones included)
  POST /v1/captures {"captures": [...]} -> last-write-wins upsert
  POST /v1/chat {"content": str, "ai": bool}
                                       -> ALWAYS saves the capture first,
                                          then runs AI (rate-limited) and
                                          returns {capture, ai}
  POST /v1/captures/delete {"uuid": ..} -> 30-day soft delete (tombstone)
  POST /v1/captures/retry  {"uuid": ..} -> requeue AI processing
  GET  /v1/reminders                    -> upcoming + recent reminder history
  GET  /v1/settings                     -> notification config (client-facing)
"""
import json
import os
import sqlite3
import ssl
import threading
import uuid as uuidlib
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse, parse_qs

DATA_DIR = os.environ.get("PERIHEL_SYNC_DIR", "/opt/perihel-sync")
DB_PATH = os.path.join(DATA_DIR, "sync.db")
TOKEN_PATH = os.path.join(DATA_DIR, "token")
CERT_PATH = os.environ.get("PERIHEL_SYNC_CERT", os.path.join(DATA_DIR, "cert.pem"))
KEY_PATH = os.environ.get("PERIHEL_SYNC_KEY", os.path.join(DATA_DIR, "key.pem"))
PORT = int(os.environ.get("PERIHEL_SYNC_PORT", "8443"))
MEDIA_DIR = os.path.join(DATA_DIR, "media")
MAX_MEDIA_BYTES = 20 * 1024 * 1024  # hard cap for image/video attachments

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
    notified INTEGER NOT NULL DEFAULT 0,
    category TEXT,
    ai_state TEXT,
    ai_attempts INTEGER DEFAULT 0,
    due_at TEXT,
    deleted_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_captures_updated ON captures (updated_at);
CREATE INDEX IF NOT EXISTS idx_captures_due ON captures (status, scheduled_at);
CREATE TABLE IF NOT EXISTS reminders (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    text TEXT NOT NULL,
    due_at TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    created_at TEXT NOT NULL,
    fired_at TEXT,
    capture_uuid TEXT
);
CREATE TABLE IF NOT EXISTS memory (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
INSERT OR IGNORE INTO settings (key, value) VALUES ('active_start', '09:00');
INSERT OR IGNORE INTO settings (key, value) VALUES ('active_end', '21:00');
INSERT OR IGNORE INTO settings (key, value) VALUES ('nudge_interval_hours', '4');
INSERT OR IGNORE INTO settings (key, value) VALUES ('last_nudge_at', '');
"""

# columns added after the first deployment, idempotent
MIGRATIONS = [
    ("captures", "category", "TEXT"),
    ("captures", "ai_state", "TEXT"),
    ("captures", "ai_attempts", "INTEGER DEFAULT 0"),
    ("captures", "due_at", "TEXT"),
    ("captures", "deleted_at", "TEXT"),
    ("captures", "media_id", "TEXT"),
    ("captures", "media_type", "TEXT"),
    ("captures", "media_size", "INTEGER"),
]


def now_utc() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds")


def db() -> sqlite3.Connection:
    conn = sqlite3.connect(DB_PATH, timeout=10)
    conn.row_factory = sqlite3.Row
    return conn


def init_db():
    conn = db()
    conn.executescript(SCHEMA)
    for table, column, decl in MIGRATIONS:
        cols = [r["name"] for r in conn.execute(f"PRAGMA table_info({table})")]
        if column not in cols:
            conn.execute(f"ALTER TABLE {table} ADD COLUMN {column} {decl}")
    conn.commit()
    conn.close()


def row_to_capture(row) -> dict:
    return {
        "uuid": row["uuid"],
        "content": row["content"],
        "content_type": row["content_type"],
        "status": row["status"],
        "created_at": row["created_at"],
        "scheduled_at": row["scheduled_at"],
        "ai_summary": row["ai_summary"],
        "category": row["category"],
        "ai_state": row["ai_state"],
        "due_at": row["due_at"],
        "updated_at": row["updated_at"],
        "deleted": bool(row["deleted"]),
        "media_id": row["media_id"],
        "media_type": row["media_type"],
        "media_size": row["media_size"],
    }


def guess_type(content: str) -> str:
    lowered = content.lower()
    if lowered.startswith(("http://", "https://")) or " www." in lowered:
        return "url"
    return "text"


def upsert_capture(conn: sqlite3.Connection, cap: dict):
    cap_uuid = str(cap.get("uuid", "")).strip()
    if not cap_uuid:
        return
    updated = str(cap.get("updated_at") or cap.get("created_at") or now_utc())
    existing = conn.execute("SELECT updated_at FROM captures WHERE uuid = ?", (cap_uuid,)).fetchone()
    if existing and existing["updated_at"] >= updated:
        return  # server copy newer — last write wins
    conn.execute(
        """INSERT INTO captures
           (uuid, content, content_type, status, created_at, scheduled_at,
            ai_summary, agent_used, updated_at, deleted, category, ai_state, due_at,
            media_id, media_type, media_size)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
           ON CONFLICT(uuid) DO UPDATE SET
             content=excluded.content, content_type=excluded.content_type,
             status=excluded.status, created_at=excluded.created_at,
             scheduled_at=excluded.scheduled_at,
             ai_summary=excluded.ai_summary,
             agent_used=excluded.agent_used, updated_at=excluded.updated_at,
             deleted=excluded.deleted, category=excluded.category,
             ai_state=excluded.ai_state, due_at=excluded.due_at,
             media_id=excluded.media_id, media_type=excluded.media_type,
             media_size=excluded.media_size""",
        (cap_uuid, str(cap.get("content", "")), str(cap.get("content_type", "text")),
         str(cap.get("status", "pending")), str(cap.get("created_at") or now_utc()),
         cap.get("scheduled_at"), cap.get("ai_summary"), cap.get("agent_used"),
         updated, 1 if cap.get("deleted") else 0, cap.get("category"),
         cap.get("ai_state"), cap.get("due_at"),
         cap.get("media_id"), cap.get("media_type"), cap.get("media_size")))


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

    def _body(self) -> dict:
        length = int(self.headers.get("Content-Length", "0"))
        return json.loads(self.rfile.read(length) or b"{}")

    # ---------------- GET ----------------

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
        if url.path == "/v1/reminders":
            with _db_lock, db() as conn:
                rows = conn.execute(
                    "SELECT * FROM reminders ORDER BY due_at DESC LIMIT 50").fetchall()
                settings = {r["key"]: r["value"] for r in conn.execute("SELECT * FROM settings")}
            return self._json(200, {"server_time": now_utc(),
                                    "reminders": [dict(r) for r in rows],
                                    "settings": settings})
        if url.path == "/v1/settings":
            with _db_lock, db() as conn:
                settings = {r["key"]: r["value"] for r in conn.execute("SELECT * FROM settings")}
            return self._json(200, settings)
        if url.path.startswith("/v1/media/"):
            return self._serve_media(url.path.rsplit("/", 1)[-1])
        return self._json(404, {"error": "not found"})

    def _serve_media(self, media_id: str):
        safe = "".join(c for c in media_id if c.isalnum() or c in "._-")
        path = os.path.join(MEDIA_DIR, safe)
        if not safe or not os.path.isfile(path):
            return self._json(404, {"error": "no such media"})
        ctype = {"jpg": "image/jpeg", "jpeg": "image/jpeg", "png": "image/png",
                 "webp": "image/webp", "gif": "image/gif", "heic": "image/heic",
                 "mp4": "video/mp4", "webm": "video/webm", "3gp": "video/3gpp",
                 "mov": "video/quicktime"}.get(safe.rsplit(".", 1)[-1].lower(),
                                               "application/octet-stream")
        with open(path, "rb") as f:
            data = f.read()
        self.send_response(200)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    # ---------------- POST ----------------

    def do_POST(self):
        if not self._auth():
            return self._json(401, {"error": "unauthorized"})
        url = urlparse(self.path)

        if url.path == "/v1/chat":
            return self._chat()

        if url.path == "/v1/captures":
            try:
                incoming = self._body().get("captures", [])
            except (ValueError, json.JSONDecodeError):
                return self._json(400, {"error": "bad json"})
            with _db_lock, db() as conn:
                for cap in incoming:
                    upsert_capture(conn, cap)
            return self._json(200, {"server_time": now_utc()})

        if url.path == "/v1/captures/delete":
            cap_uuid = self._body().get("uuid", "")
            with _db_lock, db() as conn:
                conn.execute(
                    "UPDATE captures SET deleted = 1, deleted_at = ?, updated_at = ? WHERE uuid = ?",
                    (now_utc(), now_utc(), cap_uuid))
            return self._json(200, {"ok": True, "uuid": cap_uuid,
                                    "note": "soft-deleted; purged after 30 days"})

        if url.path == "/v1/captures/retry":
            cap_uuid = self._body().get("uuid", "")
            with _db_lock, db() as conn:
                conn.execute(
                    "UPDATE captures SET ai_state = 'queued', ai_attempts = 0 WHERE uuid = ?",
                    (cap_uuid,))
            return self._json(200, {"ok": True, "uuid": cap_uuid})

        if url.path == "/v1/media":
            return self._upload_media()

        return self._json(404, {"error": "not found"})

    def _upload_media(self):
        """Raw binary body; ?filename= gives the extension. Enforces the cap."""
        length = int(self.headers.get("Content-Length", "0"))
        if length <= 0:
            return self._json(400, {"error": "empty body"})
        if length > MAX_MEDIA_BYTES:
            return self._json(413, {"error": f"file too large (max {MAX_MEDIA_BYTES // (1024*1024)} MB)"})
        data = self.rfile.read(length)
        filename = parse_qs(urlparse(self.path).query).get("filename", ["attach"])[0]
        ext = "".join(c for c in filename.rsplit(".", 1)[-1].lower() if c.isalnum())[:5] or "bin"
        media_id = f"{uuidlib.uuid4().hex}.{ext}"
        os.makedirs(MEDIA_DIR, exist_ok=True)
        with open(os.path.join(MEDIA_DIR, media_id), "wb") as f:
            f.write(data)
        kind = "video" if ext in ("mp4", "webm", "3gp", "mov") else "image"
        return self._json(200, {"id": media_id, "size": length, "kind": kind})

    # ---------------- chat ----------------

    def _chat(self):
        """Save first (always), then AI. AI failure never loses the capture.
        Optional: media (uploaded via /v1/media) and a base64 thumbnail
        data-URI the vision model can actually look at."""
        try:
            body = self._body()
            content = str(body.get("content", "")).strip()
            wants_ai = bool(body.get("ai", True))
            media = body.get("media") or {}
        except (ValueError, json.JSONDecodeError):
            return self._json(400, {"error": "bad json"})
        media_id = str(media.get("id") or "") or None
        thumb = str(media.get("thumb") or "") or None
        if thumb and not thumb.startswith("data:image/"):
            thumb = None
        if not content and not media_id:
            return self._json(400, {"error": "empty content"})
        if media_id:
            content = content or "(media attachment)"

        cap_uuid = str(uuidlib.uuid4())
        created = now_utc()
        content_type = "video" if media.get("kind") == "video" else \
                       ("image" if media_id else guess_type(content))
        cap = {"uuid": cap_uuid, "content": content, "content_type": content_type,
               "status": "pending", "created_at": created, "updated_at": created,
               "scheduled_at": None, "ai_summary": None, "category": None,
               "ai_state": None, "due_at": None, "deleted": False,
               "media_id": media_id, "media_type": media.get("kind"),
               "media_size": media.get("size")}
        with _db_lock, db() as conn:
            upsert_capture(conn, cap)  # SAVED — the thought is safe from here

        if not wants_ai:
            return self._json(200, {"capture": cap, "ai": None})

        with _db_lock, db() as conn:
            conn.execute("UPDATE captures SET ai_state = 'queued' WHERE uuid = ?", (cap_uuid,))

        import ai as ai_mod
        try:
            result = ai_mod.process_capture(cap_uuid, thumb_data_uri=thumb)
        except Exception as e:
            result = {"ai_state": "error", "reply": f"saved (AI error: {str(e)[:80]})"}

        if result["ai_state"] == "queued-retry":  # the timer retries exactly once
            with _db_lock, db() as conn:
                conn.execute(
                    "UPDATE captures SET ai_state = 'queued', ai_attempts = 1 WHERE uuid = ?",
                    (cap_uuid,))
            return self._json(200, {"capture": cap,
                                    "ai": {"state": "retrying", "reply": result["reply"]}})

        state = result["ai_state"]
        with _db_lock, db() as conn:
            if state == "done":
                row = conn.execute("SELECT * FROM captures WHERE uuid = ?", (cap_uuid,)).fetchone()
                return_val = {"capture": row_to_capture(row),
                              "ai": {"state": "done", "reply": result.get("reply", ""),
                                     "category": result.get("category"),
                                     "reminders": [{"text": t, "due_at": d}
                                                   for t, d in result.get("reminders", [])]}}
            else:
                conn.execute("UPDATE captures SET ai_state = ? WHERE uuid = ?", (state, cap_uuid))
                return_val = {"capture": cap,
                              "ai": {"state": state, "reply": result.get("reply", "")}}
        return self._json(200, return_val)


def main():
    os.makedirs(DATA_DIR, exist_ok=True)
    init_db()
    httpd = ThreadingHTTPServer(("0.0.0.0", PORT), Handler)
    use_tls = os.environ.get("PERIHEL_SYNC_NO_TLS") != "1"
    if use_tls:
        ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        ctx.load_cert_chain(CERT_PATH, KEY_PATH)
        httpd.socket = ctx.wrap_socket(httpd.socket, server_side=True)
    print(f"perihel-sync listening on :{PORT} ({'TLS' if use_tls else 'plain'}, token auth, AI on)",
          flush=True)
    import ai as _ai  # fail fast at startup if the module or key is broken
    print(f"AI model: {_ai.MODEL}", flush=True)
    httpd.serve_forever()


if __name__ == "__main__":
    main()
