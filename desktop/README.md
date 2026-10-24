# Perihel desktop

The Rust workspace: `perihel` (app), `perihel-transcribe` (dictation),
`perihel-core` (db, sync, settings, dictionary, resource guard).

## Build

Windows 10/11, Rust with GNU/MinGW toolchain. `build.ps1` puts MinGW on
PATH and forwards to cargo; the workspace has several binaries, so name the
package:

```powershell
./build.ps1 test                          # whole workspace
./build.ps1 build --release -p perihel    # main app
./build.ps1 build --release -p perihel-transcribe
./run.bat                                 # run the main app
```

Dictation weights are fetched once (gitignored):

```powershell
./scripts/download-models.ps1             # sensevoice (~226 MB, recommended)
```

## Features

- **Capture** `Ctrl+Shift+P` — quick window for a thought/URL/screenshot;
  pending captures resurface as notifications 1–4 h later (quiet hours
  respected).
- **Dictation** — hold Right Ctrl, talk, release; text lands in the focused
  app via the sherpa-onnx sidecar (nothing resident, nothing uploaded).
- **Copilot** `Ctrl+Shift+K` — tool-calling LLM. `llm_backend` picks
  `"remote"` (NVIDIA NIM, needs `NVIDIA_NIM_API_KEY`) or `"local"`
  (a `.gguf` via llama-server; models/llm stays empty otherwise).
- **Sync** — set `sync_enabled`, `server_url`, `device_token` in
  `%APPDATA%/perihel/settings.json` and captures reconcile with the server
  every 10 minutes; local SQLite stays the source of truth.

## Layout

```
crates/perihel-core/     db + sync client, paths, settings, dictionary
crates/perihel/          app: capture UI, copilot, reminders, scheduler, TTS
crates/perihel-transcribe/ dictation: audio, hotkey, injector, HUD, tray
grammar/                 GBNF grammar for local tool calling
shared/                  cross-crate test vectors
```
