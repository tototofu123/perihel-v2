# Perihel

_A local-first desktop companion that captures fleeting intentions and resurfaces them at smart times._

Built for **OpenAI Build Week** (July 2026).

## Features

- **Quick capture** — Press `Ctrl+Shift+P` anywhere to open the capture window
- **Voice input** — Dictate thoughts via Windows Speech Recognition (zh-TW/zh-CN/en depending on system)
- **Screenshot capture** — One-click screenshot attached to any capture
- **URL attachment** — Paste a URL inline to be fetched and analyzed
- **Smart resurfacing** — Captures are randomly scheduled 1-4 hours later and shown as native Windows notifications
- **AI agent piping** — Route captures through detected AI coding agents for automatic summarization
- **Dashboard** — Full history with All / Pending / Done / Dismissed filters; click any capture to expand full text
- **System tray** — Lives in the system tray; capture window auto-hides on blur after a short delay
- **Quiet hours** — Configurable quiet hours suppress notifications
- **Export** — Export all captures as JSON
- **100% local-first** — All data stored in SQLite, no cloud dependency
- **TTS read-aloud** — Uses `vox` CLI for speaking text

## Installation

### Prerequisites

- Windows (built and tested on Windows 10/11)
- Rust toolchain (GNU target recommended for MinGW)

### Build from source

```
git clone https://github.com/tototofu123/perihel.git
cd perihel
cargo build --release
```

The first build will download and compile all dependencies (approx. 2-5 minutes).

To run, double-click `target/release/perihel.exe` directly (no console window).

### Quick start

1. Launch Perihel — it starts minimized to the system tray
2. Press `Ctrl+Shift+P` to open the capture window
3. Type a thought, optionally attach a URL or screenshot from Settings
4. Optionally select an AI agent from Settings to auto-summarize
5. Click **Capture** — Perihel will randomly schedule resurfacing
6. When it resurfaces, you'll see a native Windows notification; click **Done** or **Dismiss** in the Dashboard

## Usage

### Capture tab

| Control | Action |
|---------|--------|
| Text field | Type your thought |
| URL field | Optional URL to attach and analyze |
| Voice button | Records via Windows Speech Recognition (15s max) |
| Capture button | Saves and triggers background agent processing |
| [CAM] indicator | Shows when a screenshot is attached from Settings |

### Dashboard tab

| Control | Action |
|---------|--------|
| Filter tabs | All / Pending / Done / Dismissed |
| Click a capture | Toggle expanded view to see full content |
| OK button | Mark pending capture as done |
| X button | Dismiss pending capture |

### Settings tab

| Control | Action |
|---------|--------|
| AI Agent selector | Choose which detected agent processes captures |
| Take Screenshot | Captures the primary monitor to `perihel/screenshots/` |
| Quiet Hours | Set start/end times to suppress notifications |
| Export as JSON | Writes all captures to `perihel_export.json` |

### Tray menu

| Item | Action |
|------|--------|
| New Capture | Opens the capture window |
| Dashboard | Opens the full history dashboard |
| Settings | View detected agents, configure quiet hours, export data |
| Exit | Quits Perihel entirely |

### Hotkeys

| Key | Action |
|-----|--------|
| `Ctrl+Shift+P` | Show capture window (global) |

### Agent detection

Perihel automatically detects installed AI coding agents by scanning PATH and config directories:

- OpenAI Codex (`codex`)
- Claude Code (`claude`)
- OpenCode (`opencode`)
- Gemini CLI (`gemini`)
- Cursor Agent (`cursor-agent`)
- GitHub Copilot CLI (`copilot`)
- Aider (`aider`)
- Antigravity (`antigravity`)
- Vox TTS (`vox`)

When an agent is selected in Settings, captures are piped through the agent with a summarization prompt in the background.

## Tech stack

- **Rust** — Performance and safety
- **egui/eframe** — Immediate-mode GUI
- **tray-icon** — System tray integration
- **global-hotkey** — Global keyboard shortcuts
- **rusqlite** — Local SQLite database
- **notify-rust** — Native Windows notifications
- **xcap** — Cross-platform screen capture
- **which** — Agent binary detection
- **serde** — Serialization for export

## Project structure

```
src/
  main.rs      — Entry point, tray setup, hotkey registration
  app.rs       — GUI views (capture, dashboard, settings)
  db.rs        — SQLite schema and CRUD operations
  agents.rs    — AI coding agent detection
  scheduler.rs — Background scheduling and notifications
```

## License

MIT
