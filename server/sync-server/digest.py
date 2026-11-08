#!/usr/bin/env python3
"""Perihel daily digest — AI news, repo PRs/issues summary, and plans.

Stdlib only. Builds the morning digest and sends it to every Telegram
chat the bot has seen. Triggered daily by the perihel-digest systemd
timer (08:00 VM-local time), or on demand via POST /v1/digest.

Why a VM timer and not the agent-bus scheduler (agent-bus #15): the
bus is localhost-only on the user's Windows machine and the VM cannot
see it (there is no Telegram bridge code in agent-bus), so a bus
task.offer can never reach the VM. The digest therefore runs on the
VM itself; the bus scheduler stays the mechanism for scheduled jobs
generally, but it cannot trigger this one.

Sections — defaults, each configurable (see below):
  - AI news: the search worker (search.py) for "artificial intelligence
    news", top 5 results, summarised by the worker role's model.
  - Repos: open PRs and open issues (most recently updated first, max 5
    each) for PERIHEL_DIGEST_REPOS (comma-separated owner/repo;
    default agent-bus, perihel-v2, hkust-climbing-app), via the GitHub
    REST API. Unauthenticated (60 req/hour) is enough for a daily
    digest of public repos; a read-only github.key is needed only for
    private repos.
  - Plans: plans.md in DATA_DIR (one plan per line) if it exists; else
    the top open issues across the configured repos, with a note that
    plans.md is missing.

The digest never fabricates content: a failed search, a failed GitHub
call, or an empty section is reported as such in that section.
"""
import html as html_lib
import json
import os
import sqlite3
import sys
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime

DATA_DIR = os.environ.get("PERIHEL_SYNC_DIR", "/opt/perihel-sync")
DB_PATH = os.path.join(DATA_DIR, "sync.db")
GITHUB_KEY_PATH = os.path.join(DATA_DIR, "github.key")
PLANS_PATH = os.path.join(DATA_DIR, "plans.md")

DEFAULT_REPOS = ["tototofu123/agent-bus", "tototofu123/perihel-v2",
                 "HKUST-Climbing-Society/hkust-climbing-app"]
NEWS_QUERY = "artificial intelligence news"
NEWS_MAX_RESULTS = 5
REPO_MAX_ITEMS = 5        # per repo, per kind (PRs / issues)
PLANS_MAX_LINES = 10
GITHUB_TIMEOUT_S = 15
GITHUB_UA = "Perihel-Digest"
TG_CHUNK_LIMIT = 4000     # Telegram messages cap at 4096 chars

_github_token_cache = None


def now_local() -> datetime:
    return datetime.now().astimezone()  # VM local time (set it to HKT)


def db():
    conn = sqlite3.connect(DB_PATH, timeout=10)
    conn.row_factory = sqlite3.Row
    return conn


# ---------------- config ----------------

def parse_repos(value: str | None) -> list[str]:
    """Comma-separated owner/repo list (PERIHEL_DIGEST_REPOS). Junk
    entries (no slash, extra slashes, blanks) are dropped; an empty
    result falls back to DEFAULT_REPOS."""
    repos = []
    for part in (value or "").split(","):
        part = part.strip()
        if part.count("/") == 1 and not part.startswith("/") \
                and not part.endswith("/"):
            repos.append(part)
    return repos or list(DEFAULT_REPOS)


def get_repos() -> list[str]:
    return parse_repos(os.environ.get("PERIHEL_DIGEST_REPOS"))


# ---------------- GitHub ----------------

def _github_token():
    """Read-only PAT from github.key, or None (unauthenticated). Cached;
    never printed or logged anywhere."""
    global _github_token_cache
    if _github_token_cache is not None:
        return _github_token_cache
    try:
        with open(GITHUB_KEY_PATH) as f:
            _github_token_cache = f.read().strip() or None
    except OSError:
        _github_token_cache = None
    return _github_token_cache


