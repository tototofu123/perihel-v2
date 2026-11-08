# Self-hosting Perihel sync

Everything runs on **always-free** cloud resources — no paid tiers anywhere.
Total expected bill: $0. The live deployment's private values (URL, token,
GCP project) are in `server/local.env`, which is **gitignored** — this guide
is written so anyone can recreate an equivalent server without them.

## Architecture

```
Android app ─┐
             ├─ HTTPS (Let's Encrypt via Caddy) ──► sync server (Python, stdlib only)
Desktop app ─┘                                        └─ SQLite on the VM
VM timer (10 min) ──► ntfy.sh ──► your phone (resurfacing pushes)
```

- The VM is a **relay only**. No LLM, no weights — 1 GB RAM can't host models.
  LLM answers come from a free API (NVIDIA NIM by default) or the local .gguf.
- Local SQLite on each device stays the source of truth; the server is the
  meeting point. Offline always works.

## 1. GCP project (free tier, charge-proof)

```bash
gcloud projects create perihel-$USER          # any unique id
gcloud config set project perihel-$USER
```

Link a billing account (required for Compute Engine even on free tier) and
**immediately** set a tiny budget so any surprise charge emails you:

```bash
gcloud billing budgets create \
  --billing-account=YOUR_BILLING_ACCOUNT \
  --display-name="perihel 0.01 alert" \
  --budget-amount=0.01HKD --filter-projects="projects/YOUR_PROJECT_NUMBER" \
  --threshold-rule=percent=50 --threshold-rule=percent=90 --threshold-rule=percent=100
```

(GCP has **no hard spend cap** — the budget alerts, it does not block. The
kill switch below is what actually stops the meter.)

## 2. VM

```bash
gcloud services enable compute.googleapis.com
gcloud compute instances create perihel-sync \
  --zone=us-west1-b --machine-type=e2-micro \
  --image-family=debian-12 --image-project=debian-cloud \
  --boot-disk-size=30GB --boot-disk-type=pd-standard --tags=perihel
# e2-micro + us-west1/us-central1/us-east4 + 30GB pd-standard = always-free SKUs
```

Promote the ephemeral IP to static (free for free-tier e2-micro):

```bash
gcloud compute addresses create perihel-ip --region=us-west1 \
  --addresses=$(gcloud compute instances describe perihel-sync \
    --zone=us-west1-b --format="value(networkInterfaces[0].accessConfigs[0].natIP)")
```

## 3. Sync server + Caddy (real TLS, free domain)

### Deploying updates

Use the deploy script from a checkout of this repo:

```bash
ops/deploy.sh                 # deploy, restart, health-check, queue depth
ops/deploy.sh --dry-run       # print every command without running any
```

It copies `server/sync-server/*.py` and `server/systemd/*` to the VM,
restarts `perihel-sync` (and `perihel-telegram` if it is enabled),
smoke-tests `GET /v1/health` (retried up to 6 times, 5 s apart), and
reports the AI queue depth (captures with `ai_state='queued'`). It
refuses to run with uncommitted changes to the files it deploys, so a
deploy always matches a committed tree (`--allow-dirty` overrides).
It never copies or prints a secret file, and it leaves the ntfy units
alone — the tick timer wiring is issue #10's scope. Defaults are the
`perihel-sync` VM in `us-west1-b`; override with `--instance`,
`--zone`, `--project` (or `PERIHEL_INSTANCE` / `PERIHEL_ZONE` /
`PERIHEL_GCP_PROJECT`).

### First install (manual fallback)

On the VM (`gcloud compute ssh perihel-sync --zone=us-west1-b`):

```bash
sudo mkdir -p /opt/perihel-sync && cd /opt/perihel-sync
sudo cp ~/server.py ~/ai.py ~/tick.py ~/telegram_bot.py ~/search.py ~/digest.py .   # from server/sync-server/ in this repo
openssl req -x509 -newkey rsa:2048 -keyout key.pem -out cert.pem -days 3650 \
  -nodes -subj "/CN=perihel-sync" -addext "subjectAltName=IP:YOUR_VM_IP"
openssl rand -hex 32 | sudo tee token           # bearer token — the password
openssl rand -hex 4 | sed "s/^/perihel-/" | sudo tee ntfy-topic
# NVIDIA NIM key (free at build.nvidia.com) — the only file with a secret:
cat > nim.key   # paste key, then: chmod 600 nim.key
sudo chmod 600 key.pem token ntfy-topic
```

