#!/usr/bin/env python3
"""Perihel Telegram bot — the Perihel personal bot in the agent bus group.

Stdlib only. Long-polls Telegram getUpdates and answers each message by
posting it to the local sync server's /v1/chat (which saves the capture
and runs the AI), then replies in the originating chat only.

  - Bot token: telegram.key in DATA_DIR (chmod 600), same pattern as
    token / nim.key. Missing token = fail fast at startup (see main).
  - Chats the bot has seen are stored in the telegram_chats table in
    sync.db, so tick.py can deliver reminders there too.
  - Model: PERIHEL_BOT_MODEL env var is passed through /v1/chat as the
    optional "model" field; unset means the server default.
  - Update offset is persisted in telegram_state so a restart never
    re-answers the same messages.
"""
import html
import json
import os
import sqlite3
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone

DATA_DIR = os.environ.get("PERIHEL_SYNC_DIR", "/opt/perihel-sync")
DB_PATH = os.path.join(DATA_DIR, "sync.db")
TELEGRAM_KEY_PATH = os.path.join(DATA_DIR, "telegram.key")
TOKEN_PATH = os.path.join(DATA_DIR, "token")
TG_API = "https://api.telegram.org"
CHAT_URL = os.environ.get("PERIHEL_SYNC_URL", "http://127.0.0.1:8443/v1/chat")
BOT_MODEL = os.environ.get("PERIHEL_BOT_MODEL") or None

_bot_token_cache = None
_token_missing_logged = False
_server_token_cache = None

HELP_TEXT = (
    "Hi — I'm Perihel, your thought companion. Send me a thought and I'll "
    "save it and send back a short summary. If a thought names a time "
    "('call mom Sunday'), I'll remind you here when it's due."
)
SERVER_UNREACHABLE = (
    "I couldn't reach the Perihel server right now, so your message wasn't "
    "saved. Try again in a bit."
)


def now_utc() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds")


def db():
    conn = sqlite3.connect(DB_PATH, timeout=10)
    conn.row_factory = sqlite3.Row
    return conn


def init_db():
    """Create the bot's tables (idempotent). The server owns the rest."""
    conn = db()
    conn.executescript("""
        CREATE TABLE IF NOT EXISTS telegram_chats (
            chat_id INTEGER PRIMARY KEY,
            chat_type TEXT,
            title TEXT,
            first_seen TEXT NOT NULL,
            last_seen TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS telegram_state (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
    """)
    conn.commit()
    conn.close()


def _bot_token():
    """Bot token, or None if telegram.key is missing (logged once)."""
    global _bot_token_cache, _token_missing_logged
    if _bot_token_cache is not None:
        return _bot_token_cache
    try:
        with open(TELEGRAM_KEY_PATH) as f:
            _bot_token_cache = f.read().strip()
    except OSError:
        if not _token_missing_logged:
            print("telegram.key not found — Telegram delivery disabled", flush=True)
            _token_missing_logged = True
        return None
    return _bot_token_cache


def _server_token():
    global _server_token_cache
    if _server_token_cache is None:
        with open(TOKEN_PATH) as f:
            _server_token_cache = f.read().strip()
    return _server_token_cache


def format_reply(text: str) -> str:
    """Escape HTML entities for parse_mode=HTML. Line breaks pass through
    unchanged — Telegram preserves them in the message text."""
    return html.escape(text)


def record_chat(conn, chat_id: int, chat_type: str, title: str):
    now = now_utc()
    conn.execute(
        "INSERT INTO telegram_chats (chat_id, chat_type, title, first_seen, last_seen) "
        "VALUES (?, ?, ?, ?, ?) "
        "ON CONFLICT(chat_id) DO UPDATE SET chat_type = excluded.chat_type, "
        "title = excluded.title, last_seen = excluded.last_seen",
        (chat_id, chat_type, title, now, now))
    conn.commit()


def get_chat_ids(conn) -> list:
    """Chat ids the bot has seen. Empty list if the bot never ran (the
    table does not exist yet) — callers treat that as 'no Telegram chats'."""
    try:
        rows = conn.execute("SELECT chat_id FROM telegram_chats ORDER BY chat_id").fetchall()
    except sqlite3.OperationalError:
        return []
    return [r["chat_id"] for r in rows]


def get_offset(conn) -> int:
    row = conn.execute("SELECT value FROM telegram_state WHERE key = 'offset'").fetchone()
    return int(row["value"]) if row else 0


