# Architecture — low-memory design

Inspired by `per-simmons/murmur-youtube` but Windows-first, Rust-native, no cloud.

## Flow

```
hold Right Ctrl ─► HotkeyMonitor (global-hotkey crate, WH_KEYBOARD_LL)
                     │
                     ▼
              DictationController (state machine: idle → starting → listening → finishing → idle)
                     │         │         │
                     ▼         ▼         ▼
               AudioCapture  HUDOverlay  TranscriptionEngine
                 cpal WASAPI   egui/tao   sherpa-onnx (Parakeet int8)
                     │                      │
                     └── AudioChunk (ordered AsyncChannel) ──► │
                                                               ▼
                                                         RuleBasedFormatter
                                                         + Dictionary correction
                                                               │
                                                               ▼
                                                         TextInjector (enigo SendInput → focused HWND)
```

Load-bearing details (from murmur):
1. HUD must be `WS_EX_NOACTIVATE | WS_EX_TOPMOST` + `canBecomeKey = false` — never steal focus.
2. Hotkey observed, never swallowed — prevents stuck Ctrl.
3. AudioChunks copied, never borrowed — cpal buffer recycled instantly.
4. Single consumer for ordering — `tokio::mpsc` ordered channel, no `spawn per buffer`.

## Memory budget (measured target, release build)

```
Base Rust binary (stripped, LTO) ............ 8 MB disk, 18 MB RSS
cpal + ringbuf (2s * 16kHz * 2 bytes) .......  0.06 MB
HUD (egui) ..................................  5 MB
sherpa-onnx runtime ......................... 12 MB
Parakeet int8 weights (resident) ............ 66 MB  (vs Whisper large-v3 1.5GB)
Dictionary (1000 entries) ................... <1 MB
---------------------------------------------------
Total RSS idle (no audio) ................... ~85 MB
Total RSS listening ......................... ~105 MB
Peak (transcribing 30s) ..................... ~120 MB
```

Compare: Wispr Flow (Electron + cloud) ~250 MB, murmur macOS + FluidAudio ~180 MB, Python faster-whisper + large-v3 ~1.2 GB.

Why not LLM cleanup? On-device LLM (Phi-3, Gemma) = +2-4 GB RAM. For lowest memory we use deterministic `RuleBasedFormatter` + dictionary correction only. LLM tier can be optional feature flag later.

## Modules

### 1. HotkeyMonitor — `src/core/hotkey.rs`
- Crate: `global-hotkey 0.6` (wraps RegisterHotKey + WH_KEYBOARD_LL for Right Ctrl discrimination)
- Default: `Right Ctrl` (AltGr-safe). Configurable: `Right Alt`, `F8`, `Fn` via `settings.json`
- Emits `HotkeyEvent::Pressed/Released` — DictationController owns debounce (150ms)

### 2. AudioCapture — `src/core/audio.rs`
- Crate: `cpal 0.15` + `ringbuf 0.3`
- WASAPI shared mode, 48kHz native → 16kHz mono via `rubato` sinc resampler
- RMS per 10ms for HUD VU meter
- `AudioChunk { samples: Vec<i16>, seq: u64 }` — copied, Send, ordered

### 3. TranscriptionEngine — `src/transcription/mod.rs`
- Trait: `TranscriptionEngine { fn transcribe(&mut self, chunks: Receiver<AudioChunk>) -> String }`
- Impl: `SherpaParakeetEngine` — wraps `sherpa-rs` `OfflineRecognizer` (streaming via chunked offline for lowest RAM vs full streaming)
- Biasing: pass dictionary words via `RuleFsts` / hotwords (max 20, else drift). See `docs/PARAKEET-WINDOWS.md` in murmur.
- Fallback: `WhisperTinyEngine` (39 MB) if user wants even lower disk.

### 4. TextFormatter — `src/formatting/mod.rs`
- Trait: `TextFormatter { fn format(&self, raw: &str) -> FormattedText }`
- `RuleBasedFormatter`: strip fillers (`um, uh, like`), fix spacing, caps sentences, terminal punct.
- `DictionaryCorrector`: whole-word, case-insensitive, longest-first, optional `[\s-]*` between parts. Handles `CloudCode` vs `Claude Code`. Contract: `shared/dictionary-test-vectors.json` (19 vectors from murmur).

### 5. TextInjector — `src/core/injector.rs`
- Crate: `enigo 0.2` (`SendInput` w/ `KEYEVENTF_UNICODE`)
- Steps: get foreground `HWND` via `GetForegroundWindow` *before* HUD shows → `SendInput` char-by-char (or `Ctrl+V` fallback for >500 chars via clipboard)
- Never use `SetForegroundWindow` — would steal focus.

### 6. HUDOverlay — `src/ui/hud.rs`
- Crate: `egui 0.28` + `eframe` or `tao 0.16` + `wry` (chosen: egui for no WebView RAM)
- Window: `decorations:false, transparent:true, always_on_top:true, skip_taskbar:true`, 340x76 bottom-center, `WS_EX_NOACTIVATE`
- Shows: recording dot (red #E53E3E), RMS waveform, live transcript (if streaming), `● REC 00:03`

### 7. Dictionary + Settings — `src/support/`
- File: `%APPDATA%/perihel-transcribe/dictionary.json` — plain JSON, editable outside UI
- Schema: `{ "words": ["Anthropic"], "corrections": [{ "hear": "cloud code", "write": "Claude Code" }] }`
- Settings: `settings.json` — hotkey, model path, auto-start, HUD position

## Disk layout

```
perihel-transcribe/
├─ Cargo.toml
├─ src/
│  ├─ main.rs                 tray + controller wiring
│  ├─ core/ hotkey.rs audio.rs injector.rs controller.rs
│  ├─ transcription/ mod.rs sherpa.rs
│  ├─ formatting/ mod.rs dictionary.rs
│  ├─ ui/ hud.rs tray.rs settings_window.rs
│  └─ support/ settings.rs log.rs paths.rs
├─ shared/dictionary-test-vectors.json
├─ resources/ (empty, models downloaded at runtime)
└─ docs/
```

## What we reuse from murmur without cloning

- Behavioural contract: `shared/dictionary-test-vectors.json` — same regex semantics (`RegexOptions.CultureInvariant`, NFC normalize, no `ß→ss` fold)
- State machine shape, AudioChunk ordering, HUD non-activate invariant
- Not reused: Swift code, `Package.swift`, `FluidAudio`, `SpeechAnalyzer`, Apple TCC logic

## Next steps

1. `cargo init` + deps (cpal, sherpa-rs, global-hotkey, enigo, egui)
2. Implement `DictationController` state machine + ordered channel
3. Wire `SherpaParakeetEngine` with int8 model download
4. Build HUD + tray
5. Measure RSS with `GetProcessMemoryInfo` — assert <120 MB
