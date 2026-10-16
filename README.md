# Perihel

_A local-first desktop companion that captures fleeting intentions, resurfaces them at smart times, and turns your voice into text in whatever app you're already in._

Everything runs on-device. No cloud, no account, no telemetry.

## What's in here

Perihel is one Cargo workspace with three crates:

| Crate | What it is |
|-------|------------|
| **`perihel`** | The desktop app — quick capture, smart reminders, an on-device LLM copilot, TTS read-aloud |
| **`perihel-transcribe`** | Hold-to-dictate sidecar — on-device STT injected straight into the focused window |
| **`perihel-core`** | Shared foundations: SQLite store, dictionary, text formatting, paths, settings, resource guard |

## Design principles

**Nothing heavy stays resident.** Both model paths run as *sidecar processes* that
load weights, do one job, and exit. `sherpa-onnx-offline.exe` spawns per
dictation and is gone a second later. `llama-server` spawns on first copilot
use and is stopped after a configurable idle window. Idle memory is the binary
itself plus egui — not the model.

**Perihel never competes with your real work.** The process drops to
`BELOW_NORMAL` priority, and a `ResourceGuard` gates every heavy step behind
live checks of free RAM and CPU load. On a busy machine the work is skipped or
deferred, never queued up to thrash the machine.

**Local-first.** Captures live in SQLite under `%LOCALAPPDATA%/perihel`. Config
is plain JSON under `%APPDATA%/perihel`, editable outside the UI.

## Features

### Capture & reminders — `Ctrl+Shift+P`

- Quick capture window anywhere in the OS
- Attach a URL (fetched and analyzed) or a screenshot
- Captures are randomly scheduled to resurface 1–4 hours later as native Windows notifications
- Quiet hours suppress notifications on a schedule you set
- Dashboard with All / Pending / Done / Dismissed filters; click to expand
- TTS read-aloud via the `vox` CLI
- Export everything as JSON

### Dictation — hold `RightCtrl`

Press and hold Right Ctrl, talk, release. The text lands in whatever window had
focus. No window switch, no upload.

- Three engines, all via the `sherpa-onnx-offline` sidecar:

  | Engine | Size | Notes |
  |--------|------|-------|
  | `sensevoice` | ~226 MB | Recommended. zh / en / ja / ko / yue, auto-detects language |
  | `whisper-tiny` | ~75 MB | Lightest. English only |
  | `parakeet` | ~600 MB | Highest quality. English only |

- Rule-based formatter plus a user dictionary corrects casing, punctuation and
  your own jargon before injection
- Live HUD showing recording state, a tray icon, and a configurable hotkey

### Copilot — `Ctrl+Shift+K`

Local LLM tool calling through a `llama-server` sidecar (llama.cpp), constrained
by a GBNF grammar in `grammar/perihel-tools.gbnf` so it emits valid tool calls
rather than prose. Put a `.gguf` in `models/llm/` and set `llm_model` in
`settings.json`.

### Agent piping

Perihel detects installed AI coding agents on `PATH` and in their config dirs —
Codex, Claude Code, OpenCode, Gemini CLI, Cursor Agent, Copilot CLI, Aider,
Antigravity, `vox` — and can pipe captures through one for summarization in the
background.

## Install

### Prerequisites

- Windows 10/11 (built and tested there; most crates are `cfg(windows)`-aware)
- Rust toolchain. GNU/MinGW is recommended — some transitive deps need it
- [`llama.cpp`](https://github.com/ggml-org/llama.cpp) on `PATH` (only for the copilot)
- [`vox`](https://github.com/…/vox) on `PATH` (only for TTS)

### Build

```bash
git clone https://github.com/tototofu123/perihel-v2.git
cd perihel-v2

# Weights and the sherpa-onnx runtime — gitignored, fetched once.
./scripts/download-models.ps1                    # sensevoice (recommended)
./scripts/download-models.ps1 -Engine whisper-tiny
./scripts/download-models.ps1 -Engine all

cargo build --release -p perihel-transcribe      # dictation
cargo build --release -p perihel                 # main app
```

`build.ps1` / `run.bat` wrap `cargo` with MinGW on `PATH`. Because the workspace
holds several binaries, name the package explicitly:

```powershell
./build.ps1 test
./build.ps1 run --release -p perihel-transcribe
```

Run `run.bat` for the main app, or set `PERIHEL_APP=perihel-transcribe` first
to run the dictation sidecar instead.

### Where models go

`perihel_core::paths::models_dir()` resolves in this order:

1. `$PERIHEL_MODELS`, if set
2. the repo's `models/` directory, when running from a source checkout
3. `%APPDATA%/perihel/models`

`models/stt/<engine>/` holds the weights, `models/bin/` holds the sherpa-onnx
runtime, `models/llm/` holds your `.gguf`. Only `models/stt/*/tokens.txt` is
committed — everything else is downloaded.

## Configuration

`%APPDATA%/perihel/settings.json`:

```json
{
  "stt_engine": "sensevoice",
  "dictation_hotkey": "RightCtrl",
  "llm_model": "perihel-tools-q4_k_m.gguf",
  "llm_context_window": 4096,
  "tts_enabled": true,
  "tts_voice": "",
  "low_power_mode": true,
  "pause_below_free_ram_mb": 1500,
  "quiet_start": "23:00",
  "quiet_end": "08:00"
}
```

Custom corrections live in `%APPDATA%/perihel/dictionary.json`; shared test
vectors are in `shared/dictionary-test-vectors.json`.

## Layout

```
crates/
  perihel-core/       db, dictionary, formatting, paths, resource guard, settings
  perihel/            app: agents, capture UI, copilot, reminders, TTS, vision
  perihel-transcribe/ dictation: audio, hotkey, injector, STT sidecar, HUD, tray
grammar/              GBNF grammar constraining copilot tool calls
scripts/              model download
shared/               cross-crate test vectors
```

## Testing

```powershell
./build.ps1 test
```

## License

MIT