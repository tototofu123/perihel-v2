//! perihel-transcribe 2.0 — hold Right Ctrl, talk, release; text lands in the focused app.
//!
//! Flow: HotkeyMonitor → Recorder (cpal, 16 kHz mono i16) → wav → sherpa-onnx-offline
//! sidecar (SenseVoice / whisper / Parakeet) → rule-based formatter + dictionary → enigo
//! injection. The STT engine is an external process, so nothing model-sized stays
//! resident between dictations — idle RSS is the binary itself plus egui.

mod audio;
mod controller;
mod hotkey;
mod injector;
mod stt;
mod ui;

use anyhow::Result;
use perihel_core::Settings;

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    log::info!("perihel-transcribe 2.0 — hold Right Ctrl to dictate");

    let settings = Settings::load_or_default();

    // Perihel never competes with foreground work.
    perihel_core::set_priority_idle();

    // Fail fast with a useful message if models are missing.
    if let Err(missing) = stt::check_models(&settings.stt_engine) {
        log::error!("{missing}");
        eprintln!("{missing}\nRun scripts/download-models.ps1 to fetch models, or pick another engine in settings.json.");
        // keep running so the tray/hud explains itself, but dictation will no-op
    }

    controller::run(settings)
}
