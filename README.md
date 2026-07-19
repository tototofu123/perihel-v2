# Perihel

_A local-first desktop companion that captures fleeting intentions and resurfaces them at smart times._

Built for **OpenAI Build Week** (July 2026).

## Features

- **Quick capture** — Press `Ctrl+Shift+P` anywhere to capture a thought, URL, or screenshot
- **Voice input** — Dictate thoughts via Windows Speech Recognition
- **Screenshot capture** — One-click screenshot from the capture popup
- **Smart resurfacing** — Pending captures are randomly scheduled over the next 1-4 hours and shown as native Windows notifications
- **AI agent piping** — Route captures through detected AI coding agents (Codex, Claude, OpenCode, Gemini CLI, etc.) for automatic summarization
- **Dashboard** — Full history with All / Pending / Done / Dismissed filters
- **System tray** — Lives in the system tray; capture window auto-hides on blur
- **Quiet hours** — Configurable quiet hours suppress notifications
- **Export** — Export all captures as JSON
- **100% local-first** — All data stored in SQLite, no cloud dependency

## Installation

### Prerequisites

- Windows (built and tested on Windows 10/11)
- Rust toolchain (MSVC or GNU target)

### Build from source

```
git clone https://github.com/tototofu123/perihel.git
cd perihel
cargo run --release
```

The first build will download and compile all dependencies (approx. 2-5 minutes).

### Quick start

1. Launch Perihel — it starts minimized to the system tray
2. Press `Ctrl+Shift+P` to open the capture window
3. Type a thought, paste a URL, or click the screenshot/voice buttons
4. Optionally select an AI agent to auto-summarize your capture
5. Click **Capture** — Perihel will randomly schedule resurfacing
6. When it resurfaces, you'll see a native Windows notification; click **Done** or **Dismiss**

## Usage

### Capture window

| Button | Action |
|--------|--------|
| 📷 Screenshot | Captures the primary monitor to `perihel/screenshots/` |
| 🎤 Voice | Records via Windows Speech Recognition |
| Capture | Saves the thought/URL/screenshot and triggers agent piping |

### Tray menu

| Item | Action |
|------|--------|
| New Capture | Opens the capture popup |
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

When an agent is selected in the capture popup, the capture content is piped through the agent with a summarization prompt, and the result is saved as an AI summary.

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
  app.rs       — GUI views (capture popup, dashboard, settings)
  db.rs        — SQLite schema and CRUD operations
  agents.rs    — AI coding agent detection
  scheduler.rs — Background scheduling and notifications
```

## License

MIT