def github_get(url: str):
    """One GitHub REST GET. Returns the parsed JSON. Raises HTTPError /
    URLError — the caller turns those into a section note (never a
    crash, never a fabricated item)."""
    headers = {"Accept": "application/vnd.github+json",
               "User-Agent": GITHUB_UA,
               "X-GitHub-Api-Version": "2022-11-28"}
    token = _github_token()
    if token:
        headers["Authorization"] = f"Bearer {token}"
    req = urllib.request.Request(url, headers=headers)
    with urllib.request.urlopen(req, timeout=GITHUB_TIMEOUT_S) as resp:
        return json.loads(resp.read().decode())


def parse_pulls(payload) -> list[dict]:
    """A /pulls response -> [{number, title, url, updated_at}]. Pure."""
    out = []
    for item in payload or []:
        if not isinstance(item, dict):
            continue
        out.append({"number": item.get("number"),
                    "title": (item.get("title") or "").strip(),
                    "url": item.get("html_url") or "",
                    "updated_at": item.get("updated_at") or ""})
    return out


def parse_issues(payload) -> list[dict]:
    """An /issues response -> issues only. The issues endpoint also
    returns PRs (they carry a "pull_request" key) — those are dropped
    here, they are listed under PRs. Pure."""
    out = []
    for item in payload or []:
        if not isinstance(item, dict) or "pull_request" in item:
            continue
        out.append({"number": item.get("number"),
                    "title": (item.get("title") or "").strip(),
                    "url": item.get("html_url") or "",
                    "updated_at": item.get("updated_at") or ""})
    return out


def _github_error_note(repo: str, e: Exception) -> str:
    """A short honest note for a failed repo fetch. No token, no URL."""
    if isinstance(e, urllib.error.HTTPError):
        if e.code == 404:
            return (f"{repo}: unreachable (private repo or wrong name — "
                    "a github.key token is needed for private repos)")
        if e.code == 403:
            remaining = ""
            try:
                remaining = e.headers.get("X-RateLimit-Remaining", "")
            except Exception:
                pass
            if remaining == "0":
                return (f"{repo}: GitHub rate limit exhausted "
                        "(unauthenticated, 60/hr) — add a github.key")
            return f"{repo}: GitHub denied access (403)"
        return f"{repo}: GitHub returned HTTP {e.code}"
    return f"{repo}: GitHub request failed ({type(e).__name__})"


def fetch_repo_summary(repo: str) -> dict:
    """Open PRs + open issues for one repo, most recently updated first.
    Never raises: a failure becomes an "error" note for that repo."""
    base = f"https://api.github.com/repos/{repo}"
    qs = urllib.parse.urlencode({"state": "open", "sort": "updated",
                                 "direction": "desc",
                                 "per_page": REPO_MAX_ITEMS})
    try:
        pulls = parse_pulls(github_get(f"{base}/pulls?{qs}"))
    except Exception as e:
        return {"pulls": [], "issues": [],
                "error": _github_error_note(repo, e)}
    try:
        issues = parse_issues(github_get(f"{base}/issues?{qs}"))
    except Exception as e:
        return {"pulls": pulls, "issues": [],
                "error": _github_error_note(repo, e)}
    return {"pulls": pulls, "issues": issues, "error": None}


# ---------------- plans ----------------

def parse_plans(text: str) -> list[str]:
    """plans.md: one plan per line. Blank lines and '#' comments are
    skipped; capped at PLANS_MAX_LINES. Pure."""
    out = []
    for line in (text or "").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        out.append(line)
        if len(out) >= PLANS_MAX_LINES:
            break
    return out


def read_plans() -> list[str] | None:
    """The plans.md file, or None when it does not exist (the caller
    falls back to the top open issues)."""
    try:
        with open(PLANS_PATH) as f:
            return parse_plans(f.read())
    except OSError:
        return None


# ---------------- AI news ----------------

def get_search_module():
    """The search worker, or None when search.py is not deployed. Lazy
    so this module stays importable without it."""
    try:
        import search as search_mod
        return search_mod
    except ImportError:
        return None


