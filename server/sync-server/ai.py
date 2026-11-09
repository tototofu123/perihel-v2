"""Perihel AI processor — NVIDIA NIM (free API) with tool calls + memory.

Runs ON the VM but no inference happens here: this module only orchestrates
HTTP calls to integrate.api.nvidia.com. Rate-limited (min gap between calls)
and retry-once semantics live here too.

Tool protocol: up to two model turns with parallel tool calls. Write tools
(capture_update / reminder_create / memory_write) finish the turn; read tools
(search_web / fetch_url / reminder_list / reminder_cancel / memory_search)
run immediately and their results go back to the model for a second turn,
so it can answer questions using what it found. If the model returns plain
text instead of tool calls (unsupported/changed API), we fall back to a
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

# Tool names use underscores only: the OpenAI function-calling spec allows
# only a-z, A-Z, 0-9, underscores and dashes. Dotted names worked with
# DeepSeek but strict providers reject them — MiniMax returns HTTP 400
# 'Function at index 0 has an invalid name: "capture.update"'.
TOOLS = [
    {"type": "function", "function": {
        "name": "capture_update",
        "description": "Record your analysis of the user's thought: a 1-3 sentence expansion and a category.",
        "parameters": {"type": "object", "properties": {
            "summary": {"type": "string", "description": "1-3 sentence expansion of what the user meant, context and why it matters"},
            "category": {"type": "string", "enum": CATEGORIES},
        }, "required": ["summary", "category"]},
    }},
    {"type": "function", "function": {
        "name": "reminder_create",
        "description": "Create a reminder ONLY if the thought references a time/deadline ('call mom Sunday', 'submit at 6pm', 'before Friday').",
        "parameters": {"type": "object", "properties": {
            "text": {"type": "string"},
            "due_at": {"type": "string", "description": "ISO 8601 with timezone, e.g. 2026-10-25T18:00:00+08:00"},
        }, "required": ["text", "due_at"]},
    }},
    {"type": "function", "function": {
        "name": "memory_write",
        "description": "Persist a durable fact about the user worth remembering in future conversations (interests, projects, weekly routines). Max ~20 memories.",
        "parameters": {"type": "object", "properties": {
            "key": {"type": "string"}, "value": {"type": "string"},
        }, "required": ["key", "value"]},
    }},
    {"type": "function", "function": {
        "name": "search_web",
        "description": "Search the web when the user asks about something current or something you do not know (news, facts, prices, what happened). Returns titles, URLs and snippets.",
        "parameters": {"type": "object", "properties": {
            "query": {"type": "string", "description": "search query, e.g. 'Hong Kong weather today'"},
            "max_results": {"type": "integer", "description": "1-5, default 5"},
        }, "required": ["query"]},
    }},
    {"type": "function", "function": {
        "name": "fetch_url",
        "description": "Fetch a web page the user linked or that search_web returned, and read its text. Use it when the snippets are not enough to answer.",
        "parameters": {"type": "object", "properties": {
            "url": {"type": "string", "description": "full http(s) URL to fetch"},
        }, "required": ["url"]},
    }},
    {"type": "function", "function": {
        "name": "reminder_list",
        "description": "List the user's pending reminders (text + due time). Use it when they ask what reminders they have, or what is due.",
        "parameters": {"type": "object", "properties": {}},
    }},
    {"type": "function", "function": {
        "name": "reminder_cancel",
        "description": "Cancel pending reminders matching the given text (e.g. 'call mom'). Use it only when the user asks to cancel / delete / drop a reminder.",
        "parameters": {"type": "object", "properties": {
            "text": {"type": "string", "description": "words from the reminder to cancel"},
        }, "required": ["text"]},
    }},
    {"type": "function", "function": {
        "name": "memory_search",
        "description": "Search the user's saved memories and past thoughts for a keyword (e.g. 'hiking', 'mom', a project name). Use it when they ask what they said, thought or decided before.",
        "parameters": {"type": "object", "properties": {
            "query": {"type": "string", "description": "keyword to search for"},
        }, "required": ["query"]},
    }},
]

READ_TOOLS = ("search_web", "fetch_url", "reminder_list",
              "reminder_cancel", "memory_search")

# One-line instruction per tool, used to build the system prompt for
# exactly the tools sent in this turn (see _select_tools).
TOOL_DOCS = {
    "capture_update": "capture_update — expand the thought: what they meant, the context, why it might matter.",
    "reminder_create": "reminder_create — ONLY when the thought names a time, deadline or occasion.",
    "memory_write": "memory_write — when the thought reveals a durable fact about the user's life, studies, projects or routines.",
    "search_web": "search_web — when the thought asks about something current or unknown (news, facts, what happened). Answer from the results, never invent.",
    "fetch_url": "fetch_url — when a linked page or a search result has the detail the snippets lack.",
    "reminder_list": "reminder_list — when they ask what reminders they have or what is due.",
    "reminder_cancel": "reminder_cancel — ONLY when they ask to cancel or delete a reminder.",
    "memory_search": "memory_search — when they ask what they said, thought or decided before.",
}
_TOOLS_BY_NAME = {t["function"]["name"]: t for t in TOOLS}

# Keyword routing: the free models call tools reliably with 3 tools and
# a short prompt (verified 2026-11-09: Laguna/Bolt called tools in the
# 3-tool test) but ignore them with 8 tools and a long prompt (Ultra,
# Laguna and Bolt all fell back to JSON on /v1/chat with 8 tools).
# So each turn gets only the tools that could apply to this message.
_SEARCH_WORDS = ("news", "latest", "today", "weather", "price", "what is",
                 "what's", "whats", "who is", "how many", "how much",
                 "happened", "http://", "https://")
_REMINDER_WORDS = ("reminder", "remind me", "reminders")
_RECALL_WORDS = ("what did i", "did i say", "remember", "said before",
                 "thought about", "earlier", "last week", "yesterday")


def _select_tools(content: str) -> list:
    """Tools for one capture: the 3 write tools always, plus the read
    tools whose keywords appear in the message. At most 5 tools, so the
    prompt stays short enough for the small free models to actually
    call them."""
    text = (content or "").lower()
    names = ["capture_update", "reminder_create", "memory_write"]
    if any(w in text for w in _SEARCH_WORDS):
        names += ["search_web"]
        if "http://" in text or "https://" in text:
            names.append("fetch_url")
    if any(w in text for w in _REMINDER_WORDS):
        names += ["reminder_list", "reminder_cancel"]
    if any(w in text for w in _RECALL_WORDS):
        names.append("memory_search")
    seen, out = set(), []
    for n in names[:5]:
        if n not in seen:
            seen.add(n)
            out.append(_TOOLS_BY_NAME[n])
    return out


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


# Fallback chain (issue #23): models proven to answer on the free tier,
# fastest and best tool-callers first (measured 2026-11-09: Laguna 0.2 s,
# Bolt 0.3 s, MiniMax 0.6-2.8 s; DeepSeek, Mistral, GLM, Gemma, GPT-OSS
# and Llama 90B all timed out at 60 s). The configured model is tried
# first; fallbacks run only on timeout / connection failure / 5xx —
# never on 400 (a bad request is a bug in our payload, not the model)
# or 429 (the rate limit is per account, shared by every model).
FALLBACK_MODELS = [m.strip() for m in os.environ.get(
    "PERIHEL_MODEL_FALLBACKS",
    "poolside/laguna-xs-2.1,nvidia/nemotron-3.5-bolt-e4b,"
    "minimaxai/minimax-m3.1-flash").split(",") if m.strip()]


# Dead-model cache: a model that just timed out / 5xx'd is skipped for
# 10 minutes. Without it, one dead configured model costs 30 s on EVERY
# call in a capture — the live DeepSeek test took 71 s (tool turn timed
# out, then the JSON fallback turn timed out again) against the bot's
# 90 s reply window. A model that recovers is tried again after the TTL.
_DEAD_TTL_S = 600
_dead_until: dict[str, float] = {}


def _fallback_chain(model):
    chain = [model or MODEL]
    chain += [m for m in FALLBACK_MODELS if m not in chain]
    now = time.time()
    live = [m for m in chain if _dead_until.get(m, 0) <= now]
    return live or chain[:1]


def _is_retryable(exc) -> bool:
    import urllib.error
    if isinstance(exc, urllib.error.HTTPError):
        # 404/410: the model id is wrong or retired (the old default
        # llama-3.2-11b-vision-instruct 410s) — a fallback may answer.
        # 400: bad payload, our bug. 401/403: bad key, same for every
        # model. 429: per-account limit, shared by every model.
        return exc.code >= 500 or exc.code in (404, 410)
    return isinstance(exc, (TimeoutError, urllib.error.URLError,
                            ConnectionError, OSError))


def nim_call_fallback(messages, tools=None, timeout=30, model=None,
                      role=None, max_attempts: int = 3):
    """_nim_call with a model fallback chain. Returns (response,
    model_used). Tries the configured model first, then each fallback
    once, on retryable failures only. Raises the last exception when
    every tried model fails, so callers keep their queued-retry /
    error paths. max_attempts caps the chain (vision turns pass 2:
    a 60 s timeout per model would otherwise take minutes)."""
    last_exc = None
    for m in _fallback_chain(model)[:max_attempts]:
        try:
            return _nim_call(messages, tools=tools, timeout=timeout,
                             model=m, role=role), m
        except Exception as e:
            last_exc = e
            if not _is_retryable(e):
                raise
            _dead_until[m] = time.time() + _DEAD_TTL_S
            print(f"NIM fallback: {m} failed ({str(e)[:60]}), "
                  f"trying next model", flush=True)
    raise last_exc


JSON_INSTRUCTION = ("Reply with ONLY minified JSON, no prose: "
                    f'{{"summary": str, "category": one of {CATEGORIES}, '
                    '"reminders": [{"text": str, "due_at": iso}] or [], '
                    '"memories": [{"key": str, "value": str}] or []}}')


def _json_turn(raw_content: str, system_msg: dict, model=None, role=None):
    """Strict-JSON completion (no tools). Returns (summary, category,
    reminders, memories) or (None,...) on failure."""
    try:
        fb, _used = nim_call_fallback([
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
        fb, _used = nim_call_fallback(message, timeout=60, model=model, role=role,
                                      max_attempts=2)  # vision turns queue on the free tier, but 60 s is the fail-fast cap
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


def _execute_read_tool(fn: str, args: dict, model=None, role=None) -> str | None:
    """Run a read tool and return its result as text for the model, or
    None when fn is not a read tool. Never raises: failures come back as
    a short error string so the model can say so honestly instead of
    the whole capture failing. Web tools are capped so one page cannot
    flood the next model turn."""
    try:
        if fn == "search_web":
            from search import search  # lazy: search.py is a sibling module
            query = (args.get("query") or "").strip()
            if not query:
                return "search failed: empty query"
            try:
                n = int(args.get("max_results") or 5)
            except (TypeError, ValueError):
                n = 5
            results = search(query, max_results=max(1, min(5, n)))
            if not results:
                return f"web search for '{query}' found nothing"
            lines = [f"- {r['title']} ({r['url']}): {r['snippet']}"
                     for r in results]
            return "\n".join(lines)[:2000]
        if fn == "fetch_url":
            from search import fetch
            url = (args.get("url") or "").strip()
            if not url:
                return "fetch failed: empty url"
            return fetch(url, max_chars=4000) or "fetch returned no text"
        if fn == "reminder_list":
            conn = _db()
            try:
                rows = conn.execute(
                    "SELECT text, due_at FROM reminders WHERE status = 'pending' "
                    "ORDER BY due_at LIMIT 10").fetchall()
            finally:
                conn.close()
            if not rows:
                return "no pending reminders"
            return "\n".join(f"- {r['text']} (due {r['due_at']})" for r in rows)
        if fn == "reminder_cancel":
            text = (args.get("text") or "").strip()
            if not text:
                return "cancel failed: empty text"
            conn = _db()
            try:
                cur = conn.execute(
                    "DELETE FROM reminders WHERE status = 'pending' "
                    "AND text LIKE ?", (f"%{text}%",))
                conn.commit()
                n = cur.rowcount
            finally:
                conn.close()
            return f"cancelled {n} reminder(s) matching '{text}'" if n else \
                f"no pending reminder matched '{text}'"
        if fn == "memory_search":
            query = (args.get("query") or "").strip()
            if not query:
                return "search failed: empty query"
            like = f"%{query}%"
            conn = _db()
            try:
                mem = conn.execute(
                    "SELECT key, value FROM memory WHERE key LIKE ? OR value LIKE ? "
                    "LIMIT 10", (like, like)).fetchall()
                caps = conn.execute(
                    "SELECT content, ai_summary, created_at FROM captures "
                    "WHERE deleted = 0 AND (content LIKE ? OR ai_summary LIKE ?) "
                    "ORDER BY created_at DESC LIMIT 5", (like, like)).fetchall()
            finally:
                conn.close()
            lines = [f"memory: {r['key']}: {r['value']}" for r in mem]
            lines += [f"thought ({r['created_at'][:10]}): {r['content'][:120]}"
                      + (f" — {r['ai_summary'][:120]}" if r['ai_summary'] else "")
                      for r in caps]
            return "\n".join(lines)[:2000] if lines else \
                f"nothing found for '{query}'"
    except Exception as e:
        return f"{fn} failed: {str(e)[:120]}"
    return None


SYSTEM_PROMPT = """You are Perihel, the user's thought companion. The user sends raw, fleeting thoughts. Process each one:

