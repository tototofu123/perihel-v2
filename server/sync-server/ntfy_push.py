#!/usr/bin/env python3
"""Push due captures to ntfy.sh so they resurface even when the PC is off.

Runs from a systemd timer every 10 min. Any capture that is still pending,
past its scheduled_at, and not yet notified gets pushed once, then marked.
The topic name acts as the password — subscribe on the phone with ntfy.
"""
import json
import os
import sqlite3
import urllib.request
from datetime import datetime, timezone

DATA_DIR = os.environ.get("PERIHEL_SYNC_DIR", "/opt/perihel-sync")
DB_PATH = os.path.join(DATA_DIR, "sync.db")
TOPIC = open(os.path.join(DATA_DIR, "ntfy-topic")).read().strip()
SERVER = "https://ntfy.sh"


def now_utc() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds")


def main():
    conn = sqlite3.connect(DB_PATH, timeout=10)
    conn.row_factory = sqlite3.Row
    rows = conn.execute(
        """SELECT uuid, content, scheduled_at FROM captures
           WHERE deleted = 0 AND status = 'pending' AND notified = 0
             AND scheduled_at IS NOT NULL AND scheduled_at != ''
             AND scheduled_at <= ?
           ORDER BY scheduled_at ASC LIMIT 20""",
        (now_utc(),)).fetchall()

    for row in rows:
        req = urllib.request.Request(
            f"{SERVER}/{TOPIC}",
            data=json.dumps({
                "topic": TOPIC,
                "title": "Perihel — resurfaced thought",
                "message": row["content"][:800],
                "tags": ["brain"],
                "priority": "default",
            }).encode(),
            headers={"Content-Type": "application/json"},
            method="POST")
        try:
            with urllib.request.urlopen(req, timeout=15) as resp:
                if resp.status == 200:
                    conn.execute("UPDATE captures SET notified = 1 WHERE uuid = ?",
                                 (row["uuid"],))
                    print(f"pushed {row['uuid']}", flush=True)
        except Exception as e:  # keep going; ntfy hiccup retried next tick
            print(f"push failed for {row['uuid']}: {e}", flush=True)
    conn.commit()
    conn.close()


if __name__ == "__main__":
    main()
