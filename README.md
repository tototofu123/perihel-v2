# Perihel

_A capture companion that understands your thoughts — desktop app, Android
app, and a tiny self-hosted AI server. Everything runs on free tiers; your
data lives on your own VM._

```
desktop/   Rust workspace — egui app, hold-RightCtrl dictation (sherpa-onnx),
           local/remote LLM copilot, capture resurfacing
mobile/    Kotlin + Compose Android app — AI chat capture, filterable
           logbook, alerts
server/    Python sync server + AI processor (NVIDIA NIM tool calling,
           long-term memory, reminders), Caddy TLS, systemd units
ops/       budget kill-switch Cloud Function (auto-stops the VM if the
           cloud bill ever crosses HK$0.01)
docs/      SELF-HOST.md — rebuild the whole stack from scratch, $0/month
```

## How it works

You send a thought — on the phone's chat, or `Ctrl+Shift+P` on the desktop:

1. **It is saved first.** The capture hits the server's SQLite before any AI
   call, so a dead API can never lose a thought.
2. **AI processes it** (toggleable, default on): expands what you meant,
   assigns a category (`idea · todo · link · note · question · person ·
   event`), extracts reminders when you mention a time, and files durable
   facts about you into long-term memory that improves future processing.
3. **It resurfaces at the right moment**: your explicit reminders fire at
   their time (any hour); smart nudges arrive at most every 4 hours, only
   during active hours (default 09:00–21:00), via ntfy pushes to your phone.
4. **Everything syncs** — phone, desktop, and server converge by
   last-write-wins; each device keeps a full local copy and works offline.
   Deleting is a 30-day soft delete, recoverable from the server DB.

## Stack

- **Desktop** — Rust, egui, sherpa-onnx (on-device STT), llama.cpp sidecar
  for the offline copilot, NVIDIA NIM (free API) for the cloud copilot.
- **Server** — Python stdlib only (no pip on the VM): HTTPS via Caddy +
  Let's Encrypt, bearer-token auth, SQLite. AI via NVIDIA NIM's
  OpenAI-compatible API with tool calls; a rate limiter and retry-once
  policy protect the free tier.
- **Mobile** — Kotlin + Jetpack Compose, zero networking dependencies.
- **Hosting** — GCP always-free e2-micro, free static IP, sslip.io domain,
  budget alert at HK$0.01 wired to a Cloud Function that stops the VM.

## Quick start

**Desktop** — see `desktop/README.md`.

**Mobile** — needs the Android SDK:

```bash
cd mobile
./gradlew assembleDebug -PPERIHEL_URL=https://your-vm -PPERIHEL_TOKEN=yourtoken
# APK at app/build/outputs/apk/debug/ — or just launch it and enter URL+token
```

**Server** — `docs/SELF-HOST.md` walks through the whole free-tier build.

## Principles

- **Free forever** — every component sits on an always-free tier, guarded by
  a HK$0.01 budget alert + auto-kill.
- **Save first, think second** — no AI failure can lose a capture.
- **Local-first** — sync is additive; offline Perihel is complete Perihel.

MIT