Install systemd units (see `server/systemd/`), then Caddy terminates real TLS:

```bash
sudo apt-get install -y caddy
# /etc/caddy/Caddyfile:  YOUR-VM-IP- dashes .sslip.io  →  reverse_proxy 127.0.0.1:8443
```

`sslip.io` is a free wildcard DNS that maps `203-0-113-10.sslip.io`-style
names to the IP, so Let's Encrypt issues a **real trusted certificate** with
zero domain setup. Firewall: open tcp:80+443, nothing else.

## 4. Budget kill-switch (auto-stop if ever charged)

`ops/budget-killer/` is a Cloud Function wired to the budget via Pub/Sub:
any threshold crossing stops the VM. Deploy per the README in that folder.
Also add a monitoring email channel to the budget for the alert email.

## 5. Clients

- **Desktop**: `%APPDATA%/perihel/settings.json` → `sync_enabled: true`,
  `server_url`, `device_token` (= VM token file).
- **Mobile**: `mobile/` — the app asks for server URL + token on first run.
- **Phone pushes**: install the free ntfy app, subscribe to the topic from
  `/opt/perihel-sync/ntfy-topic`.

## 6. Telegram bot (Perihel in the agent bus group)

`server/sync-server/telegram_bot.py` long-polls Telegram and answers each
message by posting it to the local server's `/v1/chat` (which saves the
capture and runs the AI), then replies in the originating chat only.
Reminders due also go to Telegram, alongside ntfy (`tick.py`).

On the VM:

```bash
# 1. Create the bot with @BotFather (name + @username), copy its token.
# 2. Save it the same way as the other secrets:
cat > /opt/perihel-sync/telegram.key   # paste the token
sudo chmod 600 /opt/perihel-sync/telegram.key
# 3. Enable the service (unit in server/systemd/):
sudo systemctl enable --now perihel-telegram
```

Group privacy: by default a bot only sees messages that @mention it. To let
it answer plain messages in the agent bus group, disable privacy with
BotFather `/setprivacy`, then remove the bot from the group and re-add it.

Model: the bot passes `PERIHEL_BOT_MODEL` through `/v1/chat` as an
optional per-call override; unset means the server default
(`PERIHEL_AI_MODEL`). Example for a better model, set it in the unit:

```ini
# in perihel-telegram.service [Service]:
Environment=PERIHEL_BOT_MODEL=deepseek-ai/deepseek-v4.1-flash
```

Keys: the bot's calls use `nim.bot.key` — see §8.

The bot never needs the NVIDIA key itself — inference stays in the server
process, which reads `nim.bot.key` (falling back to `nim.key`) for the
bot's calls — see §8.

## 7. Web search + fetch (/v1/search)

`server/sync-server/search.py` is a stdlib-only search/fetch worker the
Telegram bot and the daily digest consume over HTTP. It adds one endpoint:

- `POST /v1/search {"query": "..."}` → `{"results": [{title, url, snippet}]}`
- `POST /v1/search {"url": "..."}` → `{"url": ..., "text": ...}`
- Either form accepts `"summarise": true` (summarises the results / the
  fetched page with the NIM model) and `"model": "..."` (per-call model
  override, same rule as `/v1/chat`). Search also accepts
  `"max_results": 1-10` (default 5). If both `query` and `url` are given,
  the fetch wins.

Search provider: **Bing's keyless RSS endpoint**
(`bing.com/search?q=...&format=rss`) — no key, no signup, direct result
links. DuckDuckGo's keyless HTML endpoint was tried first but is unusable
from a cloud VM: it returns its bot-check ("anomaly") page instead of
results (verified Nov 2026). Brave's search page 429s and Mojeek serves a
captcha from the same VM, so Bing is the provider, with Bing's HTML
results page as a fallback if the RSS endpoint ever stops returning
items. Bing's RSS terms limit the results to personal, non-commercial
use, which is what Perihel is.

