#!/usr/bin/env python3
"""Perihel health watchdog — server + AI-queue monitor, one alert per incident.

Stdlib only. Runs every 10 minutes from the perihel-watchdog systemd
timer (or by hand: python3 /opt/perihel-sync/watchdog.py).

What it checks:
  - Health: GET /v1/health on the local sync server (plain HTTP on
    127.0.0.1:8443, bearer token from DATA_DIR/token). DOWN if the
    request fails, times out, is not 200, or ok is not true.
  - AI queue: sync.db read directly. STUCK when the oldest queued
    capture has waited more than STUCK_AGE_S (1 hour). SPIKE when the
    error count grew since the last check, or is at least
    ERROR_SPIKE_THRESHOLD (5) — and a spike incident latches until
    the errors clear, so a single error that appears and stays does
    not alert and then "recover" 10 minutes later while it is still
    there.

Alerts go to ntfy (the same topic as tick.py) AND Telegram (every
chat the bot has seen). One alert per incident: the last alerted
state is stored in the settings table (watchdog_state /
watchdog_state_since / watchdog_error_count); an alert fires only
when the state changes, and a recovery notice fires when the state
returns to OK. A repeated check that finds the same bad state does
NOT re-alert.

State priority: down > stuck > spike > ok. A down server masks a
stuck queue; when the server recovers, the next check re-evaluates
the queue and alerts if it is still stuck (a state change).

The watchdog never fabricates a cause: a failed health check means
"the server is unreachable", not "the server crashed" — the cause
was not measured. A stuck queue means the oldest queued capture is
older than the threshold, not that the AI is broken.
"""
import json
import os
import sqlite3
import urllib.error
import urllib.request
from datetime import datetime, timedelta, timezone

DATA_DIR = os.environ.get("PERIHEL_SYNC_DIR", "/opt/perihel-sync")
DB_PATH = os.path.join(DATA_DIR, "sync.db")
TOKEN_PATH = os.path.join(DATA_DIR, "token")
NTFY_TOPIC_PATH = os.path.join(DATA_DIR, "ntfy-topic")
NTFY = "https://ntfy.sh"
HEALTH_URL = "http://127.0.0.1:8443/v1/health"

HEALTH_TIMEOUT_S = 10
NTFY_TIMEOUT_S = 15
STUCK_AGE_S = 3600          # 1 hour: a queued capture older than this is stuck
ERROR_SPIKE_THRESHOLD = 5   # >= this many error captures is a spike on its own

STATE_OK = "ok"
STATE_DOWN = "down"
STATE_STUCK = "stuck"
STATE_SPIKE = "spike"
STATES = (STATE_OK, STATE_DOWN, STATE_STUCK, STATE_SPIKE)


def now_utc() -> datetime:
    return datetime.now(timezone.utc)


def now_local() -> datetime:
    return datetime.now().astimezone()  # VM local time (set it to HKT)


def db():
    conn = sqlite3.connect(DB_PATH, timeout=10)
    conn.row_factory = sqlite3.Row
    return conn


def format_age(seconds: float) -> str:
    """Seconds -> "45m", "1h 23m", "3h 5m", "2d 4h". Pure."""
    seconds = max(0, int(seconds))
    days, rem = divmod(seconds, 86400)
    hours, rem = divmod(rem, 3600)
    minutes = rem // 60
    if days:
        return f"{days}d {hours}h"
    if hours:
        return f"{hours}h {minutes}m"
    return f"{minutes}m"


def parse_ts(value) -> datetime | None:
    """An ISO timestamp from the DB -> aware datetime, or None when it
    cannot be parsed (never raises). Pure."""
    if not value:
        return None
    try:
        dt = datetime.fromisoformat(str(value))
    except ValueError:
        return None
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    return dt


# ---------------- health ----------------

def _server_token() -> str | None:
    """The sync server's bearer token, or None if the token file is
    missing. Never printed or logged."""
    try:
        with open(TOKEN_PATH) as f:
            return f.read().strip() or None
    except OSError:
        return None