{tools}

What you know about the user so far:
{memory}

Current time: {now}

Call your tools now (all applicable ones, in parallel). Do not reply with prose. If you used a search/fetch/list tool, use its results in your next reply — do not ask the user to look it up themselves."""


def _system_prompt(tools: list, conn) -> str:
    """System prompt listing only the tools sent in this turn, so the
    model is never told about a tool it cannot call."""
    doc = "\n".join(f"{i + 1}. {TOOL_DOCS[t['function']['name']]}"
                    for i, t in enumerate(tools))
    return SYSTEM_PROMPT.format(tools=doc, memory=_memory_block(conn),
                                now=now_utc())


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
    tools = _select_tools(row["content"])
    messages = [
        {"role": "system", "content": _system_prompt(tools, conn)},
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
        # Tool loop, at most 2 turns: write tools finish the turn, read
        # tools run and their results go back for a second turn. The
        # 30 s per-call cap plus the 5 s rate gap keeps a capture under
        # the bot's 90 s reply window.
        summary = category = None
        reminders, memories = [], []
        for _turn in range(2):
            try:
                resp, _used = nim_call_fallback(messages, tools=tools,
                                                model=model, role=role)
                msg = resp["choices"][0]["message"]
            except Exception as e:  # network / 5xx / rate-limit — retry-once later
                if summary:  # a write already landed; do not lose it
                    break
                # Name the model so the reply says which AI was attempted
                # and that it timed out, instead of a bare "hiccup".
                # (nim_call_fallback already tried the fallback chain,
                # so this means every model failed.)
                return {"ai_state": "queued-retry",
                        "reply": f"saved (AI hiccup: {model or MODEL} "
                                 f"(+ fallbacks) — {str(e)[:60]} — will retry)"}
            calls = msg.get("tool_calls") or []
            # Log each turn's tool choice: free models are flaky tool
            # callers, and this is the only way to tell a missed tool
            # from a failed tool from the server journal.
            print(f"AI turn {_turn}: tools="
                  f"{[c['function']['name'] for c in calls]} "
                  f"content={repr((msg.get('content') or '')[:60])}",
                  flush=True)
            if not calls:
                break
            messages.append({"role": "assistant",
                             "content": msg.get("content") or "",
                             "tool_calls": calls})
            read_used = False
            for call in calls:
                fn = call["function"]["name"]
                try:
                    args = json.loads(call["function"]["arguments"] or "{}")
                except (ValueError, TypeError):
                    args = {}
                # Accept the old dotted names too, in case a model returns them.
                if fn in ("capture_update", "capture.update"):
                    summary, category = args.get("summary"), args.get("category")
                elif fn in ("reminder_create", "reminder.create"):
                    reminders.append((args.get("text", ""), _fix_due_at(args.get("due_at", ""))))
                elif fn in ("memory_write", "memory.write"):
                    memories.append((args.get("key", ""), args.get("value", "")))
                elif fn in READ_TOOLS:
                    read_used = True
                    result = _execute_read_tool(fn, args, model=model, role=role)
                    messages.append({"role": "tool",
                                     "tool_call_id": call.get("id", fn),
                                     "content": result or ""})
            if summary or not read_used:
                break
            # Read tools ran and no summary yet: one more turn so the
            # model can answer from the results. Nudge it to finish —
            # without this the model re-searches instead of answering
            # (seen live on Ultra: turn 2 called search_web again
            # because the first results were poor, and the loop ran
            # out of turns). If that turn fails, the loop's except
            # above breaks to the JSON fallback.
            messages.append({"role": "user", "content": "You have the results above. "
                             "Now call capture_update with your answer "
                             "(summary + category). Do not call search_web, "
                             "fetch_url, reminder_list or memory_search again."})

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
