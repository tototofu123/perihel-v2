"""Perihel AI processor — NVIDIA NIM (free API) with tool calls + memory.

Runs ON the VM but no inference happens here: this module only orchestrates
HTTP calls to integrate.api.nvidia.com. Rate-limited (min gap between calls)
and retry-once semantics live here too.

Tool protocol: one model turn with parallel tool calls. If the model returns
plain text instead of tool calls (unsupported/changed API), we fall back to a
strict-JSON prompt so processing still succeeds.
"""
import json
import os
import sqlite3
import time
import urllib.request
from datetime import datetime, timezone

DATA_DIR = os.environ.get("PERIHEL_SYNC_DIR", "/opt/perihel-sync")
NIM_URL = "https://integrate.api.nvidia.com/v1/chat/completions"
MODEL = os.environ.get("PERIHEL_AI_MODEL", "meta/llama-3.2-11b-vision-instruct")

# Roles — each may have its own NVIDIA key (see get_nim_key). The role
# picks the KEY and the rate limiter; it does NOT pick the model (the
# model is threaded separately, per call).
ROLES = ("bot", "worker", "default")
DEFAULT_ROLE = "default"

MIN_CALL_GAP_S = 5          # never hit the API more often than this, per role
RETRY_AFTER_S = 120         # retry-once window
_last_call: dict[str, float] = {}   # role -> timestamp of its last NIM call
_call_counts: dict[str, int] = {}   # role -> NIM calls made this process
_key_cache: dict[str, str] = {}     # key filename -> key contents (hits only;
                                    # a miss is never cached, so creating a
                                    # key file while the server runs works)


def validate_role(role) -> str:
    """Normalise a role to the allowlist; unknown / blank / non-string
    means the default role (desktop, mobile, tick queue)."""
    if isinstance(role, str) and role.strip().lower() in ROLES:
        return role.strip().lower()
    return DEFAULT_ROLE


def _read_key_file(filename: str) -> str | None:
    """One key file's contents, or None when missing/blank. Successful
    reads are cached per filename; misses are not cached."""
    if filename in _key_cache:
        return _key_cache[filename]
    try:
        with open(os.path.join(DATA_DIR, filename)) as f:
            key = f.read().strip()
    except OSError:
        return None
    if not key:
        return None
    _key_cache[filename] = key
    return key


def get_nim_key(role: str = DEFAULT_ROLE) -> str:
    """The NVIDIA key for a role. Precedence: nim.<role>.key, then
    nim.key. A deployment with only nim.key keeps working unchanged.
    Raises RuntimeError naming the files it looked for when neither
    exists — keys are read lazily (never at import), so a missing key
    can never break startup; it fails on the first NIM call instead."""
    role = validate_role(role)
    if role != DEFAULT_ROLE:
        key = _read_key_file(f"nim.{role}.key")
        if key:
            return key
    key = _read_key_file("nim.key")
    if key:
        return key
    tried = [f"nim.{role}.key", "nim.key"] if role != DEFAULT_ROLE else ["nim.key"]
    raise RuntimeError(
        f"no NVIDIA key found (looked for {', '.join(tried)} in {DATA_DIR}); "
        "mint one at build.nvidia.com")


def nim_usage() -> dict:
    """Per-role NIM call counts this process (for /v1/health or the
    watchdog to read). Returns a copy."""
    return dict(_call_counts)

CATEGORIES = ["idea", "todo", "link", "note", "question", "person", "event"]

TOOLS = [
    {"type": "function", "function": {
        "name": "capture.update",
        "description": "Record your analysis of the user's thought: a 1-3 sentence expansion and a category.",
        "parameters": {"type": "object", "properties": {
            "summary": {"type": "string", "description": "1-3 sentence expansion of what the user meant, context and why it matters"},
            "category": {"type": "string", "enum": CATEGORIES},
        }, "required": ["summary", "category"]},
    }},
    {"type": "function", "function": {
        "name": "reminder.create",
        "description": "Create a reminder ONLY if the thought references a time/deadline ('call mom Sunday', 'submit at 6pm', 'before Friday').",
        "parameters": {"type": "object", "properties": {
            "text": {"type": "string"},
            "due_at": {"type": "string", "description": "ISO 8601 with timezone, e.g. 2026-10-25T18:00:00+08:00"},
        }, "required": ["text", "due_at"]},
    }},
    {"type": "function", "function": {
        "name": "memory.write",
        "description": "Persist a durable fact about the user worth remembering in future conversations (interests, projects, weekly routines). Max ~20 memories.",
        "parameters": {"type": "object", "properties": {
            "key": {"type": "string"}, "value": {"type": "string"},
        }, "required": ["key", "value"]},
    }},
]


def _db():
    conn = sqlite3.connect(os.path.join(DATA_DIR, "sync.db"), timeout=10)
    conn.row_factory = sqlite3.Row
    return conn