def check_health(url: str | None = None, token: str | None = None,
                 timeout: int = HEALTH_TIMEOUT_S) -> tuple[bool, str]:
    """GET /v1/health. Returns (ok, detail). DOWN on any failure:
    connection refused, timeout, non-200, bad JSON, or ok is not true.
    The detail never contains the token. Pure except for the request."""
    if url is None:
        url = HEALTH_URL
    if token is None:
        token = _server_token()
    if not token:
        return False, "no token file — cannot authenticate the health check"
    req = urllib.request.Request(
        url, headers={"Authorization": f"Bearer {token}"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            if resp.status != 200:
                return False, f"HTTP {resp.status}"
            body = json.loads(resp.read().decode())
    except urllib.error.HTTPError as e:
        return False, f"HTTP {e.code}"
    except urllib.error.URLError as e:
        return False, f"unreachable ({e.reason})"
    except Exception as e:  # timeout, bad JSON, ...
        return False, f"health check failed ({type(e).__name__})"
    if body.get("ok") is True:
        return True, "ok"
    return False, "server answered but ok is not true"


# ---------------- queue ----------------

def read_queue(conn) -> dict:
    """The AI queue, read straight from sync.db. Returns
    {"queued", "oldest_age_s" (None when nothing is queued or the
    timestamps are unusable), "error_count", "unknown"}. Never raises:
    a missing/broken captures table means the queue cannot be
    measured (unknown=True), not a stuck queue."""
    try:
        created = [r[0] for r in conn.execute(
            "SELECT created_at FROM captures WHERE ai_state = 'queued'")]
        errors = conn.execute(
            "SELECT COUNT(*) FROM captures WHERE ai_state = 'error'").fetchone()[0]
    except sqlite3.OperationalError:
        return {"queued": 0, "oldest_age_s": None, "error_count": 0,
                "unknown": True}
    now = now_utc()
    ages = []
    for value in created:
        ts = parse_ts(value)
        if ts is not None:
            ages.append((now - ts).total_seconds())
    return {"queued": len(created),
            "oldest_age_s": max(ages) if ages else None,
            "error_count": int(errors), "unknown": False}


# ---------------- state ----------------

def load_state(conn) -> dict:
    """The last alerted state, from the settings table. Defaults to ok
    with no error count when the watchdog has never run. Never raises:
    a missing settings table means 'never run'."""
    state = {"state": STATE_OK, "since": None, "error_count": None}
    try:
        rows = conn.execute(
            "SELECT key, value FROM settings WHERE key LIKE 'watchdog_%'").fetchall()
    except sqlite3.OperationalError:
        return state
    values = {r["key"]: r["value"] for r in rows}
    if values.get("watchdog_state") in STATES:
        state["state"] = values["watchdog_state"]
    state["since"] = parse_ts(values.get("watchdog_state_since"))
    try:
        state["error_count"] = int(values["watchdog_error_count"])
    except (TypeError, ValueError):
        state["error_count"] = None
    return state


def save_state(conn, state: str, since: datetime | None,
               error_count: int | None):
    """Persist the watchdog state. The settings table already exists
    in every deployment (server.py creates it); CREATE it here too so
    the watchdog is self-sufficient on a fresh DB."""
    conn.execute(
        "CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
    conn.execute("INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)",
                 ("watchdog_state", state))
    conn.execute("INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)",
                 ("watchdog_state_since",
                  since.isoformat() if since else ""))
    conn.execute("INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)",
                 ("watchdog_error_count",
                  str(error_count) if error_count is not None else ""))
    conn.commit()


# ---------------- evaluation ----------------

def evaluate(health_ok: bool, queue: dict, prev_state: str,
             prev_error_count: int | None,
             now: datetime | None = None) -> dict:
    """The current state from the measurements. Pure. Priority:
    down > stuck > spike > ok. A down server masks a stuck queue;
    an unmeasurable queue (unknown=True) is never stuck or spiked —
    it is not evidence of a problem, only of a missing table. A spike
    incident latches until the errors clear: a single error that
    appears and stays must not alert and then "recover" 10 minutes
    later while the error is still there."""
    now = now or now_utc()
    if not health_ok:
        return {"state": STATE_DOWN, "detail": "server unreachable",
                "queued": queue["queued"],
                "oldest_age_s": queue["oldest_age_s"],
                "error_count": queue["error_count"]}
    if not queue["unknown"] and queue["oldest_age_s"] is not None \
            and queue["oldest_age_s"] > STUCK_AGE_S:
        return {"state": STATE_STUCK,
                "detail": f"{queue['queued']} queued, oldest "
                          f"{format_age(queue['oldest_age_s'])} old",
                "queued": queue["queued"],
                "oldest_age_s": queue["oldest_age_s"],
                "error_count": queue["error_count"]}
    grew = prev_error_count is not None \
        and queue["error_count"] > prev_error_count
    if not queue["unknown"] and (
            (prev_state == STATE_SPIKE and queue["error_count"] > 0)
            or grew
            or queue["error_count"] >= ERROR_SPIKE_THRESHOLD):
        return {"state": STATE_SPIKE,
                "detail": f"{queue['error_count']} captures in error state"
                          + (" (new errors since the last check)" if grew else ""),
                "queued": queue["queued"],
                "oldest_age_s": queue["oldest_age_s"],
                "error_count": queue["error_count"]}
    return {"state": STATE_OK, "detail": "ok",
            "queued": queue["queued"],
            "oldest_age_s": queue["oldest_age_s"],
            "error_count": queue["error_count"]}


def transition(prev_state: str, new_state: str) -> str:
    """What to do when the state changes. Pure. "alert" when a new bad
    state starts, "recover" when a bad state ends, "none" otherwise —
    a repeated check that finds the same bad state does NOT re-alert."""
    if new_state == prev_state:
        return "none"
    if new_state == STATE_OK:
        return "recover" if prev_state != STATE_OK else "none"
    return "alert"


# ---------------- messages ----------------

def format_alert(state: str, detail: str, detected: datetime) -> str:
    """The alert text (ntfy + Telegram share it). Says what is wrong,
    when it was detected, and what to check — and nothing it did not
    measure. Pure."""
    when = detected.strftime("%a %d %b %H:%M")
    if state == STATE_DOWN:
        return (f"🚨 Perihel server DOWN\n"
                f"The sync server is unreachable ({detail}).\n"
                f"Detected {when}.\n"
                f"Check: sudo systemctl status perihel-sync\n"
                f"Captures and reminders are not being processed "
                f"while it is down.")
    if state == STATE_STUCK:
        return (f"⚠️ Perihel AI queue stuck\n"
                f"{detail}.\n"
                f"Detected {when}.\n"
                f"Check: sudo systemctl status perihel-sync — "
                f"the tick may not be running (see issue #10).")
    return (f"⚠️ Perihel AI errors spiked\n"
            f"{detail}.\n"
            f"Detected {when}.\n"
            f"Check: the AI queue on the VM — look at the error "
            f"captures and whether the NIM key is still valid.")


def format_recovery(prev_state: str, since: datetime | None,
                    now: datetime) -> str:
    """The recovery notice. Pure."""
    if since is not None:
        dur = format_age((now - since).total_seconds())
        return (f"✅ Perihel recovered\n"
                f"The server is back up and the queue is flowing "
                f"again (was {prev_state} for {dur}).")
    return ("✅ Perihel recovered\n"
            "The server is back up and the queue is flowing again.")


# ---------------- notify ----------------

def push_ntfy(title: str, message: str, tags: list, priority: str) -> bool:
    """One ntfy push, same pattern as tick.py. The topic is read
    lazily: a missing ntfy-topic file means ntfy is skipped (never a
    crash) — Telegram may still work. The topic is never printed or
    logged. Returns False (never raises) on any failure."""
    try:
        with open(NTFY_TOPIC_PATH) as f:
            topic = f.read().strip()
    except OSError:
        print("watchdog: ntfy-topic not found — ntfy alert skipped", flush=True)
        return False
    if not topic:
        print("watchdog: ntfy-topic is empty — ntfy alert skipped", flush=True)
        return False
    req = urllib.request.Request(
        f"{NTFY}/{topic}",
        data=json.dumps({"topic": topic, "title": title,
                         "message": message[:800], "tags": tags,
                         "priority": priority}).encode(),
        headers={"Content-Type": "application/json"}, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=NTFY_TIMEOUT_S) as resp:
            return resp.status == 200
    except Exception as e:
        print(f"watchdog: ntfy push failed: {e}", flush=True)
        return False


def send_telegram_alert(text: str) -> tuple[bool, int]:
    """The alert to every Telegram chat the bot has seen. Lazy import
    so a missing/broken telegram_bot.py can never break the ntfy path.
    Returns (any_chat_reached, chat_count). Never raises."""
    try:
        from telegram_bot import get_chat_ids, send_telegram
    except ImportError:
        print("watchdog: telegram_bot not deployed — Telegram alert skipped",
              flush=True)
        return False, 0
    try:
        with db() as conn:
            chat_ids = get_chat_ids(conn)
    except Exception as e:
        print(f"watchdog: could not read Telegram chats: {e}", flush=True)
        return False, 0
    if not chat_ids:
        print("watchdog: no Telegram chats known yet — Telegram alert skipped",
              flush=True)
        return False, 0
    ok = False
    for chat_id in chat_ids:
        if send_telegram(chat_id, text):
            ok = True
    return ok, len(chat_ids)


def notify(state: str, text: str) -> dict:
    """Send one alert/recovery via both channels. Returns
    {"ntfy": bool, "telegram": bool, "chats": int}. Never raises."""
    if state == STATE_DOWN:
        title, tags, priority = "Perihel watchdog: server down", ["warning"], "high"
    elif state == STATE_OK:
        title, tags, priority = "Perihel watchdog: recovered", ["white_check_mark"], "default"
    else:
        title, tags, priority = f"Perihel watchdog: {state}", ["warning"], "default"
    ntfy_ok = push_ntfy(title, text, tags, priority)
    tg_ok, chats = send_telegram_alert(text)
    return {"ntfy": ntfy_ok, "telegram": tg_ok, "chats": chats}


# ---------------- run ----------------

def run_once(now: datetime | None = None) -> dict:
    """One watchdog check. Returns {"state", "action", "detail",
    "notify", "error"}; never raises. Exit code is decided by main():
    0 when the check ran, 1 only when the watchdog itself could not
    run (the DB could not be opened at all)."""
    now = now or now_utc()
    try:
        conn = db()
    except Exception as e:
        return {"state": None, "action": "none", "detail": "",
                "notify": {}, "error": f"cannot open sync.db: {type(e).__name__}"}
    try:
        prev = load_state(conn)
        health_ok, health_detail = check_health()
        queue = read_queue(conn)
        current = evaluate(health_ok, queue, prev["state"],
                           prev["error_count"], now)
        action = transition(prev["state"], current["state"])
        result = {"state": current["state"], "action": action,
                  "detail": current["detail"], "notify": {}, "error": None}
        if action == "alert":
            text = format_alert(current["state"], health_detail
                                if current["state"] == STATE_DOWN
                                else current["detail"], now_local())
            result["notify"] = notify(current["state"], text)
            save_state(conn, current["state"], now, current["error_count"])
        elif action == "recover":
            text = format_recovery(prev["state"], prev["since"], now_local())
            result["notify"] = notify(STATE_OK, text)
            save_state(conn, STATE_OK, None, current["error_count"])
        else:
            # Same state as last time (or still ok): keep the stored
            # state and its start time, but refresh the error count so
            # the next check can tell whether errors grew.
            save_state(conn, prev["state"], prev["since"],
                       current["error_count"])
        return result
    finally:
        conn.close()


def main():
    result = run_once()
    if result["error"]:
        print(f"watchdog: {result['error']}", flush=True)
        return 1
    if result["action"] == "alert":
        print(f"watchdog: ALERT {result['state']} ({result['detail']}) — "
              f"ntfy={result['notify'].get('ntfy')} "
              f"telegram={result['notify'].get('telegram')} "
              f"chats={result['notify'].get('chats')}", flush=True)
    elif result["action"] == "recover":
        print(f"watchdog: RECOVERED — ntfy={result['notify'].get('ntfy')} "
              f"telegram={result['notify'].get('telegram')} "
              f"chats={result['notify'].get('chats')}", flush=True)
    else:
        print(f"watchdog: {result['state']} ({result['detail']}) — no alert",
              flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
