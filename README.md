# Perihel

_A local-first capture companion — desktop app, Android companion, and a tiny
self-hosted sync server. Everything runs on free tiers; nothing phones home
except to your own VM._

```
desktop/   Rust workspace — the main app (egui), hold-RightCtrl dictation
           (sherpa-onnx), local/remote LLM copilot, smart resurfacing
mobile/    Kotlin + Jetpack Compose Android companion (sync API client)
server/    Python stdlib sync server + ntfy push, Caddy TLS, systemd units,
           PRIVATE deployment values in server/local.env (gitignored)
ops/       budget kill-switch Cloud Function (stops the VM if GCP ever bills)
docs/      SELF-HOST.md — rebuild the whole VM stack from scratch, $0/month
```

## What it does

- **Capture** (`Ctrl+Shift+P` on desktop, text field on mobile): dump a
  thought/URL/screenshot; it resurfaces 1–4 h later as a notification — on the
  desktop, and on your phone via ntfy even when the PC is off.
- **Dictate** (hold Right Ctrl): on-device STT injected into whatever app has
  focus. No cloud.
- **Copilot** (`Ctrl+Shift+K`): tool-calling LLM — remote free API (NVIDIA NIM)
  or local `.gguf` sidecar, GBNF-constrained locally.
- **Sync**: additive, offline-first. Local SQLite is the source of truth on
  every device; the VM relay merges by last-write-wins.

## Quick start

**Desktop** — see `desktop/README.md` (build with `build.ps1`, models via
`desktop/scripts/download-models.ps1`).

**Mobile** — needs the Android SDK (free):

```bash
cd mobile
./gradlew assembleDebug -PPERIHEL_URL=https://your-vm.sslip.io -PPERIHEL_TOKEN=yourtoken
# APK at app/build/outputs/apk/debug/ — sideload it
```

(Or launch the app and enter URL + token on the setup screen.)

**Server** — `docs/SELF-HOST.md` walks the whole thing: GCP always-free VM,
real TLS via sslip.io + Caddy, budget kill-switch, phone pushes. The private
values for the live deployment live in `server/local.env` (never committed).

## Principles

- **Free forever** — every component sits on an always-free tier; a HK$0.01
  budget alert + auto-kill Cloud Function guards the meter.
- **Local-first** — sync is additive; offline Perihel is complete Perihel.
- **Nothing heavy stays resident** — sidecars load, do one job, exit.

MIT