def now_utc() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds")


def _nim_call(messages, tools=None, timeout=30, model=None, role=None):
    """Single NIM request honoring the per-role rate limit. Returns the
    response JSON. model overrides the global MODEL for this one call
    (the Telegram bot passes PERIHEL_BOT_MODEL); None means the server
    default. role picks the key file and the rate limiter (see
    get_nim_key); None/unknown means the default role. Each call is
    logged with its role and the running per-role count, so usage is
    visible in the server log. Timeout is 30 s on purpose: a model that
    has not answered in 30 s is not going to (it is queued or the key
    lacks the entitlement), so fail fast and let the caller retry
    later instead of leaving the user waiting for minutes."""
    role = validate_role(role)
    wait = MIN_CALL_GAP_S - (time.time() - _last_call.get(role, 0.0))
    if wait > 0:
        time.sleep(wait)
    body = {"model": model or MODEL, "messages": messages, "max_tokens": 450,
            "temperature": 0.3, "stream": False}
    if tools:
        body["tools"] = tools
        body["tool_choice"] = "auto"
    req = urllib.request.Request(
        NIM_URL, data=json.dumps(body).encode(), method="POST",
        headers={"Authorization": f"Bearer {get_nim_key(role)}",
                 "Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        out = json.loads(r.read().decode())
    _last_call[role] = time.time()
    _call_counts[role] = _call_counts.get(role, 0) + 1
    print(f"NIM call: role={role} model={body['model']} "
          f"calls_for_role={_call_counts[role]}", flush=True)
    return out


JSON_INSTRUCTION = ("Reply with ONLY minified JSON, no prose: "
                    f'{{"summary": str, "category": one of {CATEGORIES}, '
                    '"reminders": [{"text": str, "due_at": iso}] or [], '
                    '"memories": [{"key": str, "value": str}] or []}}')


def _json_turn(raw_content: str, system_msg: dict, model=None, role=None):
    """Strict-JSON completion (no tools). Returns (summary, category,
    reminders, memories) or (None,...) on failure."""
    try:
        fb = _nim_call([
            {"role": "system", "content": JSON_INSTRUCTION},
            {"role": "user", "content": raw_content},
        ], model=model, role=role)
        text = fb["choices"][0]["message"]["content"].strip()
        start, end = text.find("{"), text.rfind("}")
        parsed = json.loads(text[start:end + 1])
        return (parsed.get("summary"),
                parsed.get("category"),
                [(r["text"], _fix_due_at(r["due_at"])) for r in parsed.get("reminders", [])],
                [(m["key"], m["value"]) for m in parsed.get("memories", [])])
    except Exception:
        return None, None, [], []


def _process_json(messages, raw_content: str, user_content, model=None,
                  role=None) -> tuple | None:
    """Vision path. NIM vision endpoints hang on system+image turns, so the
    JSON instruction rides in the text part of the single user message."""
    try:
        text_part = user_content[0]["text"] if isinstance(user_content, list) else raw_content
        image_part = user_content[1] if isinstance(user_content, list) else None
        message = [{"role": "user", "content": [
            {"type": "text", "text": JSON_INSTRUCTION + "\n\nUser thought: " + text_part},
            image_part,
        ]}]
        fb = _nim_call(message, timeout=60, model=model, role=role)  # vision turns queue on the free tier, but 60 s is the fail-fast cap
        text = fb["choices"][0]["message"]["content"].strip()
        start, end = text.find("{"), text.rfind("}")
        parsed = json.loads(text[start:end + 1])
        return (parsed.get("summary"),
                parsed.get("category"),
                [(r["text"], _fix_due_at(r["due_at"])) for r in parsed.get("reminders", [])],
                [(m["key"], m["value"]) for m in parsed.get("memories", [])])
    except Exception as e:
        import traceback
        print(f"vision turn failed: {e}\n{traceback.format_exc()[-400:]}", flush=True)
        return None


def _fix_due_at(due: str) -> str:
    """Models frequently hallucinate past years ('Friday 5pm' -> 2024-...).
    If the parsed date is in the past, roll it forward to the next plausible
    occurrence (same month/day this year, else next year)."""
    try:
        dt = datetime.fromisoformat(due.replace("Z", "+00:00"))
        if dt.tzinfo is None:
            dt = dt.replace(tzinfo=timezone.utc)
        now = datetime.now(timezone.utc)
        if dt < now:
            try:
                dt = dt.replace(year=now.year)
            except ValueError:  # Feb 29
                dt = dt.replace(year=now.year, day=28)
            if dt < now:
                try:
                    dt = dt.replace(year=now.year + 1)
                except ValueError:
                    dt = dt.replace(year=now.year + 1, day=28)
        return dt.isoformat()
    except (ValueError, TypeError):
        return due


def _memory_block(conn) -> str:
    rows = conn.execute("SELECT key, value FROM memory ORDER BY updated_at DESC LIMIT 20").fetchall()
    if not rows:
        return "(no memories yet)"
    return "\n".join(f"- {r['key']}: {r['value']}" for r in rows)


SYSTEM_PROMPT = """You are Perihel, the user's thought companion. The user sends raw, fleeting thoughts. Process each one:

1. capture.update — expand the thought: what they meant, the context, why it might matter.
2. reminder.create — ONLY when the thought names a time, deadline or occasion.
3. memory.write — when the thought reveals a durable fact about the user's life, studies, projects or routines.

What you know about the user so far:
{memory}

Current time: {now}

Call your tools now (all applicable ones, in parallel). Do not reply with prose."""


def process_capture(uuid: str, thumb_data_uri: str | None = None,
                    model: str | None = None, role: str | None = None) -> dict:
    """Run AI over one capture. Writes results, returns {ai_state, reply}.
    thumb_data_uri: optional base64 JPEG the vision model can look at.
    model: optional per-call model override (the Telegram bot passes
    PERIHEL_BOT_MODEL); None means the global MODEL.
    role: optional role for the key file and rate limiter ("bot",
    "worker", "default"); None/unknown means the default role. The
    role does not pick the model — that is the separate `model` arg."""
    conn = _db()
    row = conn.execute("SELECT content, content_type, ai_attempts FROM captures WHERE uuid = ?", (uuid,)).fetchone()
    if row is None:
        conn.close()
        return {"ai_state": "error", "reply": "capture vanished"}

    user_content = row["content"]
    if thumb_data_uri and row["content_type"] in ("image", "video"):
        # vision turn: describe what is attached alongside the user's words
        user_content = [
            {"type": "text", "text": f"{user_content}\n(An image is attached — "
             "factor what you see into the summary and category.)"},
            {"type": "image_url", "image_url": {"url": thumb_data_uri}},
        ]
    messages = [
        {"role": "system", "content": SYSTEM_PROMPT.format(memory=_memory_block(conn), now=now_utc())},
        {"role": "user", "content": user_content},
    ]
    conn.close()

    # Vision turns: the vision endpoints reject the `tools` parameter, so
    # images take the strict-JSON path directly instead of tool calling.
    vision = thumb_data_uri and isinstance(user_content, list)
    if vision:
        parsed = _process_json(messages, row["content"], user_content, model=model,
                               role=role)
        if parsed is None:
            return {"ai_state": "error", "reply": "saved (AI could not process the image — retry available)"}
        summary, category, reminders, memories = parsed
    else:
        try:
            resp = _nim_call(messages, tools=TOOLS, model=model, role=role)
            msg = resp["choices"][0]["message"]
        except Exception as e:  # network / 5xx / rate-limit — retry-once later
            # Name the model so the reply says which AI was attempted and
            # that it timed out, instead of a bare "hiccup".
            return {"ai_state": "queued-retry",
                    "reply": f"saved (AI hiccup: {model or MODEL} — "
                             f"{str(e)[:60]} — will retry)"}

        summary = category = None
        reminders, memories = [], []

        for call in msg.get("tool_calls") or []:
            fn, args = call["function"]["name"], json.loads(call["function"]["arguments"] or "{}")
            if fn == "capture.update":
                summary, category = args.get("summary"), args.get("category")
            elif fn == "reminder.create":
                reminders.append((args.get("text", ""), _fix_due_at(args.get("due_at", ""))))
            elif fn == "memory.write":
                memories.append((args.get("key", ""), args.get("value", "")))

        if not summary:  # model ignored tools — strict JSON fallback
            summary, category, reminders, memories = _json_turn(row["content"], messages[0],
                                                                model=model, role=role)
            if summary is None:
                return {"ai_state": "error", "reply": "saved (AI could not process this one — retry available)"}

    conn = _db()
    if category not in CATEGORIES:
        category = "note"
    conn.execute(
        "UPDATE captures SET ai_summary = ?, category = ?, ai_state = 'done', "
        "ai_attempts = COALESCE(ai_attempts, 0) + 1, updated_at = ? WHERE uuid = ?",
        (summary, category, now_utc(), uuid))
    due_at = reminders[0][1] if reminders else None
    if due_at:
        conn.execute("UPDATE captures SET due_at = ? WHERE uuid = ?", (due_at, uuid))
    for text, due in reminders:
        conn.execute("INSERT INTO reminders (text, due_at, status, created_at, capture_uuid) "
                     "VALUES (?, ?, 'pending', ?, ?)", (text, due, now_utc(), uuid))
    for key, value in memories:
        conn.execute("INSERT INTO memory (key, value, updated_at) VALUES (?, ?, ?) "
                     "ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                     (key, value, now_utc()))
    conn.commit()
    conn.close()
    return {"ai_state": "done", "reply": summary or "",
            "category": category, "reminders": reminders}