def set_offset(conn, offset: int):
    conn.execute("INSERT INTO telegram_state (key, value) VALUES ('offset', ?) "
                 "ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                 (str(offset),))
    conn.commit()


def parse_update(update: dict, bot_id: int | None) -> dict | None:
    """Extract a replyable message, or None to skip. Skips bot messages
    (including our own), empty text, and non-message updates."""
    msg = update.get("message")
    if not isinstance(msg, dict):
        return None
    sender = msg.get("from") or {}
    if sender.get("is_bot"):
        return None
    if bot_id is not None and sender.get("id") == bot_id:
        return None
    text = msg.get("text")
    if not isinstance(text, str) or not text.strip():
        return None
    chat = msg.get("chat") or {}
    chat_id = chat.get("id")
    if not isinstance(chat_id, int):
        return None
    return {
        "chat_id": chat_id,
        "chat_type": chat.get("type", ""),
        "chat_title": chat.get("title") or chat.get("username") or "",
        "message_id": msg.get("message_id"),
        "text": text.strip(),
        "is_start": text.strip() == "/start",
    }


def tg_api(method: str, payload: dict, timeout: int = 30) -> dict:
    """One Telegram Bot API call. Raises on missing token / HTTP error."""
    token = _bot_token()
    if not token:
        raise RuntimeError(f"telegram.key missing at {TELEGRAM_KEY_PATH}")
    req = urllib.request.Request(
        f"{TG_API}/bot{token}/{method}",
        data=json.dumps(payload).encode(), method="POST",
        headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read().decode())


def send_telegram(chat_id: int, text: str, reply_to: int | None = None) -> bool:
    """Send a message to one chat. Safe to import from tick.py: returns
    False (never raises) when the bot is not configured or Telegram
    rejects the call."""
    try:
        payload = {"chat_id": chat_id, "text": format_reply(text),
                   "parse_mode": "HTML"}
        if reply_to is not None:
            payload["reply_to_message_id"] = reply_to
        out = tg_api("sendMessage", payload, timeout=15)
        return bool(out.get("ok"))
    except Exception as e:
        print(f"telegram send failed (chat {chat_id}): {e}", flush=True)
        return False


def post_chat(text: str, model: str | None = None) -> dict:
    """POST the text to the local sync server's /v1/chat. Raises on
    connection/auth/HTTP failure — the caller decides what to tell the user."""
    body = {"content": text, "ai": True, "role": "bot"}
    if model:
        body["model"] = model
    req = urllib.request.Request(
        CHAT_URL, data=json.dumps(body).encode(), method="POST",
        headers={"Authorization": f"Bearer {_server_token()}",
                 "Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=300) as resp:
        return json.loads(resp.read().decode())


def handle_message(msg: dict, bot_id: int | None) -> bool:
    """Answer one parsed message. Returns True if a reply was sent."""
    with db() as conn:
        record_chat(conn, msg["chat_id"], msg["chat_type"], msg["chat_title"])
    if msg["is_start"]:
        return send_telegram(msg["chat_id"], HELP_TEXT, reply_to=msg["message_id"])
    try:
        result = post_chat(msg["text"], model=BOT_MODEL)
    except Exception as e:
        print(f"/v1/chat failed: {e}", flush=True)
        return send_telegram(msg["chat_id"], SERVER_UNREACHABLE,
                             reply_to=msg["message_id"])
    ai = result.get("ai") or {}
    reply = ai.get("reply") or "saved."
    return send_telegram(msg["chat_id"], reply, reply_to=msg["message_id"])


def poll_once(offset: int, bot_id: int | None) -> int:
    """One getUpdates long poll; returns the next offset to use."""
    try:
        out = tg_api("getUpdates", {"offset": offset, "timeout": 30,
                                    "allowed_updates": ["message"]}, timeout=45)
    except urllib.error.URLError as e:
        print(f"getUpdates failed (will retry): {e}", flush=True)
        time.sleep(5)
        return offset
    for update in out.get("result", []):
        offset = max(offset, int(update.get("update_id", 0)) + 1)
        msg = parse_update(update, bot_id)
        if msg is None:
            continue
        try:
            handle_message(msg, bot_id)
        except Exception as e:
            print(f"message handling failed: {e}", flush=True)
    with db() as conn:
        set_offset(conn, offset)
    return offset


def main():
    if not _bot_token():
        raise SystemExit(
            f"telegram.key not found at {TELEGRAM_KEY_PATH}.\n"
            "Create a bot with @BotFather, then save the token:\n"
            f"  cat > {TELEGRAM_KEY_PATH}   # paste the token\n"
            f"  chmod 600 {TELEGRAM_KEY_PATH}\n"
            "Then: sudo systemctl restart perihel-telegram")
    try:
        _server_token()
    except OSError:
        raise SystemExit(
            f"server token not found at {TOKEN_PATH} — is perihel-sync installed?")
    init_db()
    me = tg_api("getMe", {})
    bot_id = (me.get("result") or {}).get("id")
    print(f"telegram bot @{me.get('result', {}).get('username')} "
          f"(id {bot_id}), model override: {BOT_MODEL or 'server default'}", flush=True)
    with db() as conn:
        offset = get_offset(conn)
    print("polling Telegram (long poll, 30s)", flush=True)
    while True:
        try:
            offset = poll_once(offset, bot_id)
        except Exception as e:
            print(f"poll loop error (will retry): {e}", flush=True)
            time.sleep(5)


if __name__ == "__main__":
    main()