def fetch_news(search_mod) -> dict:
    """Top AI-news results + a short summary (worker role's model).
    Never raises: a failure becomes an "error" note for the section."""
    if search_mod is None:
        return {"available": False, "count": 0, "items": [],
                "summary": "",
                "error": "search worker not deployed"}
    try:
        items = search_mod.search(NEWS_QUERY, max_results=NEWS_MAX_RESULTS)
    except Exception as e:
        return {"available": False, "count": 0, "items": [],
                "summary": "",
                "error": f"AI news unavailable today (search failed: {type(e).__name__})"}
    if not items:
        return {"available": False, "count": 0, "items": [],
                "summary": "",
                "error": "AI news unavailable today (no results)"}
    joined = "\n\n".join(
        f"{r.get('title', '')}\n{r.get('url', '')}\n{r.get('snippet', '')}"
        for r in items)
    summary = search_mod.summarise(joined, query=NEWS_QUERY)
    return {"available": True, "count": len(items), "items": items,
            "summary": summary, "error": None}


# ---------------- build + format ----------------

def build_digest() -> dict:
    """Build the digest. Returns {"date", "sections", "text"}; never
    raises — a failed section is reported inside its own section."""
    date = now_local().strftime("%a %d %b %Y")

    news = fetch_news(get_search_module())

    repos = {}
    for repo in get_repos():
        repos[repo] = fetch_repo_summary(repo)
    repos_available = any(not v["error"] for v in repos.values())

    plans = read_plans()
    if plans is None:
        # fallback: the top open issues across the configured repos
        # (the planned work), most recently updated first
        all_issues = []
        for repo, data in repos.items():
            for item in data["issues"]:
                all_issues.append({**item, "repo": repo})
        all_issues.sort(key=lambda i: i["updated_at"], reverse=True)
        top = all_issues[:PLANS_MAX_LINES]
        plans_data = {
            "source": "open-issues" if top else "none",
            "items": [f"{i['repo']}#{i['number']}: {i['title']}" for i in top],
            "links": {f"{i['repo']}#{i['number']}": i["url"] for i in top},
            "note": ("plans.md not found in " + DATA_DIR +
                     " — showing the top open issues instead")
            if top else ("plans.md not found in " + DATA_DIR +
                         " and no open issues to show"),
        }
    else:
        plans_data = {"source": "plans.md", "items": plans,
                      "links": {}, "note": None}

    sections = {
        "news": news,
        "repos": {"available": repos_available, "repos": repos},
        "plans": plans_data,
    }
    data = {"date": date, "sections": sections}
    data["text"] = format_digest(data)
    return data


def _esc(text: str) -> str:
    return html_lib.escape(text or "")


def _link(url: str, label: str) -> str:
    return f'<a href="{html_lib.escape(url or "", quote=True)}">{_esc(label)}</a>'


def format_digest(data: dict) -> str:
    """The digest as Telegram HTML: bold headings, inline links for
    sources. All data values are escaped first; only the markup this
    function writes is HTML. Pure."""
    sections = data["sections"]
    lines = [f"<b>☀️ Daily digest — {data['date']}</b>", ""]

    # --- AI news ---
    lines.append("<b>📰 AI news</b>")
    news = sections["news"]
    if not news["available"]:
        lines.append(news["error"] or "AI news unavailable today.")
    else:
        if news["summary"]:
            lines.append(_esc(news["summary"]))
        for item in news["items"]:
            lines.append(f"• {_link(item['url'], item['title'])}")
    lines.append("")

    # --- repos ---
    lines.append("<b>📋 Repos — open PRs &amp; issues</b>")
    for repo, summary in sections["repos"]["repos"].items():
        lines.append(f"<b>{_esc(repo)}</b>")
        if summary["error"]:
            lines.append(summary["error"])
            continue
        for pr in summary["pulls"]:
            lines.append(f"• PR #{pr['number']}: {_link(pr['url'], pr['title'])}")
        for issue in summary["issues"]:
            lines.append(f"• Issue #{issue['number']}: {_link(issue['url'], issue['title'])}")
        if not summary["pulls"] and not summary["issues"]:
            lines.append("• nothing open")
    lines.append("")

    # --- plans ---
    lines.append("<b>📅 Plans</b>")
    plans = sections["plans"]
    if plans["note"]:
        lines.append(f"<i>{_esc(plans['note'])}</i>")
    for item in plans["items"]:
        if item in plans.get("links", {}):
            lines.append(f"• {_link(plans['links'][item], item)}")
        else:
            lines.append(f"• {_esc(item)}")
    if not plans["items"]:
        lines.append("• nothing planned")
    return "\n".join(lines)