Rate limits (in-process, per server): at most one search/fetch per 5 s and
at most 10 per rolling minute. Over the limit the endpoint answers HTTP
429 with a `retry_after` (seconds) hint — it never silently drops a
request. Summarising goes through the same NIM call as the AI processor,
so it inherits that 5 s gap as well. A failed summary never fails the
request: the first ~500 chars of the text are returned instead.

Model: `PERIHEL_SEARCH_MODEL` env var overrides the summarisation model
for every search call; unset means the server default (`PERIHEL_AI_MODEL`).
Example for a better model, set it in the sync-server unit:

```ini
# in perihel-sync.service [Service]:
Environment=PERIHEL_SEARCH_MODEL=deepseek-ai/deepseek-v4.1-flash
```

Keys: the search worker's summaries use `nim.worker.key` — see §8.

## 8. Per-role NVIDIA keys

Each caller role may have its own NVIDIA key, so the bot and the search
worker no longer share one key — or one rate-limit window:

| Role | Who | Key file |
|---|---|---|
| `bot` | Telegram bot (`/v1/chat` with `"role": "bot"`) | `nim.bot.key` |
| `worker` | Search worker summaries (`/v1/search` with `summarise`) | `nim.worker.key` |
| `default` | Main capture processor — desktop, mobile, tick queue | `nim.key` |

Precedence per role: `nim.<role>.key` if it exists, else `nim.key`.
A deployment with only `nim.key` keeps working unchanged — every role
falls back to it. Save role keys the same way as `nim.key` (chmod 600).

Rate limiting is per role: each role gets its own 5 s minimum gap, so
the bot and the search worker no longer block each other. The retry-once
behaviour is unchanged.

Usage is visible in the server log — one line per NIM call:

```
NIM call: role=bot model=deepseek-ai/deepseek-v4.1-flash calls_for_role=3
```

To add a role key on the VM:

```bash
cat > /opt/perihel-sync/nim.bot.key   # paste the bot's key
sudo chmod 600 /opt/perihel-sync/nim.bot.key
sudo systemctl restart perihel-sync   # (and perihel-telegram if running)
```

The server reads keys lazily, so a missing key file never breaks
startup — it fails on the first call that needs it, with an error
naming the files it looked for.

## 9. Daily digest (morning, via Telegram)

`server/sync-server/digest.py` builds the morning digest — three
sections — and sends it to every Telegram chat the bot has seen:

1. **AI news** — the search worker (§7) for "artificial intelligence
   news", top 5 results, summarised by the worker role's model.
2. **Repo PRs/issues summary** — open PRs and open issues (most
   recently updated first, max 5 each) for the configured repos.
3. **Plans** — `plans.md` in `/opt/perihel-sync` (one plan per line;
   blank lines and `#` comments skipped, max 10 lines) if it exists;
   otherwise the top open issues across the configured repos, with a
   note that `plans.md` is missing.

Defaults (changeable): the repos are `tototofu123/agent-bus`,
`tototofu123/perihel-v2`, and `HKUST-Climbing-Society/hkust-climbing-app`;
the plans source is `plans.md`; the time is **08:00 VM-local time**.
Change the repos with the `PERIHEL_DIGEST_REPOS` env var
(comma-separated `owner/repo`, set it in the digest unit):

```ini
# in perihel-digest.service [Service]:
Environment=PERIHEL_DIGEST_REPOS=tototofu123/agent-bus,tototofu123/perihel-v2
```

GitHub access: no token is needed for public repos — unauthenticated
requests allow 60/hour, and the digest makes 2 per repo (6 for the
default list). For private repos, add a read-only personal access
token (classic PAT with `repo` scope, or a fine-grained token with
read access to the repos) as `/opt/perihel-sync/github.key`
(chmod 600, same pattern as the other secrets). Without it, a private
repo shows as "unreachable" in the digest instead of failing it. The
token is never printed or logged.

Trigger: the digest runs on the VM itself, on a systemd timer
(`server/systemd/perihel-digest.timer`, `OnCalendar=*-*-* 08:00:00`,
`Persistent=true` so a missed run fires on boot). The VM clock should
be Asia/Hong_Kong so 08:00 means 08:00 HKT:

```bash
sudo timedatectl set-timezone Asia/Hong_Kong
sudo systemctl enable --now perihel-digest.timer
```

