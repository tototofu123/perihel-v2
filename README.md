# perihel-transcribe

Part of the **Perihel** ecosystem.

Low-memory, fully **on-device** Wispr Flow clone. Hold Right Ctrl, talk, release — cleaned text is injected into the focused app. No cloud, no subscription.

**Target:** Windows 10/11 (x64) — primary. macOS/Linux later.

**Memory goal:** **<120 MB resident** (15-30 MB base + ~66 MB Parakeet int8 model). vs Wispr Flow cloud/Electron ~250 MB+ and Python Whisper ~800 MB.

## Stack — why Rust + sherpa-onnx

| Layer | Choice | RAM impact | Reason |
|-------|--------|------------|--------|
| Lang | **Rust 1.97** | 15-30 MB idle | No GC, no Electron, AOT binary ~8 MB |
| Audio | `cpal` + `ringbuf` | ~2 MB | WASAPI directly, 16kHz mono i16, ordered `AudioChunk` |
| Hotkey | `global-hotkey` + `rdev` | <1 MB | Low-level hook, Right Ctrl default (not AltGr) |
| STT | `sherpa-onnx` + **Parakeet TDT 0.6B int8** | **66 MB** | CPU-only, ONNX Runtime, 40x realtime, quantized |
| Injection | `enigo` (SendInput) | <1 MB | + clipboard fallback |
| UI | `egui` / `tao` tray + HUD | ~5 MB | Non-activating `WS_EX_NOACTIVATE` overlay, no WebView |
| Formatter | RuleBased + Dictionary | <1 MB | Deterministic, no LLM (LLM would +2-4 GB) |

## Quick start

```powershell
cd C:\Users\totot\.gemini\antigravity\antigravity_projects\perihel-transcribe
cargo run --release
# first run downloads parakeet-tdt-0.6b-int8 ~600MB to %APPDATA%/perihel-transcribe/models/
# hold Right Ctrl, talk, release
```

## Model download

On first launch, downloads via `hf-hub`:
- `istupakov/parakeet-tdt-0.6b-v2-int8-onnx` — encoder + decoder + joiner + tokens (~600MB disk, 66MB RAM)
- No cloud path. `Cloud Sync = off` by default.

See `ARCHITECTURE.md` for flow, memory budget, and murmur contract reuse.

## Perihel ecosystem

`perihel-transcribe` is the dictation layer for Perihel — designed to be embedded as a sidecar for other Perihel apps. Shared dictionary contract in `shared/dictionary-test-vectors.json`.
