#!/usr/bin/env python3
"""Perihel tick — the VM's every-10-minutes brain. Replaces ntfy_push.py.

Jobs, in order:
1. AI queue: process captures with ai_state='queued' (retry-once: only if
   ai_attempts < 2 — chat already attempted once synchronously).
2. Reminders: push user-requested reminders the moment they're due.
   These fire ANY time — the user explicitly asked for them.
3. Nudges (the "Grok/Muse dot"): at most one every nudge_interval_hours,
   ONLY inside active hours (default 09:00-21:00). Outside active hours
   nothing fires except explicit reminders.
4. Purge: soft-deleted captures older than 30 days are removed for real.
"""
import json
import os
import sqlite3
import urllib.request
from datetime import datetime, timedelta, timezone

DATA_DIR = os.environ.get("PERIHEL_SYNC_DIR", "/opt/perihel-sync")
DB_PATH = os.path.join(DATA_DIR, "sync.db")
TOPIC = open(os.path.join(DATA_DIR, "ntfy-topic")).read().strip()
NTFY = "https://ntfy.sh"
PURGE_DAYS = 30


def now_utc() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds")


def db():
    conn = sqlite3.connect(DB_PATH, timeout=10)
    conn.row_factory = sqlite3.Row
    return conn


def push(title: str, message: str, tags: list[str]) -> bool:
    req = urllib.request.Request(
        f"{NTFY}/{TOPIC}",
        data=json.dumps({"topic": TOPIC, "title": title, "message": message[:800],
                         "tags": tags, "priority": "default"}).encode(),
        headers={"Content-Type": "application/json"}, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            return resp.status == 200
    except Exception as e:
        print(f"push failed: {e}", flush=True)
        return False


def setting(conn, key: str) -> str:
    row = conn.execute("SELECT value FROM settings WHERE key = ?", (key,)).fetchone()
    return row["value"] if row else ""


def in_active_hours(local_hour_minute: tuple[int, int], start: str, end: str) -> bool:
    def m(hhmm):
        h, m = hhmm.split(":")
        return int(h) * 60 + int(m)
    now_m = local_hour_minute[0] * 60 + local_hour_minute[1]
    s, e = m(start), m(end)
    return s <= now_m < e if s < e else (now_m >= s or now_m < e)


def process_ai_queue(conn):
    rows = conn.execute(
        "SELECT uuid FROM captures WHERE ai_state = 'queued' AND ai_attempts < 2 "
        "ORDER BY created_at ASC LIMIT 3").fetchall()  # max 3 per tick, gentle on the API
    for row in rows:
        print(f"AI processing {row['uuid']}", flush=True)
        try:
            import ai as ai_mod
            result = ai_mod.process_capture(row["uuid"])
            if result["ai_state"] == "queued-retry":
                conn.execute("UPDATE captures SET ai_state = 'queued', "
                             "ai_attempts = COALESCE(ai_attempts,0) + 1 WHERE uuid = ?",
                             (row["uuid"],))
            elif result["ai_state"] == "error":
                conn.execute("UPDATE captures SET ai_state = 'error', "
                             "ai_attempts = COALESCE(ai_attempts,0) + 1 WHERE uuid = ?",
                             (row["uuid"],))
        except Exception as e:
            print(f"AI tick error for {row['uuid']}: {e}", flush=True)
            conn.execute("UPDATE captures SET ai_state = 'error', "
                         "ai_attempts = COALESCE(ai_attempts,0) + 1 WHERE uuid = ?",
                         (row["uuid"],))
    conn.commit()


def fire_reminders(conn):
    rows = conn.execute(
        "SELECT id, text, due_at FROM reminders WHERE status = 'pending' "
        "AND due_at <= ? ORDER BY due_at ASC LIMIT 10", (now_utc(),)).fetchall()
    for row in rows:
        if push("Perihel reminder", row["text"], ["alarm_clock"]):
            conn.execute("UPDATE reminders SET status = 'fired', fired_at = ? WHERE id = ?",
                         (now_utc(), row["id"]))
            print(f"reminder fired: {row['text']}", flush=True)
    conn.commit()


def send_nudges(conn):
    start = setting(conn, "active_start") or "09:00"
    end = setting(conn, "active_end") or "21:00"
    interval_h = int(setting(conn, "nudge_interval_hours") or "4")
    last = setting(conn, "last_nudge_at")

    local = datetime.now().astimezone()  # VM runs UTC; HK user -> set TZ or accept UTC.
    # The active-hours window is evaluated in the server's local timezone.
    if not in_active_hours((local.hour, local.minute), start, end):
        return

    if last:
        try:
            last_dt = datetime.fromisoformat(last)
            if datetime.now(timezone.utc) - last_dt < timedelta(hours=interval_h):
                return  # not due yet
        except ValueError:
            pass

    row = conn.execute(
        "SELECT uuid, content, ai_summary FROM captures "
        "WHERE deleted = 0 AND status = 'pending' AND notified = 0 "
        "AND scheduled_at IS NOT NULL AND scheduled_at != '' AND scheduled_at <= ? "
        "ORDER BY scheduled_at ASC LIMIT 1", (now_utc(),)).fetchone()
    if row:
        body = row["ai_summary"] or row["content"]
        if push("Perihel — a thought resurfaced", body, ["brain"]):
            conn.execute("UPDATE captures SET notified = 1 WHERE uuid = ?", (row["uuid"],))
            conn.execute("UPDATE settings SET value = ? WHERE key = 'last_nudge_at'",
                         (now_utc(),))
            print(f"nudge sent for {row['uuid']}", flush=True)
    conn.commit()


def purge_deleted(conn):
    cutoff = (datetime.now(timezone.utc) - timedelta(days=PURGE_DAYS)).isoformat()
    doomed = conn.execute(
        "SELECT uuid, media_id FROM captures WHERE deleted = 1 AND deleted_at < ?",
        (cutoff,)).fetchall()
    for row in doomed:
        if row["media_id"]:
            path = os.path.join(DATA_DIR, "media", row["media_id"])
            if os.path.isfile(path):
                os.remove(path)
                print(f"purged media {row['media_id']}", flush=True)
    cur = conn.execute("DELETE FROM captures WHERE deleted = 1 AND deleted_at < ?", (cutoff,))
    if cur.rowcount:
        print(f"purged {cur.rowcount} soft-deleted captures older than {PURGE_DAYS}d", flush=True)
    conn.commit()


def main():
    conn = db()
    try:
        process_ai_queue(conn)
        fire_reminders(conn)   # explicit user reminders — any hour
        send_nudges(conn)      # smart nudges — active hours only
        purge_deleted(conn)
    finally:
        conn.close()


if __name__ == "__main__":
    main()