def split_message(text: str, limit: int = TG_CHUNK_LIMIT) -> list[str]:
    """Split a long message into <=limit-char chunks on blank-line
    boundaries (single newlines as fallback), so a long digest never
    hits Telegram's 4096-char cap. Pure."""
    if len(text) <= limit:
        return [text]
    chunks, current = [], ""
    for block in text.split("\n\n"):
        if len(current) + len(block) + 2 > limit and current:
            chunks.append(current)
            current = ""
        if len(block) > limit:  # one huge block: split on newlines
            for line in block.split("\n"):
                if len(current) + len(line) + 1 > limit and current:
                    chunks.append(current)
                    current = ""
                current = f"{current}\n{line}" if current else line
        else:
            current = f"{current}\n\n{block}" if current else block
    if current:
        chunks.append(current)
    return chunks


# ---------------- delivery ----------------

def get_telegram():
    """(get_chat_ids, send_telegram) from telegram_bot.py, or (None,
    None) when the bot module is not deployed. Lazy so this module
    stays importable without it."""
    try:
        from telegram_bot import get_chat_ids, send_telegram
        return get_chat_ids, send_telegram
    except ImportError:
        return None, None


def send_digest() -> dict:
    """Build the digest and send it to every Telegram chat the bot has
    seen. Returns {"sent", "chats", "chunks", "data", "error"}; never
    raises. "sent" is True iff every chunk reached at least one chat."""
    try:
        data = build_digest()
    except Exception as e:  # should not happen (build never raises),
        return {"sent": False, "chats": 0, "chunks": 0, "data": {},
                "error": f"digest build failed: {type(e).__name__}"}  # but be safe
    get_chat_ids, send_telegram = get_telegram()
    if get_chat_ids is None or send_telegram is None:
        return {"sent": False, "chats": 0, "chunks": 0, "data": data,
                "error": "Telegram bot not deployed — no chats to send to"}
    try:
        with db() as conn:
            chat_ids = get_chat_ids(conn)
    except Exception as e:
        return {"sent": False, "chats": 0, "chunks": 0, "data": data,
                "error": f"could not read Telegram chats: {type(e).__name__}"}
    if not chat_ids:
        return {"sent": False, "chats": 0, "chunks": 0, "data": data,
                "error": "no Telegram chats known yet — the bot has not seen any chat"}
    chunks = split_message(data["text"])
    sent_any = False
    for chunk in chunks:
        chunk_sent = False
        for chat_id in chat_ids:
            if send_telegram(chat_id, chunk, raw_html=True):
                chunk_sent = True
        sent_any = sent_any or chunk_sent
    if not sent_any:
        return {"sent": False, "chats": len(chat_ids),
                "chunks": len(chunks), "data": data,
                "error": "Telegram rejected every chunk"}
    return {"sent": True, "chats": len(chat_ids), "chunks": len(chunks),
            "data": data, "error": None}


def main():
    result = send_digest()
    if result["sent"]:
        print(f"digest sent to {result['chats']} chat(s) "
              f"({result['chunks']} message(s))", flush=True)
        return 0
    print(f"digest not sent: {result['error']}", flush=True)
    return 1


if __name__ == "__main__":
    sys.exit(main())