(`ops/deploy.sh` copies the digest files and units, but it does not
enable the timer — enable it once, by hand, as above. Change the time
by editing `OnCalendar` in the timer unit, then
`sudo systemctl daemon-reload`.)

Why a VM timer and not the agent-bus scheduler (agent-bus #15): the
bus is localhost-only on the Windows machine and the VM cannot see it
— there is no Telegram bridge code in agent-bus — so a bus `task.offer`
can never reach the VM. The bus scheduler stays the mechanism for
scheduled jobs generally, but it cannot trigger this one.

Manual trigger: `POST /v1/digest {}` (bearer-authenticated, like the
other endpoints) builds and sends the digest now, returning
`{"sent", "sections", "chats"}`. The timer calls `digest.py`
directly, not the endpoint.

The digest never fabricates content: a failed search, a failed GitHub
call, or an empty section is reported as such in that section (e.g.
"AI news unavailable today (search failed)"). A long digest is split
into ≤4000-char Telegram messages so it never hits the 4096-char cap.

## 10. Health watchdog (server + AI queue, every 10 min)

`server/sync-server/watchdog.py` checks the server and the AI queue
and alerts when something is wrong — one alert per incident, never a
repeat every 10 minutes.

What it checks, and what each alert means (and does not mean):

- **Server down** — `GET /v1/health` on the local server fails
  (connection refused, timeout, non-200, or `ok` is not true). The
  alert says "the server is unreachable" — it does not claim a cause,
  because the cause was not measured. A single failed check is enough
  to alert: a down server is worth knowing immediately.
- **AI queue stuck** — the oldest capture with `ai_state='queued'` has
  waited more than **1 hour** (`STUCK_AGE_S = 3600` in `watchdog.py`).
  It means the queue is backed up (or the tick is not running — see
  issue #10), not that the AI itself is broken.
- **AI errors spiked** — the error count grew since the last check,
  or there are at least **5** captures in error state
  (`ERROR_SPIKE_THRESHOLD = 5`). It means captures are failing AI
  processing, not which one or why.

Alert channels: ntfy (the same topic as `tick.py`) **and** Telegram
(every chat the bot has seen). A down alert uses ntfy priority `high`;
stuck/spike/recovery use `default`.

Dedupe (one alert per incident): the last alerted state is stored in
the `settings` table (`watchdog_state`, `watchdog_state_since`,
`watchdog_error_count`). An alert fires only when the state changes
(`ok → down/stuck/spike`, or one bad state to another — a down server
that recovers into a still-stuck queue alerts again, because that is
a new incident). A repeated check that finds the same bad state does
NOT re-alert. When the state returns to `ok` after an alert, a
recovery notice is sent ("was down for 1h 23m"). Priority is
down > stuck > spike > ok, so a down server masks a stuck queue.

Trigger: a systemd timer on the VM itself
(`server/systemd/perihel-watchdog.timer`, `OnCalendar=*-*-* *:0/10:00`,
`Persistent=true`):

```bash
sudo systemctl enable --now perihel-watchdog.timer
```

(`ops/deploy.sh` copies the watchdog files and units, but it does not
enable the timer — enable it once, by hand, as above.) There is no
`POST /v1/watchdog` endpoint on purpose: a watchdog check triggered
through the very server being watched is useless when that server is
down. To check by hand, run the script directly on the VM:

```bash
python3 /opt/perihel-sync/watchdog.py   # prints the state; exit 0 when the check ran
sudo systemctl status perihel-watchdog  # last run + its output
```

Check the stored state by hand:

```bash
sudo sqlite3 /opt/perihel-sync/sync.db \
  "SELECT key, value FROM settings WHERE key LIKE 'watchdog_%';"
sudo sqlite3 /opt/perihel-sync/sync.db \
  "SELECT COUNT(*) FROM captures WHERE ai_state = 'queued';"
sudo sqlite3 /opt/perihel-sync/sync.db \
  "SELECT MIN(created_at) FROM captures WHERE ai_state = 'queued';"
```

## Kill switch (if anything ever looks wrong)

```bash
gcloud billing projects unlink perihel-$USER    # stops ALL billable activity
# or just: gcloud compute instances stop perihel-sync --zone=us-west1-b
```
